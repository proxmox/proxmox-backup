use anyhow::{Error, format_err, bail};
use std::task::{Context, Poll};
use std::os::unix::io::AsRawFd;
use std::collections::HashMap;
use std::pin::Pin;

use hyper::{Uri, Body};
use hyper::client::{Client, HttpConnector};
use http::{Request, Response};
use openssl::ssl::{SslConnector, SslMethod};
use futures::*;
use tokio::net::TcpStream;
use tokio_openssl::SslStream;

use crate::tools::{
    async_io::MaybeTlsStream,
    socket::{
        set_tcp_keepalive,
        PROXMOX_BACKUP_TCP_KEEPALIVE_TIME,
    },
};

/// Asyncrounous HTTP client implementation
pub struct SimpleHttp {
    client: Client<HttpsConnector, Body>,
}

impl SimpleHttp {

    pub fn new() -> Self {
        let ssl_connector = SslConnector::builder(SslMethod::tls()).unwrap().build();
        Self::with_ssl_connector(ssl_connector)
    }

    pub fn with_ssl_connector(ssl_connector: SslConnector) -> Self {
        let connector = HttpConnector::new();
        let https = HttpsConnector::with_connector(connector, ssl_connector);
        let client = Client::builder().build(https);
        Self { client }
    }

    pub async fn post(
        &mut self,
        uri: &str,
        body: Option<String>,
        content_type: Option<&str>,
    ) -> Result<Response<Body>, Error> {

        let body = if let Some(body) = body {
            Body::from(body)
        } else {
            Body::empty()
        };
        let content_type = content_type.unwrap_or("application/json");

        let request = Request::builder()
            .method("POST")
            .uri(uri)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header(hyper::header::CONTENT_TYPE, content_type)
            .body(body)?;

        self.client.request(request)
            .map_err(Error::from)
            .await
    }

    pub async fn get_string(
        &mut self,
        uri: &str,
        extra_headers: Option<&HashMap<String, String>>,
    ) -> Result<String, Error> {

        let mut request = Request::builder()
            .method("GET")
            .uri(uri)
            .header("User-Agent", "proxmox-backup-client/1.0");

        if let Some(hs) = extra_headers {
            for (h, v) in hs.iter() {
                request = request.header(h, v);
            }
        }

        let request = request.body(Body::empty())?;

        let res = self.client.request(request).await?;

        let status = res.status();
        if !status.is_success() {
            bail!("Got bad status '{}' from server", status)
        }

        Self::response_body_string(res).await
    }

    pub async fn response_body_string(res: Response<Body>) -> Result<String, Error> {
        let buf = hyper::body::to_bytes(res).await?;
        String::from_utf8(buf.to_vec())
            .map_err(|err| format_err!("Error converting HTTP result data: {}", err))
    }
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

impl hyper::service::Service<Uri> for HttpsConnector {
    type Response = MaybeTlsStream<TcpStream>;
    type Error = Error;
    #[allow(clippy::type_complexity)]
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

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
            let dst_str = dst.to_string(); // for error messages
            let conn = this
                .http
                .call(dst)
                .await
                .map_err(|err| format_err!("error connecting to {} - {}", dst_str, err))?;

            let _ = set_tcp_keepalive(conn.as_raw_fd(), PROXMOX_BACKUP_TCP_KEEPALIVE_TIME);

            if is_https {
                let mut conn: SslStream<TcpStream> = SslStream::new(config?.into_ssl(&host)?, conn)?;
                Pin::new(&mut conn).connect().await?;
                Ok(MaybeTlsStream::Secured(conn))
            } else {
                Ok(MaybeTlsStream::Normal(conn))
            }
        }.boxed()
    }
}
