use std::io::Write;
use std::task::{Context, Poll};

use chrono::Utc;
use failure::*;
use futures::*;
use http::Uri;
use http::header::HeaderValue;
use http::{Request, Response};
use hyper::Body;
use hyper::client::{Client, HttpConnector};
use openssl::ssl::{SslConnector, SslMethod};
use serde_json::{json, Value};
use percent_encoding::percent_encode;
use xdg::BaseDirectories;

use proxmox::tools::{
    fs::{file_get_json, file_set_contents},
};

use super::pipe_to_stream::PipeToSendStream;
use crate::tools::async_io::EitherStream;
use crate::tools::futures::{cancellable, Canceller};
use crate::tools::{self, tty, BroadcastFuture, DEFAULT_ENCODE_SET};

#[derive(Clone)]
pub struct AuthInfo {
    username: String,
    ticket: String,
    token: String,
}

/// HTTP(S) API client
pub struct HttpClient {
    client: Client<HttpsConnector>,
    server: String,
    auth: BroadcastFuture<AuthInfo>,
}

/// Delete stored ticket data (logout)
pub fn delete_ticket_info(server: &str, username: &str) -> Result<(), Error> {

    let base = BaseDirectories::with_prefix("proxmox-backup")?;

    // usually /run/user/<uid>/...
    let path = base.place_runtime_file("tickets")?;

    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);

    let mut data = file_get_json(&path, Some(json!({})))?;

    if let Some(map) = data[server].as_object_mut() {
        map.remove(username);
    }

    file_set_contents(path, data.to_string().as_bytes(), Some(mode))?;

    Ok(())
}

fn store_ticket_info(server: &str, username: &str, ticket: &str, token: &str) -> Result<(), Error> {

    let base = BaseDirectories::with_prefix("proxmox-backup")?;

    // usually /run/user/<uid>/...
    let path = base.place_runtime_file("tickets")?;

    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);

    let mut data = file_get_json(&path, Some(json!({})))?;

    let now = Utc::now().timestamp();

    data[server][username] = json!({ "timestamp": now, "ticket": ticket, "token": token});

    let mut new_data = json!({});

    let ticket_lifetime = tools::ticket::TICKET_LIFETIME - 60;

    let empty = serde_json::map::Map::new();
    for (server, info) in data.as_object().unwrap_or(&empty) {
        for (_user, uinfo) in info.as_object().unwrap_or(&empty) {
            if let Some(timestamp) = uinfo["timestamp"].as_i64() {
                let age = now - timestamp;
                if age < ticket_lifetime {
                    new_data[server][username] = uinfo.clone();
                }
            }
        }
    }

    file_set_contents(path, new_data.to_string().as_bytes(), Some(mode))?;

    Ok(())
}

fn load_ticket_info(server: &str, username: &str) -> Option<(String, String)> {
    let base = BaseDirectories::with_prefix("proxmox-backup").ok()?;

    // usually /run/user/<uid>/...
    let path = base.place_runtime_file("tickets").ok()?;
    let data = file_get_json(&path, None).ok()?;
    let now = Utc::now().timestamp();
    let ticket_lifetime = tools::ticket::TICKET_LIFETIME - 60;
    let uinfo = data[server][username].as_object()?;
    let timestamp = uinfo["timestamp"].as_i64()?;
    let age = now - timestamp;

    if age < ticket_lifetime {
        let ticket = uinfo["ticket"].as_str()?;
        let token = uinfo["token"].as_str()?;
        Some((ticket.to_owned(), token.to_owned()))
    } else {
        None
    }
}

impl HttpClient {

    pub fn new(server: &str, username: &str, password: Option<String>) -> Result<Self, Error> {
        let client = Self::build_client();

        let password = if let Some(password) = password {
            password
        } else if let Some((ticket, _token)) = load_ticket_info(server, username) {
            ticket
        } else {
            Self::get_password(&username)?
        };

        let login_future = Self::credentials(client.clone(), server.to_owned(), username.to_owned(), password);

        Ok(Self {
            client,
            server: String::from(server),
            auth: BroadcastFuture::new(Box::new(login_future)),
        })
    }

    /// Login
    ///
    /// Login is done on demand, so this is onyl required if you need
    /// access to authentication data in 'AuthInfo'.
    pub async fn login(&self) -> Result<AuthInfo, Error> {
        self.auth.listen().await
    }

