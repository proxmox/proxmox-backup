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
    size: u32,
    chunk: Vec<u8>,
}

impl UploadChunk {

    pub fn new(stream: Body,  store: Arc<DataStore>, size: u32) -> Self {
        Self { stream, store, size, chunk: vec![] }
    }
}

impl Future for UploadChunk {
    type Item = ([u8; 32], u32);
    type Error = failure::Error;

    fn poll(&mut self) -> Poll<([u8; 32], u32), failure::Error> {
        loop {
            match try_ready!(self.stream.poll()) {
                Some(chunk) => {
                    if (self.chunk.len() + chunk.len()) > (self.size as usize) {
                        bail!("uploaded chunk is larger than announced.");
                    }
                    self.chunk.extend_from_slice(&chunk);
                }
                None => {
                    if self.chunk.len() != (self.size as usize) {
                        bail!("uploaded chunk has unexpected size.");
                    }

                    let (_is_duplicate, digest, _compressed_size) = self.store.insert_chunk(&self.chunk)?;

                    return Ok(Async::Ready((digest, self.size)))
                }
            }
        }
    }
}

pub fn api_method_upload_dynamic_chunk() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        upload_dynamic_chunk,
        ObjectSchema::new("Upload chunk for dynamic index writer (variable sized chunks).")
            .required("wid", IntegerSchema::new("Dynamic writer ID.")
                      .minimum(1)
                      .maximum(256)
            )
            .required("offset", IntegerSchema::new("Chunk offset (end of chunk).")
                      .minimum(0)
            )
            .required("size", IntegerSchema::new("Chunk size.")
                      .minimum(1)
                      .maximum(1024*1024*16)
            )
    )
}

fn upload_dynamic_chunk(
    _parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    rpcenv: Box<RpcEnvironment>,
) -> Result<BoxFut, Error> {

    let size = tools::required_integer_param(&param, "size")? as u32;
    let offset = tools::required_integer_param(&param, "offset")? as u64;
    let wid = tools::required_integer_param(&param, "wid")? as usize;

    println!("upload_dynamic_chunk: {} bytes, offset {}", size, offset);

    let env: &BackupEnvironment = rpcenv.as_ref();

    let upload = UploadChunk::new(req_body, env.datastore.clone(), size);

    let resp = upload
        .then(move |result| {
            let env: &BackupEnvironment = rpcenv.as_ref();

            let result = result.and_then(|(digest, size)| {
                env.register_chunk(digest, size)?;
                env.dynamic_writer_append_chunk(wid, offset, size, &digest)?;
                Ok(json!(tools::digest_to_hex(&digest)))
            });

            Ok(env.format_response(result))
        });

    Ok(Box::new(resp))
}

pub fn api_method_upload_speedtest() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        upload_speedtest,
        ObjectSchema::new("Test uploadf speed.")
    )
}

fn upload_speedtest(
    _parts: Parts,
    req_body: Body,
    _param: Value,
    _info: &ApiAsyncMethod,
    rpcenv: Box<RpcEnvironment>,
) -> Result<BoxFut, Error> {

    let resp = req_body
        .map_err(Error::from)
        .fold(0, |size: usize, chunk| -> Result<usize, Error> {
            let sum = size + chunk.len();
            //println!("UPLOAD {} bytes, sum {}", chunk.len(), sum);
            Ok(sum)
        })
        .then(move |result| {
            match result {
                Ok(size) => {
                    println!("UPLOAD END {} bytes", size);
                }
                Err(err) => {
                    println!("Upload error: {}", err);
                }
            }
            let env: &BackupEnvironment = rpcenv.as_ref();
            Ok(env.format_response(Ok(Value::Null)))
        });

    Ok(Box::new(resp))
}
