use failure::*;

use http::Uri;
use hyper::Body;
use hyper::client::Client;
use hyper::rt::{self, Future};

use http::Request;
use futures::stream::Stream;

use serde_json::{Value};
use url::percent_encoding::{percent_encode,  DEFAULT_ENCODE_SET};

pub struct HttpClient {
    username: String,
    server: String,
}

impl HttpClient {

    pub fn new(server: &str, username: &str) -> Self {
        Self {
            server: String::from(server),
            username: String::from(username),
        }
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
            .map_err(|e| Error::from(e))
            .and_then(|resp| {

                let status = resp.status();

                resp.into_body().concat2().map_err(|e| Error::from(e))
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

    pub fn get(&self, path: &str) -> Result<Value, Error> {

        let url: Uri = format!("https://{}:8007/{}", self.server, path).parse()?;

        let ticket = self.login()?;

        let enc_ticket = percent_encode(ticket.as_bytes(), DEFAULT_ENCODE_SET).to_string();

        let request = Request::builder()
            .method("GET")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header("Cookie", format!("PBSAuthCookie={}", enc_ticket))
            .body(Body::empty())?;

        Self::run_request(request)
    }

    fn login(&self) ->  Result<String, Error> {

        let url: Uri = format!("https://{}:8007/{}", self.server, "/api2/json/access/ticket").parse()?;

        let password = match std::env::var("PBS_PASSWORD") {
            Ok(p) => p,
            Err(err) => bail!("missing passphrase - {}", err),
        };

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

        Ok(ticket.to_owned())
    }

    pub fn upload(&self, content_type: &str, body: Body, path: &str) -> Result<Value, Error> {

        let url: Uri = format!("https://{}:8007/{}", self.server, path).parse()?;

        let ticket = self.login()?;

        let enc_ticket = percent_encode(ticket.as_bytes(), DEFAULT_ENCODE_SET).to_string();

        let request = Request::builder()
            .method("POST")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header("Cookie", format!("PBSAuthCookie={}", enc_ticket))
            .header("Content-Type", content_type)
            .body(body)?;

        Self::run_request(request)
    }
}
