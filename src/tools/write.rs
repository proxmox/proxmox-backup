//! Utility traits for types which implement `std::io::Write` to easily write primitively typed
//! values such as binary integers to a stream.

use std::io;
use std::mem;

use endian_trait::Endian;

pub trait WriteUtilOps {
    /// Write a value of type `T`.
    /// Note that it *should* be `repr(C, packed)` or similar.
    fn write_value<T>(&mut self, value: &T) -> io::Result<usize>;

    /// Write a big-endian value of type `T`.
    /// Note that it *should* be `repr(C, packed)` or similar.
    fn write_value_be<T: Endian>(&mut self, value: T) -> io::Result<usize> {
        self.write_value(&value.to_be())
    }

    /// Write a little-endian value of type `T`.
    /// Note that it *should* be `repr(C, packed)` or similar.
    fn write_value_le<T: Endian>(&mut self, value: T) -> io::Result<usize> {
        self.write_value(&value.to_le())
    }

    /// Convenience `write_all()` alternative returning the length instead of `()`.
    fn write_all_len(&mut self, value: &[u8]) -> io::Result<usize>;
}

impl<R: io::Write> WriteUtilOps for R {
    fn write_value<T>(&mut self, value: &T) -> io::Result<usize> {
        let size = mem::size_of::<T>();
        self.write_all(unsafe {
            std::slice::from_raw_parts(value as *const T as *const u8, size)
        })?;
        Ok(size)
    }

    fn write_all_len(&mut self, value: &[u8]) -> io::Result<usize> {
        self.write_all(value)?;
        Ok(value.len())
    }
}
