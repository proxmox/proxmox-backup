use std::fs::File;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::RwLock;
use std::io::Write;
use std::io::{BufRead, BufReader};
use std::os::unix::io::AsRawFd;

use anyhow::{format_err, bail, Error};
use nix::fcntl::OFlag;

use proxmox::tools::fs::{atomic_open_or_create_file, create_path, CreateOptions};

use crate::rrd::{DST, CF, RRD, RRA};

const RRD_JOURNAL_NAME: &str = "rrd.journal";

/// RRD cache - keep RRD data in RAM, but write updates to disk
///
/// This cache is designed to run as single instance (no concurrent
/// access from other processes).
pub struct RRDCache {
    apply_interval: f64,
    basedir: PathBuf,
    file_options: CreateOptions,
    dir_options: CreateOptions,
    state: RwLock<RRDCacheState>,
}

// shared state behind RwLock
struct RRDCacheState {
    rrd_map: HashMap<String, RRD>,
    journal: File,
    last_journal_flush: f64,
}

struct JournalEntry {
    time: f64,
    value: f64,
    dst: DST,
    rel_path: String,
}

impl RRDCache {

    /// Creates a new instance
    pub fn new<P: AsRef<Path>>(
        basedir: P,
        file_options: Option<CreateOptions>,
        dir_options: Option<CreateOptions>,
        apply_interval: f64,
    ) -> Result<Self, Error> {
        let basedir = basedir.as_ref().to_owned();

        let file_options = file_options.unwrap_or_else(|| CreateOptions::new());
        let dir_options = dir_options.unwrap_or_else(|| CreateOptions::new());

        create_path(&basedir, Some(dir_options.clone()), Some(dir_options.clone()))
            .map_err(|err: Error| format_err!("unable to create rrdb stat dir - {}", err))?;

        let mut journal_path = basedir.clone();
        journal_path.push(RRD_JOURNAL_NAME);

        let flags = OFlag::O_CLOEXEC|OFlag::O_WRONLY|OFlag::O_APPEND;
        let journal = atomic_open_or_create_file(&journal_path, flags,  &[], file_options.clone())?;

        let state = RRDCacheState {
            journal,
            rrd_map: HashMap::new(),
            last_journal_flush: 0.0,
        };

        Ok(Self {
            basedir,
            file_options,
            dir_options,
            apply_interval,
            state: RwLock::new(state),
        })
    }

    fn create_default_rrd(dst: DST) -> RRD {

        let mut rra_list = Vec::new();

        // 1min * 1440 => 1day
        rra_list.push(RRA::new(CF::Average, 60, 1440));
        rra_list.push(RRA::new(CF::Maximum, 60, 1440));

        // 30min * 1440 => 30days = 1month
        rra_list.push(RRA::new(CF::Average, 30*60, 1440));
        rra_list.push(RRA::new(CF::Maximum, 30*60, 1440));

        // 6h * 1440 => 360days = 1year
        rra_list.push(RRA::new(CF::Average, 6*3600, 1440));
        rra_list.push(RRA::new(CF::Maximum, 6*3600, 1440));

        // 1week * 570 => 10years
        rra_list.push(RRA::new(CF::Average, 7*86400, 570));
        rra_list.push(RRA::new(CF::Maximum, 7*86400, 570));

        RRD::new(dst, rra_list)
    }

    fn parse_journal_line(line: &str) -> Result<JournalEntry, Error> {

        let line = line.trim();

        let parts: Vec<&str> = line.splitn(4, ':').collect();
        if parts.len() != 4 {
            bail!("wrong numper of components");
        }

        let time: f64 = parts[0].parse()
            .map_err(|_| format_err!("unable to parse time"))?;
        let value: f64 = parts[1].parse()
            .map_err(|_| format_err!("unable to parse value"))?;
        let dst: u8 = parts[2].parse()
            .map_err(|_| format_err!("unable to parse data source type"))?;

        let dst = match dst {
            0 => DST::Gauge,
            1 => DST::Derive,
            _ => bail!("got strange value for data source type '{}'", dst),
        };

        let rel_path = parts[3].to_string();

        Ok(JournalEntry { time, value, dst, rel_path })
    }

    fn append_journal_entry(
        state: &mut RRDCacheState,
        time: f64,
        value: f64,
        dst: DST,
        rel_path: &str,
    ) -> Result<(), Error> {
        let journal_entry = format!("{}:{}:{}:{}\n", time, value, dst as u8, rel_path);
        state.journal.write_all(journal_entry.as_bytes())?;
        Ok(())
    }

    pub fn apply_journal(&self) -> Result<(), Error> {
        let mut state = self.state.write().unwrap(); // block writers
        self.apply_journal_locked(&mut state)
    }

