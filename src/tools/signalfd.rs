//! signalfd handling for tokio

use std::os::unix::io::{AsRawFd, RawFd};

use failure::*;
use nix::sys::signalfd::{self, SigSet};
use tokio::prelude::*;
use tokio::reactor::PollEvented2;

type Result<T> = std::result::Result<T, Error>;

pub struct SignalFd {
    inner: signalfd::SignalFd,
    pinned_fd: Box<RawFd>,
    wakeup: PollEvented2<mio::unix::EventedFd<'static>>,
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
    pub fn new(mask: &SigSet) -> Result<Self> {
        let inner = signalfd::SignalFd::with_flags(
            mask,
            signalfd::SfdFlags::SFD_CLOEXEC | signalfd::SfdFlags::SFD_NONBLOCK,
        )?;

        // box the signalfd's Rawfd, turn it into a raw pointer and create a &'static reference so
        // we can store it inthe SignalFd struct...
        let pinned_fd = Box::new(inner.as_raw_fd());
        let fd_ptr: *const RawFd = &*pinned_fd;
        let static_fd: &'static RawFd = unsafe { &*fd_ptr };
        let evented = mio::unix::EventedFd(static_fd);

        let wakeup = PollEvented2::new(evented);

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

        match self.wakeup.poll_read_ready(ready) {
            Ok(Async::Ready(_)) => (), // go on
            Ok(Async::NotReady) => return Ok(Async::NotReady),
            Err(e) => return Err(e.into()),
        }

        match self.inner.read_signal() {
            Ok(Some(signal)) => Ok(Async::Ready(Some(signal))),
            Ok(None) => {
                self.wakeup.clear_read_ready(ready)?;
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
