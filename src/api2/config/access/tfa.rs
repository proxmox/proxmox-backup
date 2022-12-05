//! For now this only has the TFA subdir, which is in this file.
//! If we add more, it should be moved into a sub module.

use anyhow::{format_err, Error};
use hex::FromHex;
use serde::{Deserialize, Serialize};

use proxmox_router::list_subdirs_api_method;
use proxmox_router::{Permission, Router, RpcEnvironment, SubdirMap};
use proxmox_schema::api;

use pbs_api_types::PROXMOX_CONFIG_DIGEST_SCHEMA;

use crate::config::tfa::{self, WebauthnConfig, WebauthnConfigUpdater};

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);

const SUBDIRS: SubdirMap = &[("webauthn", &WEBAUTHN_ROUTER)];

const WEBAUTHN_ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_WEBAUTHN_CONFIG)
    .put(&API_METHOD_UPDATE_WEBAUTHN_CONFIG);

#[api(
    protected: true,
    input: {
        properties: {},
    },
    returns: {
        type: WebauthnConfig,
        optional: true,
    },
    access: {
        permission: &Permission::Anybody,
    },
)]
/// Get the TFA configuration.
pub fn get_webauthn_config(
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Option<WebauthnConfig>, Error> {
    let (config, digest) = match tfa::webauthn_config()? {
        Some(c) => c,
        None => return Ok(None),
    };
    rpcenv["digest"] = hex::encode(digest).into();
    Ok(Some(config))
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the `origin` property.
    Origin,

    /// Delete the `allow_subdomains` property.
    AllowSubdomains,
}

#[api(
    protected: true,
    input: {
        properties: {
            webauthn: {
                flatten: true,
                type: WebauthnConfigUpdater,
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
)]
/// Update the TFA configuration.
pub fn update_webauthn_config(
    webauthn: WebauthnConfigUpdater,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
) -> Result<(), Error> {
    let _lock = tfa::write_lock();

    let mut tfa = tfa::read()?;

    if let Some(wa) = &mut tfa.webauthn {
        if let Some(ref digest) = digest {
            let digest = <[u8; 32]>::from_hex(digest)?;
            crate::tools::detect_modified_configuration_file(
                &digest,
                &crate::config::tfa::webauthn_config_digest(wa)?,
            )?;
        }

        if let Some(delete) = delete {
            for delete in delete {
                match delete {
                    DeletableProperty::Origin => {
                        wa.origin = None;
                    }
                    DeletableProperty::AllowSubdomains => {
                        wa.allow_subdomains = None;
                    }
                }
            }
        }

        if let Some(rp) = webauthn.rp {
            wa.rp = rp;
        }
        if webauthn.origin.is_some() {
            wa.origin = webauthn.origin;
        }
        if webauthn.allow_subdomains.is_some() {
            wa.allow_subdomains = webauthn.allow_subdomains;
        }
        if let Some(id) = webauthn.id {
            wa.id = id;
        }
    } else {
        let rp = webauthn
            .rp
            .ok_or_else(|| format_err!("missing property: 'rp'"))?;
        let origin = webauthn.origin;
        let id = webauthn
            .id
            .ok_or_else(|| format_err!("missing property: 'id'"))?;
        let allow_subdomains = webauthn.allow_subdomains;
        tfa.webauthn = Some(WebauthnConfig {
            rp,
            origin,
            id,
            allow_subdomains,
        });
    }

    tfa::write(&tfa)?;

    Ok(())
}
