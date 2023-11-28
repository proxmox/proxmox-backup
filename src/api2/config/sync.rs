use ::serde::{Deserialize, Serialize};
use anyhow::{bail, Error};
use hex::FromHex;
use serde_json::Value;

use proxmox_router::{http_bail, Permission, Router, RpcEnvironment};
use proxmox_schema::{api, param_bail};

use pbs_api_types::{
    Authid, SyncJobConfig, SyncJobConfigUpdater, JOB_ID_SCHEMA, PRIV_DATASTORE_AUDIT,
    PRIV_DATASTORE_BACKUP, PRIV_DATASTORE_MODIFY, PRIV_DATASTORE_PRUNE, PRIV_REMOTE_AUDIT,
    PRIV_REMOTE_READ, PROXMOX_CONFIG_DIGEST_SCHEMA,
};
use pbs_config::sync;

use pbs_config::CachedUserInfo;

pub fn check_sync_job_read_access(
    user_info: &CachedUserInfo,
    auth_id: &Authid,
    job: &SyncJobConfig,
) -> bool {
    let ns_anchor_privs = user_info.lookup_privs(auth_id, &job.acl_path());
    if ns_anchor_privs & PRIV_DATASTORE_AUDIT == 0 {
        return false;
    }

    if let Some(remote) = &job.remote {
        let remote_privs = user_info.lookup_privs(auth_id, &["remote", remote]);
        remote_privs & PRIV_REMOTE_AUDIT != 0
    } else {
        let source_ds_privs = user_info.lookup_privs(auth_id, &["datastore", &job.remote_store]);
        source_ds_privs & PRIV_DATASTORE_AUDIT != 0
    }
}

/// checks whether user can run the corresponding pull job
///
/// namespace creation/deletion ACL and backup group ownership checks happen in the pull code directly.
/// remote side checks/filters remote datastore/namespace/group access.
pub fn check_sync_job_modify_access(
    user_info: &CachedUserInfo,
    auth_id: &Authid,
    job: &SyncJobConfig,
) -> bool {
    let ns_anchor_privs = user_info.lookup_privs(auth_id, &job.acl_path());
    if ns_anchor_privs & PRIV_DATASTORE_BACKUP == 0 {
        return false;
    }

    if let Some(true) = job.remove_vanished {
        if ns_anchor_privs & PRIV_DATASTORE_PRUNE == 0 {
            return false;
        }
    }

    let correct_owner = match job.owner {
        Some(ref owner) => {
            owner == auth_id
                || (owner.is_token() && !auth_id.is_token() && owner.user() == auth_id.user())
        }
        // default sync owner
        None => auth_id == Authid::root_auth_id(),
    };

    // same permission as changing ownership after syncing
    if !correct_owner && ns_anchor_privs & PRIV_DATASTORE_MODIFY == 0 {
        return false;
    }

    if let Some(remote) = &job.remote {
        let remote_privs = user_info.lookup_privs(auth_id, &["remote", remote, &job.remote_store]);
        return remote_privs & PRIV_REMOTE_READ != 0;
    }
    true
}

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List configured jobs.",
        type: Array,
        items: { type: SyncJobConfig },
    },
    access: {
        description: "Limited to sync job entries where user has Datastore.Audit on target datastore, and Remote.Audit on source remote.",
        permission: &Permission::Anybody,
    },
)]
/// List all sync jobs
pub fn list_sync_jobs(
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<SyncJobConfig>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, digest) = sync::config()?;

    let list = config.convert_to_typed_array("sync")?;

    rpcenv["digest"] = hex::encode(digest).into();

    let list = list
        .into_iter()
        .filter(|sync_job| check_sync_job_read_access(&user_info, &auth_id, sync_job))
        .collect();
    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            config: {
                type: SyncJobConfig,
                flatten: true,
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
    config: SyncJobConfig,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let _lock = sync::lock_config()?;

    if !check_sync_job_modify_access(&user_info, &auth_id, &config) {
        bail!("permission check failed");
    }

    if config.remote.is_none() && config.store.eq(&config.remote_store) {
        bail!("source and target datastore can't be the same");
    }

    if let Some(max_depth) = config.max_depth {
        if let Some(ref ns) = config.ns {
            ns.check_max_depth(max_depth)?;
        }
        if let Some(ref ns) = config.remote_ns {
            ns.check_max_depth(max_depth)?;
        }
    }

    let (mut section_config, _digest) = sync::config()?;

    if section_config.sections.get(&config.id).is_some() {
        param_bail!("id", "job '{}' already exists.", config.id);
    }

    section_config.set_data(&config.id, "sync", &config)?;

    sync::save_config(&section_config)?;

    crate::server::jobstate::create_state_file("syncjob", &config.id)?;

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
    returns: { type: SyncJobConfig },
    access: {
        description: "Limited to sync job entries where user has Datastore.Audit on target datastore, and Remote.Audit on source remote.",
        permission: &Permission::Anybody,
    },
)]
/// Read a sync job configuration.
pub fn read_sync_job(id: String, rpcenv: &mut dyn RpcEnvironment) -> Result<SyncJobConfig, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let (config, digest) = sync::config()?;

    let sync_job = config.lookup("sync", &id)?;
    if !check_sync_job_read_access(&user_info, &auth_id, &sync_job) {
        bail!("permission check failed");
    }

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(sync_job)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the remote property(-> meaning local).
    Remote,
    /// Delete the owner property.
    Owner,
    /// Delete the comment property.
    Comment,
    /// Delete the job schedule.
    Schedule,
    /// Delete the remove-vanished flag.
    RemoveVanished,
    /// Delete the group_filter property.
    GroupFilter,
    /// Delete the rate_in property.
    RateIn,
    /// Delete the burst_in property.
    BurstIn,
    /// Delete the rate_out property.
    RateOut,
    /// Delete the burst_out property.
    BurstOut,
    /// Delete the ns property,
    Ns,
    /// Delete the remote_ns property,
    RemoteNs,
    /// Delete the max_depth property,
    MaxDepth,
    /// Delete the transfer_last property,
    TransferLast,
}

