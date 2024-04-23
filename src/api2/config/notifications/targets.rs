use anyhow::Error;
use serde_json::Value;

use proxmox_notify::api::Target;
use proxmox_notify::schema::ENTITY_NAME_SCHEMA;
use proxmox_router::{list_subdirs_api_method, Permission, Router, RpcEnvironment, SubdirMap};
use proxmox_schema::api;
use proxmox_sortable_macro::sortable;

use pbs_api_types::{PRIV_SYS_AUDIT, PRIV_SYS_MODIFY};

#[api(
    protected: true,
    input: {
        properties: {},
    },
    returns: {
        description: "List of all entities which can be used as notification targets.",
        type: Array,
        items: { type: Target },
    },
    access: {
        permission: &Permission::Privilege(&["system", "notifications"], PRIV_SYS_AUDIT, false),
    },
)]
/// List all notification targets.
pub fn list_targets(_param: Value, _rpcenv: &mut dyn RpcEnvironment) -> Result<Vec<Target>, Error> {
    let config = pbs_config::notifications::config()?;
    let targets = proxmox_notify::api::get_targets(&config)?;

    Ok(targets)
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: ENTITY_NAME_SCHEMA,
            },
        }
    },
    access: {
        permission: &Permission::Privilege(&["system", "notifications"], PRIV_SYS_MODIFY, false),
    },
)]
/// Test a given notification target.
pub fn test_target(name: String, _rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let config = pbs_config::notifications::config()?;
    proxmox_notify::api::common::test_target(&config, &name)?;
    Ok(())
}

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([("test", &TEST_ROUTER),]);
const TEST_ROUTER: Router = Router::new().post(&API_METHOD_TEST_TARGET);
const ITEM_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_TARGETS)
    .match_all("name", &ITEM_ROUTER);
