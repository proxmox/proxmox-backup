use ::serde::{Deserialize, Serialize};
use anyhow::Error;
use hex::FromHex;
use serde_json::Value;

use proxmox_router::{http_bail, Permission, Router, RpcEnvironment};
use proxmox_schema::{api, param_bail};

use pbs_api_types::{
    Authid, VerificationJobConfig, VerificationJobConfigUpdater, JOB_ID_SCHEMA,
    PRIV_DATASTORE_AUDIT, PRIV_DATASTORE_VERIFY, PROXMOX_CONFIG_DIGEST_SCHEMA,
};
use pbs_config::verify;

use pbs_config::CachedUserInfo;

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List configured jobs.",
        type: Array,
        items: { type: VerificationJobConfig },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Requires Datastore.Audit or Datastore.Verify on datastore.",
    },
)]
/// List all verification jobs
pub fn list_verification_jobs(
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<VerificationJobConfig>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let required_privs = PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_VERIFY;

    let (config, digest) = verify::config()?;

    let list = config.convert_to_typed_array("verification")?;

    let list = list
        .into_iter()
        .filter(|job: &VerificationJobConfig| {
            let privs = user_info.lookup_privs(&auth_id, &job.acl_path());

            privs & required_privs != 00
        })
        .collect();

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            config: {
                type: VerificationJobConfig,
                flatten: true,
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
        description: "Requires Datastore.Verify on job's datastore.",
    },
)]
/// Create a new verification job.
pub fn create_verification_job(
    config: VerificationJobConfig,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    user_info.check_privs(&auth_id, &config.acl_path(), PRIV_DATASTORE_VERIFY, false)?;

    let _lock = verify::lock_config()?;

    let (mut section_config, _digest) = verify::config()?;

    if section_config.sections.get(&config.id).is_some() {
        param_bail!("id", "job '{}' already exists.", config.id);
    }

    section_config.set_data(&config.id, "verification", &config)?;

    verify::save_config(&section_config)?;

    crate::server::jobstate::create_state_file("verificationjob", &config.id)?;

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
    returns: { type: VerificationJobConfig },
    access: {
        permission: &Permission::Anybody,
        description: "Requires Datastore.Audit or Datastore.Verify on job's datastore.",
    },
)]
/// Read a verification job configuration.
pub fn read_verification_job(
    id: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<VerificationJobConfig, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, digest) = verify::config()?;

    let verification_job: VerificationJobConfig = config.lookup("verification", &id)?;

    let required_privs = PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_VERIFY;
    user_info.check_privs(&auth_id, &verification_job.acl_path(), required_privs, true)?;

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(verification_job)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the ignore verified property.
    IgnoreVerified,
    /// Delete the comment property.
    Comment,
    /// Delete the job schedule.
    Schedule,
    /// Delete outdated after property.
    OutdatedAfter,
    /// Delete namespace property, defaulting to root namespace then.
    Ns,
    /// Delete max-depth property, defaulting to full recursion again
    MaxDepth,
}

#[api(
    protected: true,
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
            update: {
                type: VerificationJobConfigUpdater,
                flatten: true,
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
        permission: &Permission::Anybody,
        description: "Requires Datastore.Verify on job's datastore.",
    },
)]
/// Update verification job config.
#[allow(clippy::too_many_arguments)]
pub fn update_verification_job(
    id: String,
    update: VerificationJobConfigUpdater,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let _lock = verify::lock_config()?;

    // pass/compare digest
    let (mut config, expected_digest) = verify::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: VerificationJobConfig = config.lookup("verification", &id)?;

    // check existing store and NS
    user_info.check_privs(&auth_id, &data.acl_path(), PRIV_DATASTORE_VERIFY, true)?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::IgnoreVerified => {
                    data.ignore_verified = None;
                }
                DeletableProperty::OutdatedAfter => {
                    data.outdated_after = None;
                }
                DeletableProperty::Comment => {
                    data.comment = None;
                }
                DeletableProperty::Schedule => {
                    data.schedule = None;
                }
                DeletableProperty::Ns => {
                    data.ns = None;
                }
                DeletableProperty::MaxDepth => {
                    data.max_depth = None;
                }
            }
        }
    }

    if let Some(comment) = update.comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            data.comment = None;
        } else {
            data.comment = Some(comment);
        }
    }

    if let Some(store) = update.store {
        data.store = store;
    }

    if update.ignore_verified.is_some() {
        data.ignore_verified = update.ignore_verified;
    }
    if update.outdated_after.is_some() {
        data.outdated_after = update.outdated_after;
    }
    let schedule_changed = data.schedule != update.schedule;
    if update.schedule.is_some() {
        data.schedule = update.schedule;
    }
    if let Some(ns) = update.ns {
        if !ns.is_root() {
            data.ns = Some(ns);
        }
    }
    if let Some(max_depth) = update.max_depth {
        if max_depth <= pbs_api_types::MAX_NAMESPACE_DEPTH {
            data.max_depth = Some(max_depth);
        }
    }

    // check new store and NS
    user_info.check_privs(&auth_id, &data.acl_path(), PRIV_DATASTORE_VERIFY, true)?;

    config.set_data(&id, "verification", &data)?;

    verify::save_config(&config)?;

    if schedule_changed {
        crate::server::jobstate::update_job_last_run_time("verificationjob", &id)?;
    }

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
        permission: &Permission::Anybody,
        description: "Requires Datastore.Verify on job's datastore.",
    },
)]
/// Remove a verification job configuration
pub fn delete_verification_job(
    id: String,
    digest: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let _lock = verify::lock_config()?;

    let (mut config, expected_digest) = verify::config()?;

    let job: VerificationJobConfig = config.lookup("verification", &id)?;
    user_info.check_privs(&auth_id, &job.acl_path(), PRIV_DATASTORE_VERIFY, true)?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config.sections.get(&id) {
        Some(_) => {
            config.sections.remove(&id);
        }
        None => http_bail!(NOT_FOUND, "job '{}' does not exist.", id),
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
