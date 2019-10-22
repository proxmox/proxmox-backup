use std::collections::HashSet;
use std::io::{Seek, Write};
use std::os::unix::io::FromRawFd;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::thread;

use failure::*;
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
    pin_utils::unsafe_pinned!(stream: Option<WrappedReaderStream<std::fs::File>>);

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
        let exclude_pattern = Vec::new();
        let child = thread::spawn(move || {
            let mut guard = catalog.lock().unwrap();
            let mut writer = unsafe { std::fs::File::from_raw_fd(tx) };
            if let Err(err) = pxar::Encoder::encode(
                path,
                &mut dir,
                &mut writer,
                Some(&mut *guard),
                device_set,
                verbose,
                skip_lost_and_found,
                pxar::flags::DEFAULT,
                exclude_pattern,
            ) {
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

    type Item = Result<Vec<u8>, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        { // limit lock scope
            let error = self.error.lock().unwrap();
            if let Some(ref msg) = *error {
                return Poll::Ready(Some(Err(format_err!("{}", msg))));
            }
        }
        let res = self.as_mut()
            .stream()
            .as_pin_mut()
            .unwrap()
            .poll_next(cx);
        Poll::Ready(futures::ready!(res)
            .map(|v| v.map_err(Error::from))
        )
    }
}
