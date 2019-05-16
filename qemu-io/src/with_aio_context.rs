//! This module provides `WithAioContext`, which is a helper to connect any raw I/O file descriptor
//! (`T: AsRawFd`) with an `AioContext`.

use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};

use mio::Ready;

use crate::AioContext;
use crate::util::{AioCb, AioHandlerState};

/// This provides a basic mechanism to connect a type containing a file descriptor (i.e. it
/// implements `AsRawFd`) to an `AioContext`.
///
/// If the underlying type implements `Read` this wrapper also provides an `AsyncRead`
/// implementation. Likewise it'll provide `AsyncWrite` for types implementing `Write`.
/// For this to function properly, the underlying type needs to return `io::Error` of kind
/// `io::ErrorKind::WouldBlock` on blocking operations which should be retried when the file
/// descriptor becomes ready.
///
/// `WithAioContext` _owns_ the underlying object. This is because our Drop handler wants to
/// unregister the file descriptor, but systems like linux' epoll do that automatically when the fd
/// is closed, so we cannot have our file descriptor vanish before de-registering it, otherwise we
/// may be de-registering an already re-used number.
///
/// Implements `Deref<T>` so any methods of `T` still work on a `WithAioContext<T>`.
pub struct WithAioContext<T: AsRawFd> {
    aio_context: AioContext,
    fd: RawFd,
    handlers: Arc<Mutex<AioHandlerState>>,
    inner: Option<T>,
}

impl<T: AsRawFd> std::ops::Deref for WithAioContext<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap()
    }
}

impl<T: AsRawFd> std::ops::DerefMut for WithAioContext<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().unwrap()
    }
}

impl<T: AsRawFd> WithAioContext<T> {
    pub fn new(aio_context: AioContext, inner: T) -> Self {
        Self {
            aio_context,
            fd: inner.as_raw_fd(),
            handlers: Arc::new(Mutex::new(Default::default())),
            inner: Some(inner),
        }
    }

    /// Deregister from the `AioContext` and return the inner file handle.
    pub fn into_inner(mut self) -> T {
        let out = self.inner.take().unwrap();
        std::mem::drop(self);
        out
    }

    /// Shortcut around the `unwrap()`. The `Option<>` around `inner` is only there because we have
    /// a `Drop` implementation which prevents us to move-out the value in the `into_inner()`
    /// method.
    fn inner_mut(&mut self) -> &mut T {
        self.inner.as_mut().unwrap()
    }

    /// Shortcut around the `unwrap()`, immutable variant:
    //fn inner(&self) -> &T {
    //    self.inner.as_ref().unwrap()
    //}

    /// Shortcut to set_fd_handlers. For the "real" qemu interface we'll have to turn the closures
    /// into raw function pointers here (they'll get an opaque pointer parameter).
    fn commit_handlers(
        aio_context: &AioContext,
        fd: RawFd,
        handlers: &mut MutexGuard<AioHandlerState>,
    ) {
        aio_context.set_fd_handler(
            fd,
            handlers.read.as_ref().map(|x| (*x).clone()),
            handlers.write.as_ref().map(|x| (*x).clone()),
        )
    }

    /// Create a waker closure for a context for a specific ready state. When a file descriptor is
    /// ready for reading or writing, we need to remove the corresponding handler from the
    /// `AioContext` (make it an edge-trigger instead of a level trigger) before finally calling
    /// `waker.wake_by_ref()` to queue the task for polling.
    fn make_wake_fn(&self, cx: &mut Context, ready: Ready) -> AioCb {
        let waker = cx.waker().clone();

        // we don't want to be publicly clonable so clone manually here:
        let aio_context = self.aio_context.clone();
        let fd = self.fd;
        let handlers = Arc::clone(&self.handlers);
        Arc::new(move || {
            let mut guard = handlers.lock().unwrap();

            if ready.is_readable() {
                guard.read = None;
            }

            if ready.is_writable() {
                guard.write = None;
            }

            Self::commit_handlers(&aio_context, fd, &mut guard);
            waker.wake_by_ref();
        })
    }

    /// Register our file descriptor with the `AioContext` for reading or writing.
    /// This only affects the directions present in the provided `ready` value, and will leave the
    /// other directions unchanged.
    pub fn register(&self, cx: &mut Context, ready: Ready) {
        let mut guard = self.handlers.lock().unwrap();

        if ready.is_readable() {
            guard.read = Some(self.make_wake_fn(cx, ready));
        }

        if ready.is_writable() {
            guard.write = Some(self.make_wake_fn(cx, ready));
        }

        Self::commit_handlers(&self.aio_context, self.fd, &mut guard)
    }

    /// Helper to handle an `io::Result<T>`, turning `Result<T>` into `Poll<Result<T>>`, by
    /// changing an `io::ErrorKind::WouldBlock` into `Poll::Pending` and taking care of registering
    /// the file descriptor with the AioContext for the next wake-up.
    /// `Ok` and errors other than the above will be passed through wrapped in `Poll::Ready`.
    pub fn handle_aio_result<R>(
        &self,
        cx: &mut Context,
        result: io::Result<R>,
        ready: Ready,
    ) -> Poll<io::Result<R>> {
        match result {
            Ok(res) => Poll::Ready(Ok(res)),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                self.register(cx, ready);
                Poll::Pending
            }
            Err(err) => Poll::Ready(Err(err)),
        }
    }
}

impl<T: AsRawFd> Drop for WithAioContext<T> {
    fn drop(&mut self) {
        let mut guard = self.handlers.lock().unwrap();
        (*guard).clear();
        if !guard.mio_ready().is_empty() {
            Self::commit_handlers(&self.aio_context, self.fd, &mut guard);
        }
    }
}

impl<T> futures::io::AsyncRead for WithAioContext<T>
where
    T: AsRawFd + io::Read + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let res = self.inner_mut().read(buf);
        self.handle_aio_result(cx, res, mio::Ready::readable())
    }
}

impl<T> futures::io::AsyncWrite for WithAioContext<T>
where
    T: AsRawFd + io::Write + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = self.inner_mut().write(buf);
        self.handle_aio_result(cx, result, mio::Ready::writable())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        let result = self.inner_mut().flush();
        self.handle_aio_result(cx, result, mio::Ready::writable())
    }

    // I'm not sure what they expect me to do here. The `close()` syscall has no async variant, so
    // all I can do is `flush()` and then drop the inner stream...
    //
    // Using `.into_inner()` after this will cause a panic.
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        let result = self.inner_mut().flush();
        let _ = futures::ready!(self.handle_aio_result(cx, result, mio::Ready::writable()));
        std::mem::drop(self.inner.take());
        Poll::Ready(Ok(()))
    }
}
