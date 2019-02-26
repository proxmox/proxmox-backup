use failure::*;

use http::Uri;
use hyper::Body;
use hyper::client::Client;
use hyper::rt::{self, Future};

use http::Request;
use futures::stream::Stream;

use serde_json::{Value};
use url::percent_encoding::{percent_encode,  DEFAULT_ENCODE_SET};

use crate::tools::tty;

/// HTTP(S) API client
pub struct HttpClient {
    username: String,
    server: String,

    ticket: Option<String>,
    token: Option<String>
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

    fn run_request(
        request: Request<Body>,
    ) -> Result<Value, Error> {
        let mut builder = native_tls::TlsConnector::builder();
        // FIXME: We need a CLI option for this!
        builder.danger_accept_invalid_certs(true);
        let tlsconnector = builder.build()?;
        let mut httpc = hyper::client::HttpConnector::new(1);
        httpc.enforce_http(false); // we want https...
        let mut https = hyper_tls::HttpsConnector::from((httpc, tlsconnector));
        https.https_only(true); // force it!
        let client = Client::builder().build::<_, Body>(https);

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
            .body(Body::empty())?;

        Self::run_request(request)
    }

    fn login(&mut self) ->  Result<(String, String), Error> {

        if let Some(ref ticket) = self.ticket {
            if let Some(ref token) = self.token {
                return Ok((ticket.clone(), token.clone()));
            }
        }

        let url: Uri = format!("https://{}:8007/{}", self.server, "/api2/json/access/ticket").parse()?;

        let password = self.get_password()?;

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

        self.ticket = Some(ticket.to_owned());
        self.token = Some(token.to_owned());

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
