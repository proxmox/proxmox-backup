use std::io::Write;
use std::task::{Context, Poll};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use anyhow::{bail, format_err, Error};
use futures::*;
use http::Uri;
use http::header::HeaderValue;
use http::{Request, Response};
use hyper::Body;
use hyper::client::{Client, HttpConnector};
use openssl::{ssl::{SslConnector, SslMethod}, x509::X509StoreContextRef};
use serde_json::{json, Value};
use percent_encoding::percent_encode;
use xdg::BaseDirectories;

use proxmox::{
    sys::linux::tty,
    tools::{
        fs::{file_get_json, replace_file, CreateOptions},
    }
};

use super::pipe_to_stream::PipeToSendStream;
use crate::tools::async_io::EitherStream;
use crate::tools::{self, BroadcastFuture, DEFAULT_ENCODE_SET};

#[derive(Clone)]
pub struct AuthInfo {
    pub username: String,
    pub ticket: String,
    pub token: String,
}

pub struct HttpClientOptions {
    prefix: Option<String>,
    password: Option<String>,
    fingerprint: Option<String>,
    interactive: bool,
    ticket_cache: bool,
    fingerprint_cache: bool,
    verify_cert: bool,
}

impl HttpClientOptions {

    pub fn new() -> Self {
        Self {
            prefix: None,
            password: None,
            fingerprint: None,
            interactive: false,
            ticket_cache: false,
            fingerprint_cache: false,
            verify_cert: true,
        }
    }

    pub fn prefix(mut self, prefix: Option<String>) -> Self {
        self.prefix = prefix;
        self
    }

    pub fn password(mut self, password: Option<String>) -> Self {
        self.password = password;
        self
    }

    pub fn fingerprint(mut self, fingerprint: Option<String>) -> Self {
        self.fingerprint = fingerprint;
        self
    }

    pub fn interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }

    pub fn ticket_cache(mut self, ticket_cache: bool) -> Self {
        self.ticket_cache = ticket_cache;
        self
    }

    pub fn fingerprint_cache(mut self, fingerprint_cache: bool) -> Self {
        self.fingerprint_cache = fingerprint_cache;
        self
    }

    pub fn verify_cert(mut self, verify_cert: bool) -> Self {
        self.verify_cert = verify_cert;
        self
    }
}

/// HTTP(S) API client
pub struct HttpClient {
    client: Client<HttpsConnector>,
    server: String,
    fingerprint: Arc<Mutex<Option<String>>>,
    auth: BroadcastFuture<AuthInfo>,
    _options: HttpClientOptions,
}

/// Delete stored ticket data (logout)
pub fn delete_ticket_info(prefix: &str, server: &str, username: &str) -> Result<(), Error> {

    let base = BaseDirectories::with_prefix(prefix)?;

    // usually /run/user/<uid>/...
    let path = base.place_runtime_file("tickets")?;

    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);

    let mut data = file_get_json(&path, Some(json!({})))?;

    if let Some(map) = data[server].as_object_mut() {
        map.remove(username);
    }

    replace_file(path, data.to_string().as_bytes(), CreateOptions::new().perm(mode))?;

    Ok(())
}

fn store_fingerprint(prefix: &str, server: &str, fingerprint: &str) -> Result<(), Error> {

    let base = BaseDirectories::with_prefix(prefix)?;

    // usually ~/.config/<prefix>/fingerprints
    let path = base.place_config_file("fingerprints")?;

    let raw = match std::fs::read_to_string(&path) {
        Ok(v) => v,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                String::new()
            } else {
                bail!("unable to read fingerprints from {:?} - {}", path, err);
            }
        }
    };

    let mut result = String::new();

    raw.split('\n').for_each(|line| {
        let items: Vec<String> = line.split_whitespace().map(String::from).collect();
        if items.len() == 2 {
            if &items[0] == server {
                // found, add later with new fingerprint
            } else {
                result.push_str(line);
                result.push('\n');
            }
        }
    });

    result.push_str(server);
    result.push(' ');
    result.push_str(fingerprint);
    result.push('\n');

    replace_file(path, result.as_bytes(), CreateOptions::new())?;

    Ok(())
}

