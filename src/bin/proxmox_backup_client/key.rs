use std::path::PathBuf;

use anyhow::{bail, format_err, Error};
use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};

use proxmox::api::api;
use proxmox::api::cli::{CliCommand, CliCommandMap};
use proxmox::sys::linux::tty;
use proxmox::tools::fs::{file_get_contents, replace_file, CreateOptions};

use proxmox_backup::backup::{
    encrypt_key_with_passphrase, load_and_decrypt_key, store_key_config, KeyConfig,
};
use proxmox_backup::tools;

pub const DEFAULT_ENCRYPTION_KEY_FILE_NAME: &str = "encryption-key.json";
pub const MASTER_PUBKEY_FILE_NAME: &str = "master-public.pem";

pub fn find_master_pubkey() -> Result<Option<PathBuf>, Error> {
    super::find_xdg_file(MASTER_PUBKEY_FILE_NAME, "main public key file")
}

pub fn place_master_pubkey() -> Result<PathBuf, Error> {
    super::place_xdg_file(MASTER_PUBKEY_FILE_NAME, "main public key file")
}

pub fn find_default_encryption_key() -> Result<Option<PathBuf>, Error> {
    super::find_xdg_file(DEFAULT_ENCRYPTION_KEY_FILE_NAME, "default encryption key file")
}

pub fn place_default_encryption_key() -> Result<PathBuf, Error> {
    super::place_xdg_file(DEFAULT_ENCRYPTION_KEY_FILE_NAME, "default encryption key file")
}

pub fn read_optional_default_encryption_key() -> Result<Option<Vec<u8>>, Error> {
    find_default_encryption_key()?
        .map(file_get_contents)
        .transpose()
}

pub fn get_encryption_key_password() -> Result<Vec<u8>, Error> {
    // fixme: implement other input methods

    use std::env::VarError::*;
    match std::env::var("PBS_ENCRYPTION_PASSWORD") {
        Ok(p) => return Ok(p.as_bytes().to_vec()),
        Err(NotUnicode(_)) => bail!("PBS_ENCRYPTION_PASSWORD contains bad characters"),
        Err(NotPresent) => {
            // Try another method
        }
    }

    // If we're on a TTY, query the user for a password
    if tty::stdin_isatty() {
        return Ok(tty::read_password("Encryption Key Password: ")?);
    }

    bail!("no password input mechanism available");
}

#[api(
    default: "scrypt",
)]
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
/// Key derivation function for password protected encryption keys.
pub enum Kdf {
    /// Do not encrypt the key.
    None,

    /// Encrypt they key with a password using SCrypt.
    Scrypt,
}

impl Default for Kdf {
    #[inline]
    fn default() -> Self {
        Kdf::Scrypt
    }
}

#[api(
    input: {
        properties: {
            kdf: {
                type: Kdf,
                optional: true,
            },
            path: {
                description:
                    "Output file. Without this the key will become the new default encryption key.",
                optional: true,
            }
        },
    },
)]
/// Create a new encryption key.
fn create(kdf: Option<Kdf>, path: Option<String>) -> Result<(), Error> {
    let path = match path {
        Some(path) => PathBuf::from(path),
        None => {
            let path = place_default_encryption_key()?;
            println!("creating default key at: {:?}", path);
            path
        }
    };

    let kdf = kdf.unwrap_or_default();

    let key = proxmox::sys::linux::random_data(32)?;

    match kdf {
        Kdf::None => {
            let created = Local.timestamp(Local::now().timestamp(), 0);

            store_key_config(
                &path,
                false,
                KeyConfig {
                    kdf: None,
                    created,
                    modified: created,
                    data: key,
                },
            )?;
        }
        Kdf::Scrypt => {
            // always read passphrase from tty
            if !tty::stdin_isatty() {
                bail!("unable to read passphrase - no tty");
            }

            let password = tty::read_and_verify_password("Encryption Key Password: ")?;

            let key_config = encrypt_key_with_passphrase(&key, &password)?;

            store_key_config(&path, false, key_config)?;
        }
    }

    Ok(())
}

