use failure::*;
use futures::*;
use std::sync::Arc;

use hyper::http::request::Parts;
use hyper::Body;
use serde_json::{json, Value};

use crate::tools;
use crate::backup::*;
use crate::api_schema::*;
use crate::api_schema::router::*;

use super::environment::*;

pub struct UploadChunk {
    stream: Body,
    store: Arc<DataStore>,
    size: u64,
    chunk: Vec<u8>,
}

impl UploadChunk {

    pub fn new(stream: Body,  store: Arc<DataStore>, size: u64) -> Self {
        Self { stream, store, size, chunk: vec![] }
    }
}

impl Future for UploadChunk {
    type Item = Value;
    type Error = failure::Error;

    fn poll(&mut self) -> Poll<Value, failure::Error> {
        loop {
            match try_ready!(self.stream.poll()) {
                Some(chunk) => {
                    if (self.chunk.len() + chunk.len()) > (self.size as usize) {
                        bail!("uploaded chunk is larger than announced.");
                    }
                    self.chunk.extend_from_slice(&chunk);
                }
                None => {

                    let (is_duplicate, digest, _compressed_size) = self.store.insert_chunk(&self.chunk)?;

                    let result = json!({
                        "digest": tools::digest_to_hex(&digest),
                        "duplicate": is_duplicate,
                    });
                    return Ok(Async::Ready(result))
                }
            }
        }
    }
}

pub fn api_method_upload_chunk() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        upload_chunk,
        ObjectSchema::new("Upload chunk.")
            .required("size", IntegerSchema::new("Chunk size.")
                      .minimum(1)
                      .maximum(1024*1024*16)
            )
    )
}

fn upload_chunk(
    _parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    rpcenv: Box<RpcEnvironment>,
) -> Result<BoxFut, Error> {

    let size = tools::required_integer_param(&param, "size")?;

    let env: &BackupEnvironment = rpcenv.as_ref();

    let upload = UploadChunk::new(req_body, env.datastore.clone(), size as u64);

    // fixme: do we really need abort here? We alread do that on level above.
    let abort_future = env.worker.abort_future().then(|_| Ok(Value::Null));

    let resp = upload.select(abort_future)
        .and_then(|(result, _)| Ok(result))
        .map_err(|(err, _)| err)
        .then(move |res| {
            let env: &BackupEnvironment = rpcenv.as_ref();
            Ok(env.format_response(res))
        });

    Ok(Box::new(resp))

}
