//! Some types used by both our internal testing AioContext implementation as well as our
//! WithAioContext wrapper.

/// An Aio Callback. Qemu's AioContext actually uses a void function taking an opaque pointer.
/// For simplicity we stick to closures for now.
pub type AioCb = std::sync::Arc<dyn Fn() + Send + Sync>;

/// This keeps track of our poll state (whether we wait to be notified for read or write
/// readiness.)
#[derive(Default)]
pub struct AioHandlerState {
    pub read: Option<AioCb>,
    pub write: Option<AioCb>,
}

impl AioHandlerState {
    /// Get an mio::Ready with readable set if `read` is `Some`, and writable
    /// set if `write` is `Some`.
    pub fn mio_ready(&self) -> mio::Ready {
        use mio::Ready;

        let mut ready = Ready::empty();
        if self.read.is_some() {
            ready |= Ready::readable();
        }

        if self.write.is_some() {
            ready |= Ready::writable();
        }

        ready
    }

    /// Shortcut
    pub fn clear(&mut self) {
        self.read = None;
        self.write = None;
    }
}
