use anyhow::{bail, Error};
use serde_json::{json, Value};

use proxmox_router::{
    list_subdirs_api_method, Permission, Router, RpcEnvironment, RpcEnvironmentType, SubdirMap,
};
use proxmox_schema::api;
use proxmox_sortable_macro::sortable;
use proxmox_sys::task_log;

use pbs_api_types::{
    BLOCKDEVICE_DISK_AND_PARTITION_NAME_SCHEMA, BLOCKDEVICE_NAME_SCHEMA, NODE_SCHEMA,
    PRIV_SYS_AUDIT, PRIV_SYS_MODIFY, UPID_SCHEMA,
};

use crate::tools::disks::{
    get_smart_data, inititialize_gpt_disk, wipe_blockdev, DiskManage, DiskUsageInfo,
    DiskUsageQuery, DiskUsageType, SmartData,
};
use proxmox_rest_server::WorkerTask;

pub mod directory;
pub mod zfs;

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            skipsmart: {
                description: "Skip smart checks.",
                type: bool,
                optional: true,
                default: false,
            },
            "include-partitions": {
                description: "Include partitions.",
                type: bool,
                optional: true,
                default: false,
            },
            "usage-type": {
                type: DiskUsageType,
                optional: true,
            },
        },
    },
    returns: {
        description: "Local disk list.",
        type: Array,
        items: {
            type: DiskUsageInfo,
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "disks"], PRIV_SYS_AUDIT, false),
    },
)]
/// List local disks
pub fn list_disks(
    skipsmart: bool,
    include_partitions: bool,
    usage_type: Option<DiskUsageType>,
) -> Result<Vec<DiskUsageInfo>, Error> {
    let mut list = Vec::new();

    for (_, info) in DiskUsageQuery::new()
        .smart(!skipsmart)
        .partitions(include_partitions)
        .query()?
    {
        if let Some(ref usage_type) = usage_type {
            if info.used == *usage_type {
                list.push(info);
            }
        } else {
            list.push(info);
        }
    }

    list.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            disk: {
                schema: BLOCKDEVICE_NAME_SCHEMA,
            },
            healthonly: {
                description: "If true returns only the health status.",
                type: bool,
                optional: true,
            },
        },
    },
    returns: {
        type: SmartData,
    },
    access: {
        permission: &Permission::Privilege(&["system", "disks"], PRIV_SYS_AUDIT, false),
    },
)]
/// Get SMART attributes and health of a disk.
pub fn smart_status(disk: String, healthonly: Option<bool>) -> Result<SmartData, Error> {
    let healthonly = healthonly.unwrap_or(false);

    let manager = DiskManage::new();
    let disk = manager.disk_by_name(&disk)?;
    get_smart_data(&disk, healthonly)
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            disk: {
                schema: BLOCKDEVICE_NAME_SCHEMA,
            },
            uuid: {
                description: "UUID for the GPT table.",
                type: String,
                optional: true,
                max_length: 36,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["system", "disks"], PRIV_SYS_MODIFY, false),
    },
)]
/// Initialize empty Disk with GPT
pub fn initialize_disk(
    disk: String,
    uuid: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let auth_id = rpcenv.get_auth_id().unwrap();

    let info = DiskUsageQuery::new().find(&disk)?;

    if info.used != DiskUsageType::Unused {
        bail!("disk '{}' is already in use.", disk);
    }

    let upid_str = WorkerTask::new_thread(
        "diskinit",
        Some(disk.clone()),
        auth_id,
        to_stdout,
        move |worker| {
            task_log!(worker, "initialize disk {}", disk);

            let disk_manager = DiskManage::new();
            let disk_info = disk_manager.disk_by_name(&disk)?;

            inititialize_gpt_disk(&disk_info, uuid.as_deref())?;

            Ok(())
        },
    )?;

    Ok(json!(upid_str))
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            disk: {
                schema: BLOCKDEVICE_DISK_AND_PARTITION_NAME_SCHEMA,
            },
        },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["system", "disks"], PRIV_SYS_MODIFY, false),
    },
)]
/// wipe disk
pub fn wipe_disk(disk: String, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let auth_id = rpcenv.get_auth_id().unwrap();

    let upid_str = WorkerTask::new_thread(
        "wipedisk",
        Some(disk.clone()),
        auth_id,
        to_stdout,
        move |worker| {
            task_log!(worker, "wipe disk {}", disk);

            let disk_manager = DiskManage::new();
            let disk_info = disk_manager.partition_by_name(&disk)?;

            wipe_blockdev(&disk_info, worker)?;

            Ok(())
        },
    )?;

    Ok(json!(upid_str))
}

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([
    //    ("lvm", &lvm::ROUTER),
    ("directory", &directory::ROUTER),
    ("zfs", &zfs::ROUTER),
    ("initgpt", &Router::new().post(&API_METHOD_INITIALIZE_DISK)),
    ("list", &Router::new().get(&API_METHOD_LIST_DISKS)),
    ("smart", &Router::new().get(&API_METHOD_SMART_STATUS)),
    ("wipedisk", &Router::new().put(&API_METHOD_WIPE_DISK)),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
