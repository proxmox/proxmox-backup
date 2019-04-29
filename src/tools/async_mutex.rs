use std::marker::PhantomData;

use futures::Poll;
use futures::future::Future;
use tokio::sync::lock::Lock as TokioLock;
pub use tokio::sync::lock::LockGuard as AsyncLockGuard;

pub struct AsyncMutex<T>(TokioLock<T>);

unsafe impl<T> Sync for AsyncMutex<T> {}

impl<T> AsyncMutex<T> {
    pub fn new(value: T) -> Self {
        Self(TokioLock::new(value))
    }

    // <E> to allow any error type (we never error, so we have no error type of our own)
    pub fn lock<E>(&self) -> LockFuture<T, E> {
        LockFuture {
            lock: self.0.clone(),
            _error: PhantomData,
        }
    }
}

/// Represents a lock to be held in the future:
pub struct LockFuture<T, E> {
    lock: TokioLock<T>,
    // We can't error and we don't want to enforce a specific error type either
    _error: PhantomData<E>,
}

impl<T, E> Future for LockFuture<T, E> {
    type Item = AsyncLockGuard<T>;
    type Error = E;

    fn poll(&mut self) -> Poll<AsyncLockGuard<T>, E> {
        Ok(self.lock.poll_lock())
    }
}
