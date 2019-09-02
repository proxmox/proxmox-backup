//! Generic AsyncRead/AsyncWrite utilities.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite};

pub enum EitherStream<L, R> {
    Left(L),
    Right(R),
}

impl<L: AsyncRead, R: AsyncRead> AsyncRead for EitherStream<L, R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut [u8],
    ) -> Poll<Result<usize, io::Error>> {
        match unsafe { self.get_unchecked_mut() } {
            EitherStream::Left(ref mut s) => {
                unsafe { Pin::new_unchecked(s) }.poll_read(cx, buf)
            }
            EitherStream::Right(ref mut s) => {
                unsafe { Pin::new_unchecked(s) }.poll_read(cx, buf)
            }
        }
    }

    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        match *self {
            EitherStream::Left(ref s) => s.prepare_uninitialized_buffer(buf),
            EitherStream::Right(ref s) => s.prepare_uninitialized_buffer(buf),
        }
    }

    fn poll_read_buf<B>(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut B,
    ) -> Poll<Result<usize, io::Error>>
    where
        B: bytes::BufMut,
    {
        match unsafe { self.get_unchecked_mut() } {
            EitherStream::Left(ref mut s) => {
                unsafe { Pin::new_unchecked(s) }.poll_read_buf(cx, buf)
            }
            EitherStream::Right(ref mut s) => {
                unsafe { Pin::new_unchecked(s) }.poll_read_buf(cx, buf)
            }
        }
    }
}

impl<L: AsyncWrite, R: AsyncWrite> AsyncWrite for EitherStream<L, R> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        match unsafe { self.get_unchecked_mut() } {
            EitherStream::Left(ref mut s) => {
                unsafe { Pin::new_unchecked(s) }.poll_write(cx, buf)
            }
            EitherStream::Right(ref mut s) => {
                unsafe { Pin::new_unchecked(s) }.poll_write(cx, buf)
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
        match unsafe { self.get_unchecked_mut() } {
            EitherStream::Left(ref mut s) => {
                unsafe { Pin::new_unchecked(s) }.poll_flush(cx)
            }
            EitherStream::Right(ref mut s) => {
                unsafe { Pin::new_unchecked(s) }.poll_flush(cx)
            }
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
        match unsafe { self.get_unchecked_mut() } {
            EitherStream::Left(ref mut s) => {
                unsafe { Pin::new_unchecked(s) }.poll_shutdown(cx)
            }
            EitherStream::Right(ref mut s) => {
                unsafe { Pin::new_unchecked(s) }.poll_shutdown(cx)
            }
        }
    }

    fn poll_write_buf<B>(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut B,
    ) -> Poll<Result<usize, io::Error>>
    where
        B: bytes::Buf,
    {
        match unsafe { self.get_unchecked_mut() } {
            EitherStream::Left(ref mut s) => {
                unsafe { Pin::new_unchecked(s) }.poll_write_buf(cx, buf)
            }
            EitherStream::Right(ref mut s) => {
                unsafe { Pin::new_unchecked(s) }.poll_write_buf(cx, buf)
            }
        }
    }
}
