use anyhow::{Error};

use proxmox::api::{api, Permission};
use proxmox::api::router::{Router, SubdirMap};
use proxmox::{sortable, identity};
use proxmox::{list_subdirs_api_method};

use crate::config::acl::{PRIV_SYS_AUDIT};
use crate::tools::disks::{
    DiskUsageInfo, DiskUsageType, DiskManage, SmartData,
    get_disks, get_smart_data,
};

#[api(
    protected: true,
    input: {
        properties: {
            skipsmart: {
		description: "Skip smart checks.",
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
    usage_type: Option<DiskUsageType>,
) -> Result<Vec<DiskUsageInfo>, Error> {

    let mut list = Vec::new();

    for (_, info) in get_disks(None, skipsmart)? {
        if let Some(ref usage_type) = usage_type {
            if info.used == *usage_type {
                list.push(info);
            }
        } else {
            list.push(info);
        }
    }

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            disk: {
		description: "Block device name.",
		type: String,
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
pub fn smart_status(
    disk: String,
    healthonly: Option<bool>,
) -> Result<SmartData, Error> {

    let healthonly = healthonly.unwrap_or(false);

    let manager = DiskManage::new();
    let disk = manager.disk_by_name(&disk)?;
    get_smart_data(&disk, healthonly)
}

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([
//    ("lvm", &lvm::ROUTER),
    (
        "list", &Router::new()
            .get(&API_METHOD_LIST_DISKS)
    ),
    (
        "smart", &Router::new()
            .get(&API_METHOD_SMART_STATUS)
    ),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
