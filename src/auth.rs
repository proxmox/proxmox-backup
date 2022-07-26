//! Proxmox Backup Server Authentication
//!
//! This library contains helper to authenticate users.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{bail, format_err, Error};
use serde_json::json;

use pbs_api_types::{RealmRef, Userid, UsernameRef};
use pbs_buildcfg::configdir;

pub trait ProxmoxAuthenticator {
    fn authenticate_user(&self, username: &UsernameRef, password: &str) -> Result<(), Error>;
    fn store_password(&self, username: &UsernameRef, password: &str) -> Result<(), Error>;
    fn remove_password(&self, username: &UsernameRef) -> Result<(), Error>;
}

#[allow(clippy::upper_case_acronyms)]
struct PAM();

impl ProxmoxAuthenticator for PAM {
    fn authenticate_user(&self, username: &UsernameRef, password: &str) -> Result<(), Error> {
        let mut auth = pam::Authenticator::with_password("proxmox-backup-auth").unwrap();
        auth.get_handler()
            .set_credentials(username.as_str(), password);
        auth.authenticate()?;
        Ok(())
    }

    fn store_password(&self, username: &UsernameRef, password: &str) -> Result<(), Error> {
        let mut child = Command::new("passwd")
            .arg(username.as_str())
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| {
                format_err!(
                    "unable to set password for '{}' - execute passwd failed: {}",
                    username.as_str(),
                    err,
                )
            })?;

        // Note: passwd reads password twice from stdin (for verify)
        writeln!(child.stdin.as_mut().unwrap(), "{}\n{}", password, password)?;

        let output = child.wait_with_output().map_err(|err| {
            format_err!(
                "unable to set password for '{}' - wait failed: {}",
                username.as_str(),
                err,
            )
        })?;

        if !output.status.success() {
            bail!(
                "unable to set password for '{}' - {}",
                username.as_str(),
                String::from_utf8_lossy(&output.stderr),
            );
        }

        Ok(())
    }

    // do not remove password for pam users
    fn remove_password(&self, _username: &UsernameRef) -> Result<(), Error> {
        Ok(())
    }
}

#[allow(clippy::upper_case_acronyms)]
struct PBS();

const SHADOW_CONFIG_FILENAME: &str = configdir!("/shadow.json");

impl ProxmoxAuthenticator for PBS {
    fn authenticate_user(&self, username: &UsernameRef, password: &str) -> Result<(), Error> {
        let data = proxmox_sys::fs::file_get_json(SHADOW_CONFIG_FILENAME, Some(json!({})))?;
        match data[username.as_str()].as_str() {
            None => bail!("no password set"),
            Some(enc_password) => proxmox_sys::crypt::verify_crypt_pw(password, enc_password)?,
        }
        Ok(())
    }

    fn store_password(&self, username: &UsernameRef, password: &str) -> Result<(), Error> {
        let enc_password = proxmox_sys::crypt::encrypt_pw(password)?;
        let mut data = proxmox_sys::fs::file_get_json(SHADOW_CONFIG_FILENAME, Some(json!({})))?;
        data[username.as_str()] = enc_password.into();

        let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);
        let options = proxmox_sys::fs::CreateOptions::new()
            .perm(mode)
            .owner(nix::unistd::ROOT)
            .group(nix::unistd::Gid::from_raw(0));

        let data = serde_json::to_vec_pretty(&data)?;
        proxmox_sys::fs::replace_file(SHADOW_CONFIG_FILENAME, &data, options, true)?;

        Ok(())
    }

    fn remove_password(&self, username: &UsernameRef) -> Result<(), Error> {
        let mut data = proxmox_sys::fs::file_get_json(SHADOW_CONFIG_FILENAME, Some(json!({})))?;
        if let Some(map) = data.as_object_mut() {
            map.remove(username.as_str());
        }

        let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);
        let options = proxmox_sys::fs::CreateOptions::new()
            .perm(mode)
            .owner(nix::unistd::ROOT)
            .group(nix::unistd::Gid::from_raw(0));

        let data = serde_json::to_vec_pretty(&data)?;
        proxmox_sys::fs::replace_file(SHADOW_CONFIG_FILENAME, &data, options, true)?;

        Ok(())
    }
}

/// Lookup the autenticator for the specified realm
pub fn lookup_authenticator(realm: &RealmRef) -> Result<Box<dyn ProxmoxAuthenticator>, Error> {
    match realm.as_str() {
        "pam" => Ok(Box::new(PAM())),
        "pbs" => Ok(Box::new(PBS())),
        _ => bail!("unknown realm '{}'", realm.as_str()),
    }
}

/// Authenticate users
pub fn authenticate_user(userid: &Userid, password: &str) -> Result<(), Error> {
    lookup_authenticator(userid.realm())?.authenticate_user(userid.name(), password)
}
