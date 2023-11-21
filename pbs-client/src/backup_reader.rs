use anyhow::{format_err, Error};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::Arc;

use futures::future::AbortHandle;
use serde_json::{json, Value};

use pbs_api_types::{BackupDir, BackupNamespace};
use pbs_datastore::data_blob::DataBlob;
use pbs_datastore::data_blob_reader::DataBlobReader;
use pbs_datastore::dynamic_index::DynamicIndexReader;
use pbs_datastore::fixed_index::FixedIndexReader;
use pbs_datastore::index::IndexFile;
use pbs_datastore::manifest::MANIFEST_BLOB_NAME;
use pbs_datastore::{BackupManifest, PROXMOX_BACKUP_READER_PROTOCOL_ID_V1};
use pbs_tools::crypt_config::CryptConfig;
use pbs_tools::sha::sha256;

use super::{H2Client, HttpClient};

/// Backup Reader
pub struct BackupReader {
    h2: H2Client,
    abort: AbortHandle,
    crypt_config: Option<Arc<CryptConfig>>,
}

impl Drop for BackupReader {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

impl BackupReader {
    fn new(h2: H2Client, abort: AbortHandle, crypt_config: Option<Arc<CryptConfig>>) -> Arc<Self> {
        Arc::new(Self {
            h2,
            abort,
            crypt_config,
        })
    }

    /// Create a new instance by upgrading the connection at '/api2/json/reader'
    pub async fn start(
        client: &HttpClient,
        crypt_config: Option<Arc<CryptConfig>>,
        datastore: &str,
        ns: &BackupNamespace,
        backup: &BackupDir,
        debug: bool,
    ) -> Result<Arc<BackupReader>, Error> {
        let mut param = json!({
            "backup-type": backup.ty(),
            "backup-id": backup.id(),
            "backup-time": backup.time,
            "store": datastore,
            "debug": debug,
        });

        if !ns.is_root() {
            param["ns"] = serde_json::to_value(ns)?;
        }

        let req = HttpClient::request_builder(
            client.server(),
            client.port(),
            "GET",
            "/api2/json/reader",
            Some(param),
        )
        .unwrap();

        let (h2, abort) = client
            .start_h2_connection(req, String::from(PROXMOX_BACKUP_READER_PROTOCOL_ID_V1!()))
            .await?;

        Ok(BackupReader::new(h2, abort, crypt_config))
    }

    /// Execute a GET request
    pub async fn get(&self, path: &str, param: Option<Value>) -> Result<Value, Error> {
        self.h2.get(path, param).await
    }

    /// Execute a PUT request
    pub async fn put(&self, path: &str, param: Option<Value>) -> Result<Value, Error> {
        self.h2.put(path, param).await
    }

    /// Execute a POST request
    pub async fn post(&self, path: &str, param: Option<Value>) -> Result<Value, Error> {
        self.h2.post(path, param).await
    }

    /// Execute a GET request and send output to a writer
    pub async fn download<W: Write + Send>(&self, file_name: &str, output: W) -> Result<(), Error> {
        let path = "download";
        let param = json!({ "file-name": file_name });
        self.h2.download(path, Some(param), output).await
    }

    /// Execute a special GET request and send output to a writer
    ///
    /// This writes random data, and is only useful to test download speed.
    pub async fn speedtest<W: Write + Send>(&self, output: W) -> Result<(), Error> {
        self.h2.download("speedtest", None, output).await
    }

    /// Download a specific chunk
    pub async fn download_chunk<W: Write + Send>(
        &self,
        digest: &[u8; 32],
        output: W,
    ) -> Result<(), Error> {
        let path = "chunk";
        let param = json!({ "digest": hex::encode(digest) });
        self.h2.download(path, Some(param), output).await
    }

    pub fn force_close(self) {
        self.abort.abort();
    }

    /// Download backup manifest (index.json)
    ///
    /// The manifest signature is verified if we have a crypt_config.
    pub async fn download_manifest(&self) -> Result<(BackupManifest, Vec<u8>), Error> {
        let mut raw_data = Vec::with_capacity(64 * 1024);
        self.download(MANIFEST_BLOB_NAME, &mut raw_data).await?;
        let blob = DataBlob::load_from_reader(&mut &raw_data[..])?;
        // no expected digest available
        let data = blob.decode(None, None)?;

        let manifest =
            BackupManifest::from_data(&data[..], self.crypt_config.as_ref().map(Arc::as_ref))?;

        Ok((manifest, data))
    }

    /// Download a .blob file
    ///
    /// This creates a temporary file in /tmp (using O_TMPFILE). The data is verified using
    /// the provided manifest.
    pub async fn download_blob(
        &self,
        manifest: &BackupManifest,
        name: &str,
    ) -> Result<DataBlobReader<'_, File>, Error> {
        let mut tmpfile = std::fs::OpenOptions::new()
            .write(true)
            .read(true)
            .custom_flags(libc::O_TMPFILE)
            .open("/tmp")?;

        self.download(name, &mut tmpfile).await?;

        tmpfile.seek(SeekFrom::Start(0))?;
        let (csum, size) = sha256(&mut tmpfile)?;
        manifest.verify_file(name, &csum, size)?;

        tmpfile.seek(SeekFrom::Start(0))?;

        DataBlobReader::new(tmpfile, self.crypt_config.clone())
    }

    /// Download dynamic index file
    ///
    /// This creates a temporary file in /tmp (using O_TMPFILE). The index is verified using
    /// the provided manifest.
    pub async fn download_dynamic_index(
        &self,
        manifest: &BackupManifest,
        name: &str,
    ) -> Result<DynamicIndexReader, Error> {
        let mut tmpfile = std::fs::OpenOptions::new()
            .write(true)
            .read(true)
            .custom_flags(libc::O_TMPFILE)
            .open("/tmp")?;

        self.download(name, &mut tmpfile).await?;

        let index = DynamicIndexReader::new(tmpfile)
            .map_err(|err| format_err!("unable to read dynamic index '{}' - {}", name, err))?;

        // Note: do not use values stored in index (not trusted) - instead, computed them again
        let (csum, size) = index.compute_csum();
        manifest.verify_file(name, &csum, size)?;

        Ok(index)
    }

    /// Download fixed index file
    ///
    /// This creates a temporary file in /tmp (using O_TMPFILE). The index is verified using
    /// the provided manifest.
    pub async fn download_fixed_index(
        &self,
        manifest: &BackupManifest,
        name: &str,
    ) -> Result<FixedIndexReader, Error> {
        let mut tmpfile = std::fs::OpenOptions::new()
            .write(true)
            .read(true)
            .custom_flags(libc::O_TMPFILE)
            .open("/tmp")?;

        self.download(name, &mut tmpfile).await?;

        let index = FixedIndexReader::new(tmpfile)
            .map_err(|err| format_err!("unable to read fixed index '{}' - {}", name, err))?;

        // Note: do not use values stored in index (not trusted) - instead, computed them again
        let (csum, size) = index.compute_csum();
        manifest.verify_file(name, &csum, size)?;

        Ok(index)
    }
}