    fn get_password(_username: &str) -> Result<String, Error> {
        use std::env::VarError::*;
        match std::env::var("PBS_PASSWORD") {
            Ok(p) => return Ok(p),
            Err(NotUnicode(_)) => bail!("PBS_PASSWORD contains bad characters"),
            Err(NotPresent) => {
                // Try another method
            }
        }

        // If we're on a TTY, query the user for a password
        if tty::stdin_isatty() {
            return Ok(String::from_utf8(tty::read_password("Password: ")?)?);
        }

        bail!("no password input mechanism available");
    }

    fn build_client() -> Client<HttpsConnector> {

        let mut ssl_connector_builder = SslConnector::builder(SslMethod::tls()).unwrap();

        ssl_connector_builder.set_verify(openssl::ssl::SslVerifyMode::NONE); // fixme!

        let mut httpc = hyper::client::HttpConnector::new();
        httpc.set_nodelay(true); // important for h2 download performance!
        httpc.set_recv_buffer_size(Some(1024*1024)); //important for h2 download performance!
        httpc.enforce_http(false); // we want https...

        let https = HttpsConnector::with_connector(httpc, ssl_connector_builder.build());

        Client::builder()
        //.http2_initial_stream_window_size( (1 << 31) - 2)
        //.http2_initial_connection_window_size( (1 << 31) - 2)
            .build::<_, Body>(https)
    }

    pub async fn request(&self, mut req: Request<Body>) -> Result<Value, Error> {

        let client = self.client.clone();

        let auth =  self.login().await?;

        let enc_ticket = format!("PBSAuthCookie={}", percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET));
        req.headers_mut().insert("Cookie", HeaderValue::from_str(&enc_ticket).unwrap());
        req.headers_mut().insert("CSRFPreventionToken", HeaderValue::from_str(&auth.token).unwrap());

