//! Proxmox Backup Server Authentication
//!
//! This library contains helper to authenticate users.

use std::net::IpAddr;
use std::path::PathBuf;
use std::pin::Pin;

use anyhow::{bail, Error};
use futures::Future;
use once_cell::sync::{Lazy, OnceCell};
use proxmox_router::http_bail;
use serde_json::json;

use proxmox_auth_api::api::{Authenticator, LockedTfaConfig};
use proxmox_auth_api::ticket::{Empty, Ticket};
use proxmox_auth_api::types::Authid;
use proxmox_auth_api::Keyring;
use proxmox_ldap::{Config, Connection, ConnectionMode};
use proxmox_tfa::api::{OpenUserChallengeData, TfaConfig};

use pbs_api_types::{LdapMode, LdapRealmConfig, OpenIdRealmConfig, RealmRef, Userid, UsernameRef};
use pbs_buildcfg::configdir;

use crate::auth_helpers;

pub const TERM_PREFIX: &str = "PBSTERM";

struct PbsAuthenticator;

const SHADOW_CONFIG_FILENAME: &str = configdir!("/shadow.json");

impl Authenticator for PbsAuthenticator {
    fn authenticate_user<'a>(
        &self,
        username: &'a UsernameRef,
        password: &'a str,
        _client_ip: Option<&'a IpAddr>,
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

    fn store_password(
        &self,
        username: &UsernameRef,
        password: &str,
        _client_ip: Option<&IpAddr>,
    ) -> Result<(), Error> {
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
impl Authenticator for OpenIdAuthenticator {
    fn authenticate_user<'a>(
        &'a self,
        _username: &'a UsernameRef,
        _password: &'a str,
        _client_ip: Option<&'a IpAddr>,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            http_bail!(
                NOT_IMPLEMENTED,
                "password authentication is not implemented for OpenID realms"
            );
        })
    }

    fn store_password(
        &self,
        _username: &UsernameRef,
        _password: &str,
        _client_ip: Option<&IpAddr>,
    ) -> Result<(), Error> {
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

impl Authenticator for LdapAuthenticator {
    /// Authenticate user in LDAP realm
    fn authenticate_user<'a>(
        &'a self,
        username: &'a UsernameRef,
        password: &'a str,
        _client_ip: Option<&'a IpAddr>,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            let ldap_config = Self::api_type_to_config(&self.config)?;
            let ldap = Connection::new(ldap_config);
            ldap.authenticate_user(username.as_str(), password).await?;
            Ok(())
        })
    }

    fn store_password(
        &self,
        _username: &UsernameRef,
        _password: &str,
        _client_ip: Option<&IpAddr>,
    ) -> Result<(), Error> {
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
        Self::api_type_to_config_with_password(
            config,
            auth_helpers::get_ldap_bind_password(&config.realm)?,
        )
    }

    pub fn api_type_to_config_with_password(
        config: &LdapRealmConfig,
        password: Option<String>,
    ) -> Result<Config, Error> {
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
            bind_password: password,
            tls_mode,
            verify_certificate: config.verify.unwrap_or_default(),
            additional_trusted_certificates: trusted_cert,
            certificate_store_path: ca_store,
        })
    }
}

/// Lookup the authenticator for the specified realm
pub(crate) fn lookup_authenticator(
    realm: &RealmRef,
) -> Result<Box<dyn Authenticator + Send + Sync>, Error> {
    match realm.as_str() {
        "pam" => Ok(Box::new(proxmox_auth_api::Pam::new("proxmox-backup-auth"))),
        "pbs" => Ok(Box::new(PbsAuthenticator)),
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
pub(crate) fn authenticate_user<'a>(
    userid: &'a Userid,
    password: &'a str,
    client_ip: Option<&'a IpAddr>,
) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
    Box::pin(async move {
        lookup_authenticator(userid.realm())?
            .authenticate_user(userid.name(), password, client_ip)
            .await?;
        Ok(())
    })
}

static PRIVATE_KEYRING: Lazy<Keyring> =
    Lazy::new(|| Keyring::with_private_key(crate::auth_helpers::private_auth_key().clone().into()));
static PUBLIC_KEYRING: Lazy<Keyring> =
    Lazy::new(|| Keyring::with_public_key(crate::auth_helpers::public_auth_key().clone().into()));
static AUTH_CONTEXT: OnceCell<PbsAuthContext> = OnceCell::new();

