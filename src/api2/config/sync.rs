use anyhow::{bail, Error};
use serde_json::Value;
use ::serde::{Deserialize, Serialize};

use proxmox::api::{api, Permission, Router, RpcEnvironment};
use proxmox::tools::fs::open_file_locked;

use crate::api2::types::*;

use crate::config::acl::{
    PRIV_DATASTORE_AUDIT,
    PRIV_DATASTORE_BACKUP,
    PRIV_DATASTORE_MODIFY,
    PRIV_DATASTORE_PRUNE,
    PRIV_REMOTE_AUDIT,
    PRIV_REMOTE_READ,
};

use crate::config::cached_user_info::CachedUserInfo;
use crate::config::sync::{self, SyncJobConfig};

pub fn check_sync_job_read_access(
    user_info: &CachedUserInfo,
    auth_id: &Authid,
    job: &SyncJobConfig,
) -> bool {
    let datastore_privs = user_info.lookup_privs(&auth_id, &["datastore", &job.store]);
    if datastore_privs & PRIV_DATASTORE_AUDIT == 0 {
        return false;
    }

    let remote_privs = user_info.lookup_privs(&auth_id, &["remote", &job.remote]);
    remote_privs & PRIV_REMOTE_AUDIT != 0
}

// user can run the corresponding pull job
pub fn check_sync_job_modify_access(
    user_info: &CachedUserInfo,
    auth_id: &Authid,
    job: &SyncJobConfig,
) -> bool {
    let datastore_privs = user_info.lookup_privs(&auth_id, &["datastore", &job.store]);
    if datastore_privs & PRIV_DATASTORE_BACKUP == 0 {
        return false;
    }

    if let Some(true) = job.remove_vanished {
        if datastore_privs & PRIV_DATASTORE_PRUNE == 0 {
            return false;
        }
    }

    let correct_owner = match job.owner {
        Some(ref owner) => {
            owner == auth_id
                || (owner.is_token()
                    && !auth_id.is_token()
                    && owner.user() == auth_id.user())
        },
        // default sync owner
        None => auth_id == Authid::root_auth_id(),
    };

    // same permission as changing ownership after syncing
    if !correct_owner && datastore_privs & PRIV_DATASTORE_MODIFY == 0 {
        return false;
    }

    let remote_privs = user_info.lookup_privs(&auth_id, &["remote", &job.remote, &job.remote_store]);
    remote_privs & PRIV_REMOTE_READ != 0
}

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List configured jobs.",
        type: Array,
        items: { type: sync::SyncJobConfig },
    },
    access: {
        description: "Limited to sync job entries where user has Datastore.Audit on target datastore, and Remote.Audit on source remote.",
        permission: &Permission::Anybody,
    },
)]
/// List all sync jobs
pub fn list_sync_jobs(
    _param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<SyncJobConfig>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, digest) = sync::config()?;

    let list = config.convert_to_typed_array("sync")?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    let list = list
        .into_iter()
        .filter(|sync_job| check_sync_job_read_access(&user_info, &auth_id, &sync_job))
        .collect();
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
            owner: {
                type: Authid,
                optional: true,
            },
            remote: {
                schema: REMOTE_ID_SCHEMA,
            },
            "remote-store": {
                schema: DATASTORE_SCHEMA,
            },
            "remove-vanished": {
                schema: REMOVE_VANISHED_BACKUPS_SCHEMA,
                optional: true,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            schedule: {
                optional: true,
                schema: SYNC_SCHEDULE_SCHEMA,
            },
        },
    },
    access: {
        description: "User needs Datastore.Backup on target datastore, and Remote.Read on source remote. Additionally, remove_vanished requires Datastore.Prune, and any owner other than the user themselves requires Datastore.Modify",
        permission: &Permission::Anybody,
    },
)]
/// Create a new sync job.
pub fn create_sync_job(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let _lock = open_file_locked(sync::SYNC_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let sync_job: sync::SyncJobConfig = serde_json::from_value(param)?;
    if !check_sync_job_modify_access(&user_info, &auth_id, &sync_job) {
        bail!("permission check failed");
    }

    let (mut config, _digest) = sync::config()?;

    if config.sections.get(&sync_job.id).is_some() {
        bail!("job '{}' already exists.", sync_job.id);
    }

    config.set_data(&sync_job.id, "sync", &sync_job)?;

    sync::save_config(&config)?;

    crate::server::jobstate::create_state_file("syncjob", &sync_job.id)?;

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
    returns: { type: sync::SyncJobConfig },
    access: {
        description: "Limited to sync job entries where user has Datastore.Audit on target datastore, and Remote.Audit on source remote.",
        permission: &Permission::Anybody,
    },
)]
/// Read a sync job configuration.
pub fn read_sync_job(
    id: String,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<SyncJobConfig, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, digest) = sync::config()?;

    let sync_job = config.lookup("sync", &id)?;
    if !check_sync_job_read_access(&user_info, &auth_id, &sync_job) {
        bail!("permission check failed");
    }

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(sync_job)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
#[allow(non_camel_case_types)]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the owner property.
    owner,
    /// Delete the comment property.
    comment,
    /// Delete the job schedule.
    schedule,
    /// Delete the remove-vanished flag.
    remove_vanished,
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
            owner: {
                type: Authid,
                optional: true,
            },
            remote: {
                schema: REMOTE_ID_SCHEMA,
                optional: true,
            },
            "remote-store": {
                schema: DATASTORE_SCHEMA,
                optional: true,
            },
            "remove-vanished": {
                schema: REMOVE_VANISHED_BACKUPS_SCHEMA,
                optional: true,
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
        permission: &Permission::Anybody,
        description: "User needs Datastore.Backup on target datastore, and Remote.Read on source remote. Additionally, remove_vanished requires Datastore.Prune, and any owner other than the user themselves requires Datastore.Modify",
    },
)]
/// Update sync job config.
#[allow(clippy::too_many_arguments)]
pub fn update_sync_job(
    id: String,
    store: Option<String>,
    owner: Option<Authid>,
    remote: Option<String>,
    remote_store: Option<String>,
    remove_vanished: Option<bool>,
    comment: Option<String>,
    schedule: Option<String>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let _lock = open_file_locked(sync::SYNC_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    // pass/compare digest
    let (mut config, expected_digest) = sync::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: sync::SyncJobConfig = config.lookup("sync", &id)?;

     if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::owner => { data.owner = None; },
                DeletableProperty::comment => { data.comment = None; },
                DeletableProperty::schedule => { data.schedule = None; },
                DeletableProperty::remove_vanished => { data.remove_vanished = None; },
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
    if let Some(remote) = remote { data.remote = remote; }
    if let Some(remote_store) = remote_store { data.remote_store = remote_store; }
    if let Some(owner) = owner { data.owner = Some(owner); }

    let schedule_changed = data.schedule != schedule;
    if schedule.is_some() { data.schedule = schedule; }
    if remove_vanished.is_some() { data.remove_vanished = remove_vanished; }

    if !check_sync_job_modify_access(&user_info, &auth_id, &data) {
        bail!("permission check failed");
    }

    config.set_data(&id, "sync", &data)?;

    sync::save_config(&config)?;

    if schedule_changed {
        crate::server::jobstate::update_job_last_run_time("syncjob", &id)?;
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
        description: "User needs Datastore.Backup on target datastore, and Remote.Read on source remote. Additionally, remove_vanished requires Datastore.Prune, and any owner other than the user themselves requires Datastore.Modify",
    },
)]
/// Remove a sync job configuration
pub fn delete_sync_job(
    id: String,
    digest: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let _lock = open_file_locked(sync::SYNC_CFG_LOCKFILE, std::time::Duration::new(10, 0), true)?;

    let (mut config, expected_digest) = sync::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config.lookup("sync", &id) {
        Ok(job) => {
            if !check_sync_job_modify_access(&user_info, &auth_id, &job) {
                bail!("permission check failed");
            }
            config.sections.remove(&id);
        },
        Err(_) => { bail!("job '{}' does not exist.", id) },
    };

    sync::save_config(&config)?;

    crate::server::jobstate::remove_state_file("syncjob", &id)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_SYNC_JOB)
    .put(&API_METHOD_UPDATE_SYNC_JOB)
    .delete(&API_METHOD_DELETE_SYNC_JOB);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_SYNC_JOBS)
    .post(&API_METHOD_CREATE_SYNC_JOB)
    .match_all("id", &ITEM_ROUTER);


