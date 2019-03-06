use std::path::PathBuf;
use std::sync::Arc;

use failure::*;
use futures::future::{ok, poll_fn};
use futures::{Async, Future};
use hyper::header::{HeaderValue, UPGRADE};
use hyper::http::request::Parts;
use hyper::rt;
use hyper::{Body, Response, StatusCode};
use serde_json::Value;

use proxmox_protocol::protocol::DynamicChunk;
use proxmox_protocol::server as pmx_server;
use proxmox_protocol::{ChunkEntry, FixedChunk};

use crate::api_schema::router::*;
use crate::api_schema::*;
use crate::backup::{BackupDir, DataStore, DynamicIndexWriter, FixedIndexWriter, IndexFile};
use crate::tools;

type Result<T> = std::result::Result<T, Error>;

pub fn api_method_upgrade_upload() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        upgrade_upload,
        ObjectSchema::new("Download .catar backup file.")
            .required("store", StringSchema::new("Datastore name.")),
    )
}

fn upgrade_upload(
    parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<BoxFut> {
    let store = tools::required_string_param(&param, "store")?.to_string();
    let expected_protocol: &'static str = "proxmox-backup-protocol-1";

    let protocols = parts
        .headers
        .get("UPGRADE")
        .ok_or_else(|| format_err!("missing Upgrade header"))?
        .to_str()?;

    if protocols != expected_protocol {
        bail!("invalid protocol name");
    }

    rt::spawn(
        req_body
            .on_upgrade()
            .map_err(|e| Error::from(e))
            .and_then(move |conn| backup_protocol_handler(conn, &store))
            .map_err(|e| eprintln!("error during upgrade: {}", e))
            .flatten(),
    );

    Ok(Box::new(ok(Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, HeaderValue::from_static(expected_protocol))
        .body(Body::empty())
        .unwrap())))
}

struct BackupClientHandler {
    store: Arc<DataStore>,
}

struct ChunkLister(Box<dyn IndexFile + Send>, usize);

impl pmx_server::ChunkList for ChunkLister {
    fn next(&mut self) -> Result<Option<&[u8; 32]>> {
        if self.1 == self.0.index_count() {
            Ok(None)
        } else {
            let chunk = self.0.index_digest(self.1);
            self.1 += 1;
            Ok(chunk)
        }
    }
}

impl pmx_server::HandleClient for BackupClientHandler {
    fn error(&self) {
        eprintln!("There was an error!");
    }

    fn get_chunk_list(
        &self,
        backup_name: &str,
    ) -> Result<Box<dyn pmx_server::ChunkList>> {
        Ok(Box::new(ChunkLister(self.store.open_index(backup_name)?, 0)))
    }

    fn upload_chunk(&self, chunk: &ChunkEntry, data: &[u8]) -> Result<bool> {
        let (new, _csize) = self.store.insert_chunk_noverify(&chunk.hash, data)?;
        Ok(new)
    }

    fn create_backup(
        &self,
        backup_type: &str,
        backup_id: &str,
        backup_timestamp: i64,
        new: bool,
    ) -> Result<Box<dyn pmx_server::HandleBackup + Send>> {
        let (path, is_new) = self.store.create_backup_dir(
            &BackupDir::new(backup_type, backup_id, backup_timestamp)
        )?;

        if new && !is_new {
            bail!("client requested to create a new backup, but it already existed");
        }

        Ok(Box::new(BackupHandler {
            store: Arc::clone(&self.store),
            path,
        }))
    }
}

struct BackupHandler {
    store: Arc<DataStore>,
    path: PathBuf,
}

impl pmx_server::HandleBackup for BackupHandler {
    fn finish(&mut self) -> Result<()> {
        bail!("TODO: finish");
    }

    fn create_file(
        &self,
        name: &str,
        fixed_size: Option<u64>,
        chunk_size: usize,
    ) -> Result<Box<dyn pmx_server::BackupFile + Send>> {
        if name.find('/').is_some() {
            bail!("invalid file name");
        }

        let mut path_str = self.path
            .to_str()
            .ok_or_else(|| format_err!("generated non-utf8 path"))?
            .to_string();
        path_str.push('/');
        path_str.push_str(name);

        match fixed_size {
            None => {
                path_str.push_str(".didx");
                let path = PathBuf::from(path_str.as_str());
                let writer = self.store.create_dynamic_writer(path, chunk_size)?;
                Ok(Box::new(DynamicFile {
                    writer: Some(writer),
                    path: path_str,
                }))
            }
            Some(file_size) => {
                path_str.push_str(".fidx");
                let path = PathBuf::from(path_str.as_str());
                let writer = self.store.create_fixed_writer(path, file_size as usize, chunk_size)?;
                Ok(Box::new(FixedFile {
                    writer: Some(writer),
                    path: path_str,
                }))
            }
        }
    }
}

struct DynamicFile {
    writer: Option<DynamicIndexWriter>,
    path: String,
}

impl pmx_server::BackupFile for DynamicFile {
    fn relative_path(&self) -> &str {
        self.path.as_str()
    }

    fn add_fixed_data(&mut self, _index: u64, _hash: &FixedChunk) -> Result<()> {
        bail!("add_fixed_data data on dynamic index writer!");
    }

    fn add_dynamic_data(&mut self, chunk: &DynamicChunk) -> Result<()> {
        self.writer.as_mut().unwrap()
            .add_chunk(chunk.offset, &chunk.digest)
            .map_err(Error::from)
    }

    fn finish(&mut self) -> Result<()> {
        self.writer.take().unwrap().close()
    }
}

struct FixedFile {
    writer: Option<FixedIndexWriter>,
    path: String,
}

impl pmx_server::BackupFile for FixedFile {
    fn relative_path(&self) -> &str {
        self.path.as_str()
    }

    fn add_fixed_data(&mut self, index: u64, hash: &FixedChunk) -> Result<()> {
        self.writer.as_mut().unwrap()
            .add_digest(index as usize, &hash.0)
    }

    fn add_dynamic_data(&mut self, _chunk: &DynamicChunk) -> Result<()> {
        bail!("add_dynamic_data data on fixed index writer!");
    }

    fn finish(&mut self) -> Result<()> {
        self.writer.take().unwrap().close()
    }
}

fn backup_protocol_handler(
    conn: hyper::upgrade::Upgraded,
    store_name: &str,
) -> Result<Box<Future<Item = (), Error = ()> + Send>> {
    let store = DataStore::lookup_datastore(store_name)?;
    let handler = BackupClientHandler { store };
    let mut protocol = pmx_server::Connection::new(conn, handler)?;
    Ok(Box::new(poll_fn(move || {
        match protocol.main() {
            Ok(_) => {
                if protocol.eof() {
                    eprintln!("is eof!");
                }
                Ok(Async::NotReady)
            }
            Err(e) => {
                if let Some(e) = e.downcast_ref::<std::io::Error>() {
                    if e.kind() == std::io::ErrorKind::WouldBlock {
                        eprintln!("Got EWOULDBLOCK");
                        return Ok(Async::NotReady);
                    }
                }
                // end the future
                eprintln!("Backup protocol error: {}", e);
                Err(())
            }
        }
    })))
}