#[api(
    protected: true,
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
            update: {
                type: SyncJobConfigUpdater,
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
        description: "User needs Datastore.Backup on target datastore, and Remote.Read on source remote. Additionally, remove_vanished requires Datastore.Prune, and any owner other than the user themselves requires Datastore.Modify",
    },
)]
/// Update sync job config.
#[allow(clippy::too_many_arguments)]
pub fn update_sync_job(
    id: String,
    update: SyncJobConfigUpdater,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let _lock = sync::lock_config()?;

    let (mut config, expected_digest) = sync::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: SyncJobConfig = config.lookup("sync", &id)?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::Remote => {
                    data.remote = None;
                }
                DeletableProperty::Owner => {
                    data.owner = None;
                }
                DeletableProperty::Comment => {
                    data.comment = None;
                }
                DeletableProperty::Schedule => {
                    data.schedule = None;
                }
                DeletableProperty::RemoveVanished => {
                    data.remove_vanished = None;
                }
                DeletableProperty::GroupFilter => {
                    data.group_filter = None;
                }
                DeletableProperty::RateIn => {
                    data.limit.rate_in = None;
                }
                DeletableProperty::RateOut => {
                    data.limit.rate_out = None;
                }
                DeletableProperty::BurstIn => {
                    data.limit.burst_in = None;
                }
                DeletableProperty::BurstOut => {
                    data.limit.burst_out = None;
                }
                DeletableProperty::Ns => {
                    data.ns = None;
                }
                DeletableProperty::RemoteNs => {
                    data.remote_ns = None;
                }
                DeletableProperty::MaxDepth => {
                    data.max_depth = None;
                }
                DeletableProperty::TransferLast => {
                    data.transfer_last = None;
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
    if let Some(ns) = update.ns {
        data.ns = Some(ns);
    }
    if let Some(remote) = update.remote {
        data.remote = Some(remote);
    }
    if let Some(remote_store) = update.remote_store {
        data.remote_store = remote_store;
    }
    if let Some(remote_ns) = update.remote_ns {
        data.remote_ns = Some(remote_ns);
    }
    if let Some(owner) = update.owner {
        data.owner = Some(owner);
    }
    if let Some(group_filter) = update.group_filter {
        data.group_filter = Some(group_filter);
    }
    if let Some(transfer_last) = update.transfer_last {
        data.transfer_last = Some(transfer_last);
    }

    if update.limit.rate_in.is_some() {
        data.limit.rate_in = update.limit.rate_in;
    }

    if update.limit.rate_out.is_some() {
        data.limit.rate_out = update.limit.rate_out;
    }

    if update.limit.burst_in.is_some() {
        data.limit.burst_in = update.limit.burst_in;
    }

    if update.limit.burst_out.is_some() {
        data.limit.burst_out = update.limit.burst_out;
    }

    let schedule_changed = data.schedule != update.schedule;
    if update.schedule.is_some() {
        data.schedule = update.schedule;
    }
    if update.remove_vanished.is_some() {
        data.remove_vanished = update.remove_vanished;
    }
    if let Some(max_depth) = update.max_depth {
        data.max_depth = Some(max_depth);
    }

    if let Some(max_depth) = data.max_depth {
        if let Some(ref ns) = data.ns {
            ns.check_max_depth(max_depth)?;
        }
        if let Some(ref ns) = data.remote_ns {
            ns.check_max_depth(max_depth)?;
        }
    }

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

    let _lock = sync::lock_config()?;

    let (mut config, expected_digest) = sync::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config.lookup("sync", &id) {
        Ok(job) => {
            if !check_sync_job_modify_access(&user_info, &auth_id, &job) {
                bail!("permission check failed");
            }
            config.sections.remove(&id);
        }
        Err(_) => {
            http_bail!(NOT_FOUND, "job '{}' does not exist.", id)
        }
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
    let (user_cfg, _) = pbs_config::user::test_cfg_from_str(
        r###"
user: noperm@pbs

user: read@pbs

user: write@pbs

"###,
    )
    .expect("test user.cfg is not parsable");
    let acl_tree = pbs_config::acl::AclTree::from_raw(
        r###"
acl:1:/datastore/localstore1:read@pbs,write@pbs:DatastoreAudit
acl:1:/datastore/localstore1:write@pbs:DatastoreBackup
acl:1:/datastore/localstore2:write@pbs:DatastorePowerUser
acl:1:/datastore/localstore3:write@pbs:DatastoreAdmin
acl:1:/remote/remote1:read@pbs,write@pbs:RemoteAudit
acl:1:/remote/remote1/remotestore1:write@pbs:RemoteSyncOperator
"###,
    )
    .expect("test acl.cfg is not parsable");

    let user_info = CachedUserInfo::test_new(user_cfg, acl_tree);

    let root_auth_id = Authid::root_auth_id();

    let no_perm_auth_id: Authid = "noperm@pbs".parse()?;
    let read_auth_id: Authid = "read@pbs".parse()?;
    let write_auth_id: Authid = "write@pbs".parse()?;

    let mut job = SyncJobConfig {
        id: "regular".to_string(),
        remote: Some("remote0".to_string()),
        remote_store: "remotestore1".to_string(),
        remote_ns: None,
        store: "localstore0".to_string(),
        ns: None,
        owner: Some(write_auth_id.clone()),
        comment: None,
        remove_vanished: None,
        max_depth: None,
        group_filter: None,
        schedule: None,
        limit: pbs_api_types::RateLimitConfig::default(), // no limit
        transfer_last: None,
    };

    // should work without ACLs
    assert!(check_sync_job_read_access(&user_info, root_auth_id, &job));
    assert!(check_sync_job_modify_access(&user_info, root_auth_id, &job));

    // user without permissions must fail
    assert!(!check_sync_job_read_access(
        &user_info,
        &no_perm_auth_id,
        &job
    ));
    assert!(!check_sync_job_modify_access(
        &user_info,
        &no_perm_auth_id,
        &job
    ));

    // reading without proper read permissions on either remote or local must fail
    assert!(!check_sync_job_read_access(&user_info, &read_auth_id, &job));

    // reading without proper read permissions on local end must fail
    job.remote = Some("remote1".to_string());
    assert!(!check_sync_job_read_access(&user_info, &read_auth_id, &job));

    // reading without proper read permissions on remote end must fail
    job.remote = Some("remote0".to_string());
    job.store = "localstore1".to_string();
    assert!(!check_sync_job_read_access(&user_info, &read_auth_id, &job));

    // writing without proper write permissions on either end must fail
    job.store = "localstore0".to_string();
    assert!(!check_sync_job_modify_access(
        &user_info,
        &write_auth_id,
        &job
    ));

    // writing without proper write permissions on local end must fail
    job.remote = Some("remote1".to_string());

    // writing without proper write permissions on remote end must fail
    job.remote = Some("remote0".to_string());
    job.store = "localstore1".to_string();
    assert!(!check_sync_job_modify_access(
        &user_info,
        &write_auth_id,
        &job
    ));

    // reset remote to one where users have access
    job.remote = Some("remote1".to_string());

    // user with read permission can only read, but not modify/run
    assert!(check_sync_job_read_access(&user_info, &read_auth_id, &job));
    job.owner = Some(read_auth_id.clone());
    assert!(!check_sync_job_modify_access(
        &user_info,
        &read_auth_id,
        &job
    ));
    job.owner = None;
    assert!(!check_sync_job_modify_access(
        &user_info,
        &read_auth_id,
        &job
    ));
    job.owner = Some(write_auth_id.clone());
    assert!(!check_sync_job_modify_access(
        &user_info,
        &read_auth_id,
        &job
    ));

    // user with simple write permission can modify/run
    assert!(check_sync_job_read_access(&user_info, &write_auth_id, &job));
    assert!(check_sync_job_modify_access(
        &user_info,
        &write_auth_id,
        &job
    ));

    // but can't modify/run with deletion
    job.remove_vanished = Some(true);
    assert!(!check_sync_job_modify_access(
        &user_info,
        &write_auth_id,
        &job
    ));

    // unless they have Datastore.Prune as well
    job.store = "localstore2".to_string();
    assert!(check_sync_job_modify_access(
        &user_info,
        &write_auth_id,
        &job
    ));

    // changing owner is not possible
    job.owner = Some(read_auth_id.clone());
    assert!(!check_sync_job_modify_access(
        &user_info,
        &write_auth_id,
        &job
    ));

    // also not to the default 'root@pam'
    job.owner = None;
    assert!(!check_sync_job_modify_access(
        &user_info,
        &write_auth_id,
        &job
    ));

    // unless they have Datastore.Modify as well
    job.store = "localstore3".to_string();
    job.owner = Some(read_auth_id);
    assert!(check_sync_job_modify_access(
        &user_info,
        &write_auth_id,
        &job
    ));
    job.owner = None;
    assert!(check_sync_job_modify_access(
        &user_info,
        &write_auth_id,
        &job
    ));

    Ok(())
}
