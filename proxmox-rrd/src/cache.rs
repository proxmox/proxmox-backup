use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::io::{BufRead, BufReader};
use std::time::SystemTime;
use std::thread::spawn;
use crossbeam_channel::{bounded, TryRecvError};
use anyhow::{format_err, bail, Error};

use proxmox::tools::fs::{create_path, CreateOptions};

use crate::rrd::{DST, CF, RRD, RRA};

mod journal;
use journal::*;

mod rrd_map;
use rrd_map::*;

/// RRD cache - keep RRD data in RAM, but write updates to disk
///
/// This cache is designed to run as single instance (no concurrent
/// access from other processes).
pub struct RRDCache {
    config: Arc<CacheConfig>,
    state: Arc<RwLock<JournalState>>,
    rrd_map: Arc<RwLock<RRDMap>>,
}

pub(crate) struct CacheConfig {
    apply_interval: f64,
    basedir: PathBuf,
    file_options: CreateOptions,
    dir_options: CreateOptions,
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

        let config = Arc::new(CacheConfig {
            basedir: basedir.clone(),
            file_options: file_options.clone(),
            dir_options: dir_options,
            apply_interval,
        });

        let state = JournalState::new(Arc::clone(&config))?;
        let rrd_map = RRDMap::new(Arc::clone(&config), load_rrd_cb);

        Ok(Self {
            config: Arc::clone(&config),
            state: Arc::new(RwLock::new(state)),
            rrd_map: Arc::new(RwLock::new(rrd_map)),
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

    /// Apply and commit the journal. Should be used at server startup.
    pub fn apply_journal(&self) -> Result<bool, Error> {
        let state = Arc::clone(&self.state);
        let rrd_map = Arc::clone(&self.rrd_map);

        let mut state_guard = self.state.write().unwrap();
        let journal_applied = state_guard.journal_applied;
        let now = proxmox_time::epoch_f64();
        let wants_commit = (now - state_guard.last_journal_flush) > self.config.apply_interval;

        if journal_applied && !wants_commit { return Ok(journal_applied); }

        if let Some(ref recv) = state_guard.apply_thread_result {
            match recv.try_recv() {
                Ok(Ok(())) => {
                    // finished without errors, OK
                }
                Ok(Err(err)) => {
                    // finished with errors, log them
                    log::error!("{}", err);
                }
                Err(TryRecvError::Empty) => {
                    // still running
                    return Ok(journal_applied);
                }
                Err(TryRecvError::Disconnected) => {
                    // crashed, start again
                    log::error!("apply journal thread crashed - try again");
                }
            }
        }

        state_guard.last_journal_flush = proxmox_time::epoch_f64();

        let (sender, receiver) = bounded(1);
        state_guard.apply_thread_result = Some(receiver);

        spawn(move || {
            let result = apply_and_commit_journal_thread(state, rrd_map, journal_applied)
                .map_err(|err| err.to_string());
            sender.send(result).unwrap();
        });

        Ok(journal_applied)
    }


    /// Update data in RAM and write file back to disk (journal)
    pub fn update_value(
        &self,
        rel_path: &str,
        time: f64,
        value: f64,
        dst: DST,
    ) -> Result<(), Error> {

        let journal_applied = self.apply_journal()?;

        self.state.write().unwrap()
            .append_journal_entry(time, value, dst, rel_path)?;

        if journal_applied {
            self.rrd_map.write().unwrap().update(rel_path, time, value, dst, false)?;
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
        self.rrd_map.read().unwrap()
            .extract_cached_data(base, name, cf, resolution, start, end)
    }
}


fn apply_and_commit_journal_thread(
    state: Arc<RwLock<JournalState>>,
    rrd_map: Arc<RwLock<RRDMap>>,
    commit_only: bool,
) -> Result<(), Error> {

    if commit_only {
        state.write().unwrap().rotate_journal()?; // start new journal, keep old one
    } else {
        let start_time = SystemTime::now();
        log::debug!("applying rrd journal");

        match apply_journal_impl(Arc::clone(&state), Arc::clone(&rrd_map)) {
            Ok(entries) => {
                let elapsed = start_time.elapsed().unwrap().as_secs_f64();
                log::info!("applied rrd journal ({} entries in {:.3} seconds)", entries, elapsed);
            }
            Err(err) => bail!("apply rrd journal failed - {}", err),
        }
    }

    let start_time = SystemTime::now();
    log::debug!("commit rrd journal");

    match commit_journal_impl(state, rrd_map) {
        Ok(rrd_file_count) => {
            let elapsed = start_time.elapsed().unwrap().as_secs_f64();
            log::info!("rrd journal successfully committed ({} files in {:.3} seconds)",
                       rrd_file_count, elapsed);
        }
        Err(err) => bail!("rrd journal commit failed: {}", err),
    }
    Ok(())
}

fn apply_journal_lines(
    state: Arc<RwLock<JournalState>>,
    rrd_map: Arc<RwLock<RRDMap>>,
    journal_name: &str, // used for logging
    reader: &mut BufReader<File>,
    lock_read_line: bool,
) -> Result<usize, Error> {

    let mut linenr = 0;

    loop {
        linenr += 1;
        let mut line = String::new();
        let len = if lock_read_line {
            let _lock = state.read().unwrap(); // make sure we read entire lines
            reader.read_line(&mut line)?
        } else {
            reader.read_line(&mut line)?
        };

        if len == 0 { break; }

        let entry = match RRDCache::parse_journal_line(&line) {
            Ok(entry) => entry,
            Err(err) => {
                log::warn!(
                    "unable to parse rrd journal '{}' line {} (skip) - {}",
                    journal_name, linenr, err,
                );
                continue; // skip unparsable lines
            }
        };

        rrd_map.write().unwrap().update(&entry.rel_path, entry.time, entry.value, entry.dst, true)?;
    }
    Ok(linenr)
}

fn apply_journal_impl(
    state: Arc<RwLock<JournalState>>,
    rrd_map: Arc<RwLock<RRDMap>>,
) -> Result<usize, Error> {

    let mut lines = 0;

    // Apply old journals first
    let journal_list = state.read().unwrap().list_old_journals()?;

    for (_time, filename, path) in journal_list {
        log::info!("apply old journal log {}", filename);
        let file = std::fs::OpenOptions::new().read(true).open(path)?;
        let mut reader = BufReader::new(file);
        lines += apply_journal_lines(
            Arc::clone(&state),
            Arc::clone(&rrd_map),
            &filename,
            &mut reader,
            false,
        )?;
    }

    let mut journal = state.read().unwrap().open_journal_reader()?;

    lines += apply_journal_lines(
        Arc::clone(&state),
        Arc::clone(&rrd_map),
        "rrd.journal",
        &mut journal,
        true,
    )?;

    {
        let mut state_guard = state.write().unwrap(); // block other writers

        lines += apply_journal_lines(
            Arc::clone(&state),
            Arc::clone(&rrd_map),
            "rrd.journal",
            &mut journal,
            false,
        )?;

        state_guard.rotate_journal()?; // start new journal, keep old one

        // We need to apply the journal only once, because further updates
        // are always directly applied.
        state_guard.journal_applied = true;
    }


    Ok(lines)
}

fn commit_journal_impl(
    state: Arc<RwLock<JournalState>>,
    rrd_map: Arc<RwLock<RRDMap>>,
) -> Result<usize, Error> {

    // save all RRDs - we only need a read lock here
    let rrd_file_count = rrd_map.read().unwrap().flush_rrd_files()?;

    // if everything went ok, remove the old journal files
    state.write().unwrap().remove_old_journals()?;

    Ok(rrd_file_count)
}
