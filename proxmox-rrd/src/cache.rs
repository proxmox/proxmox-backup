use std::fs::File;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::RwLock;
use std::io::Write;
use std::io::{BufRead, BufReader};
use std::os::unix::io::AsRawFd;
use std::time::SystemTime;

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
    load_rrd_cb: fn(cache: &RRDCache, path: &Path, rel_path: &str, dst: DST) -> RRD,
}

// shared state behind RwLock
struct RRDCacheState {
    rrd_map: HashMap<String, RRD>,
    journal: File,
    last_journal_flush: f64,
    journal_applied: bool,
}

struct JournalEntry {
    time: f64,
    value: f64,
    dst: DST,
    rel_path: String,
}

impl RRDCache {

    /// Creates a new instance
    ///
    /// `basedir`: All files are stored relative to this path.
    ///
    /// `file_options`: Files are created with this options.
    ///
    /// `dir_options`: Directories are created with this options.
    ///
    /// `apply_interval`: Commit journal after `apply_interval` seconds.
    ///
    /// `load_rrd_cb`; The callback function is used to load RRD files,
    /// and should return a newly generated RRD if the file does not
    /// exists (or is unreadable). This may generate RRDs with
    /// different configurations (dependent on `rel_path`).
    pub fn new<P: AsRef<Path>>(
        basedir: P,
        file_options: Option<CreateOptions>,
        dir_options: Option<CreateOptions>,
        apply_interval: f64,
        load_rrd_cb: fn(cache: &RRDCache, path: &Path, rel_path: &str, dst: DST) -> RRD,
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
            journal_applied: false,
        };

        Ok(Self {
            basedir,
            file_options,
            dir_options,
            apply_interval,
            load_rrd_cb,
            state: RwLock::new(state),
        })
    }

    /// Create a new RRD as used by the proxmox backup server
    ///
    /// It contains the following RRAs:
    ///
    /// * cf=average,r=60,n=1440 => 1day
    /// * cf=maximum,r=60,n=1440 => 1day
    /// * cf=average,r=30*60,n=1440 => 1month
    /// * cf=maximum,r=30*60,n=1440 => 1month
    /// * cf=average,r=6*3600,n=1440 => 1year
    /// * cf=maximum,r=6*3600,n=1440 => 1year
    /// * cf=average,r=7*86400,n=570 => 10years
    /// * cf=maximum,r=7*86400,n=570 => 10year
    ///
    /// The resultion data file size is about 80KB.
    pub fn create_proxmox_backup_default_rrd(dst: DST) -> RRD {

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

    /// Apply and commit the journal. Should be used at server startup.
    pub fn apply_journal(&self) -> Result<(), Error> {
        let mut state = self.state.write().unwrap(); // block writers
        self.apply_and_commit_journal_locked(&mut state)
    }

    fn apply_and_commit_journal_locked(&self, state: &mut RRDCacheState) -> Result<(), Error> {

        state.last_journal_flush = proxmox_time::epoch_f64();

        if !state.journal_applied {
            let start_time = SystemTime::now();
            log::debug!("applying rrd journal");

            match self.apply_journal_locked(state) {
                Ok(entries) => {
                    let elapsed = start_time.elapsed()?.as_secs_f64();
                    log::info!("applied rrd journal ({} entries in {:.3} seconds)", entries, elapsed);
                }
                Err(err) => bail!("apply rrd journal failed - {}", err),
            }
        }

        let start_time = SystemTime::now();
        log::debug!("commit rrd journal");

        match self.commit_journal_locked(state) {
            Ok(rrd_file_count) => {
                let elapsed = start_time.elapsed()?.as_secs_f64();
                log::info!("rrd journal successfully committed ({} files in {:.3} seconds)",
                           rrd_file_count, elapsed);
            }
            Err(err) => bail!("rrd journal commit failed: {}", err),
        }

        Ok(())
    }

    fn apply_journal_locked(&self, state: &mut RRDCacheState) -> Result<usize, Error> {

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

                let mut rrd = (self.load_rrd_cb)(&self, &path, &entry.rel_path, entry.dst);

                if entry.time > get_last_update(&entry.rel_path, &rrd) {
                    rrd.update(entry.time, entry.value);
                }
                state.rrd_map.insert(entry.rel_path.clone(), rrd);
            }
        }


        // We need to apply the journal only once, because further updates
        // are always directly applied.
        state.journal_applied = true;

        Ok(linenr)
    }

    fn commit_journal_locked(&self, state: &mut RRDCacheState) -> Result<usize, Error> {

        // save all RRDs
        let mut rrd_file_count = 0;

        let mut errors = 0;
        for (rel_path, rrd) in state.rrd_map.iter() {
            rrd_file_count += 1;
            let mut path = self.basedir.clone();
            path.push(&rel_path);
            if let Err(err) = rrd.save(&path, self.file_options.clone()) {
                errors += 1;
                log::error!("unable to save {:?}: {}", path, err);
            }
        }

       if errors != 0 {
            bail!("errors during rrd flush - unable to commit rrd journal");
        }

        // if everything went ok, commit the journal

        nix::unistd::ftruncate(state.journal.as_raw_fd(), 0)
            .map_err(|err| format_err!("unable to truncate journal - {}", err))?;

        Ok(rrd_file_count)
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

        if !state.journal_applied || (time - state.last_journal_flush) > self.apply_interval {
            self.apply_and_commit_journal_locked(&mut state)?;
        }

        Self::append_journal_entry(&mut state, time, value, dst, rel_path)?;

        if let Some(rrd) = state.rrd_map.get_mut(rel_path) {
            rrd.update(time, value);
        } else {
            let mut path = self.basedir.clone();
            path.push(rel_path);
            create_path(path.parent().unwrap(), Some(self.dir_options.clone()), Some(self.dir_options.clone()))?;

            let mut rrd = (self.load_rrd_cb)(&self, &path, rel_path, dst);

            rrd.update(time, value);
            state.rrd_map.insert(rel_path.into(), rrd);
        }

        Ok(())
    }

    /// Extract data from cached RRD
    ///
    /// `start`: Start time. If not sepecified, we simply extract 10 data points.
    ///
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