pub fn setup_auth_context(use_private_key: bool) {
    let keyring = if use_private_key {
        &*PRIVATE_KEYRING
    } else {
        &*PUBLIC_KEYRING
    };

    AUTH_CONTEXT
        .set(PbsAuthContext {
            keyring,
            csrf_secret: crate::auth_helpers::csrf_secret().to_vec(),
        })
        .map_err(drop)
        .expect("auth context setup twice");

    proxmox_auth_api::set_auth_context(AUTH_CONTEXT.get().unwrap());
}

pub(crate) fn private_auth_keyring() -> &'static Keyring {
    &PRIVATE_KEYRING
}

pub(crate) fn public_auth_keyring() -> &'static Keyring {
    &PUBLIC_KEYRING
}

struct PbsAuthContext {
    keyring: &'static Keyring,
    csrf_secret: Vec<u8>,
}

impl proxmox_auth_api::api::AuthContext for PbsAuthContext {
    fn lookup_realm(&self, realm: &RealmRef) -> Option<Box<dyn Authenticator + Send + Sync>> {
        lookup_authenticator(realm).ok()
    }

    /// Get the current authentication keyring.
    fn keyring(&self) -> &Keyring {
        self.keyring
    }

    /// The auth prefix without the separating colon. Eg. `"PBS"`.
    fn auth_prefix(&self) -> &'static str {
        "PBS"
    }

    /// API token prefix (without the `'='`).
    fn auth_token_prefix(&self) -> &'static str {
        "PBSAPIToken"
    }

    /// Auth cookie name.
    fn auth_cookie_name(&self) -> &'static str {
        "PBSAuthCookie"
    }

    /// Check if a userid is enabled and return a [`UserInformation`] handle.
    fn auth_id_is_active(&self, auth_id: &Authid) -> Result<bool, Error> {
        Ok(pbs_config::CachedUserInfo::new()?.is_active_auth_id(auth_id))
    }

    /// Access the TFA config with an exclusive lock.
    fn tfa_config_write_lock(&self) -> Result<Box<dyn LockedTfaConfig>, Error> {
        Ok(Box::new(PbsLockedTfaConfig {
            _lock: crate::config::tfa::read_lock()?,
            config: crate::config::tfa::read()?,
        }))
    }

    /// CSRF prevention token secret data.
    fn csrf_secret(&self) -> &[u8] {
        &self.csrf_secret
    }

    /// Verify a token secret.
    fn verify_token_secret(&self, token_id: &Authid, token_secret: &str) -> Result<(), Error> {
        pbs_config::token_shadow::verify_secret(token_id, token_secret)
    }

    /// Check path based tickets. (Used for terminal tickets).
    fn check_path_ticket(
        &self,
        userid: &Userid,
        password: &str,
        path: String,
        privs: String,
        port: u16,
    ) -> Result<Option<bool>, Error> {
        if !password.starts_with("PBSTERM:") {
            return Ok(None);
        }

        if let Ok(Empty) = Ticket::parse(password).and_then(|ticket| {
            ticket.verify(
                self.keyring,
                TERM_PREFIX,
                Some(&crate::tools::ticket::term_aad(userid, &path, port)),
            )
        }) {
            let user_info = pbs_config::CachedUserInfo::new()?;
            let auth_id = Authid::from(userid.clone());
            for (name, privilege) in pbs_api_types::PRIVILEGES {
                if *name == privs {
                    let mut path_vec = Vec::new();
                    for part in path.split('/') {
                        if !part.is_empty() {
                            path_vec.push(part);
                        }
                    }
                    user_info.check_privs(&auth_id, &path_vec, *privilege, false)?;
                    return Ok(Some(true));
                }
            }
        }

        Ok(Some(false))
    }
}

struct PbsLockedTfaConfig {
    _lock: pbs_config::BackupLockGuard,
    config: TfaConfig,
}

static USER_ACCESS: crate::config::tfa::UserAccess = crate::config::tfa::UserAccess;

impl LockedTfaConfig for PbsLockedTfaConfig {
    fn config_mut(&mut self) -> (&dyn OpenUserChallengeData, &mut TfaConfig) {
        (&USER_ACCESS, &mut self.config)
    }

    fn save_config(&mut self) -> Result<(), Error> {
        crate::config::tfa::write(&self.config)
    }
}
