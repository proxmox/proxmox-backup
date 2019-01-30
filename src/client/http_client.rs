use failure::*;

use http::Uri;
use hyper::Body;
use hyper::client::Client;
use hyper::rt::{self, Future};

use http::Request;
use futures::stream::Stream;

use serde_json::{Value};

pub struct HttpClient {
    server: String,
}

impl HttpClient {

    pub fn new(server: &str) -> Self {
        Self {
            server: String::from(server),
        }
    }

    fn run_request(request: Request<Body>) -> Result<Value, Error> {
        let client = Client::new();

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

        let url: Uri = format!("http://{}:8007/{}", self.server, path).parse()?;

        let request = Request::builder()
            .method("GET")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .body(Body::empty())?;

        Self::run_request(request)
    }

    pub fn upload(&self, content_type: &str, body: Body, path: &str) -> Result<Value, Error> {

        let url: Uri = format!("http://{}:8007/{}", self.server, path).parse()?;

        let request = Request::builder()
            .method("POST")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header("Content-Type", content_type)
            .body(body)?;

        Self::run_request(request)
    }
}
