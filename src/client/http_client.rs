use failure::*;

use http::Uri;
use hyper::Body;
use hyper::client::Client;
use xdg::BaseDirectories;
use chrono::Utc;

use http::{Request, Response};
use http::header::HeaderValue;

use futures::Future;
use futures::stream::Stream;

use serde_json::{json, Value};
use url::percent_encoding::{percent_encode,  DEFAULT_ENCODE_SET};

use crate::tools::{self, BroadcastFuture, tty};

#[derive(Clone)]
struct AuthInfo {
    username: String,
    ticket: String,
    token: String,
}

/// HTTP(S) API client
pub struct HttpClient {
    client: Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>,
    server: String,
    auth: BroadcastFuture<AuthInfo>,
}

fn store_ticket_info(server: &str, username: &str, ticket: &str, token: &str) -> Result<(), Error> {

    let base = BaseDirectories::with_prefix("proxmox-backup")?;

    // usually /run/user/<uid>/...
    let path = base.place_runtime_file("tickets")?;

    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);

    let mut data = tools::file_get_json(&path, Some(json!({})))?;

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

    tools::file_set_contents(path, new_data.to_string().as_bytes(), Some(mode))?;

    Ok(())
}

fn load_ticket_info(server: &str, username: &str) -> Option<(String, String)> {
    let base = match BaseDirectories::with_prefix("proxmox-backup") {
        Ok(b) => b,
        _ => return None,
    };

    // usually /run/user/<uid>/...
    let path = match base.place_runtime_file("tickets") {
        Ok(p) => p,
        _ => return None,
    };

    let data = match tools::file_get_json(&path, None) {
        Ok(v) => v,
        _ => return None,
    };

    let now = Utc::now().timestamp();

    let ticket_lifetime = tools::ticket::TICKET_LIFETIME - 60;

    if let Some(uinfo) = data[server][username].as_object() {
        if let Some(timestamp) = uinfo["timestamp"].as_i64() {
            let age = now - timestamp;
            if age < ticket_lifetime {
                let ticket = match uinfo["ticket"].as_str() {
                    Some(t) => t,
                    None => return None,
                };
                let token = match uinfo["token"].as_str() {
                    Some(t) => t,
                    None => return None,
                };
                return Some((ticket.to_owned(), token.to_owned()));
            }
        }
    }

    None
}

impl HttpClient {

    pub fn new(server: &str, username: &str) -> Result<Self, Error> {
        let client = Self::build_client();

        let password = if let Some((ticket, _token)) = load_ticket_info(server, username) {
            ticket
        } else {
            Self::get_password(&username)?
        };

        let login = Self::credentials(client.clone(), server.to_owned(), username.to_owned(), password);

        Ok(Self {
            client,
            server: String::from(server),
            auth: BroadcastFuture::new(login),
        })
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

    fn build_client() -> Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>> {
        let mut builder = native_tls::TlsConnector::builder();
        // FIXME: We need a CLI option for this!
        builder.danger_accept_invalid_certs(true);
        let tlsconnector = builder.build().unwrap();
        let mut httpc = hyper::client::HttpConnector::new(1);
        httpc.enforce_http(false); // we want https...
        let mut https = hyper_tls::HttpsConnector::from((httpc, tlsconnector));
        https.https_only(true); // force it!
        Client::builder().build::<_, Body>(https)
    }

    pub fn request(&self, mut req: Request<Body>) -> impl Future<Item=Value, Error=Error>  {

        let login = self.auth.listen();

        let client = self.client.clone();

        login.and_then(move |auth| {

            let enc_ticket = format!("PBSAuthCookie={}", percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET));
            req.headers_mut().insert("Cookie", HeaderValue::from_str(&enc_ticket).unwrap());
            req.headers_mut().insert("CSRFPreventionToken", HeaderValue::from_str(&auth.token).unwrap());

            let request = Self::api_request(client, req);

            request
        })
    }

