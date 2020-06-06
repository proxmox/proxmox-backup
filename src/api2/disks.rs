use anyhow::{bail, format_err, Error};

use serde_json::{json, Value};

use proxmox::api::{api, RpcEnvironment, Permission, UserInformation};
use proxmox::api::router::{Router, SubdirMap};
use proxmox::{sortable, identity};
use proxmox::{http_err, list_subdirs_api_method};

use crate::config::acl::{PRIV_SYS_AUDIT, PRIV_SYS_MODIFY};
use crate::tools::disks::{DiskUsageInfo, get_disks};

#[api(
    input: {
        properties: {
            skipsmart: {
		description: "Skip smart checks.",
		type: bool,
		optional: true,
		default: false,
            }
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
) -> Result<Vec<DiskUsageInfo>, Error> {

    let mut list = Vec::new();

    for (_, info) in get_disks(None, skipsmart)? {
        list.push(info);
    }

    Ok(list)
}

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([
//    ("lvm", &lvm::ROUTER),
    (
        "list", &Router::new()
            .get(&API_METHOD_LIST_DISKS)
    ),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
