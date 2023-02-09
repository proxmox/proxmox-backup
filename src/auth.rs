//! Proxmox Backup Server Authentication
//!
//! This library contains helper to authenticate users.

use std::io::Write;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::{Command, Stdio};

use anyhow::{bail, format_err, Error};
use futures::Future;
use proxmox_router::http_bail;
use serde_json::json;

use pbs_api_types::{LdapMode, LdapRealmConfig, OpenIdRealmConfig, RealmRef, Userid, UsernameRef};
use pbs_buildcfg::configdir;

use crate::auth_helpers;
use proxmox_ldap::{Config, Connection, ConnectionMode};

pub trait ProxmoxAuthenticator {
    fn authenticate_user<'a>(
        &'a self,
        username: &'a UsernameRef,
        password: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;
    fn store_password(&self, username: &UsernameRef, password: &str) -> Result<(), Error>;
    fn remove_password(&self, username: &UsernameRef) -> Result<(), Error>;
}

struct PamAuthenticator();

impl ProxmoxAuthenticator for PamAuthenticator {
    fn authenticate_user<'a>(
        &self,
        username: &'a UsernameRef,
        password: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            let mut auth = pam::Authenticator::with_password("proxmox-backup-auth").unwrap();
            auth.get_handler()
                .set_credentials(username.as_str(), password);
            auth.authenticate()?;
            Ok(())
        })
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
        http_bail!(
            NOT_IMPLEMENTED,
            "removing passwords is not implemented for PAM realms"
        );
    }
}

struct PbsAuthenticator();

const SHADOW_CONFIG_FILENAME: &str = configdir!("/shadow.json");

impl ProxmoxAuthenticator for PbsAuthenticator {
    fn authenticate_user<'a>(
        &self,
        username: &'a UsernameRef,
        password: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            let data = proxmox_sys::fs::file_get_json(SHADOW_CONFIG_FILENAME, Some(json!({})))?;
            match data[username.as_str()].as_str() {
                None => bail!("no password set"),
                Some(enc_password) => proxmox_sys::crypt::verify_crypt_pw(password, enc_password)?,
            }
            Ok(())
        })
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

struct OpenIdAuthenticator();
/// When a user is manually added, the lookup_authenticator is called to verify that
/// the realm exists. Thus, it is necessary to have an (empty) implementation for
/// OpendID as well.
impl ProxmoxAuthenticator for OpenIdAuthenticator {
    fn authenticate_user<'a>(
        &'a self,
        _username: &'a UsernameRef,
        _password: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            http_bail!(
                NOT_IMPLEMENTED,
                "password authentication is not implemented for OpenID realms"
            );
        })
    }

    fn store_password(&self, _username: &UsernameRef, _password: &str) -> Result<(), Error> {
        http_bail!(
            NOT_IMPLEMENTED,
            "storing passwords is not implemented for OpenID realms"
        );
    }

    fn remove_password(&self, _username: &UsernameRef) -> Result<(), Error> {
        http_bail!(
            NOT_IMPLEMENTED,
            "storing passwords is not implemented for OpenID realms"
        );
    }
}

#[allow(clippy::upper_case_acronyms)]
pub struct LdapAuthenticator {
    config: LdapRealmConfig,
}

impl ProxmoxAuthenticator for LdapAuthenticator {
    /// Authenticate user in LDAP realm
    fn authenticate_user<'a>(
        &'a self,
        username: &'a UsernameRef,
        password: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            let ldap_config = Self::api_type_to_config(&self.config)?;
            let ldap = Connection::new(ldap_config);
            ldap.authenticate_user(username.as_str(), password).await?;
            Ok(())
        })
    }

    fn store_password(&self, _username: &UsernameRef, _password: &str) -> Result<(), Error> {
        http_bail!(
            NOT_IMPLEMENTED,
            "storing passwords is not implemented for LDAP realms"
        );
    }

    fn remove_password(&self, _username: &UsernameRef) -> Result<(), Error> {
        http_bail!(
            NOT_IMPLEMENTED,
            "removing passwords is not implemented for LDAP realms"
        );
    }
}

impl LdapAuthenticator {
    pub fn api_type_to_config(config: &LdapRealmConfig) -> Result<Config, Error> {
        let mut servers = vec![config.server1.clone()];
        if let Some(server) = &config.server2 {
            servers.push(server.clone());
        }

        let tls_mode = match config.mode.unwrap_or_default() {
            LdapMode::Ldap => ConnectionMode::Ldap,
            LdapMode::StartTls => ConnectionMode::StartTls,
            LdapMode::Ldaps => ConnectionMode::Ldaps,
        };

        let (ca_store, trusted_cert) = if let Some(capath) = config.capath.as_deref() {
            let path = PathBuf::from(capath);
            if path.is_dir() {
                (Some(path), None)
            } else {
                (None, Some(vec![path]))
            }
        } else {
            (None, None)
        };

        Ok(Config {
            servers,
            port: config.port,
            user_attr: config.user_attr.clone(),
            base_dn: config.base_dn.clone(),
            bind_dn: config.bind_dn.clone(),
            bind_password: auth_helpers::get_ldap_bind_password(&config.realm)?,
            tls_mode,
            verify_certificate: config.verify.unwrap_or_default(),
            additional_trusted_certificates: trusted_cert,
            certificate_store_path: ca_store,
        })
    }
}

/// Lookup the autenticator for the specified realm
pub fn lookup_authenticator(
    realm: &RealmRef,
) -> Result<Box<dyn ProxmoxAuthenticator + Send + Sync + 'static>, Error> {
    match realm.as_str() {
        "pam" => Ok(Box::new(PamAuthenticator())),
        "pbs" => Ok(Box::new(PbsAuthenticator())),
        realm => {
            let (domains, _digest) = pbs_config::domains::config()?;
            if let Ok(config) = domains.lookup::<LdapRealmConfig>("ldap", realm) {
                Ok(Box::new(LdapAuthenticator { config }))
            } else if domains.lookup::<OpenIdRealmConfig>("openid", realm).is_ok() {
                Ok(Box::new(OpenIdAuthenticator()))
            } else {
                bail!("unknown realm '{}'", realm);
            }
        }
    }
}

/// Authenticate users
pub fn authenticate_user<'a>(
    userid: &'a Userid,
    password: &'a str,
) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
    Box::pin(async move {
        lookup_authenticator(userid.realm())?
            .authenticate_user(userid.name(), password)
            .await?;
        Ok(())
    })
}
