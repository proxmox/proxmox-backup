use failure::*;

use http::Uri;
use hyper::Body;
use hyper::client::Client;
use hyper::rt::{self, Future};
use xdg::BaseDirectories;
use chrono::Utc;

use http::Request;
use futures::stream::Stream;

use serde_json::{json, Value};
use url::percent_encoding::{percent_encode,  DEFAULT_ENCODE_SET};

use crate::tools::{self, tty};

/// HTTP(S) API client
pub struct HttpClient {
    username: String,
    server: String,

    ticket: Option<String>,
    token: Option<String>
}

fn store_ticket_info(server: &str, username: &str, ticket: &str, token: &str) -> Result<(), Error> {

    let base = BaseDirectories::with_prefix("proxmox-backup")?;

    // usually /run/user/<uid>/...
    let path = base.place_runtime_file("tickets")?;

    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);

    let mut data = tools::file_get_json(&path).unwrap_or(json!({}));

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

    let data = tools::file_get_json(&path).unwrap_or(json!({}));

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

    pub fn new(server: &str, username: &str) -> Self {
        Self {
            server: String::from(server),
            username: String::from(username),
            ticket: None,
            token: None,
        }
    }

    fn get_password(&self) -> Result<String, Error> {
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

    fn build_client() -> Result<Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>, Error> {
        let mut builder = native_tls::TlsConnector::builder();
        // FIXME: We need a CLI option for this!
        builder.danger_accept_invalid_certs(true);
        let tlsconnector = builder.build()?;
        let mut httpc = hyper::client::HttpConnector::new(1);
        httpc.enforce_http(false); // we want https...
        let mut https = hyper_tls::HttpsConnector::from((httpc, tlsconnector));
        https.https_only(true); // force it!
        let client = Client::builder().build::<_, Body>(https);
        Ok(client)
    }

    fn run_request(
        request: Request<Body>,
    ) -> Result<Value, Error> {
        let client = Self::build_client()?;

        let (tx, rx) = std::sync::mpsc::channel();

        let future = client
            .request(request)
            .map_err(Error::from)
            .and_then(|resp| {

                let status = resp.status();

                resp.into_body().concat2().map_err(Error::from)
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
            .then(move |res| {
                tx.send(res).unwrap();
                Ok(())
            });

        // drop client, else client keeps connectioon open (keep-alive feature)
        drop(client);

        rt::run(future);

        rx.recv().unwrap()
    }

    fn run_download(
        request: Request<Body>,
        mut output: Box<dyn std::io::Write + Send>,
    ) -> Result<(), Error> {
        let client = Self::build_client()?;

        let (tx, rx) = std::sync::mpsc::channel();

        let future = client
            .request(request)
            .map_err(Error::from)
            .and_then(move |resp| {

                let _status = resp.status(); // fixme: ??

                resp.into_body()
                    .map_err(Error::from)
                    .for_each(move |chunk| {
                        output.write_all(&chunk)?;
                        Ok(())
                    })

            })
            .then(move |res| {
                tx.send(res).unwrap();
                Ok(())
            });

        // drop client, else client keeps connectioon open (keep-alive feature)
        drop(client);

        rt::run(future);

        rx.recv().unwrap()
    }

    pub fn download(&mut self, path: &str, output: Box<dyn std::io::Write + Send>) -> Result<(), Error> {

        let path = path.trim_matches('/');
        let url: Uri = format!("https://{}:8007/{}", self.server, path).parse()?;

        let (ticket, _token) = self.login()?;

        let enc_ticket = percent_encode(ticket.as_bytes(), DEFAULT_ENCODE_SET).to_string();

        let request = Request::builder()
            .method("GET")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header("Cookie", format!("PBSAuthCookie={}", enc_ticket))
            .body(Body::empty())?;

        Self::run_download(request, output)
    }

    pub fn get(&mut self, path: &str) -> Result<Value, Error> {

        let path = path.trim_matches('/');
        let url: Uri = format!("https://{}:8007/{}", self.server, path).parse()?;

        let (ticket, _token) = self.login()?;

        let enc_ticket = percent_encode(ticket.as_bytes(), DEFAULT_ENCODE_SET).to_string();

        let request = Request::builder()
            .method("GET")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header("Cookie", format!("PBSAuthCookie={}", enc_ticket))
            .body(Body::empty())?;

        Self::run_request(request)
    }

    /// like get(), but use existing credentials (never asks for password).
    /// this simply fails when there is no ticket
    pub fn try_get(&mut self, path: &str) -> Result<Value, Error> {

        let path = path.trim_matches('/');
        let url: Uri = format!("https://{}:8007/{}", self.server, path).parse()?;

        let mut credentials = None;

        if let Some(ref ticket) = self.ticket {
            if let Some(ref token) = self.token {
                credentials = Some((ticket.clone(), token.clone()));
            }
        }

        if credentials == None {
            if let Some((ticket, token)) = load_ticket_info(&self.server, &self.username) {
                credentials = Some((ticket.clone(), token.clone()));
            }
        }

        if credentials == None {
            bail!("unable to get credentials");
        }

        let (ticket, _token) = credentials.unwrap();

        let enc_ticket = percent_encode(ticket.as_bytes(), DEFAULT_ENCODE_SET).to_string();

        let request = Request::builder()
            .method("GET")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header("Cookie", format!("PBSAuthCookie={}", enc_ticket))
            .body(Body::empty())?;

        Self::run_request(request)
    }

    pub fn delete(&mut self, path: &str) -> Result<Value, Error> {

        let path = path.trim_matches('/');
        let url: Uri = format!("https://{}:8007/{}", self.server, path).parse()?;

        let (ticket, token) = self.login()?;

        let enc_ticket = percent_encode(ticket.as_bytes(), DEFAULT_ENCODE_SET).to_string();

        let request = Request::builder()
            .method("DELETE")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header("Cookie", format!("PBSAuthCookie={}", enc_ticket))
            .header("CSRFPreventionToken", token)
            .body(Body::empty())?;

        Self::run_request(request)
    }

    pub fn post(&mut self, path: &str) -> Result<Value, Error> {

        let path = path.trim_matches('/');
        let url: Uri = format!("https://{}:8007/{}", self.server, path).parse()?;

        let (ticket, token) = self.login()?;

        let enc_ticket = percent_encode(ticket.as_bytes(), DEFAULT_ENCODE_SET).to_string();

        let request = Request::builder()
            .method("POST")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header("Cookie", format!("PBSAuthCookie={}", enc_ticket))
            .header("CSRFPreventionToken", token)
            .header(hyper::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::empty())?;

        Self::run_request(request)
    }

    pub fn post_json(&mut self, path: &str, data: Value) -> Result<Value, Error> {

        let path = path.trim_matches('/');
        let url: Uri = format!("https://{}:8007/{}", self.server, path).parse()?;

        let (ticket, token) = self.login()?;

        let enc_ticket = percent_encode(ticket.as_bytes(), DEFAULT_ENCODE_SET).to_string();

        let request = Request::builder()
            .method("POST")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header("Cookie", format!("PBSAuthCookie={}", enc_ticket))
            .header("CSRFPreventionToken", token)
            .header(hyper::header::CONTENT_TYPE, "application/json")
            .body(Body::from(data.to_string()))?;

        Self::run_request(request)
    }

    fn try_login(&mut self, password: &str) -> Result<(String, String), Error> {

        let url: Uri = format!("https://{}:8007/{}", self.server, "/api2/json/access/ticket").parse()?;

        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("username", &self.username)
            .append_pair("password", &password)
            .finish();

        let request = Request::builder()
            .method("POST")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(Body::from(query))?;

        let auth_res = Self::run_request(request)?;

        let ticket = match auth_res["data"]["ticket"].as_str() {
            Some(t) => t,
            None => bail!("got unexpected respose for login request."),
        };
        let token = match auth_res["data"]["CSRFPreventionToken"].as_str() {
            Some(t) => t,
            None => bail!("got unexpected respose for login request."),
        };

        Ok((ticket.to_owned(), token.to_owned()))
    }

    pub fn login(&mut self) ->  Result<(String, String), Error> {

        if let Some(ref ticket) = self.ticket {
            if let Some(ref token) = self.token {
                return Ok((ticket.clone(), token.clone()));
            }
        }

        if let Some((ticket, _token)) = load_ticket_info(&self.server, &self.username) {
            if let Ok((ticket, token)) = self.try_login(&ticket) {
                let _ = store_ticket_info(&self.server, &self.username, &ticket, &token);
                return Ok((ticket.to_owned(), token.to_owned()))
            }
        }

        let password = self.get_password()?;
        let (ticket, token) = self.try_login(&password)?;

        let _ = store_ticket_info(&self.server, &self.username, &ticket, &token);

        Ok((ticket.to_owned(), token.to_owned()))
    }

    pub fn upload(&mut self, content_type: &str, body: Body, path: &str) -> Result<Value, Error> {

        let path = path.trim_matches('/');
        let url: Uri = format!("https://{}:8007/{}", self.server, path).parse()?;

        let (ticket, token) = self.login()?;

        let enc_ticket = percent_encode(ticket.as_bytes(), DEFAULT_ENCODE_SET).to_string();

        let request = Request::builder()
            .method("POST")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header("Cookie", format!("PBSAuthCookie={}", enc_ticket))
            .header("CSRFPreventionToken", token)
            .header("Content-Type", content_type)
            .body(body)?;

        Self::run_request(request)
    }
}
