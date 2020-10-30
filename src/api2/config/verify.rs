use anyhow::{bail, Error};
use serde_json::Value;
use ::serde::{Deserialize, Serialize};

use proxmox::api::{api, Permission, Router, RpcEnvironment};
use proxmox::tools::fs::open_file_locked;

use crate::api2::types::*;

use crate::config::acl::{
    PRIV_DATASTORE_AUDIT,
    PRIV_DATASTORE_BACKUP,
    PRIV_DATASTORE_VERIFY,
};

use crate::config::verify::{self, VerificationJobConfig};

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List configured jobs.",
        type: Array,
        items: { type: verify::VerificationJobConfig },
    },
    access: {
        permission: &Permission::Privilege(
            &["datastore", "{store}"],
            PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_BACKUP | PRIV_DATASTORE_VERIFY,
            true),
    },
)]
/// List all verification jobs
pub fn list_verification_jobs(
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<VerificationJobConfig>, Error> {

    let (config, digest) = verify::config()?;

    let list = config.convert_to_typed_array("verification")?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(list)
}


#[api(
    protected: true,
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "ignore-verified": {
                optional: true,
                schema: IGNORE_VERIFIED_BACKUPS_SCHEMA,
            },
            "outdated-after": {
                optional: true,
                schema: VERIFICATION_OUTDATED_AFTER_SCHEMA,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            schedule: {
                optional: true,
                schema: VERIFICATION_SCHEDULE_SCHEMA,
            },
        }
    },
    access: {
        permission: &Permission::Privilege(
            &["datastore", "{store}"],
            PRIV_DATASTORE_VERIFY,
            true),
    },
)]
/// Create a new verification job.
pub fn create_verification_job(param: Value) -> Result<(), Error> {

    let _lock = open_file_locked(verify::VERIFICATION_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let verification_job: verify::VerificationJobConfig = serde_json::from_value(param.clone())?;

    let (mut config, _digest) = verify::config()?;

    if let Some(_) = config.sections.get(&verification_job.id) {
        bail!("job '{}' already exists.", verification_job.id);
    }

    config.set_data(&verification_job.id, "verification", &verification_job)?;

    verify::save_config(&config)?;

    crate::server::jobstate::create_state_file("verificationjob", &verification_job.id)?;

    Ok(())
}

#[api(
   input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
        },
    },
    returns: {
        description: "The verification job configuration.",
        type: verify::VerificationJobConfig,
    },
    access: {
        permission: &Permission::Privilege(
            &["datastore", "{store}"],
            PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_BACKUP | PRIV_DATASTORE_VERIFY,
            true),
    },
)]
/// Read a verification job configuration.
pub fn read_verification_job(
    id: String,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<VerificationJobConfig, Error> {
    let (config, digest) = verify::config()?;

    let verification_job = config.lookup("verification", &id)?;
    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(verification_job)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the ignore verified property.
    IgnoreVerified,
    /// Delete the comment property.
    Comment,
    /// Delete the job schedule.
    Schedule,
    /// Delete outdated after property.
    OutdatedAfter
}

#[api(
    protected: true,
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
            store: {
                optional: true,
                schema: DATASTORE_SCHEMA,
            },
            "ignore-verified": {
                optional: true,
                schema: IGNORE_VERIFIED_BACKUPS_SCHEMA,
            },
            "outdated-after": {
                optional: true,
                schema: VERIFICATION_OUTDATED_AFTER_SCHEMA,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            schedule: {
                optional: true,
                schema: VERIFICATION_SCHEDULE_SCHEMA,
            },
            delete: {
                description: "List of properties to delete.",
                type: Array,
                optional: true,
                items: {
                    type: DeletableProperty,
                }
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(
            &["datastore", "{store}"],
            PRIV_DATASTORE_VERIFY,
            true),
    },
)]
/// Update verification job config.
pub fn update_verification_job(
    id: String,
    store: Option<String>,
    ignore_verified: Option<bool>,
    outdated_after: Option<i64>,
    comment: Option<String>,
    schedule: Option<String>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
) -> Result<(), Error> {

    let _lock = open_file_locked(verify::VERIFICATION_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    // pass/compare digest
    let (mut config, expected_digest) = verify::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: verify::VerificationJobConfig = config.lookup("verification", &id)?;

     if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::IgnoreVerified => { data.ignore_verified = None; },
                DeletableProperty::OutdatedAfter => { data.outdated_after = None; },
                DeletableProperty::Comment => { data.comment = None; },
                DeletableProperty::Schedule => { data.schedule = None; },
            }
        }
    }

    if let Some(comment) = comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            data.comment = None;
        } else {
            data.comment = Some(comment);
        }
    }

    if let Some(store) = store { data.store = store; }

    if ignore_verified.is_some() { data.ignore_verified = ignore_verified; }
    if outdated_after.is_some() { data.outdated_after = outdated_after; }
    if schedule.is_some() { data.schedule = schedule; }

    config.set_data(&id, "verification", &data)?;

    verify::save_config(&config)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(
            &["datastore", "{store}"],
            PRIV_DATASTORE_VERIFY,
            true),
    },
)]
/// Remove a verification job configuration
pub fn delete_verification_job(id: String, digest: Option<String>) -> Result<(), Error> {

    let _lock = open_file_locked(verify::VERIFICATION_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let (mut config, expected_digest) = verify::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config.sections.get(&id) {
        Some(_) => { config.sections.remove(&id); },
        None => bail!("job '{}' does not exist.", id),
    }

    verify::save_config(&config)?;

    crate::server::jobstate::remove_state_file("verificationjob", &id)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_VERIFICATION_JOB)
    .put(&API_METHOD_UPDATE_VERIFICATION_JOB)
    .delete(&API_METHOD_DELETE_VERIFICATION_JOB);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_VERIFICATION_JOBS)
    .post(&API_METHOD_CREATE_VERIFICATION_JOB)
    .match_all("id", &ITEM_ROUTER);
