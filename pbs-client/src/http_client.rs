use std::io::{IsTerminal, Write};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::{bail, format_err, Error};
use futures::*;
use http::header::HeaderValue;
use http::Uri;
use http::{Request, Response};
use hyper::client::{Client, HttpConnector};
use hyper::Body;
use openssl::{
    ssl::{SslConnector, SslMethod},
    x509::X509StoreContextRef,
};
use percent_encoding::percent_encode;
use serde_json::{json, Value};
use xdg::BaseDirectories;

use proxmox_router::HttpError;
use proxmox_sys::fs::{file_get_json, replace_file, CreateOptions};
use proxmox_sys::linux::tty;

use proxmox_async::broadcast_future::BroadcastFuture;
use proxmox_http::client::HttpsConnector;
use proxmox_http::uri::{build_authority, json_object_to_query};
use proxmox_http::{ProxyConfig, RateLimiter};

use pbs_api_types::percent_encoding::DEFAULT_ENCODE_SET;
use pbs_api_types::{Authid, RateLimitConfig, Userid};

use super::pipe_to_stream::PipeToSendStream;
use super::PROXMOX_BACKUP_TCP_KEEPALIVE_TIME;

/// Timeout used for several HTTP operations that are expected to finish quickly but may block in
/// certain error conditions. Keep it generous, to avoid false-positive under high load.
const HTTP_TIMEOUT: Duration = Duration::from_secs(2 * 60);

#[derive(Clone)]
pub struct AuthInfo {
    pub auth_id: Authid,
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
    limit: RateLimitConfig,
}

impl HttpClientOptions {
    pub fn new_interactive(password: Option<String>, fingerprint: Option<String>) -> Self {
        Self {
            password,
            fingerprint,
            fingerprint_cache: true,
            ticket_cache: true,
            interactive: true,
            prefix: Some("proxmox-backup".to_string()),
            ..Self::default()
        }
    }

