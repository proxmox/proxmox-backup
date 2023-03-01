use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::{bail, format_err, Error};
use futures::*;
use hex::FromHex;
use hyper::http::request::Parts;
use hyper::Body;
use serde_json::{json, Value};

use proxmox_router::{ApiHandler, ApiMethod, ApiResponseFuture, RpcEnvironment};
use proxmox_schema::*;
use proxmox_sortable_macro::sortable;

use pbs_api_types::{BACKUP_ARCHIVE_NAME_SCHEMA, CHUNK_DIGEST_SCHEMA};
use pbs_datastore::file_formats::{DataBlobHeader, EncryptedDataBlobHeader};
use pbs_datastore::{DataBlob, DataStore};
use pbs_tools::json::{required_integer_param, required_string_param};

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
    pub fn new(
        stream: Body,
        store: Arc<DataStore>,
        digest: [u8; 32],
        size: u32,
        encoded_size: u32,
    ) -> Self {
        Self {
            stream,
            store,
            size,
            encoded_size,
            raw_data: Some(vec![]),
            digest,
        }
    }
}

impl Future for UploadChunk {
    type Output = Result<([u8; 32], u32, u32, bool), Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.get_mut();

        let err: Error = loop {
            match ready!(Pin::new(&mut this.stream).poll_next(cx)) {
                Some(Err(err)) => return Poll::Ready(Err(Error::from(err))),
                Some(Ok(input)) => {
                    if let Some(ref mut raw_data) = this.raw_data {
                        if (raw_data.len() + input.len()) > (this.encoded_size as usize) {
                            break format_err!("uploaded chunk is larger than announced.");
                        }
                        raw_data.extend_from_slice(&input);
                    } else {
                        break format_err!("poll upload chunk stream failed - already finished.");
                    }
                }
                None => {
                    if let Some(raw_data) = this.raw_data.take() {
                        if raw_data.len() != (this.encoded_size as usize) {
                            break format_err!("uploaded chunk has unexpected size.");
                        }

                        let (is_duplicate, compressed_size) = match proxmox_lang::try_block! {
                            let mut chunk = DataBlob::from_raw(raw_data)?;

                            proxmox_async::runtime::block_in_place(|| {
                                chunk.verify_unencrypted(this.size as usize, &this.digest)?;

                                // always comput CRC at server side
                                chunk.set_crc(chunk.compute_crc());

                                this.store.insert_chunk(&chunk, &this.digest)
                            })

                        } {
                            Ok(res) => res,
                            Err(err) => break err,
                        };

                        return Poll::Ready(Ok((
                            this.digest,
                            this.size,
                            compressed_size as u32,
                            is_duplicate,
                        )));
                    } else {
                        break format_err!("poll upload chunk stream failed - already finished.");
                    }
                }
            }
        };
        Poll::Ready(Err(err))
    }
}

#[sortable]
pub const API_METHOD_UPLOAD_FIXED_CHUNK: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&upload_fixed_chunk),
    &ObjectSchema::new(
        "Upload a new chunk.",
        &sorted!([
            (
                "wid",
                false,
                &IntegerSchema::new("Fixed writer ID.")
                    .minimum(1)
                    .maximum(256)
                    .schema()
            ),
            ("digest", false, &CHUNK_DIGEST_SCHEMA),
            (
                "size",
                false,
                &IntegerSchema::new("Chunk size.")
                    .minimum(1)
                    .maximum(1024 * 1024 * 16)
                    .schema()
            ),
            (
                "encoded-size",
                false,
                &IntegerSchema::new("Encoded chunk size.")
                    .minimum((std::mem::size_of::<DataBlobHeader>() as isize) + 1)
                    .maximum(
                        1024 * 1024 * 16
                            + (std::mem::size_of::<EncryptedDataBlobHeader>() as isize)
                    )
                    .schema()
            ),
        ]),
    ),
);

