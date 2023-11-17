use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{bail, format_err, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use proxmox_router::cli::{
    complete_file_name, format_and_print_result_full, get_output_format, CliCommand, CliCommandMap,
    ColumnConfig, OUTPUT_FORMAT,
};
use proxmox_schema::{api, ApiType, ReturnType};
use proxmox_sys::fs::{file_get_contents, replace_file, CreateOptions};
use proxmox_sys::linux::tty;

use pbs_api_types::{Kdf, KeyInfo, PASSWORD_HINT_SCHEMA};
use pbs_client::tools::key_source::{
    find_default_encryption_key, find_default_master_pubkey, get_encryption_key_password,
    place_default_encryption_key, place_default_master_pubkey,
};
use pbs_datastore::paperkey::{generate_paper_key, PaperkeyFormat};
use pbs_key_config::{rsa_decrypt_key_config, KeyConfig};

#[api]
#[derive(Deserialize, Serialize)]
/// RSA public key information
pub struct RsaPubKeyInfo {
    /// Path to key (if stored in a file)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// RSA exponent
    pub exponent: String,
    /// Hex-encoded RSA modulus
    pub modulus: String,
    /// Key (modulus) length in bits
    pub length: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl std::convert::TryFrom<openssl::rsa::Rsa<openssl::pkey::Public>> for RsaPubKeyInfo {
    type Error = anyhow::Error;

    fn try_from(value: openssl::rsa::Rsa<openssl::pkey::Public>) -> Result<Self, Self::Error> {
        let modulus = value.n().to_hex_str()?.to_string();
        let exponent = value.e().to_dec_str()?.to_string();
        let length = value.size() as usize * 8;

        Ok(Self {
            path: None,
            exponent,
            modulus,
            length,
        })
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
            },
            hint: {
                schema: PASSWORD_HINT_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Create a new encryption key.
fn create(kdf: Option<Kdf>, path: Option<String>, hint: Option<String>) -> Result<(), Error> {
    let path = match path {
        Some(path) => PathBuf::from(path),
        None => {
            let path = place_default_encryption_key()?;
            log::info!("creating default key at: {:?}", path);
            path
        }
    };

    let kdf = kdf.unwrap_or_default();

    let mut key = [0u8; 32];
    proxmox_sys::linux::fill_with_random_data(&mut key)?;

    match kdf {
        Kdf::None => {
            if hint.is_some() {
                bail!("password hint not allowed for Kdf::None");
            }

            let key_config = KeyConfig::without_password(key)?;

            key_config.store(path, false)?;
        }
        Kdf::Scrypt | Kdf::PBKDF2 => {
            // always read passphrase from tty
            if !std::io::stdin().is_terminal() {
                bail!("unable to read passphrase - no tty");
            }

            let password = tty::read_and_verify_password("Encryption Key Password: ")?;

            let mut key_config = KeyConfig::with_key(&key, &password, kdf)?;
            key_config.hint = hint;

            key_config.store(&path, false)?;
        }
    }

    Ok(())
}

#[api(
    input: {
        properties: {
            "master-keyfile": {
                description: "(Private) master key to use.",
            },
            "encrypted-keyfile": {
                description: "RSA-encrypted keyfile to import.",
            },
            kdf: {
                type: Kdf,
                optional: true,
            },
            "path": {
                description:
                    "Output file. Without this the key will become the new default encryption key.",
                optional: true,
            },
            hint: {
                schema: PASSWORD_HINT_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Import an encrypted backup of an encryption key using a (private) master key.
async fn import_with_master_key(
    master_keyfile: String,
    encrypted_keyfile: String,
    kdf: Option<Kdf>,
    path: Option<String>,
    hint: Option<String>,
) -> Result<(), Error> {
    let path = match path {
        Some(path) => PathBuf::from(path),
        None => {
            let path = place_default_encryption_key()?;
            if path.exists() {
                bail!("Please remove default encryption key at {:?} before importing to default location (or choose a non-default one).", path);
            }
            log::info!("Importing key to default location at: {:?}", path);
            path
        }
    };

    let encrypted_key = file_get_contents(encrypted_keyfile)?;
    let master_key = file_get_contents(master_keyfile)?;
    let password = tty::read_password("Master Key Password: ")?;

    let master_key = openssl::pkey::PKey::private_key_from_pem_passphrase(&master_key, &password)
        .map_err(|err| format_err!("failed to read PEM-formatted private key - {}", err))?
        .rsa()
        .map_err(|err| format_err!("not a valid private RSA key - {}", err))?;

    let (key, created, _fingerprint) =
        rsa_decrypt_key_config(master_key, &encrypted_key, &get_encryption_key_password)?;

    let kdf = kdf.unwrap_or_default();
    match kdf {
        Kdf::None => {
            if hint.is_some() {
                bail!("password hint not allowed for Kdf::None");
            }

            let mut key_config = KeyConfig::without_password(key)?;
            key_config.created = created; // keep original value

            key_config.store(path, true)?;
        }
        Kdf::Scrypt | Kdf::PBKDF2 => {
            let password = tty::read_and_verify_password("New Password: ")?;

            let mut new_key_config = KeyConfig::with_key(&key, &password, kdf)?;
            new_key_config.created = created; // keep original value
            new_key_config.hint = hint;

            new_key_config.store(path, true)?;
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
            },
            hint: {
                schema: PASSWORD_HINT_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Change the encryption key's password.
fn change_passphrase(
    kdf: Option<Kdf>,
    path: Option<String>,
    hint: Option<String>,
) -> Result<(), Error> {
    let path = match path {
        Some(path) => PathBuf::from(path),
        None => {
            let path = find_default_encryption_key()?.ok_or_else(|| {
                format_err!("no encryption file provided and no default file found")
            })?;
            log::info!("updating default key at: {:?}", path);
            path
        }
    };

    let kdf = kdf.unwrap_or_default();

    if !std::io::stdin().is_terminal() {
        bail!("unable to change passphrase - no tty");
    }

    let key_config = KeyConfig::load(&path)?;
    let (key, created, _fingerprint) = key_config.decrypt(&get_encryption_key_password)?;

    match kdf {
        Kdf::None => {
            if hint.is_some() {
                bail!("password hint not allowed for Kdf::None");
            }

            let mut key_config = KeyConfig::without_password(key)?;
            key_config.created = created; // keep original value

            key_config.store(&path, true)?;
        }
        Kdf::Scrypt | Kdf::PBKDF2 => {
            let password = tty::read_and_verify_password("New Password: ")?;

            let mut new_key_config = KeyConfig::with_key(&key, &password, kdf)?;
            new_key_config.created = created; // keep original value
            new_key_config.hint = hint;

            new_key_config.store(&path, true)?;
        }
    }

    Ok(())
}

#[api(
    input: {
        properties: {
            path: {
                description: "Key file. Without this the default key's metadata will be shown.",
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Print the encryption key's metadata.
fn show_key(path: Option<String>, param: Value) -> Result<(), Error> {
    let path = match path {
        Some(path) => PathBuf::from(path),
        None => find_default_encryption_key()?
            .ok_or_else(|| format_err!("no encryption file provided and no default file found"))?,
    };

    let config: KeyConfig = serde_json::from_slice(&file_get_contents(&path)?)?;

    let output_format = get_output_format(&param);

    let mut info: KeyInfo = (&config).into();
    info.path = Some(format!("{:?}", path));

    let options = proxmox_router::cli::default_table_format_options()
        .column(ColumnConfig::new("path"))
        .column(ColumnConfig::new("kdf"))
        .column(ColumnConfig::new("created").renderer(pbs_tools::format::render_epoch))
        .column(ColumnConfig::new("modified").renderer(pbs_tools::format::render_epoch))
        .column(ColumnConfig::new("fingerprint"))
        .column(ColumnConfig::new("hint"));

    let return_type = ReturnType::new(false, &KeyInfo::API_SCHEMA);

    format_and_print_result_full(
        &mut serde_json::to_value(info)?,
        &return_type,
        &output_format,
        &options,
    );

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
///
/// The imported key will be used as default master key for future invocations by the same local
/// user.
fn import_master_pubkey(path: String) -> Result<(), Error> {
    let pem_data = file_get_contents(&path)?;

    match openssl::pkey::PKey::public_key_from_pem(&pem_data) {
        Ok(key) => {
            let info = RsaPubKeyInfo::try_from(key.rsa()?)?;
            log::info!("Found following key at {:?}", path);
            log::info!("Modulus: {}", info.modulus);
            log::info!("Exponent: {}", info.exponent);
            log::info!("Length: {}", info.length);
        }
        Err(err) => bail!("Unable to decode PEM data - {}", err),
    };

    let target_path = place_default_master_pubkey()?;

    replace_file(&target_path, &pem_data, CreateOptions::new(), true)?;

    log::info!("Imported public master key to {:?}", target_path);

    Ok(())
}

#[api]
/// Create an RSA public/private key pair used to put an encrypted version of the symmetric backup
/// encryption key onto the backup server along with each backup.
fn create_master_key() -> Result<(), Error> {
    // we need a TTY to query the new password
    if !std::io::stdin().is_terminal() {
        bail!("unable to create master key - no tty");
    }

    let bits = 4096;
    log::info!("Generating {}-bit RSA key..", bits);
    let rsa = openssl::rsa::Rsa::generate(bits)?;
    let public =
        openssl::rsa::Rsa::from_public_components(rsa.n().to_owned()?, rsa.e().to_owned()?)?;
    let info = RsaPubKeyInfo::try_from(public)?;
    log::info!("Modulus: {}", info.modulus);
    log::info!("Exponent: {}\n", info.exponent);

    let pkey = openssl::pkey::PKey::from_rsa(rsa)?;

    let password = String::from_utf8(tty::read_and_verify_password("Master Key Password: ")?)?;

    let pub_key: Vec<u8> = pkey.public_key_to_pem()?;
    let filename_pub = "master-public.pem";
    log::info!("Writing public master key to {}", filename_pub);
    replace_file(filename_pub, pub_key.as_slice(), CreateOptions::new(), true)?;

    let cipher = openssl::symm::Cipher::aes_256_cbc();
    let priv_key: Vec<u8> =
        pkey.private_key_to_pem_pkcs8_passphrase(cipher, password.as_bytes())?;

    let filename_priv = "master-private.pem";
    log::info!("Writing private master key to {}", filename_priv);
    replace_file(
        filename_priv,
        priv_key.as_slice(),
        CreateOptions::new(),
        true,
    )?;

    Ok(())
}

#[api(
    input: {
        properties: {
            path: {
                description: "Path to the PEM formatted RSA public key. Default location will be used if not specified.",
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// List information about master key
fn show_master_pubkey(path: Option<String>, param: Value) -> Result<(), Error> {
    let path = match path {
        Some(path) => PathBuf::from(path),
        None => find_default_master_pubkey()?
            .ok_or_else(|| format_err!("No path specified and no default master key available."))?,
    };

    let path = path.canonicalize()?;

    let output_format = get_output_format(&param);

    let pem_data = file_get_contents(path.clone())?;
    let rsa = openssl::rsa::Rsa::public_key_from_pem(&pem_data)?;

    let mut info = RsaPubKeyInfo::try_from(rsa)?;
    info.path = Some(path.display().to_string());

    let options = proxmox_router::cli::default_table_format_options()
        .column(ColumnConfig::new("path"))
        .column(ColumnConfig::new("modulus"))
        .column(ColumnConfig::new("exponent"))
        .column(ColumnConfig::new("length"));

    let return_type = ReturnType::new(false, &RsaPubKeyInfo::API_SCHEMA);

    format_and_print_result_full(
        &mut serde_json::to_value(info)?,
        &return_type,
        &output_format,
        &options,
    );

    Ok(())
}

#[api(
    input: {
        properties: {
            path: {
                description: "Key file. Without this the default key's will be used.",
                optional: true,
            },
            subject: {
                description: "Include the specified subject as title text.",
                optional: true,
            },
            "output-format": {
                type: PaperkeyFormat,
                optional: true,
            },
        },
    },
)]
/// Generate a printable, human readable text file containing the encryption key.
///
/// This also includes a scanable QR code for fast key restore.
fn paper_key(
    path: Option<String>,
    subject: Option<String>,
    output_format: Option<PaperkeyFormat>,
) -> Result<(), Error> {
    let path = match path {
        Some(path) => PathBuf::from(path),
        None => find_default_encryption_key()?
            .ok_or_else(|| format_err!("no encryption file provided and no default file found"))?,
    };

    let data = file_get_contents(path)?;
    let data = String::from_utf8(data)?;

    generate_paper_key(std::io::stdout(), &data, subject, output_format)
}

pub fn cli() -> CliCommandMap {
    let key_create_cmd_def = CliCommand::new(&API_METHOD_CREATE)
        .arg_param(&["path"])
        .completion_cb("path", complete_file_name);

    let key_import_with_master_key_cmd_def = CliCommand::new(&API_METHOD_IMPORT_WITH_MASTER_KEY)
        .arg_param(&["master-keyfile"])
        .completion_cb("master-keyfile", complete_file_name)
        .arg_param(&["encrypted-keyfile"])
        .completion_cb("encrypted-keyfile", complete_file_name)
        .arg_param(&["path"])
        .completion_cb("path", complete_file_name);

    let key_change_passphrase_cmd_def = CliCommand::new(&API_METHOD_CHANGE_PASSPHRASE)
        .arg_param(&["path"])
        .completion_cb("path", complete_file_name);

    let key_create_master_key_cmd_def = CliCommand::new(&API_METHOD_CREATE_MASTER_KEY);
    let key_import_master_pubkey_cmd_def = CliCommand::new(&API_METHOD_IMPORT_MASTER_PUBKEY)
        .arg_param(&["path"])
        .completion_cb("path", complete_file_name);
    let key_show_master_pubkey_cmd_def = CliCommand::new(&API_METHOD_SHOW_MASTER_PUBKEY)
        .arg_param(&["path"])
        .completion_cb("path", complete_file_name);

    let key_show_cmd_def = CliCommand::new(&API_METHOD_SHOW_KEY)
        .arg_param(&["path"])
        .completion_cb("path", complete_file_name);

    let paper_key_cmd_def = CliCommand::new(&API_METHOD_PAPER_KEY)
        .arg_param(&["path"])
        .completion_cb("path", complete_file_name);

    CliCommandMap::new()
        .insert("create", key_create_cmd_def)
        .insert("import-with-master-key", key_import_with_master_key_cmd_def)
        .insert("create-master-key", key_create_master_key_cmd_def)
        .insert("import-master-pubkey", key_import_master_pubkey_cmd_def)
        .insert("change-passphrase", key_change_passphrase_cmd_def)
        .insert("show", key_show_cmd_def)
        .insert("show-master-pubkey", key_show_master_pubkey_cmd_def)
        .insert("paperkey", paper_key_cmd_def)
}