    pub fn new_non_interactive(password: String, fingerprint: Option<String>) -> Self {
        Self {
            password: Some(password),
            fingerprint,
            ..Self::default()
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

    pub fn rate_limit(mut self, rate_limit: RateLimitConfig) -> Self {
        self.limit = rate_limit;
        self
    }
}

impl Default for HttpClientOptions {
    fn default() -> Self {
        Self {
            prefix: None,
            password: None,
            fingerprint: None,
            interactive: false,
            ticket_cache: false,
            fingerprint_cache: false,
            verify_cert: true,
            limit: RateLimitConfig::default(), // unlimited
        }
    }
}

/// HTTP(S) API client
pub struct HttpClient {
    client: Client<HttpsConnector>,
    server: String,
    port: u16,
    fingerprint: Arc<Mutex<Option<String>>>,
    first_auth: Option<BroadcastFuture<()>>,
    auth: Arc<RwLock<AuthInfo>>,
    ticket_abort: futures::future::AbortHandle,
    _options: HttpClientOptions,
}

/// Delete stored ticket data (logout)
pub fn delete_ticket_info(prefix: &str, server: &str, username: &Userid) -> Result<(), Error> {
    let base = BaseDirectories::with_prefix(prefix)?;

    // usually /run/user/<uid>/...
    let path = base.place_runtime_file("tickets")?;

    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);

    let mut data = file_get_json(&path, Some(json!({})))?;

    if let Some(map) = data[server].as_object_mut() {
        map.remove(username.as_str());
    }

    replace_file(
        path,
        data.to_string().as_bytes(),
        CreateOptions::new().perm(mode),
        false,
    )?;

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
            if items[0] == server {
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

    replace_file(path, result.as_bytes(), CreateOptions::new(), false)?;

    Ok(())
}

fn load_fingerprint(prefix: &str, server: &str) -> Option<String> {
    let base = BaseDirectories::with_prefix(prefix).ok()?;

    // usually ~/.config/<prefix>/fingerprints
    let path = base.place_config_file("fingerprints").ok()?;

    let raw = std::fs::read_to_string(path).ok()?;

    for line in raw.split('\n') {
        let items: Vec<String> = line.split_whitespace().map(String::from).collect();
        if items.len() == 2 && items[0] == server {
            return Some(items[1].clone());
        }
    }

    None
}

fn store_ticket_info(
    prefix: &str,
    server: &str,
    username: &str,
    ticket: &str,
    token: &str,
) -> Result<(), Error> {
    let base = BaseDirectories::with_prefix(prefix)?;

    // usually /run/user/<uid>/...
    let path = base.place_runtime_file("tickets")?;

    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);

    let mut data = file_get_json(&path, Some(json!({})))?;

    let now = proxmox_time::epoch_i64();

    data[server][username] = json!({ "timestamp": now, "ticket": ticket, "token": token});

    let mut new_data = json!({});

    let ticket_lifetime = proxmox_auth_api::TICKET_LIFETIME - 60;

    let empty = serde_json::map::Map::new();
    for (server, info) in data.as_object().unwrap_or(&empty) {
        for (user, uinfo) in info.as_object().unwrap_or(&empty) {
            if let Some(timestamp) = uinfo["timestamp"].as_i64() {
                let age = now - timestamp;
                if age < ticket_lifetime {
                    new_data[server][user] = uinfo.clone();
                }
            }
        }
    }

    replace_file(
        path,
        new_data.to_string().as_bytes(),
        CreateOptions::new().perm(mode),
        false,
    )?;

    Ok(())
}

fn load_ticket_info(prefix: &str, server: &str, userid: &Userid) -> Option<(String, String)> {
    let base = BaseDirectories::with_prefix(prefix).ok()?;

    // usually /run/user/<uid>/...
    let path = base.place_runtime_file("tickets").ok()?;
    let data = file_get_json(path, None).ok()?;
    let now = proxmox_time::epoch_i64();
    let ticket_lifetime = proxmox_auth_api::TICKET_LIFETIME - 60;
    let uinfo = data[server][userid.as_str()].as_object()?;
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

fn build_uri(server: &str, port: u16, path: &str, query: Option<String>) -> Result<Uri, Error> {
    Uri::builder()
        .scheme("https")
        .authority(build_authority(server, port)?)
        .path_and_query(match query {
            Some(query) => format!("/{}?{}", path, query),
            None => format!("/{}", path),
        })
        .build()
        .map_err(|err| format_err!("error building uri - {}", err))
}

impl HttpClient {
    pub fn new(
        server: &str,
        port: u16,
        auth_id: &Authid,
        mut options: HttpClientOptions,
    ) -> Result<Self, Error> {
        let verified_fingerprint = Arc::new(Mutex::new(None));

        let mut expected_fingerprint = options.fingerprint.take();

        if expected_fingerprint.is_some() {
            // do not store fingerprints passed via options in cache
            options.fingerprint_cache = false;
        } else if options.fingerprint_cache && options.prefix.is_some() {
            expected_fingerprint = load_fingerprint(options.prefix.as_ref().unwrap(), server);
        }

        let mut ssl_connector_builder = SslConnector::builder(SslMethod::tls()).unwrap();

        if options.verify_cert {
            let server = server.to_string();
            let verified_fingerprint = verified_fingerprint.clone();
            let interactive = options.interactive;
            let fingerprint_cache = options.fingerprint_cache;
            let prefix = options.prefix.clone();
            ssl_connector_builder.set_verify_callback(
                openssl::ssl::SslVerifyMode::PEER,
                move |valid, ctx| match Self::verify_callback(
                    valid,
                    ctx,
                    expected_fingerprint.as_ref(),
                    interactive,
                ) {
                    Ok(None) => true,
                    Ok(Some(fingerprint)) => {
                        if fingerprint_cache && prefix.is_some() {
                            if let Err(err) =
                                store_fingerprint(prefix.as_ref().unwrap(), &server, &fingerprint)
                            {
                                log::error!("{}", err);
                            }
                        }
                        *verified_fingerprint.lock().unwrap() = Some(fingerprint);
                        true
                    }
                    Err(err) => {
                        log::error!("certificate validation failed - {}", err);
                        false
                    }
                },
            );
        } else {
            ssl_connector_builder.set_verify(openssl::ssl::SslVerifyMode::NONE);
        }

        let mut httpc = HttpConnector::new();
        httpc.set_nodelay(true); // important for h2 download performance!
        httpc.enforce_http(false); // we want https...

        httpc.set_connect_timeout(Some(std::time::Duration::new(10, 0)));
        let mut https = HttpsConnector::with_connector(
            httpc,
            ssl_connector_builder.build(),
            PROXMOX_BACKUP_TCP_KEEPALIVE_TIME,
        );

        if let Some(rate_in) = options.limit.rate_in {
            let burst_in = options.limit.burst_in.unwrap_or(rate_in).as_u64();
            https.set_read_limiter(Some(Arc::new(Mutex::new(RateLimiter::new(
                rate_in.as_u64(),
                burst_in,
            )))));
        }

        if let Some(rate_out) = options.limit.rate_out {
            let burst_out = options.limit.burst_out.unwrap_or(rate_out).as_u64();
            https.set_write_limiter(Some(Arc::new(Mutex::new(RateLimiter::new(
                rate_out.as_u64(),
                burst_out,
            )))));
        }

        let proxy_config = ProxyConfig::from_proxy_env()?;
        if let Some(config) = proxy_config {
            log::info!("Using proxy connection: {}:{}", config.host, config.port);
            https.set_proxy(config);
        }

        let client = Client::builder()
            //.http2_initial_stream_window_size( (1 << 31) - 2)
            //.http2_initial_connection_window_size( (1 << 31) - 2)
            .build::<_, Body>(https);

        let password = options.password.take();
        let use_ticket_cache = options.ticket_cache && options.prefix.is_some();

        let password = if let Some(password) = password {
            password
        } else {
            let userid = if auth_id.is_token() {
                bail!("API token secret must be provided!");
            } else {
                auth_id.user()
            };
            let mut ticket_info = None;
            if use_ticket_cache {
                ticket_info = load_ticket_info(options.prefix.as_ref().unwrap(), server, userid);
            }
            if let Some((ticket, _token)) = ticket_info {
                ticket
            } else {
                Self::get_password(userid, options.interactive)?
            }
        };

        let auth = Arc::new(RwLock::new(AuthInfo {
            auth_id: auth_id.clone(),
            ticket: password.clone(),
            token: "".to_string(),
        }));

        let server2 = server.to_string();
        let client2 = client.clone();
        let auth2 = auth.clone();
        let prefix2 = options.prefix.clone();

        let renewal_future = async move {
            loop {
                tokio::time::sleep(Duration::new(60 * 15, 0)).await; // 15 minutes
                let (auth_id, ticket) = {
                    let authinfo = auth2.read().unwrap().clone();
                    (authinfo.auth_id, authinfo.ticket)
                };
                match Self::credentials(
                    client2.clone(),
                    server2.clone(),
                    port,
                    auth_id.user().clone(),
                    ticket,
                )
                .await
                {
                    Ok(auth) => {
                        if use_ticket_cache && prefix2.is_some() {
                            if let Err(err) = store_ticket_info(
                                prefix2.as_ref().unwrap(),
                                &server2,
                                &auth.auth_id.to_string(),
                                &auth.ticket,
                                &auth.token,
                            ) {
                                if std::io::stdout().is_terminal() {
                                    log::error!("storing login ticket failed: {}", err);
                                }
                            }
                        }
                        *auth2.write().unwrap() = auth;
                    }
                    Err(err) => {
                        log::error!("re-authentication failed: {}", err);
                        return;
                    }
                }
            }
        };

        let (renewal_future, ticket_abort) = futures::future::abortable(renewal_future);

        let login_future = Self::credentials(
            client.clone(),
            server.to_owned(),
            port,
            auth_id.user().clone(),
            password,
        )
        .map_ok({
            let server = server.to_string();
            let prefix = options.prefix.clone();
            let authinfo = auth.clone();

            move |auth| {
                if use_ticket_cache && prefix.is_some() {
                    if let Err(err) = store_ticket_info(
                        prefix.as_ref().unwrap(),
                        &server,
                        &auth.auth_id.to_string(),
                        &auth.ticket,
                        &auth.token,
                    ) {
                        if std::io::stdout().is_terminal() {
                            log::error!("storing login ticket failed: {}", err);
                        }
                    }
                }
                *authinfo.write().unwrap() = auth;
                tokio::spawn(renewal_future);
            }
        });

        let first_auth = if auth_id.is_token() {
            // TODO check access here?
            None
        } else {
            Some(BroadcastFuture::new(Box::new(login_future)))
        };

        Ok(Self {
            client,
            server: String::from(server),
            port,
            fingerprint: verified_fingerprint,
            auth,
            ticket_abort,
            first_auth,
            _options: options,
        })
    }

    /// Login
    ///
    /// Login is done on demand, so this is only required if you need
    /// access to authentication data in 'AuthInfo'.
    ///
    /// Note: tickets a periodially re-newed, so one can use this
    /// to query changed ticket.
    pub async fn login(&self) -> Result<AuthInfo, Error> {
        if let Some(future) = &self.first_auth {
            future.listen().await?;
        }

        let authinfo = self.auth.read().unwrap();
        Ok(authinfo.clone())
    }

    /// Returns the optional fingerprint passed to the new() constructor.
    pub fn fingerprint(&self) -> Option<String> {
        (*self.fingerprint.lock().unwrap()).clone()
    }

    fn get_password(username: &Userid, interactive: bool) -> Result<String, Error> {
        // If we're on a TTY, query the user for a password
        if interactive && std::io::stdin().is_terminal() {
            let msg = format!("Password for \"{}\": ", username);
            return Ok(String::from_utf8(tty::read_password(&msg)?)?);
        }

        bail!("no password input mechanism available");
    }

    fn verify_callback(
        openssl_valid: bool,
        ctx: &mut X509StoreContextRef,
        expected_fingerprint: Option<&String>,
        interactive: bool,
    ) -> Result<Option<String>, Error> {
        if openssl_valid {
            return Ok(None);
        }

        let cert = match ctx.current_cert() {
            Some(cert) => cert,
            None => bail!("context lacks current certificate."),
        };

        let depth = ctx.error_depth();
        if depth != 0 {
            bail!("context depth != 0")
        }

        let fp = match cert.digest(openssl::hash::MessageDigest::sha256()) {
            Ok(fp) => fp,
            Err(err) => bail!("failed to calculate certificate FP - {}", err), // should not happen
        };
        let fp_string = hex::encode(fp);
        let fp_string = fp_string
            .as_bytes()
            .chunks(2)
            .map(|v| std::str::from_utf8(v).unwrap())
            .collect::<Vec<&str>>()
            .join(":");

        if let Some(expected_fingerprint) = expected_fingerprint {
            let expected_fingerprint = expected_fingerprint.to_lowercase();
            if expected_fingerprint == fp_string {
                return Ok(Some(fp_string));
            } else {
                log::warn!("WARNING: certificate fingerprint does not match expected fingerprint!");
                log::warn!("expected:    {}", expected_fingerprint);
            }
        }

        // If we're on a TTY, query the user
        if interactive && std::io::stdin().is_terminal() {
            log::info!("fingerprint: {}", fp_string);
            loop {
                eprint!("Are you sure you want to continue connecting? (y/n): ");
                let _ = std::io::stdout().flush();
                use std::io::{BufRead, BufReader};
                let mut line = String::new();
                match BufReader::new(std::io::stdin()).read_line(&mut line) {
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed == "y" || trimmed == "Y" {
                            return Ok(Some(fp_string));
                        } else if trimmed == "n" || trimmed == "N" {
                            bail!("Certificate fingerprint was not confirmed.");
                        } else {
                            continue;
                        }
                    }
                    Err(err) => bail!("Certificate fingerprint was not confirmed - {}.", err),
                }
            }
        }

        bail!("Certificate fingerprint was not confirmed.");
    }

    pub async fn request(&self, mut req: Request<Body>) -> Result<Value, Error> {
        let client = self.client.clone();

        let auth = self.login().await?;
        if auth.auth_id.is_token() {
            let enc_api_token = format!(
                "PBSAPIToken {}:{}",
                auth.auth_id,
                percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET)
            );
            req.headers_mut().insert(
                "Authorization",
                HeaderValue::from_str(&enc_api_token).unwrap(),
            );
        } else {
            let enc_ticket = format!(
                "PBSAuthCookie={}",
                percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET)
            );
            req.headers_mut()
                .insert("Cookie", HeaderValue::from_str(&enc_ticket).unwrap());
            req.headers_mut().insert(
                "CSRFPreventionToken",
                HeaderValue::from_str(&auth.token).unwrap(),
            );
        }

