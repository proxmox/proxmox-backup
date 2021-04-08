//! Generic AsyncRead/AsyncWrite utilities.

use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::stream::{Stream, TryStream};
use futures::ready;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpListener;
use hyper::client::connect::Connection;

pub enum EitherStream<L, R> {
    Left(L),
    Right(R),
}

impl<L: AsyncRead + Unpin, R: AsyncRead + Unpin> AsyncRead for EitherStream<L, R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut ReadBuf,
    ) -> Poll<Result<(), io::Error>> {
        match self.get_mut() {
            EitherStream::Left(ref mut s) => {
                Pin::new(s).poll_read(cx, buf)
            }
            EitherStream::Right(ref mut s) => {
                Pin::new(s).poll_read(cx, buf)
            }
        }
    }
}

impl<L: AsyncWrite + Unpin, R: AsyncWrite + Unpin> AsyncWrite for EitherStream<L, R> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        match self.get_mut() {
            EitherStream::Left(ref mut s) => {
                Pin::new(s).poll_write(cx, buf)
            }
            EitherStream::Right(ref mut s) => {
                Pin::new(s).poll_write(cx, buf)
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
        match self.get_mut() {
            EitherStream::Left(ref mut s) => {
                Pin::new(s).poll_flush(cx)
            }
            EitherStream::Right(ref mut s) => {
                Pin::new(s).poll_flush(cx)
            }
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
        match self.get_mut() {
            EitherStream::Left(ref mut s) => {
                Pin::new(s).poll_shutdown(cx)
            }
            EitherStream::Right(ref mut s) => {
                Pin::new(s).poll_shutdown(cx)
            }
        }
    }
}

// we need this for crate::client::http_client:
impl Connection for EitherStream<
    tokio::net::TcpStream,
    Pin<Box<tokio_openssl::SslStream<tokio::net::TcpStream>>>,
> {
    fn connected(&self) -> hyper::client::connect::Connected {
        match self {
            EitherStream::Left(s) => s.connected(),
            EitherStream::Right(s) => s.get_ref().connected(),
        }
    }
}

/// Tokio's `Incoming` now is a reference type and hyper's `AddrIncoming` misses some standard
/// stuff like `AsRawFd`, so here's something implementing hyper's `Accept` from a `TcpListener`
pub struct StaticIncoming(TcpListener);

impl From<TcpListener> for StaticIncoming {
    fn from(inner: TcpListener) -> Self {
        Self(inner)
    }
}

impl AsRawFd for StaticIncoming {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl hyper::server::accept::Accept for StaticIncoming {
    type Conn = tokio::net::TcpStream;
    type Error = std::io::Error;

    fn poll_accept(
        self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        let this = self.get_mut();
        loop {
            match ready!(this.0.poll_accept(cx)) {
                Ok((conn, _addr)) => return Poll::Ready(Some(Ok(conn))),
                Err(err) => {
                    eprintln!("error accepting connection: {}", err);
                    continue;
                }
            }
        }
    }
}

/// We also implement TryStream for this, as tokio doesn't do this anymore either and we want to be
/// able to map connections to then add eg. ssl to them. This support code makes the changes
/// required for hyper 0.13 a bit less annoying to read.
impl Stream for StaticIncoming {
    type Item = std::io::Result<(tokio::net::TcpStream, std::net::SocketAddr)>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        match self.get_mut().0.poll_accept(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => Poll::Ready(Some(result)),
        }
    }
}

/// Implement hyper's `Accept` for any `TryStream` of sockets:
pub struct HyperAccept<T>(pub T);


impl<T, I> hyper::server::accept::Accept for HyperAccept<T>
where
    T: TryStream<Ok = I> + Unpin,
    I: AsyncRead + AsyncWrite,
{
    type Conn = I;
    type Error = T::Error;

    fn poll_accept(
        self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        let this = Pin::new(&mut self.get_mut().0);
        this.try_poll_next(cx)
    }
}
