use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use failure::Error;
use futures::future::FutureExt;
use tokio::sync::Lock as TokioLock;

pub use tokio::sync::LockGuard as AsyncLockGuard;

pub struct AsyncMutex<T: Send>(TokioLock<T>);

unsafe impl<T: Send> Sync for AsyncMutex<T> {}

impl<T: Send + 'static> AsyncMutex<T> {
    pub fn new(value: T) -> Self {
        Self(TokioLock::new(value))
    }

    pub fn lock(&self) -> LockFuture<T> {
        let mut lock = self.0.clone();
        LockFuture {
            lock: async move { lock.lock().await }.boxed(),
        }
    }

    // FIXME: remove Result<> from this.
    pub fn new_locked(value: T) -> Result<(Self, AsyncLockGuard<T>), Error> {
        let mut this = Self::new(value);
        let guard = futures::executor::block_on(this.0.lock());
        Ok((this, guard))
    }
}

/// Represents a lock to be held in the future:
pub struct LockFuture<T: Send + 'static> {
    lock: Pin<Box<dyn Future<Output = AsyncLockGuard<T>> + Send + 'static>>,
}

impl<T: Send + 'static> Future for LockFuture<T> {
    type Output = AsyncLockGuard<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<AsyncLockGuard<T>> {
        self.lock.poll_unpin(cx)
    }
}
