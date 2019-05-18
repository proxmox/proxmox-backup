use failure::*;

use std::thread;
use std::os::unix::io::FromRawFd;
use std::path::{Path, PathBuf};

use futures::Poll;
use futures::stream::Stream;

use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use nix::dir::Dir;

use crate::pxar;
use crate::tools::wrapped_reader_stream::WrappedReaderStream;

/// Stream implementation to encode and upload .pxar archives.
///
/// The hyper client needs an async Stream for file upload, so we
/// spawn an extra thread to encode the .pxar data and pipe it to the
/// consumer.
///
/// Note: The currect implementation is not fully ansync and can block.
pub struct PxarBackupStream {
    stream: WrappedReaderStream<std::fs::File>,
    child: Option<thread::JoinHandle<()>>,
}

impl Drop for PxarBackupStream {

    fn drop(&mut self) {
        self.child.take().unwrap().join().unwrap();
    }
}

impl PxarBackupStream {

    pub fn new(mut dir: Dir, path: PathBuf, all_file_systems: bool, verbose: bool) -> Result<Self, Error> {

        let (rx, tx) = nix::unistd::pipe()?;

        let buffer_size = 1024*1024;
        nix::fcntl::fcntl(rx, nix::fcntl::FcntlArg::F_SETPIPE_SZ(buffer_size as i32))?;

        let child = thread::spawn(move|| {
            let mut writer = unsafe { std::fs::File::from_raw_fd(tx) };
            if let Err(err) = pxar::Encoder::encode(path, &mut dir, &mut writer, all_file_systems, verbose) {
                eprintln!("pxar encode failed - {}", err);
            }
        });

        let pipe = unsafe { std::fs::File::from_raw_fd(rx) };
        let stream = crate::tools::wrapped_reader_stream::WrappedReaderStream::new(pipe);

        Ok(Self { stream, child: Some(child) })
    }

    pub fn open(dirname: &Path,  all_file_systems: bool, verbose: bool) -> Result<Self, Error> {

        let dir = nix::dir::Dir::open(dirname, OFlag::O_DIRECTORY, Mode::empty())?;
        let path = std::path::PathBuf::from(dirname);

        Self::new(dir, path, all_file_systems, verbose)
    }
}

impl Stream for PxarBackupStream {

    type Item = Vec<u8>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Vec<u8>>, Error> {
        self.stream.poll().map_err(Error::from)
    }
}
