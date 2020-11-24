use anyhow::{bail, format_err, Context, Error};

use serde::{Deserialize, Serialize};

use crate::backup::{CryptConfig, Fingerprint};

use proxmox::api::api;
use proxmox::tools::fs::{file_get_contents, replace_file, CreateOptions};
use proxmox::try_block;

#[api(default: "scrypt")]
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
/// Key derivation function for password protected encryption keys.
pub enum Kdf {
    /// Do not encrypt the key.
    None,

    /// Encrypt they key with a password using SCrypt.
    Scrypt,

    /// Encrtypt the Key with a password using PBKDF2
    PBKDF2,
}

impl Default for Kdf {
    #[inline]
    fn default() -> Self {
        Kdf::Scrypt
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub enum KeyDerivationConfig {
    Scrypt {
        n: u64,
        r: u64,
        p: u64,
        #[serde(with = "proxmox::tools::serde::bytes_as_base64")]
        salt: Vec<u8>,
    },
    PBKDF2 {
        iter: usize,
        #[serde(with = "proxmox::tools::serde::bytes_as_base64")]
        salt: Vec<u8>,
    },
}

impl KeyDerivationConfig {

    /// Derive a key from provided passphrase
    pub fn derive_key(&self, passphrase: &[u8]) -> Result<[u8; 32], Error> {

        let mut key = [0u8; 32];

        match self {
            KeyDerivationConfig::Scrypt { n, r, p, salt } => {
                // estimated scrypt memory usage is 128*r*n*p
                openssl::pkcs5::scrypt(
                    passphrase,
                    &salt,
                    *n, *r, *p,
                    1025*1024*1024,
                    &mut key,
                )?;

                Ok(key)
            }
            KeyDerivationConfig::PBKDF2 { iter, salt } => {

                 openssl::pkcs5::pbkdf2_hmac(
                    passphrase,
                    &salt,
                    *iter,
                    openssl::hash::MessageDigest::sha256(),
                    &mut key,
                )?;

                Ok(key)
            }
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct KeyConfig {
    pub kdf: Option<KeyDerivationConfig>,
    #[serde(with = "proxmox::tools::serde::epoch_as_rfc3339")]
    pub created: i64,
    #[serde(with = "proxmox::tools::serde::epoch_as_rfc3339")]
    pub modified: i64,
    #[serde(with = "proxmox::tools::serde::bytes_as_base64")]
    pub data: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub fingerprint: Option<Fingerprint>,
 }

pub fn store_key_config(
    path: &std::path::Path,
    replace: bool,
    key_config: KeyConfig,
) -> Result<(), Error> {

    let data = serde_json::to_string(&key_config)?;

    use std::io::Write;

    try_block!({
        if replace {
            let mode = nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR;
            replace_file(&path, data.as_bytes(), CreateOptions::new().perm(mode))?;
        } else {
            use std::os::unix::fs::OpenOptionsExt;

            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .mode(0o0600)
                .create_new(true)
                .open(&path)?;

            file.write_all(data.as_bytes())?;
        }

        Ok(())
    }).map_err(|err: Error| format_err!("Unable to create file {:?} - {}", path, err))?;

    Ok(())
}

pub fn encrypt_key_with_passphrase(
    raw_key: &[u8],
    passphrase: &[u8],
    kdf: Kdf,
) -> Result<KeyConfig, Error> {

    let salt = proxmox::sys::linux::random_data(32)?;

    let kdf = match kdf {
        Kdf::Scrypt => KeyDerivationConfig::Scrypt {
            n: 65536,
            r: 8,
            p: 1,
            salt,
        },
        Kdf::PBKDF2 => KeyDerivationConfig::PBKDF2 {
            iter: 65535,
            salt,
        },
        Kdf::None => {
            bail!("No key derivation function specified");
        }
    };

    let derived_key = kdf.derive_key(passphrase)?;

    let cipher = openssl::symm::Cipher::aes_256_gcm();

    let iv = proxmox::sys::linux::random_data(16)?;
    let mut tag = [0u8; 16];

    let encrypted_key = openssl::symm::encrypt_aead(
        cipher,
        &derived_key,
        Some(&iv),
        b"",
        &raw_key,
        &mut tag,
    )?;

    let mut enc_data = vec![];
    enc_data.extend_from_slice(&iv);
    enc_data.extend_from_slice(&tag);
    enc_data.extend_from_slice(&encrypted_key);

    let created = proxmox::tools::time::epoch_i64();

    Ok(KeyConfig {
        kdf: Some(kdf),
        created,
        modified: created,
        data: enc_data,
        fingerprint: None,
    })
}

pub fn load_and_decrypt_key(
    path: &std::path::Path,
    passphrase: &dyn Fn() -> Result<Vec<u8>, Error>,
) -> Result<([u8;32], i64, Fingerprint), Error> {
    decrypt_key(&file_get_contents(&path)?, passphrase)
        .with_context(|| format!("failed to load decryption key from {:?}", path))
}

pub fn decrypt_key(
    mut keydata: &[u8],
    passphrase: &dyn Fn() -> Result<Vec<u8>, Error>,
) -> Result<([u8;32], i64, Fingerprint), Error> {
    let key_config: KeyConfig = serde_json::from_reader(&mut keydata)?;

    let raw_data = key_config.data;
    let created = key_config.created;

    let key = if let Some(kdf) = key_config.kdf {

        let passphrase = passphrase()?;
        if passphrase.len() < 5 {
            bail!("Passphrase is too short!");
        }

        let derived_key = kdf.derive_key(&passphrase)?;

        if raw_data.len() < 32 {
            bail!("Unable to encode key - short data");
        }
        let iv = &raw_data[0..16];
        let tag = &raw_data[16..32];
        let enc_data = &raw_data[32..];

        let cipher = openssl::symm::Cipher::aes_256_gcm();

        openssl::symm::decrypt_aead(
            cipher,
            &derived_key,
            Some(&iv),
            b"", //??
            &enc_data,
            &tag,
        ).map_err(|err| format_err!("Unable to decrypt key - {}", err))?

    } else {
        raw_data
    };

    let mut result = [0u8; 32];
    result.copy_from_slice(&key);

    let fingerprint = match key_config.fingerprint {
        Some(fingerprint) => fingerprint,
        None => {
            let crypt_config = CryptConfig::new(result.clone())?;
            crypt_config.fingerprint()
        },
    };

    Ok((result, created, fingerprint))
}