    pub fn get(&self, path: &str, data: Option<Value>) -> impl Future<Item=Value, Error=Error> {

        let req = Self::request_builder(&self.server, "GET", path, data).unwrap();
        self.request(req)
    }

    pub fn delete(&mut self, path: &str, data: Option<Value>) -> impl Future<Item=Value, Error=Error> {

        let req = Self::request_builder(&self.server, "DELETE", path, data).unwrap();
        self.request(req)
    }

    pub fn post(&mut self, path: &str, data: Option<Value>) -> impl Future<Item=Value, Error=Error> {

        let req = Self::request_builder(&self.server, "POST", path, data).unwrap();
        self.request(req)
    }

    pub fn download(&mut self, path: &str, mut output: Box<dyn std::io::Write + Send>) ->  impl Future<Item=(), Error=Error> {

        let mut req = Self::request_builder(&self.server, "GET", path, None).unwrap();

        let login = self.auth.listen();

        let client = self.client.clone();

        login.and_then(move |auth| {

            let enc_ticket = format!("PBSAuthCookie={}", percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET));
            req.headers_mut().insert("Cookie", HeaderValue::from_str(&enc_ticket).unwrap());

            client.request(req)
                .map_err(Error::from)
                .and_then(|resp| {

                    let _status = resp.status(); // fixme: ??

                    resp.into_body()
                        .map_err(Error::from)
                        .for_each(move |chunk| {
                            output.write_all(&chunk)?;
                            Ok(())
                        })

                })
        })
    }

    pub fn upload(&mut self, content_type: &str, body: Body, path: &str) -> impl Future<Item=Value, Error=Error> {

        let path = path.trim_matches('/');
        let url: Uri = format!("https://{}:8007/{}", &self.server, path).parse().unwrap();

        let req = Request::builder()
            .method("POST")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header("Content-Type", content_type)
            .body(body).unwrap();

        self.request(req)
    }

    pub fn h2upgrade(
        &mut self, path:
        &str, param: Option<Value>
    ) -> impl Future<Item=h2::client::SendRequest<bytes::Bytes>, Error=Error> {

        let mut req = Self::request_builder(&self.server, "GET", path, param).unwrap();

        let login = self.auth.listen();

        let client = self.client.clone();

        login.and_then(move |auth| {

            let enc_ticket = format!("PBSAuthCookie={}", percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET));
            req.headers_mut().insert("Cookie", HeaderValue::from_str(&enc_ticket).unwrap());
            req.headers_mut().insert("UPGRADE", HeaderValue::from_str("proxmox-backup-protocol-h2").unwrap());

            client.request(req)
                .map_err(Error::from)
                .and_then(|resp| {

                    let status = resp.status();
                    if status != http::StatusCode::SWITCHING_PROTOCOLS {
                        bail!("h2upgrade failed with status {:?}", status);
                    }

                    Ok(resp.into_body().on_upgrade().map_err(Error::from))
                })
                .flatten()
                .and_then(|upgraded| {
                    h2::client::handshake(upgraded).map_err(Error::from)
                })
                .and_then(|(h2, connection)| {
                    let connection = connection
                        .map_err(|_| panic!("HTTP/2.0 connection failed"));

                    // Spawn a new task to drive the connection state
                    hyper::rt::spawn(connection);

                    // Wait until the `SendRequest` handle has available capacity.
                    h2.ready().map_err(Error::from)
                })
        })
    }

    fn credentials(
        client: Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>,
        server: String,
        username: String,
        password: String,
    ) -> Box<Future<Item=AuthInfo, Error=Error> + Send> {

        let server2 = server.clone();

        let create_request = futures::future::lazy(move || {
            let data = json!({ "username": username, "password": password });
            let req = Self::request_builder(&server, "POST", "/api2/json/access/ticket", Some(data)).unwrap();
            Self::api_request(client, req)
        });

        let login_future = create_request
            .and_then(move |cred| {
                let auth = AuthInfo {
                    username: cred["data"]["username"].as_str().unwrap().to_owned(),
                    ticket: cred["data"]["ticket"].as_str().unwrap().to_owned(),
                    token: cred["data"]["CSRFPreventionToken"].as_str().unwrap().to_owned(),
                };

                let _ = store_ticket_info(&server2, &auth.username, &auth.ticket, &auth.token);

                Ok(auth)
            });

        Box::new(login_future)
    }

    fn api_request(
        client: Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>,
        req: Request<Body>
    ) -> impl Future<Item=Value, Error=Error> {

        client.request(req)
            .map_err(Error::from)
            .and_then(|resp| {

                let status = resp.status();

                resp
                    .into_body()
                    .concat2()
                    .map_err(Error::from)
                    .and_then(move |data| {

                        let text = String::from_utf8(data.to_vec()).unwrap();
                        if status.is_success() {
                            if text.len() > 0 {
                                let value: Value = serde_json::from_str(&text)?;
                                Ok(value)
                            } else {
                                Ok(Value::Null)
                            }
                        } else {
                            bail!("HTTP Error {}: {}", status, text);
                        }
                    })
            })
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

pub struct H2Client {
    h2: h2::client::SendRequest<bytes::Bytes>,
}

impl H2Client {

    pub fn new(h2: h2::client::SendRequest<bytes::Bytes>) -> Self {
        Self { h2 }
    }

    pub fn get(&self, path: &str, param: Option<Value>) -> impl Future<Item=Value, Error=Error> {
        let req = Self::request_builder("localhost", "GET", path, param).unwrap();
        self.request(req)
    }

    pub fn post(&self, path: &str, param: Option<Value>) -> impl Future<Item=Value, Error=Error> {
        let req = Self::request_builder("localhost", "POST", path, param).unwrap();
        self.request(req)
    }

    fn request(
        &self,
        request: Request<()>,
    ) -> impl Future<Item=Value, Error=Error> {

        self.h2.clone()
            .ready()
            .map_err(Error::from)
            .and_then(move |mut send_request| {
                // fixme: what about stream/upload?
                let (response, _stream) = send_request.send_request(request, true).unwrap();
                response
                    .map_err(Error::from)
                    .and_then(Self::h2api_response)
            })
    }

    fn h2api_response(response: Response<h2::RecvStream>) -> impl Future<Item=Value, Error=Error> {

        let status = response.status();

        let (_head, mut body) = response.into_parts();

        // The `release_capacity` handle allows the caller to manage
        // flow control.
        //
        // Whenever data is received, the caller is responsible for
        // releasing capacity back to the server once it has freed
        // the data from memory.
        let mut release_capacity = body.release_capacity().clone();

        body
            .map(move |chunk| {
                // Let the server send more data.
                let _ = release_capacity.release_capacity(chunk.len());
                chunk
            })
            .concat2()
            .map_err(Error::from)
            .and_then(move |data| {
                let text = String::from_utf8(data.to_vec()).unwrap();
                if status.is_success() {
                    if text.len() > 0 {
                        let mut value: Value = serde_json::from_str(&text)?;
                        if let Some(map) = value.as_object_mut() {
                            if let Some(data) = map.remove("data") {
                                return Ok(data);
                            }
                        }
                        bail!("got result without data property");
                    } else {
                        Ok(Value::Null)
                    }
                } else {
                    bail!("HTTP Error {}: {}", status, text);
                }
            })
    }

    pub fn request_builder(server: &str, method: &str, path: &str, data: Option<Value>) -> Result<Request<()>, Error> {
        let path = path.trim_matches('/');
        let url: Uri = format!("https://{}:8007/{}", server, path).parse()?;

        if let Some(data) = data {
            let query = tools::json_object_to_query(data)?;
            let url: Uri = format!("https://{}:8007/{}?{}", server, path, query).parse()?;
            let request = Request::builder()
                .method(method)
                .uri(url)
                .header("User-Agent", "proxmox-backup-client/1.0")
                .header(hyper::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(())?;
            return Ok(request);
        }

        let request = Request::builder()
            .method(method)
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header(hyper::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(())?;

        Ok(request)
    }
}
