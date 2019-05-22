//! This module provides additional operations for handling byte buffers for types implementing
//! [`Read`](std::io::Read).
//!
//! See the [`ReadExtOps`](ops::ReadExtOps) trait for examples.

use std::io;

use endian_trait::Endian;

use crate::tools::vec::{self, ops::*};

/// Adds some additional related functionality for types implementing [`Read`](std::io::Read).
///
/// Particularly for reading into a newly allocated buffer, appending to a `Vec<u8>` or reading
/// values of a specific endianess (types implementing [`Endian`]).
///
/// Examples:
/// ```
/// use crate::tools::io::ops::*;
///
/// let mut file = std::fs::File::open("some.data")?;
///
/// // read some bytes into a newly allocated Vec<u8>:
/// let mut data = file.read_exact_allocated(header.data_size as usize)?;
///
/// // appending data to a vector:
/// let actually_appended = file.append_to_vec(&mut data, length)?; // .read() version
/// file.append_exact_to_vec(&mut data, length)?; // .read_exact() version
/// ```
///
/// Or for reading values of a defined representation and endianess:
///
/// ```
/// #[derive(Endian)]
/// #[repr(C)]
/// struct Header {
///     version: u16,
///     data_size: u16,
/// }
///
/// // We have given `Header` a proper binary representation via `#[repr]`, so this is safe:
/// let header: Header = unsafe { file.read_le_value()? };
/// let mut blob = file.read_exact_allocated(header.data_size as usize)?;
/// ```
///
/// [`Endian`]: https://docs.rs/endian_trait/0.6/endian_trait/trait.Endian.html
pub trait ReadExtOps {
    /// Read data into a newly allocated vector. This is a shortcut for:
    /// ```
    /// let mut data = Vec::with_capacity(len);
    /// unsafe {
    ///     data.set_len(len);
    /// }
    /// reader.read_exact(&mut data)?;
    /// ```
    ///
    /// With this trait, we just use:
    /// ```
    /// use crate::tools::vec::ops::*;
    ///
    /// let data = reader.read_exact_allocated(len);
    /// ```
    fn read_exact_allocated(&mut self, size: usize) -> io::Result<Vec<u8>>;

    /// Append data to a vector, growing it as necessary. Returns the amount of data appended.
    fn append_to_vec(&mut self, out: &mut Vec<u8>, size: usize) -> io::Result<usize>;

    /// Append an exact amount of data to a vector, growing it as necessary.
    fn append_exact_to_vec(&mut self, out: &mut Vec<u8>, size: usize) -> io::Result<()>;

    /// Read a value with host endianess.
    ///
    /// This is limited to types implementing the [`Endian`] trait under the assumption that
    /// this is only done for types which are supposed to be read/writable directly.
    ///
    /// There's no way to directly depend on a type having a specific `#[repr(...)]`, therefore
    /// this is considered unsafe.
    ///
    /// ```
    /// use crate::tools::vec::ops::*;
    ///
    /// #[derive(Endian)]
    /// #[repr(C, packed)]
    /// struct Data {
    ///     value: u16,
    ///     count: u32,
    /// }
    ///
    /// let mut file = std::fs::File::open("my-raw.dat")?;
    /// // We know `Data` has a safe binary representation (#[repr(C, packed)]), so we can
    /// // safely use our helper:
    /// let data: Data = unsafe { file.read_host_value()? };
    /// ```
    ///
    /// [`Endian`]: https://docs.rs/endian_trait/0.6/endian_trait/trait.Endian.html
    unsafe fn read_host_value<T: Endian>(&mut self) -> io::Result<T>;

    /// Read a little endian value.
    ///
    /// The return type is required to implement the [`Endian`] trait, and we make the
    /// assumption that this is only done for types which are supposed to be read/writable
    /// directly.
    ///
    /// There's no way to directly depend on a type having a specific `#[repr(...)]`, therefore
    /// this is considered unsafe.
    ///
    /// ```
    /// use crate::tools::vec::ops::*;
    ///
    /// #[derive(Endian)]
    /// #[repr(C, packed)]
    /// struct Data {
    ///     value: u16,
    ///     count: u32,
    /// }
    ///
    /// let mut file = std::fs::File::open("my-little-endian.dat")?;
    /// // We know `Data` has a safe binary representation (#[repr(C, packed)]), so we can
    /// // safely use our helper:
    /// let data: Data = unsafe { file.read_le_value()? };
    /// ```
    ///
    /// [`Endian`]: https://docs.rs/endian_trait/0.6/endian_trait/trait.Endian.html
    unsafe fn read_le_value<T: Endian>(&mut self) -> io::Result<T>;

    /// Read a big endian value.
    ///
    /// The return type is required to implement the [`Endian`] trait, and we make the
    /// assumption that this is only done for types which are supposed to be read/writable
    /// directly.
    ///
    /// There's no way to directly depend on a type having a specific `#[repr(...)]`, therefore
    /// this is considered unsafe.
    ///
    /// ```
    /// use crate::tools::vec::ops::*;
    ///
    /// #[derive(Endian)]
    /// #[repr(C, packed)]
    /// struct Data {
    ///     value: u16,
    ///     count: u32,
    /// }
    ///
    /// let mut file = std::fs::File::open("my-big-endian.dat")?;
    /// // We know `Data` has a safe binary representation (#[repr(C, packed)]), so we can
    /// // safely use our helper:
    /// let data: Data = unsafe { file.read_be_value()? };
    /// ```
    ///
    /// [`Endian`]: https://docs.rs/endian_trait/0.6/endian_trait/trait.Endian.html
    unsafe fn read_be_value<T: Endian>(&mut self) -> io::Result<T>;
}

impl<R: io::Read> ReadExtOps for R {
    fn read_exact_allocated(&mut self, size: usize) -> io::Result<Vec<u8>> {
        let mut out = unsafe { vec::uninitialized(size) };
        self.read_exact(&mut out)?;
        Ok(out)
    }

    fn append_to_vec(&mut self, out: &mut Vec<u8>, size: usize) -> io::Result<usize> {
        let pos = out.len();
        unsafe {
            out.grow_uninitialized(size);
        }
        let got = self.read(&mut out[pos..])?;
        unsafe {
            out.set_len(pos + got);
        }
        Ok(got)
    }

    fn append_exact_to_vec(&mut self, out: &mut Vec<u8>, size: usize) -> io::Result<()> {
        let pos = out.len();
        unsafe {
            out.grow_uninitialized(size);
        }
        self.read_exact(&mut out[pos..])?;
        Ok(())
    }

    unsafe fn read_host_value<T: Endian>(&mut self) -> io::Result<T> {
        let mut value: T = std::mem::uninitialized();
        self.read_exact(std::slice::from_raw_parts_mut(
            &mut value as *mut T as *mut u8,
            std::mem::size_of::<T>(),
        ))?;
        Ok(value)
    }

    unsafe fn read_le_value<T: Endian>(&mut self) -> io::Result<T> {
        Ok(self.read_host_value::<T>()?.
            from_le()
        )
    }

    unsafe fn read_be_value<T: Endian>(&mut self) -> io::Result<T> {
        Ok(self.read_host_value::<T>()?
            .from_be()
        )
    }
}