#[test]
fn sync_job_access_test() -> Result<(), Error> {
    let (user_cfg, _) = crate::config::user::test_cfg_from_str(r###"
user: noperm@pbs

user: read@pbs

user: write@pbs

"###).expect("test user.cfg is not parsable");
    let acl_tree = crate::config::acl::AclTree::from_raw(r###"
acl:1:/datastore/localstore1:read@pbs,write@pbs:DatastoreAudit
acl:1:/datastore/localstore1:write@pbs:DatastoreBackup
acl:1:/datastore/localstore2:write@pbs:DatastorePowerUser
acl:1:/datastore/localstore3:write@pbs:DatastoreAdmin
acl:1:/remote/remote1:read@pbs,write@pbs:RemoteAudit
acl:1:/remote/remote1/remotestore1:write@pbs:RemoteSyncOperator
"###).expect("test acl.cfg is not parsable");

    let user_info = CachedUserInfo::test_new(user_cfg, acl_tree);

    let root_auth_id = Authid::root_auth_id();

    let no_perm_auth_id: Authid = "noperm@pbs".parse()?;
    let read_auth_id: Authid = "read@pbs".parse()?;
    let write_auth_id: Authid = "write@pbs".parse()?;

    let mut job = SyncJobConfig {
        id: "regular".to_string(),
        remote: "remote0".to_string(),
        remote_store: "remotestore1".to_string(),
        store: "localstore0".to_string(),
        owner: Some(write_auth_id.clone()),
        comment: None,
        remove_vanished: None,
        schedule: None,
    };

    // should work without ACLs
    assert_eq!(check_sync_job_read_access(&user_info, &root_auth_id, &job), true);
    assert_eq!(check_sync_job_modify_access(&user_info, &root_auth_id, &job), true);

    // user without permissions must fail
    assert_eq!(check_sync_job_read_access(&user_info, &no_perm_auth_id, &job), false);
    assert_eq!(check_sync_job_modify_access(&user_info, &no_perm_auth_id, &job), false);

    // reading without proper read permissions on either remote or local must fail
    assert_eq!(check_sync_job_read_access(&user_info, &read_auth_id, &job), false);

    // reading without proper read permissions on local end must fail
    job.remote = "remote1".to_string();
    assert_eq!(check_sync_job_read_access(&user_info, &read_auth_id, &job), false);

    // reading without proper read permissions on remote end must fail
    job.remote = "remote0".to_string();
    job.store = "localstore1".to_string();
    assert_eq!(check_sync_job_read_access(&user_info, &read_auth_id, &job), false);

    // writing without proper write permissions on either end must fail
    job.store = "localstore0".to_string();
    assert_eq!(check_sync_job_modify_access(&user_info, &write_auth_id, &job), false);

    // writing without proper write permissions on local end must fail
    job.remote = "remote1".to_string();

    // writing without proper write permissions on remote end must fail
    job.remote = "remote0".to_string();
    job.store = "localstore1".to_string();
    assert_eq!(check_sync_job_modify_access(&user_info, &write_auth_id, &job), false);

    // reset remote to one where users have access
    job.remote = "remote1".to_string();

    // user with read permission can only read, but not modify/run
    assert_eq!(check_sync_job_read_access(&user_info, &read_auth_id, &job), true);
    job.owner = Some(read_auth_id.clone());
    assert_eq!(check_sync_job_modify_access(&user_info, &read_auth_id, &job), false);
    job.owner = None;
    assert_eq!(check_sync_job_modify_access(&user_info, &read_auth_id, &job), false);
    job.owner = Some(write_auth_id.clone());
    assert_eq!(check_sync_job_modify_access(&user_info, &read_auth_id, &job), false);

    // user with simple write permission can modify/run
    assert_eq!(check_sync_job_read_access(&user_info, &write_auth_id, &job), true);
    assert_eq!(check_sync_job_modify_access(&user_info, &write_auth_id, &job), true);

    // but can't modify/run with deletion
    job.remove_vanished = Some(true);
    assert_eq!(check_sync_job_modify_access(&user_info, &write_auth_id, &job), false);

    // unless they have Datastore.Prune as well
    job.store = "localstore2".to_string();
    assert_eq!(check_sync_job_modify_access(&user_info, &write_auth_id, &job), true);

    // changing owner is not possible
    job.owner = Some(read_auth_id.clone());
    assert_eq!(check_sync_job_modify_access(&user_info, &write_auth_id, &job), false);

    // also not to the default 'root@pam'
    job.owner = None;
    assert_eq!(check_sync_job_modify_access(&user_info, &write_auth_id, &job), false);

    // unless they have Datastore.Modify as well
    job.store = "localstore3".to_string();
    job.owner = Some(read_auth_id);
    assert_eq!(check_sync_job_modify_access(&user_info, &write_auth_id, &job), true);
    job.owner = None;
    assert_eq!(check_sync_job_modify_access(&user_info, &write_auth_id, &job), true);

    Ok(())
}
