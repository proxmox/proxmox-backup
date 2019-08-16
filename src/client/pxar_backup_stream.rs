use failure::*;
use std::io::{Write, Seek};
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
use crate::backup::CatalogBlobWriter;

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

    pub fn new<W: Write + Seek + Send + 'static>(
        mut dir: Dir,
        path: PathBuf,
        device_set: Option<HashSet<u64>>,
        verbose: bool,
        skip_lost_and_found: bool,
        catalog: Arc<Mutex<CatalogBlobWriter<W>>>,
    ) -> Result<Self, Error> {

        let (rx, tx) = nix::unistd::pipe()?;

        let buffer_size = 1024*1024;
        nix::fcntl::fcntl(rx, nix::fcntl::FcntlArg::F_SETPIPE_SZ(buffer_size as i32))?;

        let error = Arc::new(Mutex::new(None));
        let error2 = error.clone();

        let catalog = catalog.clone();
        let child = thread::spawn(move || {
            let mut guard = catalog.lock().unwrap();
            let mut writer = unsafe { std::fs::File::from_raw_fd(tx) };
            if let Err(err) = pxar::Encoder::encode(path, &mut dir, &mut writer, Some(&mut *guard), device_set, verbose, skip_lost_and_found, pxar::flags::DEFAULT) {
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

    pub fn open<W: Write + Seek + Send + 'static>(
        dirname: &Path,
        device_set: Option<HashSet<u64>>,
        verbose: bool,
        skip_lost_and_found: bool,
        catalog: Arc<Mutex<CatalogBlobWriter<W>>>,
    ) -> Result<Self, Error> {

        let dir = nix::dir::Dir::open(dirname, OFlag::O_DIRECTORY, Mode::empty())?;
        let path = std::path::PathBuf::from(dirname);

        Self::new(dir, path, device_set, verbose, skip_lost_and_found, catalog)
    }
}

impl Stream for PxarBackupStream {

    type Item = Vec<u8>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Vec<u8>>, Error> {
        { // limit lock scope
            let error = self.error.lock().unwrap();
            if let Some(ref msg) = *error {
                return Err(format_err!("{}", msg));
            }
        }
        self.stream.as_mut().unwrap().poll().map_err(Error::from)
    }
}
