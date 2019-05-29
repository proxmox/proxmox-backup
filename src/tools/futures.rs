//! Provides utilities to deal with futures, such as a `Cancellable` future.

use std::sync::{Arc, Mutex};

use failure::Error;
use futures::{Async, Future, Poll};

use crate::tools::async_mutex::{AsyncMutex, AsyncLockGuard, LockFuture};

/// Make a future cancellable.
///
/// This simply performs a `select()` on the future and something waiting for a signal. If the
/// future finishes successfully, it yields `Some(T::Item)`. If it was cancelled, it'll yield
/// `None`.
///
/// In order to cancel the future, a `Canceller` is used.
///
/// ```no_run
/// # use failure::Error;
/// # use futures::Future;
/// # fn doc<T: Future<Item = i32, Error = Error>>(future: T) {
/// let cancel = Cancellable::new(future);
/// let canceller = cancel.canceller(); // This is clonable!
/// tokio::spawn(cancel.and_then(|res| match res {
///     Some(value) => println!("Future finished with {}", value),
///     None => println!("Future was cancelled"),
/// });
/// // Do something
/// canceller.cancel();
/// # }
/// ```
pub struct Cancellable<T: Future> {
    /// Our core: we're waiting on a future, on on a lock. The cancel method just unlocks the
    /// lock, so that our LockFuture finishes.
    inner: futures::future::Select2<T, LockFuture<(), <T as Future>::Error>>,

    /// When this future is created, this holds a guard. When a `Canceller` wants to cancel the
    /// future, it'll drop this guard, causing our inner future to resolve to `None`.
    guard: Arc<Mutex<Option<AsyncLockGuard<()>>>>,
}

/// Reference to a cancellable future. Multiple instances may exist simultaneously.
///
/// This allows cancelling another future. If the future already finished, nothing happens.
#[derive(Clone)]
pub struct Canceller(Arc<Mutex<Option<AsyncLockGuard<()>>>>);

impl Canceller {
    /// Cancel the associated future.
    ///
    /// This does nothing if the future already finished successfully.
    pub fn cancel(&self) {
        *self.0.lock().unwrap() = None;
    }
}

impl<T: Future> Cancellable<T> {
    /// Make a future cancellable.
    ///
    /// Returns a future and a `Canceller` which can be cloned and used later to cancel the future.
    pub fn new(inner: T) -> Result<(Self, Canceller), Error> {
        // we don't even need to sture the mutex...
        let (mutex, guard) = AsyncMutex::new_locked(())?;
        let this = Self {
            inner: inner.select2(mutex.lock()),
            guard: Arc::new(Mutex::new(Some(guard))),
        };
        let canceller = this.canceller();
        Ok((this, canceller))
    }

    /// Create another `Canceller` for his future..
    pub fn canceller(&self) -> Canceller {
        Canceller(self.guard.clone())
    }
}

/// Make a future cancellable.
///
/// This is a shortcut for `Cancellable::new`
pub fn cancellable<T: Future>(future: T) -> Result<(Cancellable<T>, Canceller), Error> {
    Cancellable::new(future)
}

impl<T: Future> Future for Cancellable<T> {
    type Item = Option<<T as Future>::Item>;
    type Error = <T as Future>::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        use futures::future::Either;
        match self.inner.poll() {
            Ok(Async::Ready(Either::A((item, _)))) => Ok(Async::Ready(Some(item))),
            Ok(Async::Ready(Either::B(_))) => Ok(Async::Ready(None)),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(Either::A((err, _))) => Err(err),
            Err(Either::B((err, _))) => Err(err),
        }
    }
}
