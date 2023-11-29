use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::thread::spawn;
use std::time::SystemTime;

use anyhow::{bail, format_err, Error};
use crossbeam_channel::{bounded, TryRecvError};

use proxmox_sys::fs::{create_path, CreateOptions};

use crate::rrd::{CF, DST, RRA, RRD};
use crate::Entry;

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

        let file_options = file_options.unwrap_or_else(CreateOptions::new);
        let dir_options = dir_options.unwrap_or_else(CreateOptions::new);

        create_path(
            &basedir,
            Some(dir_options.clone()),
            Some(dir_options.clone()),
        )
        .map_err(|err: Error| format_err!("unable to create rrdb stat dir - {}", err))?;

        let config = Arc::new(CacheConfig {
            basedir,
            file_options,
            dir_options,
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
    /// The resulting data file size is about 80KB.
    pub fn create_proxmox_backup_default_rrd(dst: DST) -> RRD {
        let rra_list = vec![
            // 1 min * 1440 => 1 day
            RRA::new(CF::Average, 60, 1440),
            RRA::new(CF::Maximum, 60, 1440),
            // 30 min * 1440 => 30 days ~ 1 month
            RRA::new(CF::Average, 30 * 60, 1440),
            RRA::new(CF::Maximum, 30 * 60, 1440),
            // 6 h * 1440 => 360 days ~ 1 year
            RRA::new(CF::Average, 6 * 3600, 1440),
            RRA::new(CF::Maximum, 6 * 3600, 1440),
            // 1 week * 570 => 10 years
            RRA::new(CF::Average, 7 * 86400, 570),
            RRA::new(CF::Maximum, 7 * 86400, 570),
        ];

        RRD::new(dst, rra_list)
    }

    /// Sync the journal data to disk (using `fdatasync` syscall)
    pub fn sync_journal(&self) -> Result<(), Error> {
        self.state.read().unwrap().sync_journal()
    }

    /// Apply and commit the journal. Should be used at server startup.
    pub fn apply_journal(&self) -> Result<bool, Error> {
        let config = Arc::clone(&self.config);
        let state = Arc::clone(&self.state);
        let rrd_map = Arc::clone(&self.rrd_map);

        let mut state_guard = self.state.write().unwrap();
        let journal_applied = state_guard.journal_applied;

        if let Some(ref recv) = state_guard.apply_thread_result {
            match recv.try_recv() {
                Ok(Ok(())) => {
                    // finished without errors, OK
                    state_guard.apply_thread_result = None;
                }
                Ok(Err(err)) => {
                    // finished with errors, log them
                    log::error!("{}", err);
                    state_guard.apply_thread_result = None;
                }
                Err(TryRecvError::Empty) => {
                    // still running
                    return Ok(journal_applied);
                }
                Err(TryRecvError::Disconnected) => {
                    // crashed, start again
                    log::error!("apply journal thread crashed - try again");
                    state_guard.apply_thread_result = None;
                }
            }
        }

        let now = proxmox_time::epoch_f64();
        let wants_commit = (now - state_guard.last_journal_flush) > self.config.apply_interval;

        if journal_applied && !wants_commit {
            return Ok(journal_applied);
        }

        state_guard.last_journal_flush = proxmox_time::epoch_f64();

        let (sender, receiver) = bounded(1);
        state_guard.apply_thread_result = Some(receiver);

        spawn(move || {
            let result = apply_and_commit_journal_thread(config, state, rrd_map, journal_applied)
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

        self.state
            .write()
            .unwrap()
            .append_journal_entry(time, value, dst, rel_path)?;

        if journal_applied {
            self.rrd_map
                .write()
                .unwrap()
                .update(rel_path, time, value, dst, false)?;
        }

        Ok(())
    }

    /// Extract data from cached RRD
    ///
    /// `start`: Start time. If not specified, we simply extract 10 data points.
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
    ) -> Result<Option<Entry>, Error> {
        self.rrd_map
            .read()
            .unwrap()
            .extract_cached_data(base, name, cf, resolution, start, end)
    }
}

fn apply_and_commit_journal_thread(
    config: Arc<CacheConfig>,
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
                log::info!(
                    "applied rrd journal ({} entries in {:.3} seconds)",
                    entries,
                    elapsed
                );
            }
            Err(err) => bail!("apply rrd journal failed - {}", err),
        }
    }

    let start_time = SystemTime::now();
    log::debug!("commit rrd journal");

    match commit_journal_impl(config, state, rrd_map) {
        Ok(rrd_file_count) => {
            let elapsed = start_time.elapsed().unwrap().as_secs_f64();
            log::info!(
                "rrd journal successfully committed ({} files in {:.3} seconds)",
                rrd_file_count,
                elapsed
            );
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

        if len == 0 {
            break;
        }

        let entry: JournalEntry = match line.parse() {
            Ok(entry) => entry,
            Err(err) => {
                log::warn!(
                    "unable to parse rrd journal '{}' line {} (skip) - {}",
                    journal_name,
                    linenr,
                    err,
                );
                continue; // skip unparsable lines
            }
        };

        rrd_map.write().unwrap().update(
            &entry.rel_path,
            entry.time,
            entry.value,
            entry.dst,
            true,
        )?;
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

    for entry in journal_list {
        log::info!("apply old journal log {}", entry.name);
        let file = std::fs::OpenOptions::new().read(true).open(&entry.path)?;
        let mut reader = BufReader::new(file);
        lines += apply_journal_lines(
            Arc::clone(&state),
            Arc::clone(&rrd_map),
            &entry.name,
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

fn fsync_file_or_dir(path: &Path) -> Result<(), Error> {
    let file = std::fs::File::open(path)?;
    nix::unistd::fsync(file.as_raw_fd())?;
    Ok(())
}

pub(crate) fn fsync_file_and_parent(path: &Path) -> Result<(), Error> {
    let file = std::fs::File::open(path)?;
    nix::unistd::fsync(file.as_raw_fd())?;
    if let Some(parent) = path.parent() {
        fsync_file_or_dir(parent)?;
    }
    Ok(())
}

fn rrd_parent_dir(basedir: &Path, rel_path: &str) -> PathBuf {
    let mut path = basedir.to_owned();
    let rel_path = Path::new(rel_path);
    if let Some(parent) = rel_path.parent() {
        path.push(parent);
    }
    path
}

fn commit_journal_impl(
    config: Arc<CacheConfig>,
    state: Arc<RwLock<JournalState>>,
    rrd_map: Arc<RwLock<RRDMap>>,
) -> Result<usize, Error> {
    let files = rrd_map.read().unwrap().file_list();

    let mut rrd_file_count = 0;
    let mut errors = 0;

    let mut dir_set = BTreeSet::new();

    log::info!("write rrd data back to disk");

    // save all RRDs - we only need a read lock here
    // Note: no fsync here (we do it afterwards)
    for rel_path in files.iter() {
        let parent_dir = rrd_parent_dir(&config.basedir, rel_path);
        dir_set.insert(parent_dir);
        rrd_file_count += 1;
        if let Err(err) = rrd_map.read().unwrap().flush_rrd_file(rel_path) {
            errors += 1;
            log::error!("unable to save rrd {}: {}", rel_path, err);
        }
    }

    if errors != 0 {
        bail!("errors during rrd flush - unable to commit rrd journal");
    }

    // Important: We fsync files after writing all data! This increase
    // the likelihood that files are already synced, so this is
    // much faster (although we need to re-open the files).

    log::info!("starting rrd data sync");

    for rel_path in files.iter() {
        let mut path = config.basedir.clone();
        path.push(rel_path);
        fsync_file_or_dir(&path)
            .map_err(|err| format_err!("fsync rrd file {} failed - {}", rel_path, err))?;
    }

    // also fsync directories
    for dir_path in dir_set {
        fsync_file_or_dir(&dir_path)
            .map_err(|err| format_err!("fsync rrd dir {:?} failed - {}", dir_path, err))?;
    }

    // if everything went ok, remove the old journal files
    state.write().unwrap().remove_old_journals()?;

    Ok(rrd_file_count)
}
