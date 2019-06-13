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
use crate::api2::types::*;

use super::environment::*;

pub struct UploadChunk {
    stream: Body,
    store: Arc<DataStore>,
    digest: [u8; 32],
    size: u32,
    encoded_size: u32,
    raw_data: Option<Vec<u8>>,
}

impl UploadChunk {

    pub fn new(stream: Body,  store: Arc<DataStore>, digest: [u8; 32], size: u32, encoded_size: u32) -> Self {
        Self { stream, store, size, encoded_size, raw_data: Some(vec![]), digest }
    }
}

impl Future for UploadChunk {
    type Item = ([u8; 32], u32, u32, bool);
    type Error = failure::Error;

    fn poll(&mut self) -> Poll<([u8; 32], u32, u32, bool), failure::Error> {
        loop {
            match try_ready!(self.stream.poll()) {
                Some(input) => {
                    if let Some(ref mut raw_data) = self.raw_data {
                        if (raw_data.len() + input.len()) > (self.encoded_size as usize) {
                            bail!("uploaded chunk is larger than announced.");
                        }
                        raw_data.extend_from_slice(&input);
                    } else {
                        bail!("poll upload chunk stream failed - already finished.");
                    }
                }
                None => {
                    if let Some(raw_data) = self.raw_data.take() {
                        if raw_data.len() != (self.encoded_size as usize) {
                            bail!("uploaded chunk has unexpected size.");
                        }

                        let chunk = DataChunk::from_raw(raw_data, self.digest)?;

                        let (is_duplicate, compressed_size) = self.store.insert_chunk(&chunk)?;

                        return Ok(Async::Ready((self.digest, self.size, compressed_size as u32, is_duplicate)))
                    } else {
                        bail!("poll upload chunk stream failed - already finished.");
                    }
                }
            }
        }
    }
}

pub fn api_method_upload_fixed_chunk() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        upload_fixed_chunk,
        ObjectSchema::new("Upload a new chunk.")
            .required("wid", IntegerSchema::new("Fixed writer ID.")
                      .minimum(1)
                      .maximum(256)
            )
            .required("digest", CHUNK_DIGEST_SCHEMA.clone())
            .required("size", IntegerSchema::new("Chunk size.")
                      .minimum(1)
                      .maximum(1024*1024*16)
            )
            .required("encoded-size", IntegerSchema::new("Encoded chunk size.")
                      .minimum(9)
                      // fixme: .maximum(1024*1024*16+40)
            )
    )
}

fn upload_fixed_chunk(
    _parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> Result<BoxFut, Error> {

    let wid = tools::required_integer_param(&param, "wid")? as usize;
    let size = tools::required_integer_param(&param, "size")? as u32;
    let encoded_size = tools::required_integer_param(&param, "encoded-size")? as u32;

    let digest_str = tools::required_string_param(&param, "digest")?;
    let digest = crate::tools::hex_to_digest(digest_str)?;

    let env: &BackupEnvironment = rpcenv.as_ref();

    let upload = UploadChunk::new(req_body, env.datastore.clone(), digest, size, encoded_size);

    let resp = upload
        .then(move |result| {
            let env: &BackupEnvironment = rpcenv.as_ref();

             let result = result.and_then(|(digest, size, compressed_size, is_duplicate)| {
                 env.register_fixed_chunk(wid, digest, size, compressed_size, is_duplicate)?;
                 let digest_str = tools::digest_to_hex(&digest);
                 env.debug(format!("upload_chunk done: {} bytes, {}", size, digest_str));
                 Ok(json!(digest_str))
             });

            Ok(env.format_response(result))
        });

    Ok(Box::new(resp))
}

pub fn api_method_upload_dynamic_chunk() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        upload_dynamic_chunk,
        ObjectSchema::new("Upload a new chunk.")
            .required("wid", IntegerSchema::new("Dynamic writer ID.")
                      .minimum(1)
                      .maximum(256)
            )
            .required("digest", CHUNK_DIGEST_SCHEMA.clone())
            .required("size", IntegerSchema::new("Chunk size.")
                      .minimum(1)
                      .maximum(1024*1024*16)
            )
            .required("encoded-size", IntegerSchema::new("Encoded chunk size.")
                      .minimum(9)
                      // fixme: .maximum(1024*1024*16+40)
            )
    )
}

fn upload_dynamic_chunk(
    _parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> Result<BoxFut, Error> {

    let wid = tools::required_integer_param(&param, "wid")? as usize;
    let size = tools::required_integer_param(&param, "size")? as u32;
    let encoded_size = tools::required_integer_param(&param, "encoded-size")? as u32;

    let digest_str = tools::required_string_param(&param, "digest")?;
    let digest = crate::tools::hex_to_digest(digest_str)?;

    let env: &BackupEnvironment = rpcenv.as_ref();

    let upload = UploadChunk::new(req_body, env.datastore.clone(), digest, size, encoded_size);

    let resp = upload
        .then(move |result| {
            let env: &BackupEnvironment = rpcenv.as_ref();

             let result = result.and_then(|(digest, size, compressed_size, is_duplicate)| {
                 env.register_dynamic_chunk(wid, digest, size, compressed_size, is_duplicate)?;
                 let digest_str = tools::digest_to_hex(&digest);
                 env.debug(format!("upload_chunk done: {} bytes, {}", size, digest_str));
                 Ok(json!(digest_str))
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
    rpcenv: Box<dyn RpcEnvironment>,
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

pub fn api_method_upload_config() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        upload_config,
        ObjectSchema::new("Upload configuration file.")
            .required("file-name", crate::api2::types::BACKUP_ARCHIVE_NAME_SCHEMA.clone())
            .required("size", IntegerSchema::new("File size.")
                      .minimum(1)
                      .maximum(1024*1024*16)
            )
    )
}

fn upload_config(
    _parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> Result<BoxFut, Error> {

    let mut file_name = tools::required_string_param(&param, "file-name")?.to_owned();
    let size = tools::required_integer_param(&param, "size")? as usize;

    if !file_name.ends_with(".conf") {
        bail!("wrong config file extension: '{}'", file_name);
    } else {
        file_name.push_str(".zstd");
    }

    let env: &BackupEnvironment = rpcenv.as_ref();

    let mut path = env.datastore.base_path();
    path.push(env.backup_dir.relative_path());
    path.push(&file_name);

    let env2 = env.clone();
    let env3 = env.clone();

    let resp = req_body
        .map_err(Error::from)
        .concat2()
        .and_then(move |data| {
            if size != data.len() {
                bail!("got configuration file with unexpected length ({} != {})", size, data.len());
            }

            let data = zstd::block::compress(&data, 0)?;

            tools::file_set_contents(&path, &data, None)?;

            env2.debug(format!("upload config {:?} ({} bytes, comp: {})", path, size, data.len()));

            Ok(())
        })
        .and_then(move |_| {
            Ok(env3.format_response(Ok(Value::Null)))
        })
        ;

    Ok(Box::new(resp))
}
