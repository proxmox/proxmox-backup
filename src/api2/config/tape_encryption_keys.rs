use anyhow::{bail, Error};
use serde_json::Value;

use proxmox::{
    api::{
        api,
        ApiMethod,
        Router,
        RpcEnvironment,
    },
    tools::fs::open_file_locked,
};

use crate::{
    config::{
        tape_encryption_keys::{
            TAPE_KEYS_LOCKFILE,
            EncryptionKeyInfo,
            load_keys,
            save_keys,
        },
    },
    api2::types::{
        TAPE_ENCRYPTION_KEY_FINGERPRINT_SCHEMA,
        PROXMOX_CONFIG_DIGEST_SCHEMA,
        TapeKeyMetadata,
    },
    backup::Fingerprint,
    tools::format::as_fingerprint,
};

#[api(
    protected: true,
    input: {
        properties: {},
    },
    returns: {
        description: "The list of tape encryption keys (with config digest).",
        type: Array,
        items: { type: TapeKeyMetadata },
    },
)]
/// List existing keys
pub fn list_keys(
    _param: Value,
    _info: &ApiMethod,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<TapeKeyMetadata>, Error> {

    let (key_map, digest) = load_keys()?;

    let mut list = Vec::new();
    
    for (_fingerprint, item) in key_map {
        list.push(TapeKeyMetadata {
            hint: item.hint,
            fingerprint: as_fingerprint(item.fingerprint.bytes()),
        });
    }
    
    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(list)
}
#[api(
    protected: true,
    input: {
        properties: {
            password: {
                description: "A secret password.",
                min_length: 5,
            },
            hint: {
                description: "Password restore hint",
                min_length: 1,
            },
        },
    },
)]
/// Create a new encryption key
pub fn create_key(
    password: String,
    hint: String,
    _rpcenv: &mut dyn RpcEnvironment
) -> Result<Fingerprint, Error> {

    let key = openssl::sha::sha256(password.as_bytes()); // fixme: better KDF ??

    let item = EncryptionKeyInfo::new(&key, hint);

    let _lock = open_file_locked(
        TAPE_KEYS_LOCKFILE,
        std::time::Duration::new(10, 0),
        true,
    )?;

    let (mut key_map, _) = load_keys()?;

    let fingerprint = item.fingerprint.clone();

    if let Some(_) = key_map.get(&fingerprint) {
        bail!("encryption key '{}' already exists.", fingerprint);
    }

    key_map.insert(fingerprint.clone(), item);
    save_keys(key_map)?;

    Ok(fingerprint)
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
)]
/// Remove a encryption key from the database
///
/// Please note that you can no longer access tapes using this key.
pub fn delete_key(
    fingerprint: Fingerprint,
    digest: Option<String>,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
 
    let _lock = open_file_locked(
        TAPE_KEYS_LOCKFILE,
        std::time::Duration::new(10, 0),
        true,
    )?;

    let (mut key_map, expected_digest) = load_keys()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match key_map.get(&fingerprint) {
        Some(_) => { key_map.remove(&fingerprint); },
        None => bail!("tape encryption key '{}' does not exist.", fingerprint),
    }

    save_keys(key_map)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    //.get(&API_METHOD_READ_KEY_METADATA)
    //.put(&API_METHOD_UPDATE_KEY_METADATA)
    .delete(&API_METHOD_DELETE_KEY);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_KEYS)
    .post(&API_METHOD_CREATE_KEY)
    .match_all("fingerprint", &ITEM_ROUTER);
