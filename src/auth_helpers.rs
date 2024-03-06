use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::Error;

use pbs_config::BackupLockGuard;
use proxmox_auth_api::{HMACKey, PrivateKey, PublicKey};
use proxmox_sys::fs::{file_get_contents, replace_file, CreateOptions};

use pbs_buildcfg::configdir;
use serde_json::json;

pub use crate::auth::setup_auth_context;
pub use proxmox_auth_api::api::assemble_csrf_prevention_token;

pub fn generate_csrf_key() -> Result<(), Error> {
    let path = PathBuf::from(configdir!("/csrf.key"));

    if path.exists() {
        return Ok(());
    }

    let key = HMACKey::generate()?.to_base64()?;

    use nix::sys::stat::Mode;
    let backup_user = pbs_config::backup_user()?;

    replace_file(
        &path,
        key.as_bytes(),
        CreateOptions::new()
            .perm(Mode::from_bits_truncate(0o0640))
            .owner(nix::unistd::ROOT)
            .group(backup_user.gid),
        true,
    )?;

    Ok(())
}

pub fn generate_auth_key() -> Result<(), Error> {
    let priv_path = PathBuf::from(configdir!("/authkey.key"));

    let mut public_path = priv_path.clone();
    public_path.set_extension("pub");

    if priv_path.exists() && public_path.exists() {
        return Ok(());
    }

    let key = proxmox_auth_api::PrivateKey::generate_ec()?;

    use nix::sys::stat::Mode;

    replace_file(
        &priv_path,
        &key.private_key_to_pem()?,
        CreateOptions::new().perm(Mode::from_bits_truncate(0o0600)),
        true,
    )?;

    let backup_user = pbs_config::backup_user()?;

    replace_file(
        &public_path,
        &key.public_key_to_pem()?,
        CreateOptions::new()
            .perm(Mode::from_bits_truncate(0o0640))
            .owner(nix::unistd::ROOT)
            .group(backup_user.gid),
        true,
    )?;

    Ok(())
}

pub fn csrf_secret() -> &'static HMACKey {
    static SECRET: OnceLock<HMACKey> = OnceLock::new();

    SECRET.get_or_init(|| {
        let bytes = file_get_contents(configdir!("/csrf.key")).unwrap();
        std::str::from_utf8(&bytes)
            .map_err(anyhow::Error::new)
            .and_then(HMACKey::from_base64)
            // legacy fall back to load legacy csrf secrets
            // TODO: remove once we move away from legacy token verification
            .unwrap_or_else(|_| {
                let key_as_b64 = base64::encode_config(bytes, base64::STANDARD_NO_PAD);
                HMACKey::from_base64(&key_as_b64).unwrap()
            })
    })
}

pub fn public_auth_key() -> &'static PublicKey {
    static KEY: OnceLock<PublicKey> = OnceLock::new();

    KEY.get_or_init(|| {
        let pem = file_get_contents(configdir!("/authkey.pub")).unwrap();
        PublicKey::from_pem(&pem).unwrap()
    })
}

pub fn private_auth_key() -> &'static PrivateKey {
    static KEY: OnceLock<PrivateKey> = OnceLock::new();

    KEY.get_or_init(|| {
        let pem = file_get_contents(configdir!("/authkey.key")).unwrap();
        PrivateKey::from_pem(&pem).unwrap()
    })
}

const LDAP_PASSWORDS_FILENAME: &str = configdir!("/ldap_passwords.json");

/// Store LDAP bind passwords in protected file. The domain config must be locked while this
/// function is executed.
pub fn store_ldap_bind_password(
    realm: &str,
    password: &str,
    _domain_lock: &BackupLockGuard,
) -> Result<(), Error> {
    let mut data = proxmox_sys::fs::file_get_json(LDAP_PASSWORDS_FILENAME, Some(json!({})))?;
    data[realm] = password.into();

    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);
    let options = proxmox_sys::fs::CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(nix::unistd::Gid::from_raw(0));

    let data = serde_json::to_vec_pretty(&data)?;
    proxmox_sys::fs::replace_file(LDAP_PASSWORDS_FILENAME, &data, options, true)?;

    Ok(())
}

/// Remove stored LDAP bind password. The domain config must be locked while this
/// function is executed.
pub fn remove_ldap_bind_password(realm: &str, _domain_lock: &BackupLockGuard) -> Result<(), Error> {
    let mut data = proxmox_sys::fs::file_get_json(LDAP_PASSWORDS_FILENAME, Some(json!({})))?;
    if let Some(map) = data.as_object_mut() {
        map.remove(realm);
    }

    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);
    let options = proxmox_sys::fs::CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(nix::unistd::Gid::from_raw(0));

    let data = serde_json::to_vec_pretty(&data)?;
    proxmox_sys::fs::replace_file(LDAP_PASSWORDS_FILENAME, &data, options, true)?;

    Ok(())
}

/// Retrieve stored LDAP bind password
pub fn get_ldap_bind_password(realm: &str) -> Result<Option<String>, Error> {
    let data = proxmox_sys::fs::file_get_json(LDAP_PASSWORDS_FILENAME, Some(json!({})))?;

    let password = data
        .get(realm)
        .and_then(|s| s.as_str())
        .map(|s| s.to_owned());

    Ok(password)
}