        Self::api_request(client, req).await
    }

    pub async fn get(&self, path: &str, data: Option<Value>) -> Result<Value, Error> {
        let req = Self::request_builder(&self.server, self.port, "GET", path, data)?;
        self.request(req).await
    }

    pub async fn delete(&self, path: &str, data: Option<Value>) -> Result<Value, Error> {
        let req = Self::request_builder(&self.server, self.port, "DELETE", path, data)?;
        self.request(req).await
    }

    pub async fn post(&self, path: &str, data: Option<Value>) -> Result<Value, Error> {
        let req = Self::request_builder(&self.server, self.port, "POST", path, data)?;
        self.request(req).await
    }

    pub async fn put(&self, path: &str, data: Option<Value>) -> Result<Value, Error> {
        let req = Self::request_builder(&self.server, self.port, "PUT", path, data)?;
        self.request(req).await
    }

    pub async fn download(&self, path: &str, output: &mut (dyn Write + Send)) -> Result<(), Error> {
        let mut req = Self::request_builder(&self.server, self.port, "GET", path, None)?;

        let client = self.client.clone();

        let auth = self.login().await?;

        let enc_ticket = format!(
            "PBSAuthCookie={}",
            percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET)
        );
        req.headers_mut()
            .insert("Cookie", HeaderValue::from_str(&enc_ticket).unwrap());

        let resp = tokio::time::timeout(HTTP_TIMEOUT, client.request(req))
            .await
            .map_err(|_| format_err!("http download request timed out"))??;
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
        &self,
        content_type: &str,
        body: Body,
        path: &str,
        data: Option<Value>,
    ) -> Result<Value, Error> {
        let query = match data {
            Some(data) => Some(json_object_to_query(data)?),
            None => None,
        };
        let url = build_uri(&self.server, self.port, path, query)?;

        let req = Request::builder()
            .method("POST")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header("Content-Type", content_type)
            .body(body)
            .unwrap();

        self.request(req).await
    }

    pub async fn start_h2_connection(
        &self,
        mut req: Request<Body>,
        protocol_name: String,
    ) -> Result<(H2Client, futures::future::AbortHandle), Error> {
        let client = self.client.clone();
        let auth = self.login().await?;

        if auth.auth_id.is_token() {
            let enc_api_token = format!(
                "PBSAPIToken {}:{}",
                auth.auth_id,
                percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET)
            );
            req.headers_mut().insert(
                "Authorization",
                HeaderValue::from_str(&enc_api_token).unwrap(),
            );
        } else {
            let enc_ticket = format!(
                "PBSAuthCookie={}",
                percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET)
            );
            req.headers_mut()
                .insert("Cookie", HeaderValue::from_str(&enc_ticket).unwrap());
            req.headers_mut().insert(
                "CSRFPreventionToken",
                HeaderValue::from_str(&auth.token).unwrap(),
            );
        }

        req.headers_mut()
            .insert("Connection", HeaderValue::from_str("upgrade").unwrap());
        req.headers_mut()
            .insert("UPGRADE", HeaderValue::from_str(&protocol_name).unwrap());

        let resp = tokio::time::timeout(HTTP_TIMEOUT, client.request(req))
            .await
            .map_err(|_| format_err!("http upgrade request timed out"))??;
        let status = resp.status();

        if status != http::StatusCode::SWITCHING_PROTOCOLS {
            Self::api_response(resp).await?;
            bail!("unknown error");
        }

        let upgraded = hyper::upgrade::on(resp).await?;

        let max_window_size = (1 << 31) - 2;

        let (h2, connection) = h2::client::Builder::new()
            .initial_connection_window_size(max_window_size)
            .initial_window_size(max_window_size)
            .max_frame_size(4 * 1024 * 1024)
            .handshake(upgraded)
            .await?;

        let connection = connection.map_err(|_| log::error!("HTTP/2.0 connection failed"));

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
        port: u16,
        username: Userid,
        password: String,
    ) -> Result<AuthInfo, Error> {
        let data = json!({ "username": username, "password": password });
        let req = Self::request_builder(
            &server,
            port,
            "POST",
            "/api2/json/access/ticket",
            Some(data),
        )?;
        let cred = Self::api_request(client, req).await?;
        let auth = AuthInfo {
            auth_id: cred["data"]["username"].as_str().unwrap().parse()?,
            ticket: cred["data"]["ticket"].as_str().unwrap().to_owned(),
            token: cred["data"]["CSRFPreventionToken"]
                .as_str()
                .unwrap()
                .to_owned(),
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
            Err(Error::from(HttpError::new(status, text)))
        }
    }

    async fn api_request(
        client: Client<HttpsConnector>,
        req: Request<Body>,
    ) -> Result<Value, Error> {
        Self::api_response(
            tokio::time::timeout(HTTP_TIMEOUT, client.request(req))
                .await
                .map_err(|_| format_err!("http request timed out"))??,
        )
        .await
    }

    // Read-only access to server property
    pub fn server(&self) -> &str {
        &self.server
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn request_builder(
        server: &str,
        port: u16,
        method: &str,
        path: &str,
        data: Option<Value>,
    ) -> Result<Request<Body>, Error> {
        if let Some(data) = data {
            if method == "POST" {
                let url = build_uri(server, port, path, None)?;
                let request = Request::builder()
                    .method(method)
                    .uri(url)
                    .header("User-Agent", "proxmox-backup-client/1.0")
                    .header(hyper::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(data.to_string()))?;
                Ok(request)
            } else {
                let query = json_object_to_query(data)?;
                let url = build_uri(server, port, path, Some(query))?;
                let request = Request::builder()
                    .method(method)
                    .uri(url)
                    .header("User-Agent", "proxmox-backup-client/1.0")
                    .header(
                        hyper::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .body(Body::empty())?;
                Ok(request)
            }
        } else {
            let url = build_uri(server, port, path, None)?;
            let request = Request::builder()
                .method(method)
                .uri(url)
                .header("User-Agent", "proxmox-backup-client/1.0")
                .header(
                    hyper::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(Body::empty())?;

            Ok(request)
        }
    }
}

impl Drop for HttpClient {
    fn drop(&mut self) {
        self.ticket_abort.abort();
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

    pub async fn get(&self, path: &str, param: Option<Value>) -> Result<Value, Error> {
        let req = Self::request_builder("localhost", "GET", path, param, None).unwrap();
        self.request(req).await
    }

    pub async fn put(&self, path: &str, param: Option<Value>) -> Result<Value, Error> {
        let req = Self::request_builder("localhost", "PUT", path, param, None).unwrap();
        self.request(req).await
    }

    pub async fn post(&self, path: &str, param: Option<Value>) -> Result<Value, Error> {
        let req = Self::request_builder("localhost", "POST", path, param, None).unwrap();
        self.request(req).await
    }

    pub async fn download<W: Write + Send>(
        &self,
        path: &str,
        param: Option<Value>,
        mut output: W,
    ) -> Result<(), Error> {
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

        Ok(())
    }

    pub async fn upload(
        &self,
        method: &str, // POST or PUT
        path: &str,
        param: Option<Value>,
        content_type: &str,
        data: Vec<u8>,
    ) -> Result<Value, Error> {
        let request =
            Self::request_builder("localhost", method, path, param, Some(content_type)).unwrap();

        let mut send_request = self.h2.clone().ready().await?;

        let (response, stream) = send_request.send_request(request, false).unwrap();

        PipeToSendStream::new(bytes::Bytes::from(data), stream).await?;

        response
            .map_err(Error::from)
            .and_then(Self::h2api_response)
            .await
    }

    async fn request(&self, request: Request<()>) -> Result<Value, Error> {
        self.send_request(request, None)
            .and_then(move |response| response.map_err(Error::from).and_then(Self::h2api_response))
            .await
    }

    pub fn send_request(
        &self,
        request: Request<()>,
        data: Option<bytes::Bytes>,
    ) -> impl Future<Output = Result<h2::client::ResponseFuture, Error>> {
        self.h2
            .clone()
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

    pub async fn h2api_response(response: Response<h2::RecvStream>) -> Result<Value, Error> {
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
            Err(Error::from(HttpError::new(status, text)))
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
        let query = match param {
            Some(param) => {
                let query = json_object_to_query(param)?;
                // We detected problem with hyper around 6000 characters - so we try to keep on the safe side
                if query.len() > 4096 {
                    bail!(
                        "h2 query data too large ({} bytes) - please encode data inside body",
                        query.len()
                    );
                }
                Some(query)
            }
            None => None,
        };

        let url = build_uri(server, 8007, path, query)?;
        let request = Request::builder()
            .method(method)
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header(hyper::header::CONTENT_TYPE, content_type)
            .body(())?;
        Ok(request)
    }
}
