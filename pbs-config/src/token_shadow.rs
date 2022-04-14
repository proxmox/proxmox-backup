use std::collections::HashMap;

use anyhow::{bail, format_err, Error};
use serde::{Deserialize, Serialize};
use serde_json::{from_value, Value};

use proxmox_sys::fs::CreateOptions;

use pbs_api_types::Authid;
//use crate::auth;
use crate::{open_backup_lockfile, BackupLockGuard};

const LOCK_FILE: &str = pbs_buildcfg::configdir!("/token.shadow.lock");
const CONF_FILE: &str = pbs_buildcfg::configdir!("/token.shadow");

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// ApiToken id / secret pair
pub struct ApiTokenSecret {
    pub tokenid: Authid,
    pub secret: String,
}

// Get exclusive lock
fn lock_config() -> Result<BackupLockGuard, Error> {
    open_backup_lockfile(LOCK_FILE, None, true)
}

fn read_file() -> Result<HashMap<Authid, String>, Error> {
    let json = proxmox_sys::fs::file_get_json(CONF_FILE, Some(Value::Null))?;

    if json == Value::Null {
        Ok(HashMap::new())
    } else {
        // swallow serde error which might contain sensitive data
        from_value(json).map_err(|_err| format_err!("unable to parse '{}'", CONF_FILE))
    }
}

fn write_file(data: HashMap<Authid, String>) -> Result<(), Error> {
    let backup_user = crate::backup_user()?;
    let options = CreateOptions::new()
        .perm(nix::sys::stat::Mode::from_bits_truncate(0o0640))
        .owner(backup_user.uid)
        .group(backup_user.gid);

    let json = serde_json::to_vec(&data)?;
    proxmox_sys::fs::replace_file(CONF_FILE, &json, options, true)
}

/// Verifies that an entry for given tokenid / API token secret exists
pub fn verify_secret(tokenid: &Authid, secret: &str) -> Result<(), Error> {
    if !tokenid.is_token() {
        bail!("not an API token ID");
    }

    let data = read_file()?;
    match data.get(tokenid) {
        Some(hashed_secret) => proxmox_sys::crypt::verify_crypt_pw(secret, hashed_secret),
        None => bail!("invalid API token"),
    }
}

/// Adds a new entry for the given tokenid / API token secret. The secret is stored as salted hash.
pub fn set_secret(tokenid: &Authid, secret: &str) -> Result<(), Error> {
    if !tokenid.is_token() {
        bail!("not an API token ID");
    }

    let _guard = lock_config()?;

    let mut data = read_file()?;
    let hashed_secret = proxmox_sys::crypt::encrypt_pw(secret)?;
    data.insert(tokenid.clone(), hashed_secret);
    write_file(data)?;

    Ok(())
}

/// Deletes the entry for the given tokenid.
pub fn delete_secret(tokenid: &Authid) -> Result<(), Error> {
    if !tokenid.is_token() {
        bail!("not an API token ID");
    }

    let _guard = lock_config()?;

    let mut data = read_file()?;
    data.remove(tokenid);
    write_file(data)?;

    Ok(())
}
