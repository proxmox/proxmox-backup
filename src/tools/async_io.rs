//! AsyncRead/AsyncWrite utilities.

use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::stream::{Stream, TryStream};
use futures::ready;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpListener;
use tokio_openssl::SslStream;
use hyper::client::connect::{Connection, Connected};

/// Asynchronous stream, possibly encrypted and proxied
///
/// Usefule for HTTP client implementations using hyper.
pub enum MaybeTlsStream<S> {
    Normal(S),
    Proxied(S),
    Secured(SslStream<S>),
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for MaybeTlsStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut ReadBuf,
    ) -> Poll<Result<(), io::Error>> {
        match self.get_mut() {
            MaybeTlsStream::Normal(ref mut s) => {
                Pin::new(s).poll_read(cx, buf)
            }
            MaybeTlsStream::Proxied(ref mut s) => {
                Pin::new(s).poll_read(cx, buf)
            }
            MaybeTlsStream::Secured(ref mut s) => {
                Pin::new(s).poll_read(cx, buf)
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for MaybeTlsStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        match self.get_mut() {
            MaybeTlsStream::Normal(ref mut s) => {
                Pin::new(s).poll_write(cx, buf)
            }
            MaybeTlsStream::Proxied(ref mut s) => {
                Pin::new(s).poll_write(cx, buf)
            }
            MaybeTlsStream::Secured(ref mut s) => {
                Pin::new(s).poll_write(cx, buf)
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
        match self.get_mut() {
            MaybeTlsStream::Normal(ref mut s) => {
                Pin::new(s).poll_flush(cx)
            }
            MaybeTlsStream::Proxied(ref mut s) => {
                Pin::new(s).poll_flush(cx)
            }
            MaybeTlsStream::Secured(ref mut s) => {
                Pin::new(s).poll_flush(cx)
            }
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
        match self.get_mut() {
            MaybeTlsStream::Normal(ref mut s) => {
                Pin::new(s).poll_shutdown(cx)
            }
            MaybeTlsStream::Proxied(ref mut s) => {
                Pin::new(s).poll_shutdown(cx)
            }
            MaybeTlsStream::Secured(ref mut s) => {
                Pin::new(s).poll_shutdown(cx)
            }
        }
    }
}

// we need this for the hyper http client
impl <S: Connection + AsyncRead + AsyncWrite + Unpin> Connection for MaybeTlsStream<S>
{
    fn connected(&self) -> Connected {
        match self {
            MaybeTlsStream::Normal(s) => s.connected(),
            MaybeTlsStream::Proxied(s) => s.connected().proxy(true),
            MaybeTlsStream::Secured(s) => s.get_ref().connected(),
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
