use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::{bail, format_err, Error};
use futures::*;
use http::Uri;
use http::{Request, Response};
use hyper::client::connect::{Connected, Connection};
use hyper::client::Client;
use hyper::Body;
use pin_project_lite::pin_project;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::UnixStream;

use proxmox_http::uri::json_object_to_query;
use proxmox_router::HttpError;

pub const DEFAULT_VSOCK_PORT: u16 = 807;

#[derive(Clone)]
struct VsockConnector;

pin_project! {
    /// Wrapper around UnixStream so we can implement hyper::client::connect::Connection
    struct UnixConnection {
        #[pin]
        stream: UnixStream,
    }
}

impl tower_service::Service<Uri> for VsockConnector {
    type Response = UnixConnection;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<UnixConnection, Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, dst: Uri) -> Self::Future {
        use nix::sys::socket::*;
        use std::os::unix::io::FromRawFd;

        // connect can block, so run in blocking task (though in reality it seems to immediately
        // return with either ENODEV or ETIMEDOUT in case of error)
        tokio::task::spawn_blocking(move || {
            if dst.scheme_str().unwrap_or_default() != "vsock" {
                bail!("invalid URI (scheme) for vsock connector: {}", dst);
            }

            let cid = match dst.host() {
                Some(host) => host.parse().map_err(|err| {
                    format_err!(
                        "invalid URI (host not a number) for vsock connector: {} ({})",
                        dst,
                        err
                    )
                })?,
                None => bail!("invalid URI (no host) for vsock connector: {}", dst),
            };

            let port = match dst.port_u16() {
                Some(port) => port,
                None => bail!("invalid URI (bad port) for vsock connector: {}", dst),
            };

            let sock_fd = socket(
                AddressFamily::Vsock,
                SockType::Stream,
                SockFlag::empty(),
                None,
            )?;

            let sock_addr = VsockAddr::new(cid, port as u32);
            connect(sock_fd, &sock_addr)?;

            // connect sync, but set nonblock after (tokio requires it)
            let std_stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(sock_fd) };
            std_stream.set_nonblocking(true)?;

            let stream = tokio::net::UnixStream::from_std(std_stream)?;
            let connection = UnixConnection { stream };

            Ok(connection)
        })
        // unravel the thread JoinHandle to a usable future
        .map(|res| match res {
            Ok(res) => res,
            Err(err) => Err(format_err!("thread join error on vsock connect: {}", err)),
        })
        .boxed()
    }
}

impl Connection for UnixConnection {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}

impl AsyncRead for UnixConnection {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<Result<(), std::io::Error>> {
        let this = self.project();
        this.stream.poll_read(cx, buf)
    }
}

impl AsyncWrite for UnixConnection {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<tokio::io::Result<usize>> {
        let this = self.project();
        this.stream.poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<tokio::io::Result<()>> {
        let this = self.project();
        this.stream.poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<tokio::io::Result<()>> {
        let this = self.project();
        this.stream.poll_shutdown(cx)
    }
}

/// Slimmed down version of HttpClient for virtio-vsock connections (file restore daemon)
pub struct VsockClient {
    client: Client<VsockConnector>,
    cid: i32,
    port: u16,
    auth: Option<String>,
}

impl VsockClient {
    pub fn new(cid: i32, port: u16, auth: Option<String>) -> Self {
        let conn = VsockConnector {};
        let client = Client::builder().build::<_, Body>(conn);
        Self {
            client,
            cid,
            port,
            auth,
        }
    }

    pub async fn get(&self, path: &str, data: Option<Value>) -> Result<Value, Error> {
        let req = self.request_builder("GET", path, data)?;
        self.api_request(req).await
    }

    pub async fn post(&self, path: &str, data: Option<Value>) -> Result<Value, Error> {
        let req = self.request_builder("POST", path, data)?;
        self.api_request(req).await
    }

    pub async fn download(
        &self,
        path: &str,
        data: Option<Value>,
        output: &mut (dyn AsyncWrite + Send + Unpin),
    ) -> Result<(), Error> {
        let req = self.request_builder("GET", path, data)?;

        let client = self.client.clone();

        let resp = client
            .request(req)
            .await
            .map_err(|_| format_err!("vsock download request timed out"))?;
        let status = resp.status();
        if !status.is_success() {
            Self::api_response(resp).await.map(|_| ())?
        } else {
            resp.into_body()
                .map_err(Error::from)
                .try_fold(output, move |acc, chunk| async move {
                    acc.write_all(&chunk).await?;
                    Ok::<_, Error>(acc)
                })
                .await?;
        }
        Ok(())
    }

    async fn api_response(response: Response<Body>) -> Result<Value, Error> {
        let status = response.status();
        let data = hyper::body::to_bytes(response.into_body()).await?;

        let text = String::from_utf8(data.to_vec()).unwrap();
        if status.is_success() {
            if text.is_empty() {
                Ok(Value::Null)
            } else {
                let value: Value = serde_json::from_str(&text)?;
                Ok(value)
            }
        } else {
            Err(Error::from(HttpError::new(status, text)))
        }
    }

    async fn api_request(&self, req: Request<Body>) -> Result<Value, Error> {
        self.client
            .request(req)
            .map_err(Error::from)
            .and_then(Self::api_response)
            .await
    }

    fn request_builder(
        &self,
        method: &str,
        path: &str,
        data: Option<Value>,
    ) -> Result<Request<Body>, Error> {
        let path = path.trim_matches('/');
        let url: Uri = format!("vsock://{}:{}/{}", self.cid, self.port, path).parse()?;

        let make_builder = |content_type: &str, url: &Uri| {
            let mut builder = Request::builder()
                .method(method)
                .uri(url)
                .header(hyper::header::CONTENT_TYPE, content_type);
            if let Some(auth) = &self.auth {
                builder = builder.header(hyper::header::AUTHORIZATION, auth);
            }
            builder
        };

        if let Some(data) = data {
            if method == "POST" {
                let builder = make_builder("application/json", &url);
                let request = builder.body(Body::from(data.to_string()))?;
                return Ok(request);
            } else {
                let query = json_object_to_query(data)?;
                let url: Uri =
                    format!("vsock://{}:{}/{}?{}", self.cid, self.port, path, query).parse()?;
                let builder = make_builder("application/x-www-form-urlencoded", &url);
                let request = builder.body(Body::empty())?;
                return Ok(request);
            }
        }

        let builder = make_builder("application/x-www-form-urlencoded", &url);
        let request = builder.body(Body::empty())?;

        Ok(request)
    }
}
