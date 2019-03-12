//! signalfd handling for tokio, with some re-exports for convenience

use std::os::unix::io::{AsRawFd, RawFd};

use failure::*;
use nix::sys::signalfd;
use tokio::prelude::*;
use tokio::reactor::PollEvented2;

pub use nix::sys::signal::{SigSet, Signal};

/// Wrapper for `nix::sys::signal::SignalFd` to provide an async `Stream` of `siginfo`.
pub struct SignalFd {
    inner: signalfd::SignalFd,
    pinned_fd: Box<RawFd>,
    wakeup: Option<PollEvented2<mio::unix::EventedFd<'static>>>,
}

impl std::ops::Deref for SignalFd {
    type Target = signalfd::SignalFd;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::ops::DerefMut for SignalFd {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl SignalFd {
    pub fn new(mask: &SigSet) -> Result<Self, Error> {
        let inner = signalfd::SignalFd::with_flags(
            mask,
            signalfd::SfdFlags::SFD_CLOEXEC | signalfd::SfdFlags::SFD_NONBLOCK,
        )?;

        // EventedFd takes a reference and therefore a lifetime parameter. Since we want to
        // reference something that is part of our own Self, we need to find a work around:
        // Pin the file descriptor in memory by boxing it and fake a &'static lifetime.
        //
        // Note that we must not provide access to this lifetime to the outside!
        let pinned_fd = Box::new(inner.as_raw_fd());
        let fd_ptr: *const RawFd = &*pinned_fd;
        let static_fd: &'static RawFd = unsafe { &*fd_ptr };
        let evented = mio::unix::EventedFd(static_fd);

        let wakeup = Some(PollEvented2::new(evented));

        Ok(Self {
            inner,
            pinned_fd,
            wakeup,
        })
    }
}

impl Stream for SignalFd {
    type Item = signalfd::siginfo;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let ready = mio::Ready::readable();

        match self.wakeup.as_mut().unwrap().poll_read_ready(ready) {
            Ok(Async::Ready(_)) => (), // go on
            Ok(Async::NotReady) => return Ok(Async::NotReady),
            Err(e) => return Err(e.into()),
        }

        match self.inner.read_signal() {
            Ok(Some(signal)) => Ok(Async::Ready(Some(signal))),
            Ok(None) => {
                self.wakeup.as_mut().unwrap().clear_read_ready(ready)?;
                Ok(Async::NotReady)
            }
            Err(e) => Err(e.into()),
        }
    }
}

impl AsRawFd for SignalFd {
    fn as_raw_fd(&self) -> RawFd {
        *self.pinned_fd
    }
}

impl Drop for SignalFd {
    fn drop(&mut self) {
        self.wakeup = None; // enforce drop order
    }
}
