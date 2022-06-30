//! Map a raw data reader as a loop device via FUSE

use anyhow::{bail, format_err, Error};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{read_to_string, remove_file, File, OpenOptions};
use std::io::prelude::*;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};

use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use regex::Regex;

use futures::channel::mpsc::{Receiver, Sender};
use futures::stream::{StreamExt, TryStreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt};

use super::loopdev;
use proxmox_fuse::{requests::FuseRequest, *};
use proxmox_time::epoch_i64;

const RUN_DIR: &str = "/run/pbs-loopdev";

lazy_static::lazy_static! {
    static ref LOOPDEV_REGEX: Regex = Regex::new(r"^loop\d+$").unwrap();
}

/// Represents an ongoing FUSE-session that has been mapped onto a loop device.
/// Create with map_loop, then call 'main' and poll until startup_chan reports
/// success. Then, daemonize or otherwise finish setup, and continue polling
/// main's future until completion.
pub struct FuseLoopSession<R: AsyncRead + AsyncSeek + Unpin> {
    session: Option<Fuse>,
    stat: libc::stat,
    reader: R,
    fuse_path: String,
    pid_path: String,
    pub loopdev_path: String,
}

impl<R: AsyncRead + AsyncSeek + Unpin> FuseLoopSession<R> {
    /// Prepare for mapping the given reader as a block device node at
    /// /dev/loopN. Creates a temporary file for FUSE and a PID file for unmap.
    pub async fn map_loop<P: AsRef<str>>(
        size: u64,
        mut reader: R,
        name: P,
        options: &OsStr,
    ) -> Result<Self, Error> {
        // attempt a single read to check if the reader is configured correctly
        let _ = reader.read_u8().await?;

        std::fs::create_dir_all(RUN_DIR)?;
        let mut path = PathBuf::from(RUN_DIR);
        path.push(name.as_ref());
        let mut pid_path = path.clone();
        pid_path.set_extension("pid");

        // cleanup previous instance with same name
        // if loopdev is actually still mapped, this will do nothing and the
        // create_new below will fail as intended
        cleanup_unused_run_files(Some(name.as_ref().to_owned()));

        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => { /* file created, continue on */ }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    bail!("the given archive is already mapped, cannot map twice");
                } else {
                    bail!("error while creating backing file ({:?}) - {}", &path, e);
                }
            }
        }

        let session = Fuse::builder("pbs-block-dev")?
            .options_os(options)?
            .enable_read()
            .build()?
            .mount(&path)?;

        let loopdev_path = loopdev::get_or_create_free_dev()
            .map_err(|err| format_err!("loop-control GET_FREE failed - {}", err))?;

        // write pidfile so unmap can later send us a signal to exit
        Self::write_pidfile(&pid_path)?;

        Ok(Self {
            session: Some(session),
            reader,
            stat: minimal_stat(size as i64),
            fuse_path: path.to_string_lossy().into_owned(),
            pid_path: pid_path.to_string_lossy().into_owned(),
            loopdev_path,
        })
    }

    fn write_pidfile(path: &Path) -> Result<(), Error> {
        let pid = unsafe { libc::getpid() };
        let mut file = File::create(path)?;
        write!(file, "{}", pid)?;
        Ok(())
    }

    /// Runs the FUSE request loop and assigns the loop device. Will send a
    /// message on startup_chan once the loop device is assigned (or assignment
    /// fails). Send a message on abort_chan to trigger cleanup and exit FUSE.
    /// An error on loopdev assignment does *not* automatically close the FUSE
    /// handle or do cleanup, trigger abort_chan manually in case startup fails.
    pub async fn main(
        &mut self,
        mut startup_chan: Sender<Result<(), Error>>,
        abort_chan: Receiver<()>,
    ) -> Result<(), Error> {
        if self.session.is_none() {
            panic!("internal error: fuse_loop::main called before ::map_loop");
        }
        let mut session = self.session.take().unwrap().fuse();
        let mut abort_chan = abort_chan.fuse();

        let (loopdev_path, fuse_path) = (self.loopdev_path.clone(), self.fuse_path.clone());
        tokio::task::spawn_blocking(move || {
            if let Err(err) = loopdev::assign(loopdev_path, fuse_path) {
                let _ = startup_chan.try_send(Err(format_err!(
                    "error while assigning loop device - {}",
                    err
                )));
            } else {
                // device is assigned successfully, which means not only is the
                // loopdev ready, but FUSE is also okay, since the assignment
                // would have failed otherwise
                let _ = startup_chan.try_send(Ok(()));
            }
        });

        let (loopdev_path, fuse_path, pid_path) = (
            self.loopdev_path.clone(),
            self.fuse_path.clone(),
            self.pid_path.clone(),
        );
        let cleanup = |session: futures::stream::Fuse<Fuse>| {
            // only warn for errors on cleanup, if these fail nothing is lost
            if let Err(err) = loopdev::unassign(&loopdev_path) {
                log::warn!(
                    "cleanup: warning: could not unassign file {} from loop device {} - {}",
                    &fuse_path,
                    &loopdev_path,
                    err,
                );
            }

            // force close FUSE handle before attempting to remove backing file
            std::mem::drop(session);

            if let Err(err) = remove_file(&fuse_path) {
                log::warn!(
                    "cleanup: warning: could not remove temporary file {} - {}",
                    &fuse_path,
                    err,
                );
            }
            if let Err(err) = remove_file(&pid_path) {
                log::warn!(
                    "cleanup: warning: could not remove PID file {} - {}",
                    &pid_path,
                    err,
                );
            }
        };

        loop {
            tokio::select! {
                _ = abort_chan.next() => {
                    // aborted, do cleanup and exit
                    break;
                },
                req = session.try_next() => {
                    let res = match req? {
                        Some(Request::Lookup(req)) => {
                            let stat = self.stat;
                            let entry = EntryParam::simple(stat.st_ino, stat);
                            req.reply(&entry)
                        },
                        Some(Request::Getattr(req)) => {
                            req.reply(&self.stat, f64::MAX)
                        },
                        Some(Request::Read(req)) => {
                            match self.reader.seek(SeekFrom::Start(req.offset)).await {
                                Ok(_) => {
                                    let mut buf = vec![0u8; req.size];
                                    match self.reader.read_exact(&mut buf).await {
                                        Ok(_) => {
                                            req.reply(&buf)
                                        },
                                        Err(e) => {
                                            req.io_fail(e)
                                        }
                                    }
                                },
                                Err(e) => {
                                    req.io_fail(e)
                                }
                            }
                        },
                        Some(_) => {
                            // only FUSE requests necessary for loop-mapping are implemented
                            log::error!("Unimplemented FUSE request type encountered");
                            Ok(())
                        },
                        None => {
                            // FUSE connection closed
                            break;
                        }
                    };
                    if let Err(err) = res {
                        // error during FUSE reply, cleanup and exit
                        cleanup(session);
                        bail!(err);
                    }
                }
            }
        }

        // non-error FUSE exit
        cleanup(session);
        Ok(())
    }
}

