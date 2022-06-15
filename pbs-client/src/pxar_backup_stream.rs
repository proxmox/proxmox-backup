use std::io::Write;
//use std::os::unix::io::FromRawFd;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use anyhow::{format_err, Error};
use futures::future::{AbortHandle, Abortable};
use futures::stream::Stream;
use nix::dir::Dir;
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;

use proxmox_async::blocking::TokioWriterAdapter;
use proxmox_io::StdChannelWriter;

use pbs_datastore::catalog::CatalogWriter;

/// Stream implementation to encode and upload .pxar archives.
///
/// The hyper client needs an async Stream for file upload, so we
/// spawn an extra thread to encode the .pxar data and pipe it to the
/// consumer.
pub struct PxarBackupStream {
    rx: Option<std::sync::mpsc::Receiver<Result<Vec<u8>, Error>>>,
    handle: Option<AbortHandle>,
    error: Arc<Mutex<Option<String>>>,
}

impl Drop for PxarBackupStream {
    fn drop(&mut self) {
        self.rx = None;
        self.handle.take().unwrap().abort();
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
        let error2 = Arc::clone(&error);
        let handler = async move {
            let writer = TokioWriterAdapter::new(std::io::BufWriter::with_capacity(
                buffer_size,
                StdChannelWriter::new(tx),
            ));

            let writer = pxar::encoder::sync::StandardWriter::new(writer);
            if let Err(err) = crate::pxar::create_archive(
                dir,
                writer,
                crate::pxar::Flags::DEFAULT,
                move |path| {
                    log::debug!("{:?}", path);
                    Ok(())
                },
                Some(catalog),
                options,
            )
            .await
            {
                let mut error = error2.lock().unwrap();
                *error = Some(err.to_string());
            }
        };

        let (handle, registration) = AbortHandle::new_pair();
        let future = Abortable::new(handler, registration);
        tokio::spawn(future);

        Ok(Self {
            rx: Some(rx),
            handle: Some(handle),
            error,
        })
    }

    pub fn open<W: Write + Send + 'static>(
        dirname: &Path,
        catalog: Arc<Mutex<CatalogWriter<W>>>,
        options: crate::pxar::PxarCreateOptions,
    ) -> Result<Self, Error> {
        let dir = nix::dir::Dir::open(dirname, OFlag::O_DIRECTORY, Mode::empty())?;

        Self::new(dir, catalog, options)
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

        match proxmox_async::runtime::block_in_place(|| self.rx.as_ref().unwrap().recv()) {
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