fn load_fingerprint(prefix: &str, server: &str) -> Option<String> {

    let base = BaseDirectories::with_prefix(prefix).ok()?;

    // usually ~/.config/<prefix>/fingerprints
    let path = base.place_config_file("fingerprints").ok()?;

    let raw = std::fs::read_to_string(&path).ok()?;

    for line in raw.split('\n') {
        let items: Vec<String> = line.split_whitespace().map(String::from).collect();
        if items.len() == 2 {
            if &items[0] == server {
                return Some(items[1].clone());
            }
        }
    }

    None
}

fn store_ticket_info(prefix: &str, server: &str, username: &str, ticket: &str, token: &str) -> Result<(), Error> {

    let base = BaseDirectories::with_prefix(prefix)?;

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

    replace_file(path, new_data.to_string().as_bytes(), CreateOptions::new().perm(mode))?;

    Ok(())
}

fn load_ticket_info(prefix: &str, server: &str, username: &str) -> Option<(String, String)> {
    let base = BaseDirectories::with_prefix(prefix).ok()?;

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

    pub fn new(server: &str, username: &str, mut options: HttpClientOptions) -> Result<Self, Error> {

        let verified_fingerprint = Arc::new(Mutex::new(None));

        let mut fingerprint = options.fingerprint.take();

        if fingerprint.is_some() {
            // do not store fingerprints passed via options in cache
            options.fingerprint_cache = false;
        } else if options.fingerprint_cache && options.prefix.is_some() {
            fingerprint = load_fingerprint(options.prefix.as_ref().unwrap(), server);
        }

        let mut ssl_connector_builder = SslConnector::builder(SslMethod::tls()).unwrap();

        if options.verify_cert {
            let server = server.to_string();
            let verified_fingerprint = verified_fingerprint.clone();
            let interactive = options.interactive;
            let fingerprint_cache = options.fingerprint_cache;
            let prefix = options.prefix.clone();
            ssl_connector_builder.set_verify_callback(openssl::ssl::SslVerifyMode::PEER, move |valid, ctx| {
                let (valid, fingerprint) = Self::verify_callback(valid, ctx, fingerprint.clone(), interactive);
                if valid {
                    if let Some(fingerprint) = fingerprint {
                        if fingerprint_cache && prefix.is_some() {
                            if let Err(err) = store_fingerprint(
                                prefix.as_ref().unwrap(), &server, &fingerprint) {
                                eprintln!("{}", err);
                            }
                        }
                        *verified_fingerprint.lock().unwrap() = Some(fingerprint);
                    }
                }
                valid
            });
        } else {
            ssl_connector_builder.set_verify(openssl::ssl::SslVerifyMode::NONE);
        }

        let mut httpc = hyper::client::HttpConnector::new();
        httpc.set_nodelay(true); // important for h2 download performance!
        httpc.set_recv_buffer_size(Some(1024*1024)); //important for h2 download performance!
        httpc.enforce_http(false); // we want https...

        let https = HttpsConnector::with_connector(httpc, ssl_connector_builder.build());

        let client = Client::builder()
        //.http2_initial_stream_window_size( (1 << 31) - 2)
        //.http2_initial_connection_window_size( (1 << 31) - 2)
            .build::<_, Body>(https);

        let password = options.password.take();
        let use_ticket_cache = options.ticket_cache && options.prefix.is_some();

        let password = if let Some(password) = password {
            password
        } else {
            let mut ticket_info = None;
            if use_ticket_cache {
                ticket_info = load_ticket_info(options.prefix.as_ref().unwrap(), server, username);
            }
            if let Some((ticket, _token)) = ticket_info {
                ticket
            } else {
                Self::get_password(&username, options.interactive)?
            }
        };

        let login_future = Self::credentials(
            client.clone(),
            server.to_owned(),
            username.to_owned(),
            password,
        ).map_ok({
            let server = server.to_string();
            let prefix = options.prefix.clone();

            move |auth| {
                if use_ticket_cache & &prefix.is_some() {
                    let _ = store_ticket_info(prefix.as_ref().unwrap(), &server, &auth.username, &auth.ticket, &auth.token);
                }

                auth
            }
        });

        Ok(Self {
            client,
            server: String::from(server),
            fingerprint: verified_fingerprint,
            auth: BroadcastFuture::new(Box::new(login_future)),
            _options: options,
        })
    }

    /// Login
    ///
    /// Login is done on demand, so this is only required if you need
    /// access to authentication data in 'AuthInfo'.
    pub async fn login(&self) -> Result<AuthInfo, Error> {
        self.auth.listen().await
    }

    /// Returns the optional fingerprint passed to the new() constructor.
    pub fn fingerprint(&self) -> Option<String> {
        (*self.fingerprint.lock().unwrap()).clone()
    }

    fn get_password(username: &str, interactive: bool) -> Result<String, Error> {
        // If we're on a TTY, query the user for a password
        if interactive && tty::stdin_isatty() {
            let msg = format!("Password for \"{}\": ", username);
            return Ok(String::from_utf8(tty::read_password(&msg)?)?);
        }

        bail!("no password input mechanism available");
    }

    fn verify_callback(
        valid: bool, ctx:
        &mut X509StoreContextRef,
        expected_fingerprint: Option<String>,
        interactive: bool,
    ) -> (bool, Option<String>) {
        if valid { return (true, None); }

        let cert = match ctx.current_cert() {
            Some(cert) => cert,
            None => return (false, None),
        };

        let depth = ctx.error_depth();
        if depth != 0 { return (false, None); }

        let fp = match cert.digest(openssl::hash::MessageDigest::sha256()) {
            Ok(fp) => fp,
            Err(_) => return (false, None), // should not happen
        };
        let fp_string = proxmox::tools::digest_to_hex(&fp);
        let fp_string = fp_string.as_bytes().chunks(2).map(|v| std::str::from_utf8(v).unwrap())
            .collect::<Vec<&str>>().join(":");

        if let Some(expected_fingerprint) = expected_fingerprint {
            if expected_fingerprint.to_lowercase() == fp_string {
                return (true, Some(fp_string));
            } else {
                return (false, None);
            }
        }

        // If we're on a TTY, query the user
        if interactive && tty::stdin_isatty() {
            println!("fingerprint: {}", fp_string);
            loop {
                print!("Are you sure you want to continue connecting? (y/n): ");
                let _ = std::io::stdout().flush();
                use std::io::{BufRead, BufReader};
                let mut line = String::new();
                match BufReader::new(std::io::stdin()).read_line(&mut line) {
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed == "y" || trimmed == "Y" {
                            return (true, Some(fp_string));
                        } else if trimmed == "n" || trimmed == "N" {
                            return (false, None);
                        } else {
                            continue;
                        }
                    }
                    Err(_) => return (false, None),
                }
            }
        }
        (false, None)
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
    ) -> Result<(H2Client, futures::future::AbortHandle), Error> {

        let auth = self.login().await?;
        let client = self.client.clone();

        let enc_ticket = format!("PBSAuthCookie={}", percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET));
        req.headers_mut().insert("Cookie", HeaderValue::from_str(&enc_ticket).unwrap());
        req.headers_mut().insert("UPGRADE", HeaderValue::from_str(&protocol_name).unwrap());

        let resp = client.request(req).await?;
        let status = resp.status();

        if status != http::StatusCode::SWITCHING_PROTOCOLS {
            Self::api_response(resp).await?;
            bail!("unknown error");
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

        let (connection, abort) = futures::future::abortable(connection);
        // A cancellable future returns an Option which is None when cancelled and
        // Some when it finished instead, since we don't care about the return type we
        // need to map it away:
        let connection = connection.map(|_| ());

        // Spawn a new task to drive the connection state
        tokio::spawn(connection);

        // Wait until the `SendRequest` handle has available capacity.
        let c = h2.ready().await?;
        Ok((H2Client::new(c), abort))
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
