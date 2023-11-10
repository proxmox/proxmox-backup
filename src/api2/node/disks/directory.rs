use ::serde::{Deserialize, Serialize};
use anyhow::{bail, Error};
use serde_json::json;

use proxmox_router::{Permission, Router, RpcEnvironment, RpcEnvironmentType};
use proxmox_schema::api;
use proxmox_section_config::SectionConfigData;
use proxmox_sys::task_log;

use pbs_api_types::{
    DataStoreConfig, BLOCKDEVICE_NAME_SCHEMA, DATASTORE_SCHEMA, NODE_SCHEMA, PRIV_SYS_AUDIT,
    PRIV_SYS_MODIFY, UPID_SCHEMA,
};

use crate::tools::disks::{
    create_file_system, create_single_linux_partition, get_fs_uuid, DiskManage, DiskUsageQuery,
    DiskUsageType, FileSystemType,
};
use crate::tools::systemd::{self, types::*};

use proxmox_rest_server::WorkerTask;

const BASE_MOUNT_DIR: &str = "/mnt/datastore/";

#[api(
    properties: {
        "filesystem": {
            type: FileSystemType,
            optional: true,
        },
    },
)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Datastore mount info.
pub struct DatastoreMountInfo {
    /// The path of the mount unit.
    pub unitfile: String,
    /// The name of the mount
    pub name: String,
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
pub fn list_datastore_mounts() -> Result<Vec<DatastoreMountInfo>, Error> {
    lazy_static::lazy_static! {
        static ref MOUNT_NAME_REGEX: regex::Regex = regex::Regex::new(r"^mnt-datastore-(.+)\.mount$").unwrap();
    }

    let mut list = Vec::new();

    let basedir = "/etc/systemd/system";
    for item in proxmox_sys::fs::scan_subdir(libc::AT_FDCWD, basedir, &MOUNT_NAME_REGEX)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().to_string();

        let unitfile = format!("{}/{}", basedir, name);
        let config = systemd::config::parse_systemd_mount(&unitfile)?;
        let data: SystemdMountSection = config.lookup("Mount", "Mount")?;

        let name = data
            .Where
            .strip_prefix(BASE_MOUNT_DIR)
            .unwrap_or(&data.Where)
            .to_string();

        list.push(DatastoreMountInfo {
            unitfile,
            name,
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
/// Create a Filesystem on an unused disk. Will be mounted under `/mnt/datastore/<name>`.
pub fn create_datastore_disk(
    name: String,
    disk: String,
    add_datastore: Option<bool>,
    filesystem: Option<FileSystemType>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let auth_id = rpcenv.get_auth_id().unwrap();

    let info = DiskUsageQuery::new().smart(false).find(&disk)?;

    if info.used != DiskUsageType::Unused {
        bail!("disk '{}' is already in use.", disk);
    }

    let mount_point = format!("{}{}", BASE_MOUNT_DIR, &name);

    // check if the default path does exist already and bail if it does
    let default_path = std::path::PathBuf::from(&mount_point);

    match std::fs::metadata(&default_path) {
        Err(_) => {} // path does not exist
        Ok(_) => {
            bail!("path {:?} already exists", default_path);
        }
    }

    let upid_str = WorkerTask::new_thread(
        "dircreate",
        Some(name.clone()),
        auth_id,
        to_stdout,
        move |worker| {
            task_log!(worker, "create datastore '{}' on disk {}", name, disk);

            let add_datastore = add_datastore.unwrap_or(false);
            let filesystem = filesystem.unwrap_or(FileSystemType::Ext4);

            let manager = DiskManage::new();

            let disk = manager.disk_by_name(&disk)?;

            let partition = create_single_linux_partition(&disk)?;
            create_file_system(&partition, filesystem)?;

            let uuid = get_fs_uuid(&partition)?;
            let uuid_path = format!("/dev/disk/by-uuid/{}", uuid);

            let mount_unit_name =
                create_datastore_mount_unit(&name, &mount_point, filesystem, &uuid_path)?;

            crate::tools::systemd::reload_daemon()?;
            crate::tools::systemd::enable_unit(&mount_unit_name)?;
            crate::tools::systemd::start_unit(&mount_unit_name)?;

            if add_datastore {
                let lock = pbs_config::datastore::lock_config()?;
                let datastore: DataStoreConfig =
                    serde_json::from_value(json!({ "name": name, "path": mount_point }))?;

                let (config, _digest) = pbs_config::datastore::config()?;

                if config.sections.get(&datastore.name).is_some() {
                    bail!("datastore '{}' already exists.", datastore.name);
                }

                crate::api2::config::datastore::do_create_datastore(
                    lock,
                    config,
                    datastore,
                    Some(&worker),
                )?;
            }

            Ok(())
        },
    )?;

    Ok(upid_str)
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
        }
    },
    access: {
        permission: &Permission::Privilege(&["system", "disks"], PRIV_SYS_MODIFY, false),
    },
)]
/// Remove a Filesystem mounted under `/mnt/datastore/<name>`.
pub fn delete_datastore_disk(name: String) -> Result<(), Error> {
    let path = format!("{}{}", BASE_MOUNT_DIR, name);
    // path of datastore cannot be changed
    let (config, _) = pbs_config::datastore::config()?;
    let datastores: Vec<DataStoreConfig> = config.convert_to_typed_array("datastore")?;
    let conflicting_datastore: Option<DataStoreConfig> =
        datastores.into_iter().find(|ds| ds.path == path);

    if let Some(conflicting_datastore) = conflicting_datastore {
        bail!(
            "Can't remove '{}' since it's required by datastore '{}'",
            conflicting_datastore.path,
            conflicting_datastore.name
        );
    }

    // disable systemd mount-unit
    let mut mount_unit_name = proxmox_sys::systemd::escape_unit(&path, true);
    mount_unit_name.push_str(".mount");
    crate::tools::systemd::disable_unit(&mount_unit_name)?;

    // delete .mount-file
    let mount_unit_path = format!("/etc/systemd/system/{}", mount_unit_name);
    let full_path = std::path::Path::new(&mount_unit_path);
    log::info!("removing systemd mount unit {:?}", full_path);
    std::fs::remove_file(full_path)?;

    // try to unmount, if that fails tell the user to reboot or unmount manually
    let mut command = std::process::Command::new("umount");
    command.arg(&path);
    match proxmox_sys::command::run_command(command, None) {
        Err(_) => bail!(
            "Could not umount '{}' since it is busy. It will stay mounted \
             until the next reboot or until unmounted manually!",
            path
        ),
        Ok(_) => Ok(()),
    }
}

const ITEM_ROUTER: Router = Router::new().delete(&API_METHOD_DELETE_DATASTORE_DISK);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_DATASTORE_MOUNTS)
    .post(&API_METHOD_CREATE_DATASTORE_DISK)
    .match_all("name", &ITEM_ROUTER);

fn create_datastore_mount_unit(
    datastore_name: &str,
    mount_point: &str,
    fs_type: FileSystemType,
    what: &str,
) -> Result<String, Error> {
    let mut mount_unit_name = proxmox_sys::systemd::escape_unit(mount_point, true);
    mount_unit_name.push_str(".mount");

    let mount_unit_path = format!("/etc/systemd/system/{}", mount_unit_name);

    let unit = SystemdUnitSection {
        Description: format!(
            "Mount datatstore '{}' under '{}'",
            datastore_name, mount_point
        ),
        ..Default::default()
    };

    let install = SystemdInstallSection {
        WantedBy: Some(vec!["multi-user.target".to_string()]),
        ..Default::default()
    };

    let mount = SystemdMountSection {
        What: what.to_string(),
        Where: mount_point.to_string(),
        Type: Some(fs_type.to_string()),
        Options: Some(String::from("defaults")),
        ..Default::default()
    };

    let mut config = SectionConfigData::new();
    config.set_data("Unit", "Unit", unit)?;
    config.set_data("Install", "Install", install)?;
    config.set_data("Mount", "Mount", mount)?;

    systemd::config::save_systemd_mount(&mount_unit_path, &config)?;

    Ok(mount_unit_name)
}
