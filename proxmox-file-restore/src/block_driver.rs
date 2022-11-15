//! Abstraction layer over different methods of accessing a block backup
use std::collections::HashMap;
use std::future::Future;
use std::hash::BuildHasher;
use std::pin::Pin;

use anyhow::{bail, Error};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use proxmox_router::cli::*;
use proxmox_schema::api;

use pbs_api_types::{file_restore::FileRestoreFormat, BackupDir, BackupNamespace};
use pbs_client::BackupRepository;
use pbs_datastore::catalog::ArchiveEntry;
use pbs_datastore::manifest::BackupManifest;

use super::block_driver_qemu::QemuBlockDriver;

/// Contains details about a snapshot that is to be accessed by block file restore
pub struct SnapRestoreDetails {
    pub repo: BackupRepository,
    pub namespace: BackupNamespace,
    pub snapshot: BackupDir,
    pub manifest: BackupManifest,
    pub keyfile: Option<String>,
}

/// Return value of a BlockRestoreDriver.status() call, 'id' must be valid for .stop(id)
pub struct DriverStatus {
    pub id: String,
    pub data: Value,
}

pub type Async<R> = Pin<Box<dyn Future<Output = R> + Send>>;

/// An abstract implementation for retrieving data out of a block file backup
pub trait BlockRestoreDriver {
    /// List ArchiveEntrys for the given image file and path
    fn data_list(
        &self,
        details: SnapRestoreDetails,
        img_file: String,
        path: Vec<u8>,
    ) -> Async<Result<Vec<ArchiveEntry>, Error>>;

    /// pxar=true:
    /// Attempt to create a pxar archive of the given file path and return a reader instance for it
    /// pxar=false:
    /// Attempt to read the file or folder at the given path and return the file content or a zip
    /// file as a stream
    fn data_extract(
        &self,
        details: SnapRestoreDetails,
        img_file: String,
        path: Vec<u8>,
        format: Option<FileRestoreFormat>,
        zstd: bool,
    ) -> Async<Result<Box<dyn tokio::io::AsyncRead + Unpin + Send>, Error>>;

    /// Return status of all running/mapped images, result value is (id, extra data), where id must
    /// match with the ones returned from list()
    fn status(&self) -> Async<Result<Vec<DriverStatus>, Error>>;
    /// Stop/Close a running restore method
    fn stop(&self, id: String) -> Async<Result<(), Error>>;
    /// Returned ids must be prefixed with driver type so that they cannot collide between drivers,
    /// the returned values must be passable to stop()
    fn list(&self) -> Vec<String>;
}

#[api()]
#[derive(Debug, Serialize, Deserialize, Eq, PartialEq, Clone, Copy)]
pub enum BlockDriverType {
    /// Uses a small QEMU/KVM virtual machine to map images securely. Requires PVE-patched QEMU.
    Qemu,
}

impl BlockDriverType {
    fn resolve(&self) -> impl BlockRestoreDriver {
        match self {
            BlockDriverType::Qemu => QemuBlockDriver {},
        }
    }
}

const DEFAULT_DRIVER: BlockDriverType = BlockDriverType::Qemu;
const ALL_DRIVERS: &[BlockDriverType] = &[BlockDriverType::Qemu];

pub async fn data_list(
    driver: Option<BlockDriverType>,
    details: SnapRestoreDetails,
    img_file: String,
    path: Vec<u8>,
) -> Result<Vec<ArchiveEntry>, Error> {
    let driver = driver.unwrap_or(DEFAULT_DRIVER).resolve();
    driver.data_list(details, img_file, path).await
}

pub async fn data_extract(
    driver: Option<BlockDriverType>,
    details: SnapRestoreDetails,
    img_file: String,
    path: Vec<u8>,
    format: Option<FileRestoreFormat>,
    zstd: bool,
) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>, Error> {
    let driver = driver.unwrap_or(DEFAULT_DRIVER).resolve();
    driver
        .data_extract(details, img_file, path, format, zstd)
        .await
}

#[api(
   input: {
       properties: {
            "driver": {
                type: BlockDriverType,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
   },
)]
/// Retrieve status information about currently running/mapped restore images
pub async fn status(driver: Option<BlockDriverType>, param: Value) -> Result<(), Error> {
    let output_format = get_output_format(&param);
    let text = output_format == "text";

    let mut ret = json!({});

    for dt in ALL_DRIVERS {
        if driver.is_some() && &driver.unwrap() != dt {
            continue;
        }

        let drv_name = format!("{:?}", dt);
        let drv = dt.resolve();
        match drv.status().await {
            Ok(data) if data.is_empty() => {
                if text {
                    println!("{}: no mappings", drv_name);
                } else {
                    ret[drv_name] = json!({});
                }
            }
            Ok(data) => {
                if text {
                    println!("{}:", &drv_name);
                }

                ret[&drv_name]["ids"] = json!({});
                for status in data {
                    if text {
                        println!("{} \t({})", status.id, status.data);
                    } else {
                        ret[&drv_name]["ids"][status.id] = status.data;
                    }
                }
            }
            Err(err) => {
                if text {
                    eprintln!("error getting status from driver '{drv_name}' - {err}");
                } else {
                    ret[drv_name] = json!({ "error": format!("{err}") });
                }
            }
        }
    }

    if !text {
        format_and_print_result(&ret, &output_format);
    }

    Ok(())
}

#[api(
   input: {
       properties: {
            "name": {
                type: String,
                description: "The name of the VM to stop.",
            },
        },
   },
)]
/// Immediately stop/unmap a given image. Not typically necessary, as VMs will stop themselves
/// after a timer anyway.
pub async fn stop(name: String) -> Result<(), Error> {
    for drv in ALL_DRIVERS.iter().map(BlockDriverType::resolve) {
        if drv.list().contains(&name) {
            return drv.stop(name).await;
        }
    }

    bail!("no mapping with name '{name}' found");
}

/// Autocompletion handler for block mappings
pub fn complete_block_driver_ids<S: BuildHasher>(
    _arg: &str,
    _param: &HashMap<String, String, S>,
) -> Vec<String> {
    ALL_DRIVERS
        .iter()
        .map(BlockDriverType::resolve)
        .flat_map(|d| d.list())
        .collect()
}
