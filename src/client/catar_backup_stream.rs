use failure::*;

use std::thread;
use std::os::unix::io::FromRawFd;
use std::path::{Path, PathBuf};

use futures::{Async, Poll};
use futures::stream::Stream;

use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use nix::dir::Dir;

use crate::catar::encoder::*;

/// Stream implementation to encode and upload .catar archives.
///
/// The hyper client needs an async Stream for file upload, so we
/// spawn an extra thread to encode the .catar data and pipe it to the
/// consumer.
///
/// Note: The currect implementation is not fully ansync and can block.
pub struct CaTarBackupStream {
    pipe: Option<std::fs::File>,
    buffer: Vec<u8>,
    child: Option<thread::JoinHandle<()>>,
}

impl Drop for CaTarBackupStream {

    fn drop(&mut self) {
        drop(self.pipe.take());
        self.child.take().unwrap().join().unwrap();
    }
}

impl CaTarBackupStream {

    pub fn new(mut dir: Dir, path: PathBuf, all_file_systems: bool, verbose: bool) -> Result<Self, Error> {
        let mut buffer = Vec::with_capacity(4096);
        unsafe { buffer.set_len(buffer.capacity()); }

        let (rx, tx) = nix::unistd::pipe()?;

        let child = thread::spawn(move|| {
            let mut writer = unsafe { std::fs::File::from_raw_fd(tx) };
             if let Err(err) = CaTarEncoder::encode(path, &mut dir, &mut writer, all_file_systems, verbose) {
                eprintln!("catar encode failed - {}", err);
            }
        });

        let pipe = unsafe { std::fs::File::from_raw_fd(rx) };

        Ok(Self { pipe: Some(pipe), buffer, child: Some(child) })
    }

    pub fn open(dirname: &Path,  all_file_systems: bool, verbose: bool) -> Result<Self, Error> {

        let dir = nix::dir::Dir::open(dirname, OFlag::O_DIRECTORY, Mode::empty())?;
        let path = std::path::PathBuf::from(dirname);

        Self::new(dir, path, all_file_systems, verbose)
    }
}

impl Stream for CaTarBackupStream {

    type Item = Vec<u8>;
    type Error = Error;

    // Note: This is not async!!

    fn poll(&mut self) -> Poll<Option<Vec<u8>>, Error> {

        use std::io::Read;

        loop {
            let pipe = match self.pipe {
                Some(ref mut pipe) => pipe,
                None => unreachable!(),
            };
            match pipe.read(&mut self.buffer) {
                Ok(n) => {
                    if n == 0 {
                        return Ok(Async::Ready(None))
                    } else {
                        let data = self.buffer[..n].to_vec();
                        return Ok(Async::Ready(Some(data)))
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    // try again
                }
                Err(err) => {
                    return Err(err.into())
                }
            };
        }
    }
}
