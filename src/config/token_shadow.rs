use std::collections::HashMap;
use std::time::Duration;

use anyhow::{bail, format_err, Error};
use serde::{Serialize, Deserialize};
use serde_json::{from_value, Value};

use proxmox::tools::fs::{open_file_locked, CreateOptions};

use crate::api2::types::Authid;
use crate::auth;

const LOCK_FILE: &str = pbs_buildcfg::configdir!("/token.shadow.lock");
const CONF_FILE: &str = pbs_buildcfg::configdir!("/token.shadow");
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
/// ApiToken id / secret pair
pub struct ApiTokenSecret {
    pub tokenid: Authid,
    pub secret: String,
}

fn read_file() -> Result<HashMap<Authid, String>, Error> {
    let json = proxmox::tools::fs::file_get_json(CONF_FILE, Some(Value::Null))?;

    if json == Value::Null {
        Ok(HashMap::new())
    } else {
        // swallow serde error which might contain sensitive data
        from_value(json).map_err(|_err| format_err!("unable to parse '{}'", CONF_FILE))
    }
}

fn write_file(data: HashMap<Authid, String>) -> Result<(), Error> {
    let backup_user = crate::backup::backup_user()?;
    let options = CreateOptions::new()
        .perm(nix::sys::stat::Mode::from_bits_truncate(0o0640))
        .owner(backup_user.uid)
        .group(backup_user.gid);

    let json = serde_json::to_vec(&data)?;
    proxmox::tools::fs::replace_file(CONF_FILE, &json, options)
}

/// Verifies that an entry for given tokenid / API token secret exists
pub fn verify_secret(tokenid: &Authid, secret: &str) -> Result<(), Error> {
    if !tokenid.is_token() {
        bail!("not an API token ID");
    }

    let data = read_file()?;
    match data.get(tokenid) {
        Some(hashed_secret) => {
            auth::verify_crypt_pw(secret, &hashed_secret)
        },
        None => bail!("invalid API token"),
    }
}

/// Adds a new entry for the given tokenid / API token secret. The secret is stored as salted hash.
pub fn set_secret(tokenid: &Authid, secret: &str) -> Result<(), Error> {
    if !tokenid.is_token() {
        bail!("not an API token ID");
    }

    let _guard = open_file_locked(LOCK_FILE, LOCK_TIMEOUT, true)?;

    let mut data = read_file()?;
    let hashed_secret = auth::encrypt_pw(secret)?;
    data.insert(tokenid.clone(), hashed_secret);
    write_file(data)?;

    Ok(())
}

/// Deletes the entry for the given tokenid.
pub fn delete_secret(tokenid: &Authid) -> Result<(), Error> {
    if !tokenid.is_token() {
        bail!("not an API token ID");
    }

    let _guard = open_file_locked(LOCK_FILE, LOCK_TIMEOUT, true)?;

    let mut data = read_file()?;
    data.remove(tokenid);
    write_file(data)?;

    Ok(())
}
