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
    state: RwLock<RRDCacheState>,
    rrd_map: RwLock<RRDMap>,
}

struct RRDMap {
    basedir: PathBuf,
    file_options: CreateOptions,
    dir_options: CreateOptions,
    map: HashMap<String, RRD>,
    load_rrd_cb: fn(path: &Path, rel_path: &str, dst: DST) -> RRD,
}

impl RRDMap {

    fn update(
        &mut self,
        rel_path: &str,
        time: f64,
        value: f64,
        dst: DST,
        new_only: bool,
    ) -> Result<(), Error> {
        if let Some(rrd) = self.map.get_mut(rel_path) {
            if !new_only || time > rrd.last_update() {
                rrd.update(time, value);
            }
        } else {
            let mut path = self.basedir.clone();
            path.push(rel_path);
            create_path(path.parent().unwrap(), Some(self.dir_options.clone()), Some(self.dir_options.clone()))?;

            let mut rrd = (self.load_rrd_cb)(&path, rel_path, dst);

            if !new_only || time > rrd.last_update() {
                rrd.update(time, value);
            }
            self.map.insert(rel_path.to_string(), rrd);
        }
        Ok(())
    }

    fn flush_rrd_files(&self) -> Result<usize, Error> {
        let mut rrd_file_count = 0;

        let mut errors = 0;
        for (rel_path, rrd) in self.map.iter() {
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

        Ok(rrd_file_count)
    }

    fn extract_cached_data(
        &self,
        base: &str,
        name: &str,
        cf: CF,
        resolution: u64,
        start: Option<u64>,
        end: Option<u64>,
    ) -> Result<Option<(u64, u64, Vec<Option<f64>>)>, Error> {
        match self.map.get(&format!("{}/{}", base, name)) {
            Some(rrd) => Ok(Some(rrd.extract_data(cf, resolution, start, end)?)),
            None => Ok(None),
        }
    }
}

// shared state behind RwLock
struct RRDCacheState {
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
        load_rrd_cb: fn(path: &Path, rel_path: &str, dst: DST) -> RRD,
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
            last_journal_flush: 0.0,
            journal_applied: false,
        };

        let rrd_map = RRDMap {
            basedir: basedir.clone(),
            file_options: file_options.clone(),
            dir_options: dir_options,
            map: HashMap::new(),
            load_rrd_cb,
        };

        Ok(Self {
            basedir,
            file_options,
            apply_interval,
            state: RwLock::new(state),
            rrd_map: RwLock::new(rrd_map),
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

        // fixme: apply blocked to avoid too many calls to self.rrd_map.write() ??
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

            self.rrd_map.write().unwrap().update(&entry.rel_path, entry.time, entry.value, entry.dst, true)?;
        }

        // We need to apply the journal only once, because further updates
        // are always directly applied.
        state.journal_applied = true;

        Ok(linenr)
    }

    fn commit_journal_locked(&self, state: &mut RRDCacheState) -> Result<usize, Error> {

        // save all RRDs - we only need a read lock here
        let rrd_file_count = self.rrd_map.read().unwrap().flush_rrd_files()?;

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

        self.rrd_map.write().unwrap().update(rel_path, time, value, dst, false)?;

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
        self.rrd_map.read().unwrap()
            .extract_cached_data(base, name, cf, resolution, start, end)
    }
}
