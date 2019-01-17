use failure::*;

use http::Uri;
use hyper::Body;
use hyper::client::Client;
use hyper::rt::{self, Future};

pub struct HttpClient {
    server: String,
}

impl HttpClient {

    pub fn new(server: &str) -> Self {
        Self {
            server: String::from(server),
        }
    }

    pub fn upload(&self, body: Body, path: &str) -> Result<(), Error> {

        let client = Client::new();

        let url: Uri = format!("http://{}:8007/{}", self.server, path).parse()?;

        use http::Request;
        use futures::stream::Stream;

        let request = Request::builder()
            .method("POST")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .body(body)?;

        let future = client
            .request(request)
            .map_err(|e| Error::from(e))
            .and_then(|resp| {

                let status = resp.status();

                resp.into_body().concat2().map_err(|e| Error::from(e))
                    .and_then(move |data| {

                        let text = String::from_utf8(data.to_vec()).unwrap();
                        if status.is_success() {
                            println!("Result {} {}", status, text);
                        } else {
                            eprintln!("HTTP Error {}: {}", status, text);
                        }
                        Ok(())
                    })
            })
            .map_err(|err| {
                eprintln!("Error: {}", err);
            });

        // drop client, else client keeps connectioon open (keep-alive feature)
        drop(client);

        rt::run(future);

        Ok(())
    }
}
