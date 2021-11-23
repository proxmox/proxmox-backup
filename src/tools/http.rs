use anyhow::{Error, format_err, bail};
use std::task::{Context, Poll};
use std::os::unix::io::AsRawFd;
use std::pin::Pin;
use std::sync::Arc;

use hyper::client::HttpConnector;
use http::{Uri, uri::Authority};
use openssl::ssl::SslConnector;
use futures::*;
use tokio::{
    io::{
        AsyncRead,
        AsyncReadExt,
        AsyncWriteExt,
    },
    net::TcpStream,
};
use tokio_openssl::SslStream;

use proxmox_sys::linux::socket::set_tcp_keepalive;
use proxmox_http::http::{MaybeTlsStream, ProxyConfig};