/// Clean up leftover files as well as FUSE instances without a loop device
/// connected. Best effort, never returns an error.
/// If filter_name is Some("..."), only this name will be cleaned up.
pub fn cleanup_unused_run_files(filter_name: Option<String>) {
    if let Ok(maps) = find_all_mappings() {
        for (name, loopdev) in maps {
            if loopdev.is_none()
                && (filter_name.is_none() || &name == filter_name.as_ref().unwrap())
            {
                let mut path = PathBuf::from(RUN_DIR);
                path.push(&name);

                // clean leftover FUSE instances (e.g. user called 'losetup -d' or similar)
                // does nothing if files are already stagnant (e.g. instance crashed etc...)
                if unmap_from_backing(&path, None).is_ok() {
                    // we have reaped some leftover instance, tell the user
                    log::info!(
                        "Cleaned up dangling mapping '{}': no loop device assigned",
                        &name
                    );
                }

                // remove remnant files
                // these we're not doing anything, so no need to inform the user
                let _ = remove_file(&path);
                path.set_extension("pid");
                let _ = remove_file(&path);
            }
        }
    }
}

fn get_backing_file(loopdev: &str) -> Result<String, Error> {
    let num = loopdev.split_at(9).1.parse::<u8>().map_err(|err| {
        format_err!(
            "malformed loopdev path, does not end with valid number - {}",
            err
        )
    })?;

    let block_path = PathBuf::from(format!(
        "/sys/devices/virtual/block/loop{}/loop/backing_file",
        num
    ));
    let backing_file = read_to_string(block_path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            format_err!("nothing mapped to {}", loopdev)
        } else {
            format_err!("error reading backing file - {}", err)
        }
    })?;

    let backing_file = backing_file.trim();

    if !backing_file.starts_with(RUN_DIR) {
        bail!(
            "loopdev {} is in use, but not by proxmox-backup-client (mapped to '{}')",
            loopdev,
            backing_file,
        );
    }

    Ok(backing_file.to_owned())
}