        Self::api_request(client, req).await
    }

    pub async fn get(
        &self,
        path: &str,
        data: Option<Value>,
    ) -> Result<Value, Error> {
        let req = Self::request_builder(&self.server, "GET", path, data).unwrap();
        self.request(req).await
    }

    pub async fn delete(
        &mut self,
        path: &str,
        data: Option<Value>,
    ) -> Result<Value, Error> {
        let req = Self::request_builder(&self.server, "DELETE", path, data).unwrap();
        self.request(req).await
    }

    pub async fn post(
        &mut self,
        path: &str,
        data: Option<Value>,
    ) -> Result<Value, Error> {
        let req = Self::request_builder(&self.server, "POST", path, data).unwrap();
        self.request(req).await
    }

    pub async fn download(
        &mut self,
        path: &str,
        output: &mut (dyn Write + Send),
    ) ->  Result<(), Error> {
        let mut req = Self::request_builder(&self.server, "GET", path, None).unwrap();

        let client = self.client.clone();

        let auth = self.login().await?;

        let enc_ticket = format!("PBSAuthCookie={}", percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET));
        req.headers_mut().insert("Cookie", HeaderValue::from_str(&enc_ticket).unwrap());

        let resp = client.request(req).await?;
        let status = resp.status();
        if !status.is_success() {
            HttpClient::api_response(resp)
                .map(|_| Err(format_err!("unknown error")))
                .await?
        } else {
            resp.into_body()
                .map_err(Error::from)
                .try_fold(output, move |acc, chunk| async move {
                    acc.write_all(&chunk)?;
                    Ok::<_, Error>(acc)
                })
                .await?;
        }
        Ok(())
    }

    pub async fn upload(
        &mut self,
        content_type: &str,
        body: Body,
        path: &str,
        data: Option<Value>,
    ) -> Result<Value, Error> {

        let path = path.trim_matches('/');
        let mut url = format!("https://{}:8007/{}", &self.server, path);

        if let Some(data) = data {
            let query = tools::json_object_to_query(data).unwrap();
            url.push('?');
            url.push_str(&query);
        }

        let url: Uri = url.parse().unwrap();

        let req = Request::builder()
            .method("POST")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header("Content-Type", content_type)
            .body(body).unwrap();

        self.request(req).await
    }

    pub async fn start_h2_connection(
        &self,
        mut req: Request<Body>,
        protocol_name: String,
    ) -> Result<(H2Client, Canceller), Error> {

        let auth = self.login().await?;
        let client = self.client.clone();

        let enc_ticket = format!("PBSAuthCookie={}", percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET));
        req.headers_mut().insert("Cookie", HeaderValue::from_str(&enc_ticket).unwrap());
        req.headers_mut().insert("UPGRADE", HeaderValue::from_str(&protocol_name).unwrap());

        let resp = client.request(req).await?;
        let status = resp.status();

        if status != http::StatusCode::SWITCHING_PROTOCOLS {
            Self::api_response(resp)
                .map(|_| Err(format_err!("unknown error")))
                .await?;
            unreachable!();
        }

        let upgraded = resp
            .into_body()
            .on_upgrade()
            .await?;

        let max_window_size = (1 << 31) - 2;

        let (h2, connection) = h2::client::Builder::new()
            .initial_connection_window_size(max_window_size)
            .initial_window_size(max_window_size)
            .max_frame_size(4*1024*1024)
            .handshake(upgraded)
            .await?;

        let connection = connection
            .map_err(|_| panic!("HTTP/2.0 connection failed"));

        let (connection, canceller) = cancellable(connection)?;
        // A cancellable future returns an Option which is None when cancelled and
        // Some when it finished instead, since we don't care about the return type we
        // need to map it away:
        let connection = connection.map(|_| ());

        // Spawn a new task to drive the connection state
        tokio::spawn(connection);

        // Wait until the `SendRequest` handle has available capacity.
        let c = h2.ready().await?;
        Ok((H2Client::new(c), canceller))
    }

    async fn credentials(
        client: Client<HttpsConnector>,
        server: String,
        username: String,
        password: String,
    ) -> Result<AuthInfo, Error> {
        let data = json!({ "username": username, "password": password });
        let req = Self::request_builder(&server, "POST", "/api2/json/access/ticket", Some(data)).unwrap();
        let cred = Self::api_request(client, req).await?;
        let auth = AuthInfo {
            username: cred["data"]["username"].as_str().unwrap().to_owned(),
            ticket: cred["data"]["ticket"].as_str().unwrap().to_owned(),
            token: cred["data"]["CSRFPreventionToken"].as_str().unwrap().to_owned(),
        };

        let _ = store_ticket_info(&server, &auth.username, &auth.ticket, &auth.token);

        Ok(auth)
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
            bail!("HTTP Error {}: {}", status, text);
        }
    }

    async fn api_request(
        client: Client<HttpsConnector>,
        req: Request<Body>
    ) -> Result<Value, Error> {

        client.request(req)
            .map_err(Error::from)
            .and_then(Self::api_response)
            .await
    }

    // Read-only access to server property
    pub fn server(&self) -> &str {
        &self.server
    }

    pub fn request_builder(server: &str, method: &str, path: &str, data: Option<Value>) -> Result<Request<Body>, Error> {
        let path = path.trim_matches('/');
        let url: Uri = format!("https://{}:8007/{}", server, path).parse()?;

        if let Some(data) = data {
            if method == "POST" {
                let request = Request::builder()
                    .method(method)
                    .uri(url)
                    .header("User-Agent", "proxmox-backup-client/1.0")
                    .header(hyper::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(data.to_string()))?;
                return Ok(request);
            } else {
                let query = tools::json_object_to_query(data)?;
                let url: Uri = format!("https://{}:8007/{}?{}", server, path, query).parse()?;
                let request = Request::builder()
                    .method(method)
                    .uri(url)
                    .header("User-Agent", "proxmox-backup-client/1.0")
                    .header(hyper::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::empty())?;
                return Ok(request);
            }
        }

        let request = Request::builder()
            .method(method)
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header(hyper::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::empty())?;

        Ok(request)
    }
}


#[derive(Clone)]
pub struct H2Client {
    h2: h2::client::SendRequest<bytes::Bytes>,
}

impl H2Client {

    pub fn new(h2: h2::client::SendRequest<bytes::Bytes>) -> Self {
        Self { h2 }
    }

    pub async fn get(
        &self,
        path: &str,
        param: Option<Value>
    ) -> Result<Value, Error> {
        let req = Self::request_builder("localhost", "GET", path, param, None).unwrap();
        self.request(req).await
    }

    pub async fn put(
        &self,
        path: &str,
        param: Option<Value>
    ) -> Result<Value, Error> {
        let req = Self::request_builder("localhost", "PUT", path, param, None).unwrap();
        self.request(req).await
    }

    pub async fn post(
        &self,
        path: &str,
        param: Option<Value>
    ) -> Result<Value, Error> {
        let req = Self::request_builder("localhost", "POST", path, param, None).unwrap();
        self.request(req).await
    }

