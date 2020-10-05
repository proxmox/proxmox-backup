use anyhow::{Error, format_err, bail};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::fs::{File, remove_file, read_to_string};
use std::io::SeekFrom;
use std::io::prelude::*;

use nix::unistd::{Pid, mkstemp};
use nix::sys::signal::{self, Signal};

use tokio::io::{AsyncRead, AsyncSeek, AsyncReadExt, AsyncSeekExt};
use futures::stream::{StreamExt, TryStreamExt};
use futures::channel::mpsc::{Sender, Receiver};

use proxmox::try_block;
use proxmox_fuse::{*, requests::FuseRequest};
use super::loopdev;

const RUN_DIR: &'static str = "/run/pbs-loopdev";

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
    pub async fn map_loop(size: u64, mut reader: R, options: &OsStr)
        -> Result<Self, Error>
    {
        // attempt a single read to check if the reader is configured correctly
        let _ = reader.read_u8().await?;

        std::fs::create_dir_all(RUN_DIR)?;
        let mut base_path = PathBuf::from(RUN_DIR);
        base_path.push("XXXXXX"); // template for mkstemp
        let (_, path) = mkstemp(&base_path)?;
        let mut pid_path = path.clone();
        pid_path.set_extension("pid");

        let res: Result<(Fuse, String), Error> = try_block!{
            let session = Fuse::builder("pbs-block-dev")?
                .options_os(options)?
                .enable_read()
                .build()?
                .mount(&path)?;

            let loopdev_path = loopdev::get_or_create_free_dev().map_err(|err| {
                format_err!("loop-control GET_FREE failed - {}", err)
            })?;

            // write pidfile so unmap can later send us a signal to exit
            Self::write_pidfile(&pid_path)?;

            Ok((session, loopdev_path))
        };

        match res {
            Ok((session, loopdev_path)) =>
                Ok(Self {
                    session: Some(session),
                    reader,
                    stat: minimal_stat(size as i64),
                    fuse_path: path.to_string_lossy().into_owned(),
                    pid_path: pid_path.to_string_lossy().into_owned(),
                    loopdev_path,
                }),
            Err(e) => {
                // best-effort temp file cleanup in case of error
                let _ = remove_file(&path);
                let _ = remove_file(&pid_path);
                Err(e)
            }
        }
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

        if let None = self.session {
            panic!("internal error: fuse_loop::main called before ::map_loop");
        }
        let mut session = self.session.take().unwrap().fuse();
        let mut abort_chan = abort_chan.fuse();

        let (loopdev_path, fuse_path) = (self.loopdev_path.clone(), self.fuse_path.clone());
        tokio::task::spawn_blocking(move || {
            if let Err(err) = loopdev::assign(loopdev_path, fuse_path) {
                let _ = startup_chan.try_send(Err(format_err!("error while assigning loop device - {}", err)));
            } else {
                // device is assigned successfully, which means not only is the
                // loopdev ready, but FUSE is also okay, since the assignment
                // would have failed otherwise
                let _ = startup_chan.try_send(Ok(()));
            }
        });

        let (loopdev_path, fuse_path, pid_path) =
            (self.loopdev_path.clone(), self.fuse_path.clone(), self.pid_path.clone());
        let cleanup = |session: futures::stream::Fuse<Fuse>| {
            // only warn for errors on cleanup, if these fail nothing is lost
            if let Err(err) = loopdev::unassign(&loopdev_path) {
                eprintln!(
                    "cleanup: warning: could not unassign file {} from loop device {} - {}",
                    &fuse_path,
                    &loopdev_path,
                    err,
                );
            }

            // force close FUSE handle before attempting to remove backing file
            std::mem::drop(session);

            if let Err(err) = remove_file(&fuse_path) {
                eprintln!(
                    "cleanup: warning: could not remove temporary file {} - {}",
                    &fuse_path,
                    err,
                );
            }
            if let Err(err) = remove_file(&pid_path) {
                eprintln!(
                    "cleanup: warning: could not remove PID file {} - {}",
                    &pid_path,
                    err,
                );
            }
        };

        loop {
            tokio::select!{
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
                            req.reply(&self.stat, std::f64::MAX)
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
                            eprintln!("Unimplemented FUSE request type encountered");
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

/// Try and unmap a running proxmox-backup-client instance from the given
/// /dev/loopN device
pub fn unmap(loopdev: String) -> Result<(), Error> {
    if loopdev.len() < 10 || !loopdev.starts_with("/dev/loop") {
        bail!("malformed loopdev path, must be in format '/dev/loopX'");
    }
    let num = loopdev.split_at(9).1.parse::<u8>().map_err(|err|
        format_err!("malformed loopdev path, does not end with valid number - {}", err))?;

    let block_path = PathBuf::from(format!("/sys/devices/virtual/block/loop{}/loop/backing_file", num));
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

    let mut pid_path = PathBuf::from(backing_file);
    pid_path.set_extension("pid");

    let pid_str = read_to_string(&pid_path).map_err(|err|
        format_err!("error reading pidfile {:?}: {}", &pid_path, err))?;
    let pid = pid_str.parse::<i32>().map_err(|err|
        format_err!("malformed PID ({}) in pidfile - {}", pid_str, err))?;

    // send SIGINT to trigger cleanup and exit in target process
    signal::kill(Pid::from_raw(pid), Signal::SIGINT)?;

    Ok(())
}

fn minimal_stat(size: i64) -> libc::stat {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    stat.st_mode = libc::S_IFREG;
    stat.st_ino = 1;
    stat.st_nlink = 1;
    stat.st_size = size;
    stat
}
