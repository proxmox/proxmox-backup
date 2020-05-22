//! Sync datastore from remote server

use anyhow::{format_err, Error};

use proxmox::api::api;
use proxmox::api::{ApiMethod, Router, RpcEnvironment, Permission};

use crate::server::{WorkerTask};
use crate::backup::DataStore;
use crate::client::{HttpClient, HttpClientOptions, BackupRepository, pull::pull_store};
use crate::api2::types::*;
use crate::config::{
    remote,
    acl::{PRIV_DATASTORE_BACKUP, PRIV_DATASTORE_PRUNE, PRIV_REMOTE_READ},
    cached_user_info::CachedUserInfo,
};

#[api(
    input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
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
        },
    },
    access: {
        // Note: used parameters are no uri parameters, so we need to test inside function body
        description: r###"The user needs Datastore.Backup privilege on '/datastore/{store}',
and needs to own the backup group. Remote.Read is required on '/remote/{remote}/{remote-store}'.
The delete flag additionally requires the Datastore.Prune privilege on '/datastore/{store}'.
"###,
        permission: &Permission::Anybody,
    },
)]
/// Sync store from other repository
async fn pull (
    store: String,
    remote: String,
    remote_store: String,
    remove_vanished: Option<bool>,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {

    let user_info = CachedUserInfo::new()?;

    let username = rpcenv.get_user().unwrap();
    user_info.check_privs(&username, &["datastore", &store], PRIV_DATASTORE_BACKUP, false)?;
    user_info.check_privs(&username, &["remote", &remote, &remote_store], PRIV_REMOTE_READ, false)?;

    let delete = remove_vanished.unwrap_or(true);

    if delete {
        user_info.check_privs(&username, &["datastore", &store], PRIV_DATASTORE_PRUNE, false)?;
    }

    let tgt_store = DataStore::lookup_datastore(&store)?;

    let (remote_config, _digest) = remote::config()?;
    let remote: remote::Remote = remote_config.lookup("remote", &remote)?;

    let options = HttpClientOptions::new()
        .password(Some(remote.password.clone()))
        .fingerprint(remote.fingerprint.clone());

    let client = HttpClient::new(&remote.host, &remote.userid, options)?;
    let _auth_info = client.login() // make sure we can auth
        .await
        .map_err(|err| format_err!("remote connection to '{}' failed - {}", remote.host, err))?;

    let src_repo = BackupRepository::new(Some(remote.userid), Some(remote.host), remote_store);

    // fixme: set to_stdout to false?
    let upid_str = WorkerTask::spawn("sync", Some(store.clone()), &username.clone(), true, move |worker| async move {

        worker.log(format!("sync datastore '{}' start", store));

        pull_store(&worker, &client, &src_repo, tgt_store.clone(), delete, username).await?;

        worker.log(format!("sync datastore '{}' end", store));

        Ok(())
    })?;

    Ok(upid_str)
}

pub const ROUTER: Router = Router::new()
    .post(&API_METHOD_PULL);
