use anyhow::{bail, Error};
use serde_json::Value;
use ::serde::{Deserialize, Serialize};

use proxmox::api::{api, Router, RpcEnvironment, Permission};
use proxmox::tools::fs::open_file_locked;

use crate::{
    api2::types::{
        Authid,
        Userid,
        JOB_ID_SCHEMA,
        DATASTORE_SCHEMA,
        DRIVE_NAME_SCHEMA,
        PROXMOX_CONFIG_DIGEST_SCHEMA,
        SINGLE_LINE_COMMENT_SCHEMA,
        MEDIA_POOL_NAME_SCHEMA,
        SYNC_SCHEDULE_SCHEMA,
    },
    config::{
        self,
        cached_user_info::CachedUserInfo,
        acl::{
            PRIV_TAPE_AUDIT,
            PRIV_TAPE_MODIFY,
        },
        tape_job::{
            TAPE_JOB_CFG_LOCKFILE,
            TapeBackupJobConfig,
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
    access: {
        description: "List configured tape jobs filtered by Tape.Audit privileges",
        permission: &Permission::Anybody,
    },
)]
/// List all tape backup jobs
pub fn list_tape_backup_jobs(
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<TapeBackupJobConfig>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, digest) = config::tape_job::config()?;

    let list = config.convert_to_typed_array::<TapeBackupJobConfig>("backup")?;

    let list = list
        .into_iter()
        .filter(|job| {
            let privs = user_info.lookup_privs(&auth_id, &["tape", "job", &job.id]);
            privs & PRIV_TAPE_AUDIT != 0
        })
        .collect();

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
    access: {
        permission: &Permission::Privilege(&["tape", "job"], PRIV_TAPE_MODIFY, false),
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
    access: {
        permission: &Permission::Privilege(&["tape", "job", "{id}"], PRIV_TAPE_AUDIT, false),
    },
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
    /// Delete the 'latest-only' property
    LatestOnly,
    /// Delete the 'notify-user' property
    NotifyUser,
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
                optional: true,
            },
            pool: {
                schema: MEDIA_POOL_NAME_SCHEMA,
                optional: true,
            },
            drive: {
                schema: DRIVE_NAME_SCHEMA,
                optional: true,
            },
            "eject-media": {
                description: "Eject media upon job completion.",
                type: bool,
                optional: true,
            },
            "export-media-set": {
                description: "Export media set upon job completion.",
                type: bool,
                optional: true,
            },
            "latest-only": {
                description: "Backup latest snapshots only.",
                type: bool,
                optional: true,
            },
            "notify-user": {
                optional: true,
                type: Userid,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            schedule: {
                optional: true,
                schema: SYNC_SCHEDULE_SCHEMA,
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
        permission: &Permission::Privilege(&["tape", "job", "{id}"], PRIV_TAPE_MODIFY, false),
    },
)]
/// Update the tape backup job
pub fn update_tape_backup_job(
    id: String,
    store: Option<String>,
    pool: Option<String>,
    drive: Option<String>,
    eject_media: Option<bool>,
    export_media_set: Option<bool>,
    latest_only: Option<bool>,
    notify_user: Option<Userid>,
    comment: Option<String>,
    schedule: Option<String>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
) -> Result<(), Error> {
    let _lock = open_file_locked(TAPE_JOB_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let (mut config, expected_digest) = config::tape_job::config()?;

    let mut data: TapeBackupJobConfig = config.lookup("backup", &id)?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::EjectMedia => { data.setup.eject_media = None; },
                DeletableProperty::ExportMediaSet => { data.setup.export_media_set = None; },
                DeletableProperty::LatestOnly => { data.setup.latest_only = None; },
                DeletableProperty::NotifyUser => { data.setup.notify_user = None; },
                DeletableProperty::Schedule => { data.schedule = None; },
                DeletableProperty::Comment => { data.comment = None; },
            }
        }
    }

    if let Some(store) = store { data.setup.store = store; }
    if let Some(pool) = pool { data.setup.pool = pool; }
    if let Some(drive) = drive { data.setup.drive = drive; }

    if eject_media.is_some() { data.setup.eject_media = eject_media; };
    if export_media_set.is_some() { data.setup.export_media_set = export_media_set; }
    if latest_only.is_some() { data.setup.latest_only = latest_only; }
    if notify_user.is_some() { data.setup.notify_user = notify_user; }

    let schedule_changed = data.schedule != schedule;
    if schedule.is_some() { data.schedule = schedule; }

    if let Some(comment) = comment {
        let comment = comment.trim();
        if comment.is_empty() {
            data.comment = None;
        } else {
            data.comment = Some(comment.to_string());
        }
    }

    config.set_data(&id, "backup", &data)?;

    config::tape_job::save_config(&config)?;

    if schedule_changed {
        crate::server::jobstate::try_update_state_file("tape-backup-job", &id)?;
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
        permission: &Permission::Privilege(&["tape", "job", "{id}"], PRIV_TAPE_MODIFY, false),
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
    .put(&API_METHOD_UPDATE_TAPE_BACKUP_JOB)
    .delete(&API_METHOD_DELETE_TAPE_BACKUP_JOB);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_TAPE_BACKUP_JOBS)
    .post(&API_METHOD_CREATE_TAPE_BACKUP_JOB)
    .match_all("id", &ITEM_ROUTER);
