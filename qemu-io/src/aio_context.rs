//! Provides a handle to an AioContext.

#[cfg(feature="standalone")]
mod standalone;
#[cfg(feature="standalone")]
pub use standalone::AioContext;

// TODO: Add the non-standalone variant to be linked with Qemu:
//    The AioContext struct should provide a high-level version of `set_fd_handler` with the same
//    interface the standalone version provides out of the box (transparently turning closures into
//    `extern "C" fn(opaque: *const c_void)` calls.
