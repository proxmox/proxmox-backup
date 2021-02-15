use anyhow::{bail, Error};
use serde_json::Value;
use ::serde::{Deserialize, Serialize};

use proxmox::api::{api, Router, RpcEnvironment, schema::Updatable};
use proxmox::tools::fs::open_file_locked;

use crate::{
    api2::types::{
        JOB_ID_SCHEMA,
        PROXMOX_CONFIG_DIGEST_SCHEMA,
    },
    config::{
        self,
        tape_job::{
            TAPE_JOB_CFG_LOCKFILE,
            TapeBackupJobConfig,
            TapeBackupJobConfigUpdater,
        }
    },
};

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List configured jobs.",
        type: Array,
        items: { type: TapeBackupJobConfig },
    },
)]
/// List all tape backup jobs
pub fn list_tape_backup_jobs(
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<TapeBackupJobConfig>, Error> {

    let (config, digest) = config::tape_job::config()?;

    let list = config.convert_to_typed_array("backup")?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            job: {
                type: TapeBackupJobConfig,
                flatten: true,
            },
        },
    },
)]
/// Create a new tape backup job.
pub fn create_tape_backup_job(
    job: TapeBackupJobConfig,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let _lock = open_file_locked(TAPE_JOB_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let (mut config, _digest) = config::tape_job::config()?;

    if config.sections.get(&job.id).is_some() {
        bail!("job '{}' already exists.", job.id);
    }

    config.set_data(&job.id, "backup", &job)?;

    config::tape_job::save_config(&config)?;

    crate::server::jobstate::create_state_file("tape-backup-job", &job.id)?;

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
    returns: { type: TapeBackupJobConfig },
)]
/// Read a tape backup job configuration.
pub fn read_tape_backup_job(
    id: String,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<TapeBackupJobConfig, Error> {

    let (config, digest) = config::tape_job::config()?;

    let job = config.lookup("backup", &id)?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(job)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the comment property.
    Comment,
    /// Delete the job schedule.
    Schedule,
    /// Delete the eject-media property
    EjectMedia,
    /// Delete the export-media-set property
    ExportMediaSet,
}

#[api(
    protected: true,
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
            update: {
                flatten: true,
                type: TapeBackupJobConfigUpdater,
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
)]
/// Update the tape backup job
pub fn update_tape_backup_job(
    id: String,
    update: TapeBackupJobConfigUpdater,
    delete: Option<Vec<String>>,
    digest: Option<String>,
) -> Result<(), Error> {
    let _lock = open_file_locked(TAPE_JOB_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let (mut config, expected_digest) = config::tape_job::config()?;

    let mut job: TapeBackupJobConfig = config.lookup("backup", &id)?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    job.update_from(update, &delete.unwrap_or(Vec::new()))?;

    config.set_data(&job.id, "backup", &job)?;

    config::tape_job::save_config(&config)?;

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
)]
/// Remove a tape backup job configuration
pub fn delete_tape_backup_job(
    id: String,
    digest: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let _lock = open_file_locked(TAPE_JOB_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let (mut config, expected_digest) = config::tape_job::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config.lookup::<TapeBackupJobConfig>("backup", &id) {
        Ok(_job) => {
            config.sections.remove(&id);
        },
        Err(_) => { bail!("job '{}' does not exist.", id) },
    };

    config::tape_job::save_config(&config)?;

    crate::server::jobstate::remove_state_file("tape-backup-job", &id)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_TAPE_BACKUP_JOB)
//.put(&API_METHOD_UPDATE_TAPE_BACKUP_JOB)
    .delete(&API_METHOD_DELETE_TAPE_BACKUP_JOB);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_TAPE_BACKUP_JOBS)
    .post(&API_METHOD_CREATE_TAPE_BACKUP_JOB)
    .match_all("id", &ITEM_ROUTER);