// call in broken state: we found the mapping, but the client is already dead,
// only thing to do is clean up what we can
fn emerg_cleanup(loopdev: Option<&str>, mut backing_file: PathBuf) {
    log::warn!(
        "warning: found mapping with dead process ({:?}), attempting cleanup",
        &backing_file
    );

    if let Some(loopdev) = loopdev {
        let _ = loopdev::unassign(loopdev);
    }

    // killing the backing process does not cancel the FUSE mount automatically
    let mut command = std::process::Command::new("fusermount");
    command.arg("-u");
    command.arg(&backing_file);
    let _ = proxmox_sys::command::run_command(command, None);

    let _ = remove_file(&backing_file);
    backing_file.set_extension("pid");
    let _ = remove_file(&backing_file);
}

fn unmap_from_backing(backing_file: &Path, loopdev: Option<&str>) -> Result<(), Error> {
    let mut pid_path = PathBuf::from(backing_file);
    pid_path.set_extension("pid");

    let pid_str = read_to_string(&pid_path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            emerg_cleanup(loopdev, backing_file.to_owned());
        }
        format_err!("error reading pidfile {:?}: {}", &pid_path, err)
    })?;
    let pid = pid_str
        .parse::<i32>()
        .map_err(|err| format_err!("malformed PID ({}) in pidfile - {}", pid_str, err))?;

    let pid = Pid::from_raw(pid);

    // send SIGINT to trigger cleanup and exit in target process
    match signal::kill(pid, Signal::SIGINT) {
        Ok(()) => {}
        Err(nix::errno::Errno::ESRCH) => {
            emerg_cleanup(loopdev, backing_file.to_owned());
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    }

    // block until unmap is complete or timeout
    let start = epoch_i64();
    loop {
        match signal::kill(pid, None) {
            Ok(_) => {
                // 10 second timeout, then assume failure
                if (epoch_i64() - start) > 10 {
                    return Err(format_err!("timed out waiting for PID '{}' to exit", &pid));
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(nix::errno::Errno::ESRCH) => {
                break;
            }
            Err(e) => return Err(e.into()),
        }
    }

    Ok(())
}

/// Returns an Iterator over a set of currently active mappings, i.e.
/// FuseLoopSession instances. Returns ("backing-file-name", Some("/dev/loopX"))
/// where .1 is None when a user has manually called 'losetup -d' or similar but
/// the FUSE instance is still running.
pub fn find_all_mappings() -> Result<impl Iterator<Item = (String, Option<String>)>, Error> {
    // get map of all /dev/loop mappings belonging to us
    let mut loopmap = HashMap::new();
    for ent in
        proxmox_sys::fs::scan_subdir(libc::AT_FDCWD, Path::new("/dev/"), &LOOPDEV_REGEX)?.flatten()
    {
        let loopdev = format!("/dev/{}", ent.file_name().to_string_lossy());
        if let Ok(file) = get_backing_file(&loopdev) {
            // insert filename only, strip RUN_DIR/
            loopmap.insert(file[RUN_DIR.len() + 1..].to_owned(), loopdev);
        }
    }

    Ok(
        proxmox_sys::fs::read_subdir(libc::AT_FDCWD, Path::new(RUN_DIR))?.filter_map(move |ent| {
            match ent {
                Ok(ent) => {
                    let file = ent.file_name().to_string_lossy();
                    if file == "." || file == ".." || file.ends_with(".pid") {
                        None
                    } else {
                        let loopdev = loopmap.get(file.as_ref()).map(String::to_owned);
                        Some((file.into_owned(), loopdev))
                    }
                }
                Err(_) => None,
            }
        }),
    )
}

/// Try and unmap a running proxmox-backup-client instance from the given
/// /dev/loopN device
pub fn unmap_loopdev<S: AsRef<str>>(loopdev: S) -> Result<(), Error> {
    let loopdev = loopdev.as_ref();
    if loopdev.len() < 10 || !loopdev.starts_with("/dev/loop") {
        bail!("malformed loopdev path, must be in format '/dev/loopX'");
    }

    let backing_file = get_backing_file(loopdev)?;
    unmap_from_backing(Path::new(&backing_file), Some(loopdev))
}

/// Try and unmap a running proxmox-backup-client instance from the given name
pub fn unmap_name<S: AsRef<str>>(name: S) -> Result<(), Error> {
    for (mapping, loopdev) in find_all_mappings()? {
        if mapping.ends_with(name.as_ref()) {
            let mut path = PathBuf::from(RUN_DIR);
            path.push(&mapping);
            return unmap_from_backing(&path, loopdev.as_deref());
        }
    }
    Err(format_err!("no mapping for name '{}' found", name.as_ref()))
}

fn minimal_stat(size: i64) -> libc::stat {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    stat.st_mode = libc::S_IFREG;
    stat.st_ino = 1;
    stat.st_nlink = 1;
    stat.st_size = size;
    stat
}
