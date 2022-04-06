use std::ffi::OsStr;
use std::fs::File;
use std::io::{BufReader, Write};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{bail, format_err, Error};
use crossbeam_channel::Receiver;
use nix::fcntl::OFlag;

use proxmox_sys::fs::atomic_open_or_create_file;

const RRD_JOURNAL_NAME: &str = "rrd.journal";

use crate::cache::CacheConfig;
use crate::rrd::DST;

// shared state behind RwLock
pub struct JournalState {
    config: Arc<CacheConfig>,
    journal: File,
    pub last_journal_flush: f64,
    pub journal_applied: bool,
    pub apply_thread_result: Option<Receiver<Result<(), String>>>,
}

pub struct JournalEntry {
    pub time: f64,
    pub value: f64,
    pub dst: DST,
    pub rel_path: String,
}

impl FromStr for JournalEntry {
    type Err = Error;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let line = line.trim();

        let parts: Vec<&str> = line.splitn(4, ':').collect();
        if parts.len() != 4 {
            bail!("wrong numper of components");
        }

        let time: f64 = parts[0]
            .parse()
            .map_err(|_| format_err!("unable to parse time"))?;
        let value: f64 = parts[1]
            .parse()
            .map_err(|_| format_err!("unable to parse value"))?;
        let dst: u8 = parts[2]
            .parse()
            .map_err(|_| format_err!("unable to parse data source type"))?;

        let dst = match dst {
            0 => DST::Gauge,
            1 => DST::Derive,
            _ => bail!("got strange value for data source type '{}'", dst),
        };

        let rel_path = parts[3].to_string();

        Ok(JournalEntry {
            time,
            value,
            dst,
            rel_path,
        })
    }
}

pub struct JournalFileInfo {
    pub time: u64,
    pub name: String,
    pub path: PathBuf,
}

impl JournalState {
    pub(crate) fn new(config: Arc<CacheConfig>) -> Result<Self, Error> {
        let journal = JournalState::open_journal_writer(&config)?;
        Ok(Self {
            config,
            journal,
            last_journal_flush: 0.0,
            journal_applied: false,
            apply_thread_result: None,
        })
    }

    pub fn sync_journal(&self) -> Result<(), Error> {
        nix::unistd::fdatasync(self.journal.as_raw_fd())?;
        Ok(())
    }

    pub fn append_journal_entry(
        &mut self,
        time: f64,
        value: f64,
        dst: DST,
        rel_path: &str,
    ) -> Result<(), Error> {
        let journal_entry = format!("{}:{}:{}:{}\n", time, value, dst as u8, rel_path);
        self.journal.write_all(journal_entry.as_bytes())?;
        Ok(())
    }

    pub fn open_journal_reader(&self) -> Result<BufReader<File>, Error> {
        // fixme : dup self.journal instead??
        let mut journal_path = self.config.basedir.clone();
        journal_path.push(RRD_JOURNAL_NAME);

        let flags = OFlag::O_CLOEXEC | OFlag::O_RDONLY;
        let journal = atomic_open_or_create_file(
            &journal_path,
            flags,
            &[],
            self.config.file_options.clone(),
            false,
        )?;
        Ok(BufReader::new(journal))
    }

    fn open_journal_writer(config: &CacheConfig) -> Result<File, Error> {
        let mut journal_path = config.basedir.clone();
        journal_path.push(RRD_JOURNAL_NAME);

        let flags = OFlag::O_CLOEXEC | OFlag::O_WRONLY | OFlag::O_APPEND;
        let journal = atomic_open_or_create_file(
            &journal_path,
            flags,
            &[],
            config.file_options.clone(),
            false,
        )?;
        Ok(journal)
    }

    pub fn rotate_journal(&mut self) -> Result<(), Error> {
        let mut journal_path = self.config.basedir.clone();
        journal_path.push(RRD_JOURNAL_NAME);

        let mut new_name = journal_path.clone();
        let now = proxmox_time::epoch_i64();
        new_name.set_extension(format!("journal-{:08x}", now));
        std::fs::rename(journal_path, &new_name)?;

        self.journal = Self::open_journal_writer(&self.config)?;

        // make sure the old journal data landed on the disk
        super::fsync_file_and_parent(&new_name)?;

        Ok(())
    }

    pub fn remove_old_journals(&self) -> Result<(), Error> {
        let journal_list = self.list_old_journals()?;

        for entry in journal_list {
            std::fs::remove_file(entry.path)?;
        }

        Ok(())
    }

    pub fn list_old_journals(&self) -> Result<Vec<JournalFileInfo>, Error> {
        let mut list = Vec::new();
        for entry in std::fs::read_dir(&self.config.basedir)? {
            let entry = entry?;
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            match path.file_stem() {
                None => continue,
                Some(stem) if stem != OsStr::new("rrd") => continue,
                Some(_) => (),
            }

            if let Some(extension) = path.extension() {
                if let Some(extension) = extension.to_str() {
                    if let Some(rest) = extension.strip_prefix("journal-") {
                        if let Ok(time) = u64::from_str_radix(rest, 16) {
                            list.push(JournalFileInfo {
                                time,
                                name: format!("rrd.{}", extension),
                                path: path.to_owned(),
                            });
                        }
                    }
                }
            }
        }
        list.sort_unstable_by_key(|entry| entry.time);
        Ok(list)
    }
}
