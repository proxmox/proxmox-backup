use anyhow::{Error, format_err, bail};
use lazy_static::lazy_static;
use std::task::{Context, Poll};
use std::os::unix::io::AsRawFd;

use hyper::{Uri, Body};
use hyper::client::{Client, HttpConnector};
use openssl::ssl::{SslConnector, SslMethod};
use futures::*;

use crate::tools::{
    async_io::EitherStream,
    socket::{
        set_tcp_keepalive,
        PROXMOX_BACKUP_TCP_KEEPALIVE_TIME,
    },
};

lazy_static! {
    static ref HTTP_CLIENT: Client<HttpsConnector, Body> = {
        let connector = SslConnector::builder(SslMethod::tls()).unwrap().build();
        let httpc = HttpConnector::new();
        let https = HttpsConnector::with_connector(httpc, connector);
        Client::builder().build(https)
    };
}

pub async fn get_string<U: AsRef<str>>(uri: U) -> Result<String, Error> {
    let res = HTTP_CLIENT.get(uri.as_ref().parse()?).await?;

    let status = res.status();
    if !status.is_success() {
        bail!("Got bad status '{}' from server", status)
    }

    let buf = hyper::body::to_bytes(res).await?;
    String::from_utf8(buf.to_vec())
        .map_err(|err| format_err!("Error converting HTTP result data: {}", err))
}

#[derive(Clone)]
pub struct HttpsConnector {
    http: HttpConnector,
    ssl_connector: std::sync::Arc<SslConnector>,
}

impl HttpsConnector {
    pub fn with_connector(mut http: HttpConnector, ssl_connector: SslConnector) -> Self {
        http.enforce_http(false);

        Self {
            http,
            ssl_connector: std::sync::Arc::new(ssl_connector),
        }
    }
}

type MaybeTlsStream = EitherStream<
    tokio::net::TcpStream,
    tokio_openssl::SslStream<tokio::net::TcpStream>,
>;

impl hyper::service::Service<Uri> for HttpsConnector {
    type Response = MaybeTlsStream;
    type Error = Error;
    type Future = std::pin::Pin<Box<
        dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static
    >>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // This connector is always ready, but others might not be.
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, dst: Uri) -> Self::Future {
        let mut this = self.clone();
        async move {
            let is_https = dst
                .scheme()
                .ok_or_else(|| format_err!("missing URL scheme"))?
                == "https";
            let host = dst
                .host()
                .ok_or_else(|| format_err!("missing hostname in destination url?"))?
                .to_string();

            let config = this.ssl_connector.configure();
            let conn = this.http.call(dst).await?;

            let _ = set_tcp_keepalive(conn.as_raw_fd(), PROXMOX_BACKUP_TCP_KEEPALIVE_TIME);

            if is_https {
                let conn = tokio_openssl::connect(config?, &host, conn).await?;
                Ok(MaybeTlsStream::Right(conn))
            } else {
                Ok(MaybeTlsStream::Left(conn))
            }
        }.boxed()
    }
}
