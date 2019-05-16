//! This implements the parts of qemu's AioContext interface we need for testing outside qemu.

use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;

use failure::Error;
use mio::{Events, Poll, Token};
use mio::unix::EventedFd;

use crate::util::{AioCb, AioHandlerState};

/// This is a reference to a standalone `AioContextImpl` and allows instantiating a new context
/// with a polling thread.
#[derive(Clone)]
#[repr(transparent)]
pub struct AioContext(Arc<AioContextImpl>);

impl std::ops::Deref for AioContext {
    type Target = AioContextImpl;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl AioContext {
    /// Create a new `AioContext` instance with an associated polling thread, which will live as
    /// long as there are references to it.
    pub fn new() -> Result<Self, Error> {
        Ok(Self(AioContextImpl::new()?))
    }
}

pub struct AioContextImpl {
    poll: Poll,
    handlers: RwLock<HashMap<Token, AioHandlerState>>,
    poll_thread: Mutex<Option<thread::JoinHandle<()>>>,
}

impl AioContextImpl {
    pub fn new() -> Result<Arc<Self>, Error> {
        let this = Arc::new(Self {
            poll: Poll::new()?,
            handlers: RwLock::new(HashMap::new()),
            poll_thread: Mutex::new(None),
        });

        let this2 = Arc::clone(&this);
        this.poll_thread.lock().unwrap().replace(thread::spawn(|| this2.main_loop()));

        Ok(this)
    }

    /// Qemu's aio_set_fd_handler. We're skipping the `io_poll` parameter for this implementation
    /// as we don't use it.
    /// ```
    /// void aio_set_fd_handler(AioContext *ctx,
    ///                         int fd,
    ///                         bool is_external,
    ///                         IOHandler *io_read,
    ///                         IOHandler *io_write,
    ///                         AioPollFn *io_poll,
    ///                         void *opaque);
    /// ```
    ///
    /// Since this does not have any ways of returning errors, wrong usage will cause a panic in
    /// this test implementation.
    pub fn set_fd_handler(
        &self,
        fd: RawFd,
        io_read: Option<AioCb>,
        io_write: Option<AioCb>,
        // skipping io_poll,
        //opaque: *const (),
    ) {
        self.set_fd_handler_impl(fd, io_read, io_write, mio::PollOpt::level())
    }

    /// This is going to be a proposed new api for Qemu's AioContext.
    pub fn set_fd_handler_edge(
        &self,
        fd: RawFd,
        io_read: Option<AioCb>,
        io_write: Option<AioCb>,
        // skipping io_poll,
        //opaque: *const (),
    ) {
        self.set_fd_handler_impl(fd, io_read, io_write, mio::PollOpt::edge())
    }

    fn set_fd_handler_impl(
        &self,
        fd: RawFd,
        io_read: Option<AioCb>,
        io_write: Option<AioCb>,
        // skipping io_poll,
        //opaque: *const (),
        poll_opt: mio::PollOpt,
    ) {
        if io_read.is_none() && io_write.is_none() {
            return self.remove_fd_handler(fd);
        }

        let handlers = AioHandlerState {
            read: io_read,
            write: io_write,
        };

        let mio_ready = handlers.mio_ready();

        let token = Token(fd as usize);

        use std::collections::hash_map::Entry;
        match self.handlers.write().unwrap().entry(token) {
            Entry::Vacant(entry) => {
                self.poll.register(&EventedFd(&fd), token, mio_ready, poll_opt)
                    .expect("failed to register a new fd for polling");
                entry.insert(handlers);
            }
            Entry::Occupied(mut entry) => {
                self.poll.reregister(&EventedFd(&fd), token, mio_ready, poll_opt)
                    .expect("failed to update an existing poll fd");
                entry.insert(handlers);
            }
        }
    }

    fn remove_fd_handler(&self, fd: RawFd) {
        let mut guard = self.handlers.write().unwrap();
        self.poll.deregister(&EventedFd(&fd))
            .expect("failed to remove an existing poll fd");
        guard.remove(&Token(fd as usize));
    }

    /// We don't use qemu's aio_poll, so let's make this easy:
    ///
    /// ```
    /// bool aio_poll(AioContext *ctx, bool blocking);
    /// ```
    pub fn poll(&self) -> Result<(), Error> {
        let timeout = Some(std::time::Duration::from_millis(100));

        let mut events = Events::with_capacity(16);

        if self.poll.poll(&mut events, timeout)? == 0 {
            return Ok(());
        }

        for event in events.iter() {
            let token = event.token();
            let ready = event.readiness();
            // NOTE: We need to read-lock while fetching handlers, but handlers need a write-lock!!!
            // because they need to be edge-triggered and therefore *update* this handler list!
            //
            // While we could instead do this here (or use edge triggering from mio), this would
            // not properly simulate Qemu's AioContext, so we enforce this behavior here as well.
            //
            // This means we cannot just hold a read lock during the events.iter() iteration
            // though.
            let handler = self.handlers.read().unwrap().get(&token).map(|h| AioHandlerState {
                // Those are Option<Arc>!
                read: h.read.clone(),
                write: h.write.clone(),
            });
            if let Some(handler) = handler {
                if ready.is_readable() {
                    handler.read.as_ref().map(|func| func());
                }
                if ready.is_writable() {
                    handler.write.as_ref().map(|func| func());
                }
            }
        }

        Ok(())
    }

    fn main_loop(mut self: Arc<Self>) {
        while Arc::get_mut(&mut self).is_none() {
            if let Err(err) = self.poll() {
                dbg!("error AioContextImpl::poll(): {}", err);
                break;
            }
        }
    }
}
