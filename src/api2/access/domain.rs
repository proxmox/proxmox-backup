//! List Authentication domains/realms

use anyhow::{format_err, Error};
use serde_json::{json, Value};

use proxmox_router::{Permission, Router, RpcEnvironment, RpcEnvironmentType, SubdirMap};
use proxmox_schema::api;

use pbs_api_types::{
    Authid, BasicRealmInfo, Realm, PRIV_PERMISSIONS_MODIFY, REMOVE_VANISHED_SCHEMA, UPID_SCHEMA,
};

use crate::server::jobstate::Job;

#[api(
    returns: {
        description: "List of realms with basic info.",
        type: Array,
        items: {
            type: BasicRealmInfo,
        }
    },
    access: {
        description: "Anyone can access this, because we need that list for the login box (before the user is authenticated).",
        permission: &Permission::World,
    }
)]
/// Authentication domain/realm index.
fn list_domains(rpcenv: &mut dyn RpcEnvironment) -> Result<Vec<BasicRealmInfo>, Error> {
    let mut list = Vec::new();

    list.push(serde_json::from_value(json!({
        "realm": "pam",
        "type": "pam",
        "comment": "Linux PAM standard authentication",
        "default": Some(true),
    }))?);
    list.push(serde_json::from_value(json!({
        "realm": "pbs",
        "type": "pbs",
        "comment": "Proxmox Backup authentication server",
    }))?);

    let (config, digest) = pbs_config::domains::config()?;

    for (_, (section_type, v)) in config.sections.iter() {
        let mut entry = v.clone();
        entry["type"] = Value::from(section_type.clone());
        list.push(serde_json::from_value(entry)?);
    }

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            realm: {
                type: Realm,
            },
            "dry-run": {
                type: bool,
                description: "If set, do not create/delete anything",
                default: false,
                optional: true,
            },
            "remove-vanished": {
                optional: true,
                schema: REMOVE_VANISHED_SCHEMA,
            },
            "enable-new": {
                description: "Enable newly synced users immediately",
                optional: true,
            }
         },
    },
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["access", "users"], PRIV_PERMISSIONS_MODIFY, false),
    },
)]
/// Synchronize users of a given realm
pub fn sync_realm(
    realm: Realm,
    dry_run: bool,
    remove_vanished: Option<String>,
    enable_new: Option<bool>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let job = Job::new("realm-sync", realm.as_str())
        .map_err(|_| format_err!("realm sync already running"))?;

    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let upid_str = crate::server::do_realm_sync_job(
        job,
        realm.clone(),
        &auth_id,
        None,
        to_stdout,
        dry_run,
        remove_vanished,
        enable_new,
    )
    .map_err(|err| {
        format_err!(
            "unable to start realm sync job on realm {} - {}",
            realm.as_str(),
            err
        )
    })?;

    Ok(json!(upid_str))
}

const SYNC_ROUTER: Router = Router::new().post(&API_METHOD_SYNC_REALM);
const SYNC_SUBDIRS: SubdirMap = &[("sync", &SYNC_ROUTER)];

const REALM_ROUTER: Router = Router::new().subdirs(SYNC_SUBDIRS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_DOMAINS)
    .match_all("realm", &REALM_ROUTER);