    fn apply_journal_locked(&self, state: &mut RRDCacheState) -> Result<(), Error> {

        log::info!("applying rrd journal");

        state.last_journal_flush = proxmox_time::epoch_f64();

        let mut journal_path = self.basedir.clone();
        journal_path.push(RRD_JOURNAL_NAME);

        let flags = OFlag::O_CLOEXEC|OFlag::O_RDONLY;
        let journal = atomic_open_or_create_file(&journal_path, flags,  &[], self.file_options.clone())?;
        let mut journal = BufReader::new(journal);

        let mut last_update_map = HashMap::new();

        let mut get_last_update = |rel_path: &str, rrd: &RRD| {
            if let Some(time) = last_update_map.get(rel_path) {
                return *time;
            }
            let last_update =  rrd.last_update();
            last_update_map.insert(rel_path.to_string(), last_update);
            last_update
        };

        let mut linenr = 0;
        loop {
            linenr += 1;
            let mut line = String::new();
            let len = journal.read_line(&mut line)?;
            if len == 0 { break; }

            let entry = match Self::parse_journal_line(&line) {
                Ok(entry) => entry,
                Err(err) => {
                    log::warn!("unable to parse rrd journal line {} (skip) - {}", linenr, err);
                    continue; // skip unparsable lines
                }
            };

            if let Some(rrd) = state.rrd_map.get_mut(&entry.rel_path) {
                if entry.time > get_last_update(&entry.rel_path, &rrd) {
                    rrd.update(entry.time, entry.value);
                }
            } else {
                let mut path = self.basedir.clone();
                path.push(&entry.rel_path);
                create_path(path.parent().unwrap(), Some(self.dir_options.clone()), Some(self.dir_options.clone()))?;

                let mut rrd = Self::load_rrd(&path, entry.dst);

                if entry.time > get_last_update(&entry.rel_path, &rrd) {
                    rrd.update(entry.time, entry.value);
                }
                state.rrd_map.insert(entry.rel_path.clone(), rrd);
            }
        }

        // save all RRDs

        let mut errors = 0;
        for (rel_path, rrd) in state.rrd_map.iter() {
            let mut path = self.basedir.clone();
            path.push(&rel_path);
            if let Err(err) = rrd.save(&path, self.file_options.clone()) {
                errors += 1;
                log::error!("unable to save {:?}: {}", path, err);
            }
        }

        // if everything went ok, commit the journal

        if errors == 0 {
            nix::unistd::ftruncate(state.journal.as_raw_fd(), 0)
                .map_err(|err| format_err!("unable to truncate journal - {}", err))?;
            log::info!("rrd journal successfully committed");
        } else {
            log::error!("errors during rrd flush - unable to commit rrd journal");
        }

        Ok(())
    }

    fn load_rrd(path: &Path, dst: DST) -> RRD {
        match RRD::load(path) {
            Ok(rrd) => rrd,
            Err(err) => {
                if err.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("overwriting RRD file {:?}, because of load error: {}", path, err);
                }
                Self::create_default_rrd(dst)
            },
        }
    }

    /// Update data in RAM and write file back to disk (journal)
    pub fn update_value(
        &self,
        rel_path: &str,
        time: f64,
        value: f64,
        dst: DST,
    ) -> Result<(), Error> {

        let mut state = self.state.write().unwrap(); // block other writers

        if (time - state.last_journal_flush) > self.apply_interval {
            if let Err(err) = self.apply_journal_locked(&mut state) {
                log::error!("apply journal failed: {}", err);
            }
        }

        Self::append_journal_entry(&mut state, time, value, dst, rel_path)?;

        if let Some(rrd) = state.rrd_map.get_mut(rel_path) {
            rrd.update(time, value);
        } else {
            let mut path = self.basedir.clone();
            path.push(rel_path);
            create_path(path.parent().unwrap(), Some(self.dir_options.clone()), Some(self.dir_options.clone()))?;

            let mut rrd = Self::load_rrd(&path, dst);

            rrd.update(time, value);
            state.rrd_map.insert(rel_path.into(), rrd);
        }

        Ok(())
    }

    /// Extract data from cached RRD
    ///
    /// `start`: Start time. If not sepecified, we simply extract 10 data points.
    /// `end`: End time. Default is to use the current time.
    pub fn extract_cached_data(
        &self,
        base: &str,
        name: &str,
        cf: CF,
        resolution: u64,
        start: Option<u64>,
        end: Option<u64>,
    ) -> Result<Option<(u64, u64, Vec<Option<f64>>)>, Error> {

        let state = self.state.read().unwrap();

        match state.rrd_map.get(&format!("{}/{}", base, name)) {
            Some(rrd) => Ok(Some(rrd.extract_data(cf, resolution, start, end)?)),
            None => Ok(None),
        }
    }
}