fn upload_fixed_chunk(
    _parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {
    async move {
        let wid = required_integer_param(&param, "wid")? as usize;
        let size = required_integer_param(&param, "size")? as u32;
        let encoded_size = required_integer_param(&param, "encoded-size")? as u32;

        let digest_str = required_string_param(&param, "digest")?;
        let digest = <[u8; 32]>::from_hex(digest_str)?;

        let env: &BackupEnvironment = rpcenv.as_ref();

        let (digest, size, compressed_size, is_duplicate) =
            UploadChunk::new(req_body, env.datastore.clone(), digest, size, encoded_size).await?;

        env.register_fixed_chunk(wid, digest, size, compressed_size, is_duplicate)?;
        let digest_str = hex::encode(digest);
        env.debug(format!("upload_chunk done: {} bytes, {}", size, digest_str));

        let result = Ok(json!(digest_str));

        Ok(env.format_response(result))
    }
    .boxed()
}

#[sortable]
pub const API_METHOD_UPLOAD_DYNAMIC_CHUNK: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&upload_dynamic_chunk),
    &ObjectSchema::new(
        "Upload a new chunk.",
        &sorted!([
            (
                "wid",
                false,
                &IntegerSchema::new("Dynamic writer ID.")
                    .minimum(1)
                    .maximum(256)
                    .schema()
            ),
            ("digest", false, &CHUNK_DIGEST_SCHEMA),
            (
                "size",
                false,
                &IntegerSchema::new("Chunk size.")
                    .minimum(1)
                    .maximum(1024 * 1024 * 16)
                    .schema()
            ),
            (
                "encoded-size",
                false,
                &IntegerSchema::new("Encoded chunk size.")
                    .minimum((std::mem::size_of::<DataBlobHeader>() as isize) + 1)
                    .maximum(
                        1024 * 1024 * 16
                            + (std::mem::size_of::<EncryptedDataBlobHeader>() as isize)
                    )
                    .schema()
            ),
        ]),
    ),
);

fn upload_dynamic_chunk(
    _parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {
    async move {
        let wid = required_integer_param(&param, "wid")? as usize;
        let size = required_integer_param(&param, "size")? as u32;
        let encoded_size = required_integer_param(&param, "encoded-size")? as u32;

        let digest_str = required_string_param(&param, "digest")?;
        let digest = <[u8; 32]>::from_hex(digest_str)?;

        let env: &BackupEnvironment = rpcenv.as_ref();

        let (digest, size, compressed_size, is_duplicate) =
            UploadChunk::new(req_body, env.datastore.clone(), digest, size, encoded_size).await?;

        env.register_dynamic_chunk(wid, digest, size, compressed_size, is_duplicate)?;
        let digest_str = hex::encode(digest);
        env.debug(format!("upload_chunk done: {} bytes, {}", size, digest_str));

        let result = Ok(json!(digest_str));
        Ok(env.format_response(result))
    }
    .boxed()
}

pub const API_METHOD_UPLOAD_SPEEDTEST: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&upload_speedtest),
    &ObjectSchema::new("Test upload speed.", &[]),
);

fn upload_speedtest(
    _parts: Parts,
    req_body: Body,
    _param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {
    async move {
        let result = req_body
            .map_err(Error::from)
            .try_fold(0, |size: usize, chunk| {
                let sum = size + chunk.len();
                //println!("UPLOAD {} bytes, sum {}", chunk.len(), sum);
                future::ok::<usize, Error>(sum)
            })
            .await;

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
    }
    .boxed()
}

#[sortable]
pub const API_METHOD_UPLOAD_BLOB: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&upload_blob),
    &ObjectSchema::new(
        "Upload binary blob file.",
        &sorted!([
            ("file-name", false, &BACKUP_ARCHIVE_NAME_SCHEMA),
            (
                "encoded-size",
                false,
                &IntegerSchema::new("Encoded blob size.")
                    .minimum(std::mem::size_of::<DataBlobHeader>() as isize)
                    .maximum(
                        1024 * 1024 * 16
                            + (std::mem::size_of::<EncryptedDataBlobHeader>() as isize)
                    )
                    .schema()
            )
        ]),
    ),
);

fn upload_blob(
    _parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {
    async move {
        let file_name = required_string_param(&param, "file-name")?.to_owned();
        let encoded_size = required_integer_param(&param, "encoded-size")? as usize;

        let env: &BackupEnvironment = rpcenv.as_ref();

        if !file_name.ends_with(".blob") {
            bail!("wrong blob file extension: '{}'", file_name);
        }

        let data = req_body
            .map_err(Error::from)
            .try_fold(Vec::new(), |mut acc, chunk| {
                acc.extend_from_slice(&chunk);
                future::ok::<_, Error>(acc)
            })
            .await?;

        if encoded_size != data.len() {
            bail!(
                "got blob with unexpected length ({} != {})",
                encoded_size,
                data.len()
            );
        }

        env.add_blob(&file_name, data)?;

        Ok(env.format_response(Ok(Value::Null)))
    }
    .boxed()
}
