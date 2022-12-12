use anyhow::{bail, format_err, Error};
use hex::FromHex;
use serde_json::Value;

use proxmox_router::{http_bail, ApiMethod, Permission, Router, RpcEnvironment};
use proxmox_schema::{api, param_bail};

use pbs_api_types::{
    Authid, Fingerprint, Kdf, KeyInfo, PASSWORD_HINT_SCHEMA, PRIV_TAPE_AUDIT, PRIV_TAPE_MODIFY,
    PROXMOX_CONFIG_DIGEST_SCHEMA, TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
};

use pbs_config::CachedUserInfo;

use pbs_config::open_backup_lockfile;
use pbs_key_config::KeyConfig;

use crate::tape::encryption_keys::{
    insert_key, load_key_configs, load_keys, save_key_configs, save_keys, TAPE_KEYS_LOCKFILE,
};

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "The list of tape encryption keys (with config digest).",
        type: Array,
        items: { type: KeyInfo },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "pool"], PRIV_TAPE_AUDIT, false),
    },
)]
/// List existing keys
pub fn list_keys(
    _param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<KeyInfo>, Error> {
    let (key_map, digest) = load_key_configs()?;

    let mut list = Vec::new();

    for (_fingerprint, item) in key_map.iter() {
        list.push(item.into());
    }

    rpcenv["digest"] = hex::encode(digest).into();

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            kdf: {
                type: Kdf,
                optional: true,
            },
            fingerprint: {
                schema: TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
            },
            password: {
                description: "The current password.",
                min_length: 5,
                optional: true,
            },
            "new-password": {
                description: "The new password.",
                min_length: 5,
            },
            hint: {
                schema: PASSWORD_HINT_SCHEMA,
            },
            force: {
                optional: true,
                type: bool,
                description: "Reset the passphrase for a tape key, using the root-only accessible copy.",
                default: false,
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "pool"], PRIV_TAPE_MODIFY, false),
    },
)]
/// Change the encryption key's password (and password hint).
#[allow(clippy::too_many_arguments)]
pub fn change_passphrase(
    kdf: Option<Kdf>,
    password: Option<String>,
    new_password: String,
    hint: String,
    force: bool,
    fingerprint: Fingerprint,
    digest: Option<String>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let kdf = kdf.unwrap_or_default();

    if let Kdf::None = kdf {
        param_bail!(
            "kdf",
            format_err!("Please specify a key derivation function (none is not allowed here).")
        );
    }

    let _lock = open_backup_lockfile(TAPE_KEYS_LOCKFILE, None, true)?;

    let (mut config_map, expected_digest) = load_key_configs()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let key_config = match config_map.get(&fingerprint) {
        Some(key_config) => key_config,
        None => http_bail!(
            NOT_FOUND,
            "tape encryption key configuration '{}' does not exist.",
            fingerprint
        ),
    };

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    if force && !user_info.is_superuser(&auth_id) {
        bail!("resetting the key's passphrase requires root privileges")
    }

    let (key, created, fingerprint) = match (force, &password) {
        (true, Some(_)) => param_bail!(
            "password",
            format_err!("password is not allowed when using force")
        ),
        (false, None) => param_bail!("password", format_err!("missing parameter: password")),
        (false, Some(pass)) => key_config.decrypt(&|| Ok(pass.as_bytes().to_vec()))?,
        (true, None) => {
            let key = load_keys()?
                .0
                .get(&fingerprint)
                .ok_or_else(|| {
                    format_err!(
                        "failed to reset passphrase, could not find key '{}'",
                        fingerprint
                    )
                })?
                .key;

            (key, key_config.created, fingerprint)
        }
    };

    let mut new_key_config = KeyConfig::with_key(&key, new_password.as_bytes(), kdf)?;
    new_key_config.created = created; // keep original value
    new_key_config.hint = Some(hint);

    config_map.insert(fingerprint, new_key_config);

    save_key_configs(config_map)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            kdf: {
                type: Kdf,
                optional: true,
            },
            password: {
                description: "A secret password.",
                min_length: 5,
            },
            hint: {
                schema: PASSWORD_HINT_SCHEMA,
                optional: true,
            },
            key: {
                description: "Restore/Re-create a key from this JSON string.",
                type: String,
                min_length: 300,
                max_length: 600,
                optional: true,
            },
        },
    },
    returns: {
        schema: TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "pool"], PRIV_TAPE_MODIFY, false),
    },
)]
/// Create a new encryption key
pub fn create_key(
    kdf: Option<Kdf>,
    password: String,
    hint: Option<String>,
    key: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Fingerprint, Error> {
    let kdf = kdf.unwrap_or_default();

    if key.is_none() {
        if let Kdf::None = kdf {
            param_bail!(
                "kdf",
                format_err!("Please specify a key derivation function (none is not allowed here).")
            );
        }
        if hint.is_none() {
            param_bail!("hint", format_err!("Please specify either a hint or a key"));
        }
    }

    let (key_decrypt, mut key_config) = match key {
        Some(key) => {
            let key_config: KeyConfig =
                serde_json::from_str(&key).map_err(|err| format_err!("<errmsg>: {}", err))?;
            let (key_decrypt, _created, _fp) =
                key_config.decrypt(&|| Ok(password.as_bytes().to_vec()))?;
            (key_decrypt, key_config)
        }
        None => KeyConfig::new(password.as_bytes(), kdf)?,
    };

    if hint.is_some() {
        key_config.hint = hint;
    }

    let fingerprint = key_config.fingerprint.clone().unwrap();
    insert_key(key_decrypt, key_config, false)?;

    Ok(fingerprint)
}

#[api(
    input: {
        properties: {
            fingerprint: {
                schema: TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
            },
        },
    },
    returns: {
        type: KeyInfo,
    },
    access: {
        permission: &Permission::Privilege(&["tape", "pool"], PRIV_TAPE_AUDIT, false),
    },
)]
/// Get key config (public key part)
pub fn read_key(
    fingerprint: Fingerprint,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<KeyInfo, Error> {
    let (config_map, _digest) = load_key_configs()?;

    let key_config = match config_map.get(&fingerprint) {
        Some(key_config) => key_config,
        None => http_bail!(
            NOT_FOUND,
            "tape encryption key '{}' does not exist.",
            fingerprint
        ),
    };

    if key_config.kdf.is_none() {
        bail!("found unencrypted key - internal error");
    }

    Ok(key_config.into())
}

#[api(
    protected: true,
    input: {
        properties: {
            fingerprint: {
                schema: TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["tape", "pool"], PRIV_TAPE_MODIFY, false),
    },
)]
/// Remove a encryption key from the database
///
/// Please note that you can no longer access tapes using this key.
pub fn delete_key(
    fingerprint: Fingerprint,
    digest: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let _lock = open_backup_lockfile(TAPE_KEYS_LOCKFILE, None, true)?;

    let (mut config_map, expected_digest) = load_key_configs()?;
    let (mut key_map, _) = load_keys()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config_map.get(&fingerprint) {
        Some(_) => {
            config_map.remove(&fingerprint);
        }
        None => http_bail!(
            NOT_FOUND,
            "tape encryption key '{}' does not exist.",
            fingerprint
        ),
    }
    save_key_configs(config_map)?;

    key_map.remove(&fingerprint);
    save_keys(key_map)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_KEY)
    .put(&API_METHOD_CHANGE_PASSPHRASE)
    .delete(&API_METHOD_DELETE_KEY);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_KEYS)
    .post(&API_METHOD_CREATE_KEY)
    .match_all("fingerprint", &ITEM_ROUTER);
