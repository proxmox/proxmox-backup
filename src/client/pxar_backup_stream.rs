use std::io::Write;
//use std::os::unix::io::FromRawFd;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::thread;

use anyhow::{format_err, Error};
use futures::stream::Stream;
use nix::dir::Dir;
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;

use crate::backup::CatalogWriter;

/// Stream implementation to encode and upload .pxar archives.
///
/// The hyper client needs an async Stream for file upload, so we
/// spawn an extra thread to encode the .pxar data and pipe it to the
/// consumer.
pub struct PxarBackupStream {
    rx: Option<std::sync::mpsc::Receiver<Result<Vec<u8>, Error>>>,
    child: Option<thread::JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
}

impl Drop for PxarBackupStream {
    fn drop(&mut self) {
        self.rx = None;
        self.child.take().unwrap().join().unwrap();
    }
}

impl PxarBackupStream {
    pub fn new<W: Write + Send + 'static>(
        dir: Dir,
        catalog: Arc<Mutex<CatalogWriter<W>>>,
        options: crate::pxar::PxarCreateOptions,
    ) -> Result<Self, Error> {
        let (tx, rx) = std::sync::mpsc::sync_channel(10);

        let buffer_size = 256 * 1024;

        let error = Arc::new(Mutex::new(None));
        let child = std::thread::Builder::new()
            .name("PxarBackupStream".to_string())
            .spawn({
                let error = Arc::clone(&error);
                move || {
                    let mut catalog_guard = catalog.lock().unwrap();
                    let writer = std::io::BufWriter::with_capacity(
                        buffer_size,
                        crate::tools::StdChannelWriter::new(tx),
                    );

                    let verbose = options.verbose;

                    let writer = pxar::encoder::sync::StandardWriter::new(writer);
                    if let Err(err) = crate::pxar::create_archive(
                        dir,
                        writer,
                        crate::pxar::Flags::DEFAULT,
                        |path| {
                            if verbose {
                                println!("{:?}", path);
                            }
                            Ok(())
                        },
                        Some(&mut *catalog_guard),
                        options,
                    ) {
                        let mut error = error.lock().unwrap();
                        *error = Some(err.to_string());
                    }
                }
            })?;

        Ok(Self {
            rx: Some(rx),
            child: Some(child),
            error,
        })
    }

    pub fn open<W: Write + Send + 'static>(
        dirname: &Path,
        catalog: Arc<Mutex<CatalogWriter<W>>>,
        options: crate::pxar::PxarCreateOptions,
    ) -> Result<Self, Error> {
        let dir = nix::dir::Dir::open(dirname, OFlag::O_DIRECTORY, Mode::empty())?;

        Self::new(
            dir,
            catalog,
            options,
        )
    }
}

impl Stream for PxarBackupStream {
    type Item = Result<Vec<u8>, Error>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Option<Self::Item>> {
        {
            // limit lock scope
            let error = self.error.lock().unwrap();
            if let Some(ref msg) = *error {
                return Poll::Ready(Some(Err(format_err!("{}", msg))));
            }
        }

        match crate::tools::runtime::block_in_place(|| self.rx.as_ref().unwrap().recv()) {
            Ok(data) => Poll::Ready(Some(data)),
            Err(_) => {
                let error = self.error.lock().unwrap();
                if let Some(ref msg) = *error {
                    return Poll::Ready(Some(Err(format_err!("{}", msg))));
                }
                Poll::Ready(None) // channel closed, no error
            }
        }
    }
}
