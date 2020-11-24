use std::path::PathBuf;
use std::io::Write;
use std::process::{Stdio, Command};

use anyhow::{bail, format_err, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use proxmox::api::api;
use proxmox::api::cli::{
    ColumnConfig,
    CliCommand,
    CliCommandMap,
    format_and_print_result_full,
    get_output_format,
    OUTPUT_FORMAT,
};
use proxmox::sys::linux::tty;
use proxmox::tools::fs::{file_get_contents, replace_file, CreateOptions};

use proxmox_backup::backup::{
    encrypt_key_with_passphrase,
    load_and_decrypt_key,
    store_key_config,
    CryptConfig,
    Kdf,
    KeyConfig,
    KeyDerivationConfig,
};
use proxmox_backup::tools;

#[api()]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Paperkey output format
pub enum PaperkeyFormat {
    /// Format as Utf8 text. Includes QR codes as ascii-art.
    Text,
    /// Format as Html. Includes QR codes as png images.
    Html,
}

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

    let mut key_array = [0u8; 32];
    proxmox::sys::linux::fill_with_random_data(&mut key_array)?;
    let crypt_config = CryptConfig::new(key_array.clone())?;
    let key = key_array.to_vec();

    match kdf {
        Kdf::None => {
            let created = proxmox::tools::time::epoch_i64();

            store_key_config(
                &path,
                false,
                KeyConfig {
                    kdf: None,
                    created,
                    modified: created,
                    data: key,
                    fingerprint: Some(crypt_config.fingerprint()),
                },
            )?;
        }
        Kdf::Scrypt | Kdf::PBKDF2 => {
            // always read passphrase from tty
            if !tty::stdin_isatty() {
                bail!("unable to read passphrase - no tty");
            }

            let password = tty::read_and_verify_password("Encryption Key Password: ")?;

            let mut key_config = encrypt_key_with_passphrase(&key, &password, kdf)?;
            key_config.fingerprint = Some(crypt_config.fingerprint());

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

    let (key, created, fingerprint) = load_and_decrypt_key(&path, &get_encryption_key_password)?;

    match kdf {
        Kdf::None => {
            let modified = proxmox::tools::time::epoch_i64();

            store_key_config(
                &path,
                true,
                KeyConfig {
                    kdf: None,
                    created, // keep original value
                    modified,
                    data: key.to_vec(),
                    fingerprint: Some(fingerprint),
                },
            )?;
        }
        Kdf::Scrypt | Kdf::PBKDF2 => {
            let password = tty::read_and_verify_password("New Password: ")?;

            let mut new_key_config = encrypt_key_with_passphrase(&key, &password, kdf)?;
            new_key_config.created = created; // keep original value
            new_key_config.fingerprint = Some(fingerprint);

            store_key_config(&path, true, new_key_config)?;
        }
    }

    Ok(())
}

#[api(
    properties: {
        kdf: {
            type: Kdf,
        },
    },
)]
#[derive(Deserialize, Serialize)]
/// Encryption Key Information
struct KeyInfo {
    /// Path to key
    path: String,
    kdf: Kdf,
    /// Key creation time
    pub created: i64,
    /// Key modification time
    pub modified: i64,
    /// Key fingerprint
    pub fingerprint: Option<String>,
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
fn show_key(
    path: Option<String>,
    param: Value,
) -> Result<(), Error> {
    let path = match path {
        Some(path) => PathBuf::from(path),
        None => {
            let path = find_default_encryption_key()?
                .ok_or_else(|| {
                    format_err!("no encryption file provided and no default file found")
                })?;
            path
        }
    };


    let config: KeyConfig = serde_json::from_slice(&file_get_contents(path.clone())?)?;

    let output_format = get_output_format(&param);

    let info = KeyInfo {
        path: format!("{:?}", path),
        kdf: match config.kdf {
            Some(KeyDerivationConfig::PBKDF2 { .. }) => Kdf::PBKDF2,
            Some(KeyDerivationConfig::Scrypt { .. }) => Kdf::Scrypt,
            None => Kdf::None,
        },
        created: config.created,
        modified: config.modified,
        fingerprint:  match config.fingerprint {
            Some(ref fp) => Some(format!("{}", fp)),
            None => None,
        },
    };

    let options = proxmox::api::cli::default_table_format_options()
        .column(ColumnConfig::new("path"))
        .column(ColumnConfig::new("kdf"))
        .column(ColumnConfig::new("created").renderer(tools::format::render_epoch))
        .column(ColumnConfig::new("modified").renderer(tools::format::render_epoch))
        .column(ColumnConfig::new("fingerprint"));

    let schema = &KeyInfo::API_SCHEMA;

    format_and_print_result_full(&mut serde_json::to_value(info)?, schema, &output_format, &options);

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

#[api(
    input: {
        properties: {
            path: {
                description: "Key file. Without this the default key's will be used.",
                optional: true,
            },
            subject: {
                description: "Include the specified subject as titel text.",
                optional: true,
            },
            "output-format": {
                type: PaperkeyFormat,
                description: "Output format. Text or Html.",
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
        None => {
            let path = find_default_encryption_key()?
                .ok_or_else(|| {
                    format_err!("no encryption file provided and no default file found")
                })?;
            path
        }
    };

    let data = file_get_contents(&path)?;
    let data = std::str::from_utf8(&data)?;

    let format = output_format.unwrap_or(PaperkeyFormat::Html);

    match format {
        PaperkeyFormat::Html => paperkey_html(data, subject),
        PaperkeyFormat::Text => paperkey_text(data, subject),
    }
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

    let key_show_cmd_def = CliCommand::new(&API_METHOD_SHOW_KEY)
        .arg_param(&["path"])
        .completion_cb("path", tools::complete_file_name);

    let paper_key_cmd_def = CliCommand::new(&API_METHOD_PAPER_KEY)
        .arg_param(&["path"])
        .completion_cb("path", tools::complete_file_name);

    CliCommandMap::new()
        .insert("create", key_create_cmd_def)
        .insert("create-master-key", key_create_master_key_cmd_def)
        .insert("import-master-pubkey", key_import_master_pubkey_cmd_def)
        .insert("change-passphrase", key_change_passphrase_cmd_def)
        .insert("show", key_show_cmd_def)
        .insert("paperkey", paper_key_cmd_def)
}

fn paperkey_html(data: &str, subject: Option<String>) -> Result<(), Error> {

    let img_size_pt = 500;

    println!("<!DOCTYPE html>");
    println!("<html lang=\"en\">");
    println!("<head>");
    println!("<meta charset=\"utf-8\">");
    println!("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">");
    println!("<title>Proxmox Backup Paperkey</title>");
    println!("<style type=\"text/css\">");

    println!("  p {{");
    println!("    font-size: 12pt;");
    println!("    font-family: monospace;");
    println!("    white-space: pre-wrap;");
    println!("    line-break: anywhere;");
    println!("  }}");

    println!("</style>");

    println!("</head>");

    println!("<body>");

    if let Some(subject) = subject {
        println!("<p>Subject: {}</p>", subject);
    }

    if data.starts_with("-----BEGIN ENCRYPTED PRIVATE KEY-----\n") {
        let lines: Vec<String> = data.lines()
            .map(|s| s.trim_end())
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

        if !lines[lines.len()-1].starts_with("-----END ENCRYPTED PRIVATE KEY-----") {
            bail!("unexpected key format");
        }

        if lines.len() < 20 {
            bail!("unexpected key format");
        }

        const BLOCK_SIZE: usize = 20;
        let blocks = (lines.len() + BLOCK_SIZE -1)/BLOCK_SIZE;

        for i in 0..blocks {
            let start = i*BLOCK_SIZE;
            let mut end = start + BLOCK_SIZE;
            if end > lines.len() {
                end = lines.len();
            }
            let data = &lines[start..end];

            println!("<div style=\"page-break-inside: avoid;page-break-after: always\">");
            println!("<p>");

            for l in start..end {
                println!("{:02}: {}", l, lines[l]);
            }

            println!("</p>");

            let data = data.join("\n");
            let qr_code = generate_qr_code("svg", data.as_bytes())?;
            let qr_code = base64::encode_config(&qr_code, base64::STANDARD_NO_PAD);

            println!("<center>");
            println!("<img");
            println!("width=\"{}pt\" height=\"{}pt\"", img_size_pt, img_size_pt);
            println!("src=\"data:image/svg+xml;base64,{}\"/>", qr_code);
            println!("</center>");
            println!("</div>");
       }

        println!("</body>");
        println!("</html>");
        return Ok(());
    }

    let key_config: KeyConfig = serde_json::from_str(&data)?;
    let key_text = serde_json::to_string_pretty(&key_config)?;

    println!("<div style=\"page-break-inside: avoid\">");

    println!("<p>");

    println!("-----BEGIN PROXMOX BACKUP KEY-----");

    for line in key_text.lines() {
        println!("{}", line);
    }

    println!("-----END PROXMOX BACKUP KEY-----");

    println!("</p>");

    let qr_code = generate_qr_code("svg", key_text.as_bytes())?;
    let qr_code = base64::encode_config(&qr_code, base64::STANDARD_NO_PAD);

    println!("<center>");
    println!("<img");
    println!("width=\"{}pt\" height=\"{}pt\"", img_size_pt, img_size_pt);
    println!("src=\"data:image/svg+xml;base64,{}\"/>", qr_code);
    println!("</center>");

    println!("</div>");

    println!("</body>");
    println!("</html>");

    Ok(())
}

fn paperkey_text(data: &str, subject: Option<String>) -> Result<(), Error> {

    if let Some(subject) = subject {
        println!("Subject: {}\n", subject);
    }

    if data.starts_with("-----BEGIN ENCRYPTED PRIVATE KEY-----\n") {
        let lines: Vec<String> = data.lines()
            .map(|s| s.trim_end())
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

        if !lines[lines.len()-1].starts_with("-----END ENCRYPTED PRIVATE KEY-----") {
            bail!("unexpected key format");
        }

        if lines.len() < 20 {
            bail!("unexpected key format");
        }

        const BLOCK_SIZE: usize = 5;
        let blocks = (lines.len() + BLOCK_SIZE -1)/BLOCK_SIZE;

        for i in 0..blocks {
            let start = i*BLOCK_SIZE;
            let mut end = start + BLOCK_SIZE;
            if end > lines.len() {
                end = lines.len();
            }
            let data = &lines[start..end];

            for l in start..end {
                println!("{:-2}: {}", l, lines[l]);
            }
            let data = data.join("\n");
            let qr_code = generate_qr_code("utf8i", data.as_bytes())?;
            let qr_code = String::from_utf8(qr_code)
                .map_err(|_| format_err!("Failed to read qr code (got non-utf8 data)"))?;
            println!("{}", qr_code);
            println!("{}", char::from(12u8)); // page break

        }
        return Ok(());
    }

    let key_config: KeyConfig = serde_json::from_str(&data)?;
    let key_text = serde_json::to_string_pretty(&key_config)?;

    println!("-----BEGIN PROXMOX BACKUP KEY-----");
    println!("{}", key_text);
    println!("-----END PROXMOX BACKUP KEY-----");

    let qr_code = generate_qr_code("utf8i", key_text.as_bytes())?;
    let qr_code = String::from_utf8(qr_code)
        .map_err(|_| format_err!("Failed to read qr code (got non-utf8 data)"))?;

    println!("{}", qr_code);

    Ok(())
}

fn generate_qr_code(output_type: &str, data: &[u8]) -> Result<Vec<u8>, Error> {

    let mut child = Command::new("qrencode")
        .args(&["-t", output_type, "-m0", "-s1", "-lm", "--output", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    {
        let stdin = child.stdin.as_mut()
            .ok_or_else(|| format_err!("Failed to open stdin"))?;
        stdin.write_all(data)
            .map_err(|_| format_err!("Failed to write to stdin"))?;
    }

    let output = child.wait_with_output()
        .map_err(|_| format_err!("Failed to read stdout"))?;

    let output = crate::tools::command_output(output, None)?;

    Ok(output)
}
