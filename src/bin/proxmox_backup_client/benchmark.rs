use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Error};
use serde_json::Value;
use chrono::{TimeZone, Utc};

use proxmox::api::{ApiMethod, RpcEnvironment};
use proxmox::api::api;

use proxmox_backup::backup::{
   load_and_decrypt_key,
   CryptConfig,

};

use proxmox_backup::client::*;

use crate::{
    KEYFILE_SCHEMA, REPO_URL_SCHEMA,
    extract_repository_from_value,
    get_encryption_key_password,
    record_repository,
    connect,
};

#[api(
   input: {
       properties: {
           repository: {
               schema: REPO_URL_SCHEMA,
               optional: true,
           },
           keyfile: {
               schema: KEYFILE_SCHEMA,
               optional: true,
           },
       }
   }
)]
/// Run benchmark tests
pub async fn benchmark(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let repo = extract_repository_from_value(&param)?;

    let keyfile = param["keyfile"].as_str().map(PathBuf::from);

    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            let (key, _) = load_and_decrypt_key(&path, &get_encryption_key_password)?;
            let crypt_config = CryptConfig::new(key)?;
            Some(Arc::new(crypt_config))
        }
    };

    let backup_time = Utc.timestamp(Utc::now().timestamp(), 0);

    let client = connect(repo.host(), repo.user())?;
    record_repository(&repo);

    let client = BackupWriter::start(
        client,
        crypt_config.clone(),
        repo.store(),
        "host",
        "benshmark",
        backup_time,
        false,
    ).await?;

    println!("Start upload speed test");
    let speed = client.upload_speedtest().await?;

    println!("Upload speed: {} MiB/s", speed);

    Ok(())
}
