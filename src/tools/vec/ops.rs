//! This module provides additional operations for `Vec<u8>`.
//!
//! Example:
//! ```
//! use crate::tools::vec::{self, ops::*};
//!
//! fn append_1024_to_vec<T: Read>(input: T, buffer: &mut Vec<u8>) -> std::io::Result<()> {
//!     input.read_exact(unsafe { buffer.grow_uninitialized(1024) })
//! }
//! ```

/// Some additional byte vector operations useful for I/O code.
/// Example:
/// ```
/// use crate::tools::vec::{self, ops::*};
///
/// let mut data = file.read_exact_allocated(1024)?;
/// do_something();
/// file.read_exact(unsafe {
///     data.grow_uninitialized(1024);
/// })?;
/// ```
///
/// Note that this module also provides a safe alternative for the case where
/// `grow_uninitialized()` is directly followed by a `read_exact()` call via the [`ReadExtOps`]
/// trait:
/// ```
/// file.append_to_vec(&mut data, 1024)?;
/// ```
///
/// [`ReadExtOps`]: crate::tools::io::ops::ReadExtOps
pub trait VecU8ExtOps {
    /// Grow a vector without initializing its elements. The difference to simply using `reserve`
    /// is that it also updates the actual length, making the newly allocated data part of the
    /// slice.
    ///
    /// This is a shortcut for:
    /// ```
    /// vec.reserve(more);
    /// let total = vec.len() + more;
    /// unsafe {
    ///     vec.set_len(total);
    /// }
    /// ```
    ///
    /// This returns a mutable slice to the newly allocated space, so it can be used inline:
    /// ```
    /// file.read_exact(unsafe { buffer.grow_uninitialized(1024) })?;
    /// ```
    ///
    /// Although for the above case it is recommended to use the even shorter version from the
    /// [`ReadExtOps`] trait:
    /// ```
    /// // use crate::tools::vec::ops::ReadExtOps;
    /// file.append_to_vec(&mut buffer, 1024)?;
    /// ```
    ///
    /// [`ReadExtOps`]: crate::tools::io::ops::ReadExtOps
    unsafe fn grow_uninitialized(&mut self, more: usize) -> &mut [u8];

    /// Resize a vector to a specific size without initializing its data. This is a shortcut for:
    /// ```
    /// if new_size <= vec.len() {
    ///     vec.truncate(new_size);
    /// } else {
    ///     unsafe {
    ///         vec.grow_uninitialized(new_size - vec.len());
    ///     }
    /// }
    /// ```
    unsafe fn resize_uninitialized(&mut self, total: usize);
}

impl VecU8ExtOps for Vec<u8> {
    unsafe fn grow_uninitialized(&mut self, more: usize) -> &mut [u8] {
        let old_len = self.len();
        self.reserve(more);
        let total = old_len + more;
        self.set_len(total);
        &mut self[old_len..]
    }

    unsafe fn resize_uninitialized(&mut self, new_size: usize) {
        if new_size <= self.len() {
            self.truncate(new_size);
        } else {
            self.grow_uninitialized(new_size - self.len());
        }
    }
}
