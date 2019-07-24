use failure::*;

use std::thread;
use std::sync::{Arc, Mutex};
use std::os::unix::io::FromRawFd;
use std::path::{Path, PathBuf};
use std::collections::HashSet;

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
pub struct PxarBackupStream {
    stream: Option<WrappedReaderStream<std::fs::File>>,
    child: Option<thread::JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
}

impl Drop for PxarBackupStream {

    fn drop(&mut self) {
        self.stream = None;
        self.child.take().unwrap().join().unwrap();
    }
}

impl PxarBackupStream {

    pub fn new(mut dir: Dir, path: PathBuf, device_set: Option<HashSet<u64>>, verbose: bool) -> Result<Self, Error> {

        let (rx, tx) = nix::unistd::pipe()?;

        let buffer_size = 1024*1024;
        nix::fcntl::fcntl(rx, nix::fcntl::FcntlArg::F_SETPIPE_SZ(buffer_size as i32))?;

        let error = Arc::new(Mutex::new(None));
        let error2 = error.clone();

        let child = thread::spawn(move|| {
            let mut writer = unsafe { std::fs::File::from_raw_fd(tx) };
            if let Err(err) = pxar::Encoder::encode(path, &mut dir, &mut writer, device_set, verbose, pxar::CA_FORMAT_DEFAULT) {
                eprintln!("pxar encode failed - {}", err);
                let mut error = error2.lock().unwrap();
                *error = Some(err.to_string());
            }
        });

        let pipe = unsafe { std::fs::File::from_raw_fd(rx) };
        let stream = crate::tools::wrapped_reader_stream::WrappedReaderStream::new(pipe);

        Ok(Self {
            stream: Some(stream),
            child: Some(child),
            error,
        })
    }

    pub fn open(dirname: &Path, device_set: Option<HashSet<u64>>, verbose: bool) -> Result<Self, Error> {

        let dir = nix::dir::Dir::open(dirname, OFlag::O_DIRECTORY, Mode::empty())?;
        let path = std::path::PathBuf::from(dirname);

        Self::new(dir, path, device_set, verbose)
    }
}

impl Stream for PxarBackupStream {

    type Item = Vec<u8>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Vec<u8>>, Error> {
        let error = self.error.lock().unwrap();
        if let Some(ref msg) = *error {
            return Err(format_err!("{}", msg));
        }
        self.stream.as_mut().unwrap().poll().map_err(Error::from)
    }
}
