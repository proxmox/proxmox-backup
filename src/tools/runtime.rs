//! Helpers for quirks of the current tokio runtime.

use std::cell::RefCell;
use std::future::Future;

use lazy_static::lazy_static;
use tokio::runtime::{self, Runtime};

thread_local! {
    static HAS_RUNTIME: RefCell<bool> = RefCell::new(false);
    static IN_TOKIO: RefCell<bool> = RefCell::new(false);
}

fn is_in_tokio() -> bool {
    IN_TOKIO.with(|v| *v.borrow())
}

fn has_runtime() -> bool {
    HAS_RUNTIME.with(|v| *v.borrow())
}

struct RuntimeGuard(bool);

impl RuntimeGuard {
    fn enter() -> Self {
        Self(HAS_RUNTIME.with(|v| {
            let old = *v.borrow();
            *v.borrow_mut() = true;
            old
        }))
    }
}

impl Drop for RuntimeGuard {
    fn drop(&mut self) {
        HAS_RUNTIME.with(|v| {
            *v.borrow_mut() = self.0;
        });
    }
}

lazy_static! {
    static ref RUNTIME: Runtime = {
        runtime::Builder::new()
            .threaded_scheduler()
            .enable_all()
            .on_thread_start(|| IN_TOKIO.with(|v| *v.borrow_mut() = true))
            .build()
            .expect("failed to spawn tokio runtime")
    };
}

/// Get or create the current main tokio runtime.
///
/// This makes sure that tokio's worker threads are marked for us so that we know whether we
/// can/need to use `block_in_place` in our `block_on` helper.
pub fn get_runtime() -> &'static Runtime {
    &RUNTIME
}

/// Associate the current newly spawned thread with the main tokio runtime.
pub fn enter_runtime<R>(f: impl FnOnce() -> R) -> R {
    let _guard = RuntimeGuard::enter();
    get_runtime().enter(f)
}

/// Block on a synchronous piece of code.
pub fn block_in_place<R>(fut: impl FnOnce() -> R) -> R {
    if is_in_tokio() {
        // we are in an actual tokio worker thread, block it:
        tokio::task::block_in_place(fut)
    } else {
        // we're not inside a tokio worker, so just run the code:
        fut()
    }
}

/// Block on a future in this thread.
pub fn block_on<R, F>(fut: F) -> R
where
    R: Send + 'static,
    F: Future<Output = R> + Send,
{

    if is_in_tokio() {
        // inside a tokio worker we need to tell tokio that we're about to really block:
        tokio::task::block_in_place(move || futures::executor::block_on(fut))
    } else if has_runtime() {
        // we're already associated with a runtime, but we're not a worker-thread, we can just
        // block this thread directly
        // This is not strictly necessary, but it's a bit quicker tha the else branch below.
        futures::executor::block_on(fut)
    } else {
        // not a worker thread, not associated with a runtime, make sure we have a runtime (spawn
        // it on demand if necessary), then enter it:
        enter_runtime(move || futures::executor::block_on(fut))
    }
}

/*
fn block_on_impl<F>(mut fut: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    let fut_ptr = &mut fut as *mut F as usize; // hack to not require F to be 'static
    tokio::spawn(async move {
        let fut: F = unsafe { std::ptr::read(fut_ptr as *mut F) };
        tx
            .send(fut.await)
            .map_err(drop)
            .expect("failed to send block_on result to channel")
    });

    futures::executor::block_on(async move {
        rx.await.expect("failed to receive block_on result from channel")
    })
    std::mem::forget(fut);
}
*/

/// This used to be our tokio main entry point. Now this just calls out to `block_on` for
/// compatibility, which will perform all the necessary tasks on-demand anyway.
pub fn main<F>(fut: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send + 'static,
{
    block_on(fut)
}