#[api(
    input: {
        properties: {
            kdf: {
                type: Kdf,
                optional: true,
            },
            path: {
                description: "Key file. Without this the default key's password will be changed.",
                optional: true,
            }
        },
    },
)]
/// Change the encryption key's password.
fn change_passphrase(kdf: Option<Kdf>, path: Option<String>) -> Result<(), Error> {
    let path = match path {
        Some(path) => PathBuf::from(path),
        None => {
            let path = find_default_encryption_key()?
                .ok_or_else(|| {
                    format_err!("no encryption file provided and no default file found")
                })?;
            println!("updating default key at: {:?}", path);
            path
        }
    };

    let kdf = kdf.unwrap_or_default();

    if !tty::stdin_isatty() {
        bail!("unable to change passphrase - no tty");
    }

    let (key, created) = load_and_decrypt_key(&path, &get_encryption_key_password)?;

    match kdf {
        Kdf::None => {
            let modified = Local.timestamp(Local::now().timestamp(), 0);

            store_key_config(
                &path,
                true,
                KeyConfig {
                    kdf: None,
                    created, // keep original value
                    modified,
                    data: key.to_vec(),
                },
            )?;
        }
        Kdf::Scrypt => {
            let password = tty::read_and_verify_password("New Password: ")?;

            let mut new_key_config = encrypt_key_with_passphrase(&key, &password)?;
            new_key_config.created = created; // keep original value

            store_key_config(&path, true, new_key_config)?;
        }
    }

    Ok(())
}

#[api(
    input: {
        properties: {
            path: {
                description: "Path to the PEM formatted RSA public key.",
            },
        },
    },
)]
/// Import an RSA public key used to put an encrypted version of the symmetric backup encryption
/// key onto the backup server along with each backup.
fn import_master_pubkey(path: String) -> Result<(), Error> {
    let pem_data = file_get_contents(&path)?;

    if let Err(err) = openssl::pkey::PKey::public_key_from_pem(&pem_data) {
        bail!("Unable to decode PEM data - {}", err);
    }

    let target_path = place_master_pubkey()?;

    replace_file(&target_path, &pem_data, CreateOptions::new())?;

    println!("Imported public master key to {:?}", target_path);

    Ok(())
}

#[api]
/// Create an RSA public/private key pair used to put an encrypted version of the symmetric backup
/// encryption key onto the backup server along with each backup.
fn create_master_key() -> Result<(), Error> {
    // we need a TTY to query the new password
    if !tty::stdin_isatty() {
        bail!("unable to create master key - no tty");
    }

    let rsa = openssl::rsa::Rsa::generate(4096)?;
    let pkey = openssl::pkey::PKey::from_rsa(rsa)?;

    let password = String::from_utf8(tty::read_and_verify_password("Master Key Password: ")?)?;

    let pub_key: Vec<u8> = pkey.public_key_to_pem()?;
    let filename_pub = "master-public.pem";
    println!("Writing public master key to {}", filename_pub);
    replace_file(filename_pub, pub_key.as_slice(), CreateOptions::new())?;

    let cipher = openssl::symm::Cipher::aes_256_cbc();
    let priv_key: Vec<u8> = pkey.private_key_to_pem_pkcs8_passphrase(cipher, password.as_bytes())?;

    let filename_priv = "master-private.pem";
    println!("Writing private master key to {}", filename_priv);
    replace_file(filename_priv, priv_key.as_slice(), CreateOptions::new())?;

    Ok(())
}

pub fn cli() -> CliCommandMap {
    let key_create_cmd_def = CliCommand::new(&API_METHOD_CREATE)
        .arg_param(&["path"])
        .completion_cb("path", tools::complete_file_name);

    let key_change_passphrase_cmd_def = CliCommand::new(&API_METHOD_CHANGE_PASSPHRASE)
        .arg_param(&["path"])
        .completion_cb("path", tools::complete_file_name);

    let key_create_master_key_cmd_def = CliCommand::new(&API_METHOD_CREATE_MASTER_KEY);
    let key_import_master_pubkey_cmd_def = CliCommand::new(&API_METHOD_IMPORT_MASTER_PUBKEY)
        .arg_param(&["path"])
        .completion_cb("path", tools::complete_file_name);

    CliCommandMap::new()
        .insert("create", key_create_cmd_def)
        .insert("create-master-key", key_create_master_key_cmd_def)
        .insert("import-master-pubkey", key_import_master_pubkey_cmd_def)
        .insert("change-passphrase", key_change_passphrase_cmd_def)
}
