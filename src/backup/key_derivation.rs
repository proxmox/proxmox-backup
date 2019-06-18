use failure::*;

use serde::{Deserialize, Serialize};
use chrono::{Local, TimeZone, DateTime};

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
    kdf: Option<KeyDerivationConfig>,
    #[serde(with = "proxmox::tools::serde::date_time_as_rfc3339")]
    created: DateTime<Local>,
    #[serde(with = "proxmox::tools::serde::bytes_as_base64")]
    data: Vec<u8>,
 }


pub fn store_key_with_passphrase(
    path: &std::path::Path,
    raw_key: &[u8],
    passphrase: &[u8],
    replace: bool,
) -> Result<(), Error> {

    let salt = proxmox::sys::linux::random_data(32)?;

    let kdf = KeyDerivationConfig::Scrypt {
        n: 65536,
        r: 8,
        p: 1,
        salt,
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

    let created =  Local.timestamp(Local::now().timestamp(), 0);


    let key_config = KeyConfig {
        kdf: Some(kdf),
        created,
        data: enc_data,
    };

    let data = serde_json::to_string(&key_config)?;

    use std::io::Write;

    try_block!({
        if replace {
            let mode = nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR;
            crate::tools::file_set_contents(&path, data.as_bytes(), Some(mode))?;
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

pub fn load_and_decrtypt_key(path: &std::path::Path, passphrase: fn() -> Result<Vec<u8>, Error>) -> Result<Vec<u8>, Error> {

    let raw = crate::tools::file_get_contents(&path)?;
    let data = String::from_utf8(raw)?;

    let key_config: KeyConfig = serde_json::from_str(&data)?;

    let raw_data = key_config.data;

    if let Some(kdf) = key_config.kdf {

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

        let decr_data = openssl::symm::decrypt_aead(
            cipher,
            &derived_key,
            Some(&iv),
            b"", //??
            &enc_data,
            &tag,
        ).map_err(|err| format_err!("Unable to decrypt key - {}", err))?;

        Ok(decr_data)
    } else {
        Ok(raw_data)
    }
}