    pub async fn download<W: Write + Send>(
        &self,
        path: &str,
        param: Option<Value>,
        mut output: W,
    ) -> Result<W, Error> {
        let request = Self::request_builder("localhost", "GET", path, param, None).unwrap();

        let response_future = self.send_request(request, None).await?;

        let resp = response_future.await?;

        let status = resp.status();
        if !status.is_success() {
            H2Client::h2api_response(resp).await?; // raise error
            unreachable!();
        }

        let mut body = resp.into_body();
        while let Some(chunk) = body.data().await {
            let chunk = chunk?;
            body.flow_control().release_capacity(chunk.len())?;
            output.write_all(&chunk)?;
        }

        Ok(output)
    }

    pub async fn upload(
        &self,
        method: &str, // POST or PUT
        path: &str,
        param: Option<Value>,
        content_type: &str,
        data: Vec<u8>,
    ) -> Result<Value, Error> {
        let request = Self::request_builder("localhost", method, path, param, Some(content_type)).unwrap();

        let mut send_request = self.h2.clone().ready().await?;

        let (response, stream) = send_request.send_request(request, false).unwrap();

        PipeToSendStream::new(bytes::Bytes::from(data), stream).await?;

        response
            .map_err(Error::from)
            .and_then(Self::h2api_response)
            .await
    }

    async fn request(
        &self,
        request: Request<()>,
    ) -> Result<Value, Error> {

        self.send_request(request, None)
            .and_then(move |response| {
                response
                    .map_err(Error::from)
                    .and_then(Self::h2api_response)
            })
            .await
    }

    pub fn send_request(
        &self,
        request: Request<()>,
        data: Option<bytes::Bytes>,
    ) -> impl Future<Output = Result<h2::client::ResponseFuture, Error>> {

        self.h2.clone()
            .ready()
            .map_err(Error::from)
            .and_then(move |mut send_request| async move {
                if let Some(data) = data {
                    let (response, stream) = send_request.send_request(request, false).unwrap();
                    PipeToSendStream::new(data, stream).await?;
                    Ok(response)
                } else {
                    let (response, _stream) = send_request.send_request(request, true).unwrap();
                    Ok(response)
                }
            })
    }

    pub async fn h2api_response(
        response: Response<h2::RecvStream>,
    ) -> Result<Value, Error> {
        let status = response.status();

        let (_head, mut body) = response.into_parts();

        let mut data = Vec::new();
        while let Some(chunk) = body.data().await {
            let chunk = chunk?;
            // Whenever data is received, the caller is responsible for
            // releasing capacity back to the server once it has freed
            // the data from memory.
            // Let the server send more data.
            body.flow_control().release_capacity(chunk.len())?;
            data.extend(chunk);
        }

        let text = String::from_utf8(data.to_vec()).unwrap();
        if status.is_success() {
            if text.is_empty() {
                Ok(Value::Null)
            } else {
                let mut value: Value = serde_json::from_str(&text)?;
                if let Some(map) = value.as_object_mut() {
                    if let Some(data) = map.remove("data") {
                        return Ok(data);
                    }
                }
                bail!("got result without data property");
            }
        } else {
            bail!("HTTP Error {}: {}", status, text);
        }
    }

    // Note: We always encode parameters with the url
    pub fn request_builder(
        server: &str,
        method: &str,
        path: &str,
        param: Option<Value>,
        content_type: Option<&str>,
    ) -> Result<Request<()>, Error> {
        let path = path.trim_matches('/');

        let content_type = content_type.unwrap_or("application/x-www-form-urlencoded");

        if let Some(param) = param {
            let query = tools::json_object_to_query(param)?;
            // We detected problem with hyper around 6000 characters - seo we try to keep on the safe side
            if query.len() > 4096 { bail!("h2 query data too large ({} bytes) - please encode data inside body", query.len()); }
            let url: Uri = format!("https://{}:8007/{}?{}", server, path, query).parse()?;
             let request = Request::builder()
                .method(method)
                .uri(url)
                .header("User-Agent", "proxmox-backup-client/1.0")
                .header(hyper::header::CONTENT_TYPE, content_type)
                .body(())?;
            Ok(request)
        } else {
            let url: Uri = format!("https://{}:8007/{}", server, path).parse()?;
            let request = Request::builder()
                .method(method)
                .uri(url)
                .header("User-Agent", "proxmox-backup-client/1.0")
                .header(hyper::header::CONTENT_TYPE, content_type)
                .body(())?;

            Ok(request)
        }
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
            if is_https {
                let conn = tokio_openssl::connect(config?, &host, conn).await?;
                Ok(MaybeTlsStream::Right(conn))
            } else {
                Ok(MaybeTlsStream::Left(conn))
            }
        }.boxed()
    }
}
