//! Module providing I/O helpers (sync and async).
//!
//! The [`ops`](io::ops) module provides helper traits for types implementing [`Read`](std::io::Read).
//!
//! The top level functions in of this module here are used for standalone implementations of
//! various functionality which is actually intended to be available as methods to types
//! implementing `AsyncRead`, which, however, without async/await cannot be methods due to them
//! having non-static lifetimes in that case.
//!
//! ```
//! use std::io;
//!
//! use crate::tools::io::read_exact_allocated;
//! use crate::tools::vec::{self, ops::*};
//!
//! // Currently usable:
//! fn do_something() -> impl Future<Item = Vec<u8>, Error = io::Error> {
//!     tokio::fs::File::open("some.file")
//!         .and_then(|file| read_exact_allocated(file, unsafe { vec::uninitialized(1024) }))
//!         .and_then(|(file, mut buffer)| {
//!             so_something_with(&buffer);
//!             // append more data:
//!             tokio::io::read_exact(file, unsafe { buffer.grow_uninitialized(1024) })
//!         })
//!         .and_then(|(_file, bigger_buffer)| {
//!             use_the(bigger_buffer);
//!             Ok(bigger_buffer)
//!         });
//! }
//!
//! // Future async/await variant:
//! async fn do_something() -> Vec<u8> {
//!     let mut file = tokio::fs::File::open("some.file").await?;
//!     let mut buffer = file.read_exact_allocated(1024).await?;
//!     do_something_with(buffer);
//!     file.append_to_vec(&mut buffer, 1024).await?;
//!     buffer
//! }
//! ```

use std::io;

use futures::Future;
use futures::{Async, Poll};
use tokio::io::AsyncRead;

use crate::tools::vec::{self, ops::*};

pub mod ops;

/// Create a future which reads an exact amount of bytes from an input.
///
/// The future's output is a tuple containing the input and a newly allocated `Vec<u8>` containing
/// the data.
///
/// Example:
/// ```
/// tokio::fs::File::open("some.file")
///     .and_then(|file| read_exact_allocated(file, 1024))
///     .and_then(|(_file, data)| {
///         use_the(data);
///     })
/// ```
pub fn read_exact_allocated<R: AsyncRead>(reader: R, size: usize) -> ReadExactAllocated<R> {
    ReadExactAllocated(Some(reader), None, size)
}

/// A future returned by [`read_exact_allocated`].
pub struct ReadExactAllocated<R: AsyncRead>(Option<R>, Option<Vec<u8>>, usize);

impl<R: AsyncRead> Future for ReadExactAllocated<R> {
    type Item = (R, Vec<u8>);
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        assert!(self.0.is_some(), "polled after ready");

        // allocation happens on first poll:
        if self.1.is_none() {
            self.1 = Some(unsafe { vec::uninitialized(self.2) });
            // now self.2 is the position:
            self.2 = 0; 
        }

        let mut buffer = self.1.take().unwrap();

        loop {
            match self.0.as_mut().unwrap().poll_read(&mut buffer[self.2..]) {
                Ok(Async::Ready(0)) => {
                    self.0 = None;
                    return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                }
                Ok(Async::Ready(some)) => {
                    self.2 += some;
                    if self.2 == buffer.len() {
                        self.0 = None;
                        return Ok(Async::Ready((self.0.take().unwrap(), buffer)));
                    }
                    continue;
                }
                Ok(Async::NotReady) => {
                    self.1 = Some(buffer);
                    return Ok(Async::NotReady);
                }
                Err(err) => {
                    self.0 = None;
                    return Err(err);
                }
            }
        }
    }
}

/// Create a future which appends up to at most `size` bytes to a vector, growing it as needed.
///
/// This will grow the vector as if a single `.reserve(amount_to_read)` call was made and fill it
/// with as much data as a single read call will provide.
///
/// The future's output is a tuple containing the input, the vector and the number of bytes
/// actually read.
///
/// Example:
/// ```
/// tokio::fs::File::open("some.file")
///     .and_then(|file| append_to_vec(file, Vec::new(), 1024))
///     .and_then(|(_file, data, size)| {
///         assert!(data.len() == size);
///         println!("Actually got {} bytes of data.", size);
///         use_the(data);
///     })
/// ```
pub fn append_to_vec<R, V>(reader: R, mut vector: V, size: usize) -> AppendToVec<R, V>
where
    R: AsyncRead,
    V: AsMut<Vec<u8>>,
{
    let pos = vector.as_mut().len();
    unsafe {
        vector.as_mut().grow_uninitialized(size);
    }
    AppendToVec(Some(reader), Some(vector), pos)
}

pub struct AppendToVec<R, V>(Option<R>, Option<V>, usize)
where
    R: AsyncRead,
    V: AsMut<Vec<u8>>;

