//! For now this only has the TFA subdir, which is in this file.
//! If we add more, it should be moved into a sub module.

use anyhow::Error;

use crate::api2::types::PROXMOX_CONFIG_DIGEST_SCHEMA;
use proxmox::api::{api, Permission, Router, RpcEnvironment, SubdirMap};
use proxmox::list_subdirs_api_method;

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
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Option<WebauthnConfig>, Error> {
    let (config, digest) = match tfa::webauthn_config()? {
        Some(c) => c,
        None => return Ok(None),
    };
    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();
    Ok(Some(config))
}

#[api(
    protected: true,
    input: {
        properties: {
            webauthn: {
                flatten: true,
                type: WebauthnConfigUpdater,
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
    digest: Option<String>,
) -> Result<(), Error> {
    let _lock = tfa::write_lock();

    let mut tfa = tfa::read()?;

    if let Some(wa) = &mut tfa.webauthn {
        if let Some(ref digest) = digest {
            let digest = proxmox::tools::hex_to_digest(digest)?;
            crate::tools::detect_modified_configuration_file(&digest, &wa.digest()?)?;
        }
        if let Some(ref rp) = webauthn.rp { wa.rp = rp.clone(); }
        if let Some(ref origin) = webauthn.rp { wa.origin = origin.clone(); }
        if let Some(ref id) = webauthn.id { wa.id = id.clone(); }
    } else {
        let rp = webauthn.rp.unwrap();
        let origin = webauthn.origin.unwrap();
        let id = webauthn.id.unwrap();
        tfa.webauthn = Some(WebauthnConfig { rp, origin, id });
    }

    tfa::write(&tfa)?;

    Ok(())
}
