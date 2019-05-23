//! Byte vector helpers.
//!
//! We have a lot of I/O code such as:
//! ```ignore
//! let mut buffer = vec![0u8; header_size];
//! file.read_exact(&mut buffer)?;
//! ```
//! (We even have this case with a 4M buffer!)
//!
//! This needlessly initializes the buffer to zero (which not only wastes time (an insane amount of
//! time on debug builds, actually) but also prevents tools such as valgrind from pointing out
//! access to actually uninitialized data, which may hide bugs...)
//!
//! This module provides some helpers for this kind of code. Many of these are supposed to stay on
//! a lower level, with I/O helpers for types implementing [`Read`](std::io::Read) being available
//! in the [`tools::io`](crate::tools::io) module.
//!
//! Examples:
//! ```no_run
//! use proxmox_backup::tools::vec::{self, ops::*};
//!
//! # let size = 64usize;
//! # let more = 64usize;
//! let mut buffer = vec::undefined(size); // A zero-initialized buffer with valgrind support
//!
//! let mut buffer = unsafe { vec::uninitialized(size) }; // an actually uninitialized buffer
//! vec::clear(&mut buffer); // zero out an &mut [u8]
//!
//! vec::clear(unsafe {
//!     buffer.grow_uninitialized(more) // grow the buffer with uninitialized bytes
//! });
//! ```

pub mod ops;

/// Create an uninitialized byte vector of a specific size.
///
/// This is just a shortcut for:
/// ```no_run
/// # let len = 64usize;
/// let mut v = Vec::<u8>::with_capacity(len);
/// unsafe {
///     v.set_len(len);
/// }
/// ```
#[inline]
pub unsafe fn uninitialized(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    out.set_len(len);
    out
}

/// Shortcut to zero out a slice of bytes.
#[inline]
pub fn clear(data: &mut [u8]) {
    unsafe {
        std::ptr::write_bytes(data.as_mut_ptr(), 0, data.len());
    }
}

/// Create a newly allocated, zero initialized byte vector.
#[inline]
pub fn zeroed(len: usize) -> Vec<u8> {
    unsafe {
        let mut out = uninitialized(len);
        clear(&mut out);
        out
    }
}

/// Create a newly allocated byte vector of a specific size with "undefined" content.
///
/// The data will be zero initialized, but, if the `valgrind` feature is activated, it will be
/// marked as uninitialized for debugging.
#[inline]
pub fn undefined(len: usize) -> Vec<u8> {
    undefined_impl(len)
}

#[cfg(not(feature = "valgrind"))]
fn undefined_impl(len: usize) -> Vec<u8> {
    zeroed(len)
}

#[cfg(feature = "valgrind")]
fn undefined_impl(len: usize) -> Vec<u8> {
    let out = zeroed(len);
    vg::make_slice_undefined(&out[..]);
    out
}

#[cfg(feature = "valgrind")]
mod vg {
    type ValgrindValue = valgrind_request::Value;

    /// Mark a memory region as undefined when using valgrind, causing it to treat read access to
    /// it as error.
    #[inline]
    pub(crate) fn make_mem_undefined(addr: *const u8, len: usize) -> ValgrindValue {
        const MAKE_MEM_UNDEFINED: ValgrindValue =
            (((b'M' as ValgrindValue) << 24) | ((b'C' as ValgrindValue) << 16)) + 1;
        unsafe {
            valgrind_request::do_client_request(
                0,
                &[
                    MAKE_MEM_UNDEFINED,
                    addr as usize as ValgrindValue,
                    len as ValgrindValue,
                    0, 0, 0,
                ],
            )
        }
    }

    /// Mark a slice of bytes as undefined when using valgrind, causing it to treat read access to
    /// it as error.
    #[inline]
    pub(crate) fn make_slice_undefined(data: &[u8]) -> ValgrindValue {
        make_mem_undefined(data.as_ptr(), data.len())
    }
}
