use anyhow::{Error, format_err, bail};
use std::task::{Context, Poll};
use std::os::unix::io::AsRawFd;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use hyper::{Uri, Body};
use hyper::client::{Client, HttpConnector};
use http::{Request, Response};
use openssl::ssl::{SslConnector, SslMethod};
use futures::*;
use tokio::{
    io::{
        AsyncBufReadExt,
        AsyncWriteExt,
        BufStream,
    },
    net::TcpStream,
};
use tokio_openssl::SslStream;

use crate::tools::{
    async_io::MaybeTlsStream,
    socket::{
        set_tcp_keepalive,
        PROXMOX_BACKUP_TCP_KEEPALIVE_TIME,
    },
};

/// HTTP Proxy Configuration
#[derive(Clone)]
pub struct ProxyConfig {
    pub host: String,
    pub port: u16,
    pub force_connect: bool,
}

/// Asyncrounous HTTP client implementation
pub struct SimpleHttp {
    client: Client<HttpsConnector, Body>,
}

impl SimpleHttp {

    pub fn new(proxy_config: Option<ProxyConfig>) -> Self {
        let ssl_connector = SslConnector::builder(SslMethod::tls()).unwrap().build();
        Self::with_ssl_connector(ssl_connector, proxy_config)
    }

    pub fn with_ssl_connector(ssl_connector: SslConnector, proxy_config: Option<ProxyConfig>) -> Self {
        let connector = HttpConnector::new();
        let mut https = HttpsConnector::with_connector(connector, ssl_connector);
        if let Some(proxy_config) = proxy_config {
            https.set_proxy(proxy_config);
        }
        let client = Client::builder().build(https);
        Self { client }
    }

    pub async fn request(&self, request: Request<Body>) -> Result<Response<Body>, Error> {
        self.client.request(request)
            .map_err(Error::from)
            .await
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
    connector: HttpConnector,
    ssl_connector: Arc<SslConnector>,
    proxy: Option<ProxyConfig>,
}

impl HttpsConnector {
    pub fn with_connector(mut connector: HttpConnector, ssl_connector: SslConnector) -> Self {
        connector.enforce_http(false);
        Self {
            connector,
            ssl_connector: Arc::new(ssl_connector),
            proxy: None,
        }
    }

    pub fn set_proxy(&mut self, proxy: ProxyConfig) {
        self.proxy = Some(proxy);
    }

    async fn secure_stream(
        tcp_stream: TcpStream,
        ssl_connector: &SslConnector,
        host: &str,
    ) -> Result<MaybeTlsStream<TcpStream>, Error> {
        let config = ssl_connector.configure()?;
        let mut conn: SslStream<TcpStream> = SslStream::new(config.into_ssl(host)?, tcp_stream)?;
        Pin::new(&mut conn).connect().await?;
        Ok(MaybeTlsStream::Secured(conn))
    }

    async fn parse_connect_status(
        stream: &mut BufStream<TcpStream>,
    ) -> Result<(), Error> {

        let mut status_str = String::new();

        // TODO: limit read-length

        if stream.read_line(&mut status_str).await? == 0 {
            bail!("proxy connect failed - unexpected EOF")
        }

        if !(status_str.starts_with("HTTP/1.1 200") || status_str.starts_with("HTTP/1.0 200")) {
            bail!("proxy connect failed - invalid status: {}", status_str)
        }

        loop {
            // skip rest until \r\n
            let mut response = String::new();
            if stream.read_line(&mut response).await? == 0 {
                bail!("proxy connect failed - unexpected EOF")
            }
            if response.len() > 8192 {
                bail!("proxy connect failed - long lines in connect rtesponse")
            }
            if response == "\r\n" {
                break;
            }
        }
        Ok(())
    }
}

impl hyper::service::Service<Uri> for HttpsConnector {
    type Response = MaybeTlsStream<TcpStream>;
    type Error = Error;
    #[allow(clippy::type_complexity)]
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.connector
            .poll_ready(ctx)
            .map_err(|err| err.into())
    }

    fn call(&mut self, dst: Uri) -> Self::Future {
        let mut connector = self.connector.clone();
        let ssl_connector = Arc::clone(&self.ssl_connector);
        let is_https = dst.scheme() == Some(&http::uri::Scheme::HTTPS);
        let host = match dst.host() {
            Some(host) => host.to_owned(),
            None => {
                return futures::future::err(format_err!("missing URL scheme")).boxed();
            }
        };
        let port = dst.port_u16().unwrap_or(if is_https { 443 } else { 80 });

        if let Some(ref proxy) = self.proxy {

            let use_connect = is_https || proxy.force_connect;

            let proxy_url = format!("{}:{}", proxy.host, proxy.port);
            let proxy_uri = match Uri::builder()
                .scheme("http")
                .authority(proxy_url.as_str())
                .path_and_query("/")
                .build()
            {
                Ok(uri) => uri,
                Err(err) => return futures::future::err(err.into()).boxed(),
            };

            if use_connect {
                async move {

                    let proxy_stream = connector
                        .call(proxy_uri)
                        .await
                        .map_err(|err| format_err!("error connecting to {} - {}", proxy_url, err))?;

                    let _ = set_tcp_keepalive(proxy_stream.as_raw_fd(), PROXMOX_BACKUP_TCP_KEEPALIVE_TIME);

                    let mut stream = BufStream::new(proxy_stream);

                    let connect_request = format!(
                        "CONNECT {0}:{1} HTTP/1.1\r\n\
                         Host: {0}:{1}\r\n\r\n",
                        host, port,
                    );

                    stream.write_all(connect_request.as_bytes()).await?;
                    stream.flush().await?;

                    Self::parse_connect_status(&mut stream).await?;

                    let tcp_stream = stream.into_inner();

                    if is_https {
                        Self::secure_stream(tcp_stream, &ssl_connector, &host).await
                    } else {
                        Ok(MaybeTlsStream::Normal(tcp_stream))
                    }
                }.boxed()
            } else {
               async move {
                   let tcp_stream = connector
                       .call(proxy_uri)
                       .await
                       .map_err(|err| format_err!("error connecting to {} - {}", proxy_url, err))?;

                   let _ = set_tcp_keepalive(tcp_stream.as_raw_fd(), PROXMOX_BACKUP_TCP_KEEPALIVE_TIME);

                   Ok(MaybeTlsStream::Proxied(tcp_stream))
               }.boxed()
            }
        } else {
            async move {
                let dst_str = dst.to_string(); // for error messages
                let tcp_stream = connector
                    .call(dst)
                    .await
                    .map_err(|err| format_err!("error connecting to {} - {}", dst_str, err))?;

                let _ = set_tcp_keepalive(tcp_stream.as_raw_fd(), PROXMOX_BACKUP_TCP_KEEPALIVE_TIME);

                if is_https {
                    Self::secure_stream(tcp_stream, &ssl_connector, &host).await
                } else {
                    Ok(MaybeTlsStream::Normal(tcp_stream))
                }
            }.boxed()
        }
    }
}
