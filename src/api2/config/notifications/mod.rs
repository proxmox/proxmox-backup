use anyhow::Error;
use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;

use proxmox_router::{list_subdirs_api_method, ApiMethod, Permission, RpcEnvironment};
use proxmox_router::{Router, SubdirMap};
use proxmox_schema::api;
use proxmox_sortable_macro::sortable;

use crate::api2::admin::datastore::get_datastore_list;
use pbs_api_types::PRIV_SYS_AUDIT;

use crate::api2::admin::prune::list_prune_jobs;
use crate::api2::admin::sync::list_sync_jobs;
use crate::api2::admin::verify::list_verification_jobs;
use crate::api2::config::media_pool::list_pools;
use crate::api2::tape::backup::list_tape_backup_jobs;

pub mod gotify;
pub mod matchers;
pub mod sendmail;
pub mod smtp;
pub mod targets;

#[sortable]
const SUBDIRS: SubdirMap = &sorted!([
    ("endpoints", &ENDPOINT_ROUTER),
    ("matcher-fields", &FIELD_ROUTER),
    ("matcher-field-values", &VALUE_ROUTER),
    ("targets", &targets::ROUTER),
    ("matchers", &matchers::ROUTER),
]);

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);

#[sortable]
const ENDPOINT_SUBDIRS: SubdirMap = &sorted!([
    ("gotify", &gotify::ROUTER),
    ("sendmail", &sendmail::ROUTER),
    ("smtp", &smtp::ROUTER),
]);

const ENDPOINT_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(ENDPOINT_SUBDIRS))
    .subdirs(ENDPOINT_SUBDIRS);

const FIELD_ROUTER: Router = Router::new().get(&API_METHOD_GET_FIELDS);
const VALUE_ROUTER: Router = Router::new().get(&API_METHOD_GET_VALUES);

#[api]
#[derive(Serialize)]
/// A matchable field.
pub struct MatchableField {
    /// Name of the field
    name: String,
}

#[api]
#[derive(Serialize)]
/// A matchable metadata field value.
pub struct MatchableValue {
    /// Field this value belongs to.
    field: String,
    /// Notification metadata value known by the system.
    value: String,
    /// Additional comment for this value.
    comment: Option<String>,
}

#[api(
    protected: false,
    input: {
        properties: {},
    },
    returns: {
        description: "List of known metadata fields.",
        type: Array,
        items: { type: MatchableField },
    },
    access: {
        permission: &Permission::Privilege(&["system", "notifications"], PRIV_SYS_AUDIT, false),
    },
)]
/// Get all known metadata fields.
pub fn get_fields() -> Result<Vec<MatchableField>, Error> {
    let fields = ["datastore", "hostname", "job-id", "media-pool", "type"]
        .into_iter()
        .map(Into::into)
        .map(|name| MatchableField { name })
        .collect();

    Ok(fields)
}

#[api(
    protected: false,
    input: {
        properties: {},
    },
    returns: {
        description: "List of known metadata field values.",
        type: Array,
        items: { type: MatchableValue },
    },
    access: {
        permission: &Permission::Privilege(&["system", "notifications"], PRIV_SYS_AUDIT, false),
    },
)]
/// List all known, matchable metadata field values.
pub fn get_values(
    param: Value,
    info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<MatchableValue>, Error> {
    let mut values = Vec::new();

    let datastores = get_datastore_list(param.clone(), info, rpcenv)?;

    for datastore in datastores {
        values.push(MatchableValue {
            field: "datastore".into(),
            value: datastore.store.clone(),
            comment: datastore.comment.clone(),
        });
    }

    let pools = list_pools(rpcenv)?;
    for pool in pools {
        values.push(MatchableValue {
            field: "media-pool".into(),
            value: pool.name.clone(),
            comment: None,
        });
    }

    let tape_backup_jobs = list_tape_backup_jobs(param.clone(), rpcenv)?;
    for job in tape_backup_jobs {
        values.push(MatchableValue {
            field: "job-id".into(),
            value: job.config.id,
            comment: job.config.comment,
        });
    }

    let prune_jobs = list_prune_jobs(None, param.clone(), rpcenv)?;
    for job in prune_jobs {
        values.push(MatchableValue {
            field: "job-id".into(),
            value: job.config.id,
            comment: job.config.comment,
        });
    }

    let sync_jobs = list_sync_jobs(None, param.clone(), rpcenv)?;
    for job in sync_jobs {
        values.push(MatchableValue {
            field: "job-id".into(),
            value: job.config.id,
            comment: job.config.comment,
        });
    }

    let verify_jobs = list_verification_jobs(None, param.clone(), rpcenv)?;
    for job in verify_jobs {
        values.push(MatchableValue {
            field: "job-id".into(),
            value: job.config.id,
            comment: job.config.comment,
        });
    }

    values.push(MatchableValue {
        field: "hostname".into(),
        value: proxmox_sys::nodename().into(),
        comment: None,
    });

    for ty in [
        "acme",
        "gc",
        "package-updates",
        "prune",
        "sync",
        "system-mail",
        "tape-backup",
        "tape-load",
        "verify",
    ] {
        values.push(MatchableValue {
            field: "type".into(),
            value: ty.into(),
            comment: None,
        });
    }

    values.sort_by(|a, b| match a.field.cmp(&b.field) {
        Ordering::Equal => a.value.cmp(&b.value),
        ord => ord,
    });

    Ok(values)
}
