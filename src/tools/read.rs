//! Utility traits for types which implement `std::io::Read` to quickly read primitively typed
//! values such as binary integers from a stream.

use std::io;
use std::mem;

use endian_trait::Endian;

pub trait ReadUtilOps {
    /// Read a value of type `T`.
    /// Note that it *should* be `repr(C, packed)` or similar.
    fn read_value<T>(&mut self) -> io::Result<T>;

    /// Read a big-endian value of type `T`.
    /// Note that it *should* be `repr(C, packed)` or similar.
    fn read_value_be<T: Endian>(&mut self) -> io::Result<T> {
        Ok(self.read_value::<T>()?.from_be())
    }

    /// Read a little-endian value of type `T`.
    /// Note that it *should* be `repr(C, packed)` or similar.
    fn read_value_le<T: Endian>(&mut self) -> io::Result<T> {
        Ok(self.read_value::<T>()?.from_le())
    }

    /// Read an exact number of bytes into a newly allocated vector.
    fn read_exact_allocated(&mut self, size: usize) -> io::Result<Vec<u8>>;
}

impl<R: io::Read> ReadUtilOps for R {
    fn read_value<T>(&mut self) -> io::Result<T> {
        let mut data: T = unsafe { mem::uninitialized() };
        self.read_exact(unsafe {
            std::slice::from_raw_parts_mut(
                &mut data as *mut T as *mut u8,
                mem::size_of::<T>(),
            )
        })?;
        Ok(data)
    }

    fn read_exact_allocated(&mut self, size: usize) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(size);
        unsafe {
            out.set_len(size);
        }
        self.read_exact(&mut out)?;
        Ok(out)
    }
}