impl<R, V> Future for AppendToVec<R, V>
where
    R: AsyncRead,
    V: AsMut<Vec<u8>>,
{
    type Item = (R, V, usize);
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        assert!(self.0.is_some() && self.1.is_some(), "polled after ready");

        let mut output = self.1.take().unwrap();
        match self.0.as_mut().unwrap().poll_read(&mut output.as_mut()[self.2..]) {
            Ok(Async::Ready(some)) => {
                unsafe {
                    output.as_mut().set_len(self.2 + some);
                }
                return Ok(Async::Ready((self.0.take().unwrap(), output, some)));
            }
            Ok(Async::NotReady) => {
                self.1 = Some(output);
                return Ok(Async::NotReady);
            }
            Err(err) => {
                self.0 = None;
                return Err(err);
            }
        }
    }
}

/// Create a future which appends an exact amount of bytes to a vector, growing it as needed.
///
/// This will grow the vector as if a single `.reserve(amount_to_read)` call was made and fill it
/// as much data as requested. If not enough data is available, this produces an
/// [`io::Error`](std::io::Error) of kind
/// [`ErrorKind::UnexpectedEof`](std::io::ErrorKind::UnexpectedEof).
///
/// The future's output is a tuple containing the input and the vector.
///
/// Example:
/// ```
/// tokio::fs::File::open("some.file")
///     .and_then(|file| append_exact_to_vec(file, Vec::new(), 1024))
///     .and_then(|(_file, data)| {
///         assert!(data.len() == size);
///         println!("Actually got {} bytes of data.", size);
///         use_the(data);
///     })
/// ```
pub fn append_exact_to_vec<R, V>(reader: R, mut vector: V, size: usize) -> AppendExactToVec<R, V>
where
    R: AsyncRead,
    V: AsMut<Vec<u8>>,
{
    let pos = vector.as_mut().len();
    unsafe {
        vector.as_mut().grow_uninitialized(size);
    }
    AppendExactToVec(Some(reader), Some(vector), pos)
}

pub struct AppendExactToVec<R, V>(Option<R>, Option<V>, usize)
where
    R: AsyncRead,
    V: AsMut<Vec<u8>>;

impl<R, V> Future for AppendExactToVec<R, V>
where
    R: AsyncRead,
    V: AsMut<Vec<u8>>,
{
    type Item = (R, V);
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        assert!(self.0.is_some() && self.1.is_some(), "polled after ready");

        let mut output = self.1.take().unwrap();
        loop {
            match self.0.as_mut().unwrap().poll_read(&mut output.as_mut()[self.2..]) {
                Ok(Async::Ready(0)) => {
                    self.0 = None;
                    return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                }
                Ok(Async::Ready(some)) => {
                    self.2 += some;
                    if self.2 == output.as_mut().len() {
                        self.0 = None;
                        return Ok(Async::Ready((self.0.take().unwrap(), output)));
                    }
                    continue;
                }
                Ok(Async::NotReady) => {
                    self.1 = Some(output);
                    return Ok(Async::NotReady);
                }
                Err(err) => {
                    self.0 = None;
                    return Err(err);
                }
            }
        }
    }
}


/*
 * TODO: A trait such as the one below is only useful inside `async fn`, so this partwill have to
 * wait...
 *
 * When we have async/await we can finish this and move it into io/async_read.rs

/// Some additional related functionality for types implementing `AsyncRead`. Note that most of 
/// these methods map to functions from the [`io`](super::io) module, which are standalone
/// variants.
/// 
/// This trait only works with standard futures or as part of `poll_fn` bodies, due to it requiring
/// non-static lifetimes on futures.
pub trait AsyncReadExtOps: AsyncRead + Sized {
    /// Read data into a newly allocated vector. This is a shortcut for:
    /// ```
    /// let mut data = Vec::with_capacity(len);
    /// unsafe {
    ///     data.set_len(len);
    /// }
    /// reader.read_exact(&mut data)
    /// ```
    ///
    /// With this trait, we just use:
    /// ```
    /// use crate::tools::vec::ops::*;
    ///
    /// let data = reader.read_exact_allocated(len).await?;
    /// ```
    fn read_exact_allocated(&mut self, size: usize) -> ReadExactAllocated<&mut Self> {
        ReadExactAllocated(crate::tools::io::read_exact_allocated(self, size))
    }
}

impl<T: AsyncRead + Sized> AsyncReadExtOps for T {
}

pub struct ReadExactAllocated<R: AsyncRead>(crate::tools::io::ReadExactAllocated<R>);

impl<R: AsyncRead> futures::Future for ReadExactAllocated<R> {
    type Item = Vec<u8>;
    type Error = io::Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        let (_this, data) = futures::try_ready!(self.0.poll());
        Ok(futures::Async::Ready(data))
    }
}
 */
