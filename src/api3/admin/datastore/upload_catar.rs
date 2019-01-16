use failure::*;

use crate::backup::datastore::*;
use crate::backup::archive_index::*;
//use crate::server::rest::*;
use crate::api::schema::*;
use crate::api::router::*;

use serde_json::Value;
use std::io::Write;
use futures::*;

pub struct UploadCaTar {
    stream: hyper::Body,
    index: ArchiveIndexWriter,
    count: usize,
}

impl Future for UploadCaTar {
    type Item = ();
    type Error = failure::Error;

    fn poll(&mut self) -> Poll<(), failure::Error> {
        loop {
            match try_ready!(self.stream.poll()) {
                Some(chunk) => {
                    self.count += chunk.len();
                    if let Err(err) = self.index.write(&chunk) {
                        bail!("writing chunk failed - {}", err);
                    }
                    return Ok(Async::NotReady);
                }
                None => {
                    self.index.close()?;
                    return Ok(Async::Ready(()))
                }
            }
        }
    }
}

fn upload_catar(req_body: hyper::Body, param: Value, _info: &ApiUploadMethod) -> BoxFut {

    let store = param["name"].as_str().unwrap();

    println!("Upload .catar to {}", store);

    let chunk_size = 4*1024*1024;
    let datastore = DataStore::lookup_datastore(store).unwrap().clone();

    let index = datastore.create_archive_writer("upload.aidx", chunk_size).unwrap();

    let upload = UploadCaTar { stream: req_body, index, count: 0};

    let resp = upload.and_then(|_| {

        let response = http::Response::builder()
            .status(200)
            .body(hyper::Body::empty())
            .unwrap();

        Ok(response)
    });

    Box::new(resp)
}

pub fn api_method_upload_catar() -> ApiUploadMethod {
    ApiUploadMethod::new(
        upload_catar,
        ObjectSchema::new("Upload .catar backup file.")
            .required("name", StringSchema::new("Datastore name."))
    )
}
