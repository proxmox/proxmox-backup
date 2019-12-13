//! Provides utilities to deal with futures, such as a `Cancellable` future.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use failure::Error;
use futures::future::FutureExt;
use tokio::sync::oneshot;

/// Make a future cancellable.
///
/// This simply performs a `select()` on the future and something waiting for a signal. If the
/// future finishes successfully, it yields `Some(T::Item)`. If it was cancelled, it'll yield
/// `None`.
///
/// In order to cancel the future, a `Canceller` is used.
///
/// ```no_run
/// # use std::future::Future;
/// # use failure::Error;
/// # use futures::future::FutureExt;
/// # use proxmox_backup::tools::futures::Cancellable;
/// # fn doc<T>(future: T) -> Result<(), Error>
/// # where
/// #     T: Future<Output = i32> + Unpin + Send + Sync + 'static,
/// # {
/// let (future, canceller) = Cancellable::new(future)?;
/// tokio::spawn(future.map(|res| {
///     match res {
///         Some(value) => println!("Future finished with {}", value),
///         None => println!("Future was cancelled"),
///     }
/// }));
/// // Do something
/// canceller.cancel();
/// # Ok(())
/// # }
/// ```
pub struct Cancellable<T: Future + Unpin> {
    /// Our core: we're waiting on a future, on on a lock. The cancel method just unlocks the
    /// lock, so that our LockFuture finishes.
    inner: futures::future::Select<T, oneshot::Receiver<()>>,

    /// When this future is created, this holds a guard. When a `Canceller` wants to cancel the
    /// future, it'll drop this guard, causing our inner future to resolve to `None`.
    sender: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

/// Reference to a cancellable future. Multiple instances may exist simultaneously.
///
/// This allows cancelling another future. If the future already finished, nothing happens.
///
/// This can be cloned to be used in multiple places.
#[derive(Clone)]
pub struct Canceller(Arc<Mutex<Option<oneshot::Sender<()>>>>);

impl Canceller {
    /// Cancel the associated future.
    ///
    /// This does nothing if the future already finished successfully.
    pub fn cancel(&self) {
        if let Some(sender) = self.0.lock().unwrap().take() {
            let _ = sender.send(());
        }
    }
}

impl<T: Future + Unpin> Cancellable<T> {
    /// Make a future cancellable.
    ///
    /// Returns a future and a `Canceller` which can be cloned and used later to cancel the future.
    pub fn new(inner: T) -> Result<(Self, Canceller), Error> {
        // we don't even need to store the mutex...
        let (tx, rx) = oneshot::channel();
        let this = Self {
            inner: futures::future::select(inner, rx),
            sender: Arc::new(Mutex::new(Some(tx))),
        };

        let canceller = this.canceller();
        Ok((this, canceller))
    }

    /// Create another `Canceller` for this future.
    pub fn canceller(&self) -> Canceller {
        Canceller(Arc::clone(&self.sender))
    }
}

/// Make a future cancellable.
///
/// This is a shortcut for `Cancellable::new`
pub fn cancellable<T: Future + Unpin>(future: T) -> Result<(Cancellable<T>, Canceller), Error> {
    Cancellable::new(future)
}

impl<T: Future + Unpin> Future for Cancellable<T> {
    type Output = Option<<T as Future>::Output>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        use futures::future::Either;
        match self.inner.poll_unpin(cx) {
            Poll::Ready(Either::Left((output, _))) => Poll::Ready(Some(output)),
            Poll::Ready(Either::Right(_)) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
