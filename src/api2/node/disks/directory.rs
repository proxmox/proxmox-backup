use anyhow::{Error};
use serde_json::json;
use ::serde::{Deserialize, Serialize};

use proxmox::api::{api, Permission, RpcEnvironment, RpcEnvironmentType};
use proxmox::api::section_config::SectionConfigData;
use proxmox::api::router::Router;

use crate::config::acl::{PRIV_SYS_AUDIT, PRIV_SYS_MODIFY};
use crate::tools::disks::{
    DiskManage, FileSystemType,
    create_file_system, create_single_linux_partition, get_fs_uuid,
};
use crate::tools::systemd::{self, types::*};

use crate::server::WorkerTask;

use crate::api2::types::*;

#[api(
    properties: {
        "filesystem": {
            type: FileSystemType,
            optional: true,
        },
    },
)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
/// Datastore mount info.
pub struct DatastoreMountInfo {
    /// The path of the mount unit.
    pub unitfile: String,
    /// The mount path.
    pub path: String,
    /// The mounted device.
    pub device: String,
    /// File system type
    pub filesystem: Option<String>,
    /// Mount options
    pub options: Option<String>,
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
        }
    },
    returns: {
        description: "List of systemd datastore mount units.",
        type: Array,
        items: {
            type: DatastoreMountInfo,
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "disks"], PRIV_SYS_AUDIT, false),
    },
)]
/// List systemd datastore mount units.
fn  list_datastore_mounts() -> Result<Vec<DatastoreMountInfo>, Error> {

    lazy_static::lazy_static! {
        static ref MOUNT_NAME_REGEX: regex::Regex = regex::Regex::new(r"^mnt-datastore-(.+)\.mount$").unwrap();
    }

    let mut list = Vec::new();

    let basedir = "/etc/systemd/system";
    for item in crate::tools::fs::scan_subdir(libc::AT_FDCWD, basedir, &MOUNT_NAME_REGEX)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().to_string();

        let unitfile = format!("{}/{}", basedir, name);
        let config = systemd::config::parse_systemd_mount(&unitfile)?;
        let data: SystemdMountSection = config.lookup("Mount", "Mount")?;

        list.push(DatastoreMountInfo {
            unitfile,
            device: data.What,
            path: data.Where,
            filesystem: data.Type,
            options: data.Options,
        });
    }

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            name: {
                schema: DATASTORE_SCHEMA,
            },
            disk: {
                schema: BLOCKDEVICE_NAME_SCHEMA,
            },
            "add-datastore": {
                description: "Configure a datastore using the directory.",
                type: bool,
                optional: true,
            },
            filesystem: {
                type: FileSystemType,
                optional: true,
            },
         }
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["system", "disks"], PRIV_SYS_MODIFY, false),
    },
)]
/// Create a Filesystem on an unused disk. Will be mounted under '/mnt/datastore/<name>'.".
fn create_datastore_disk(
    name: String,
    disk: String,
    add_datastore: Option<bool>,
    filesystem: Option<FileSystemType>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {

    let to_stdout = if rpcenv.env_type() == RpcEnvironmentType::CLI { true } else { false };

    let username = rpcenv.get_user().unwrap();

    let upid_str = WorkerTask::new_thread(
        "dircreate", Some(name.clone()), &username.clone(), to_stdout, move |worker|
        {
            worker.log(format!("create datastore '{}' on disk {}", name, disk));

            let add_datastore = add_datastore.unwrap_or(false);
            let filesystem = filesystem.unwrap_or(FileSystemType::Ext4);

            let manager = DiskManage::new();

            let disk = manager.clone().disk_by_name(&disk)?;

            let partition = create_single_linux_partition(&disk)?;
            create_file_system(&partition, filesystem)?;

            let uuid = get_fs_uuid(&partition)?;
            let uuid_path = format!("/dev/disk/by-uuid/{}", uuid);

            let (mount_unit_name, mount_point) = create_datastore_mount_unit(&name, filesystem, &uuid_path)?;

            systemd::reload_daemon()?;
            systemd::enable_unit(&mount_unit_name)?;
            systemd::start_unit(&mount_unit_name)?;

            if add_datastore {
                crate::api2::config::datastore::create_datastore(json!({ "name": name, "path": mount_point }))?
            }

            Ok(())
        })?;

    Ok(upid_str)
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_DATASTORE_MOUNTS)
    .post(&API_METHOD_CREATE_DATASTORE_DISK);


fn create_datastore_mount_unit(
    datastore_name: &str,
    fs_type: FileSystemType,
    what: &str,
) -> Result<(String, String), Error> {

    let mount_point = format!("/mnt/datastore/{}", datastore_name);
    let mut mount_unit_name = systemd::escape_unit(&mount_point, true);
    mount_unit_name.push_str(".mount");

    let mount_unit_path = format!("/etc/systemd/system/{}", mount_unit_name);

    let unit = SystemdUnitSection {
        Description: format!("Mount datatstore '{}' under '{}'", datastore_name, mount_point),
        ..Default::default()
    };

    let install = SystemdInstallSection {
        WantedBy: Some(vec!["multi-user.target".to_string()]),
        ..Default::default()
    };

    let mount = SystemdMountSection {
        What: what.to_string(),
        Where: mount_point.clone(),
        Type: Some(fs_type.to_string()),
        Options: Some(String::from("defaults")),
        ..Default::default()
    };

    let mut config = SectionConfigData::new();
    config.set_data("Unit", "Unit", unit)?;
    config.set_data("Install", "Install", install)?;
    config.set_data("Mount", "Mount", mount)?;

    systemd::config::save_systemd_mount(&mount_unit_path, &config)?;

    Ok((mount_unit_name, mount_point))
}
