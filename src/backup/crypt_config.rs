//! Wrappers for OpenSSL crypto functions
//!
//! We use this to encrypt and decryprt data chunks. Cipher is
//! AES_256_GCM, which is fast and provides authenticated encryption.
//!
//! See the Wikipedia Artikel for [Authenticated
//! encryption](https://en.wikipedia.org/wiki/Authenticated_encryption)
//! for a short introduction.

use std::fmt;
use std::fmt::Display;
use std::io::Write;

use anyhow::{Error};
use openssl::hash::MessageDigest;
use openssl::pkcs5::pbkdf2_hmac;
use openssl::symm::{decrypt_aead, Cipher, Crypter, Mode};
use serde::{Deserialize, Serialize};

use proxmox::api::api;

use pbs_tools::format::{as_fingerprint, bytes_as_fingerprint};

// openssl::sha::sha256(b"Proxmox Backup Encryption Key Fingerprint")
/// This constant is used to compute fingerprints.
const FINGERPRINT_INPUT: [u8; 32] = [
    110, 208, 239, 119,  71,  31, 255,  77,
    85, 199, 168, 254,  74, 157, 182,  33,
    97,  64, 127,  19,  76, 114,  93, 223,
    48, 153,  45,  37, 236,  69, 237,  38,
];
#[api(default: "encrypt")]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
/// Defines whether data is encrypted (using an AEAD cipher), only signed, or neither.
pub enum CryptMode {
    /// Don't encrypt.
    None,
    /// Encrypt.
    Encrypt,
    /// Only sign.
    SignOnly,
}

#[derive(Debug, Eq, PartialEq, Hash, Clone, Deserialize, Serialize)]
#[serde(transparent)]
/// 32-byte fingerprint, usually calculated with SHA256.
pub struct Fingerprint {
    #[serde(with = "bytes_as_fingerprint")]
    bytes: [u8; 32],
}

impl Fingerprint {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }
    pub fn bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

/// Display as short key ID
impl Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", as_fingerprint(&self.bytes[0..8]))
    }
}

impl std::str::FromStr for Fingerprint {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let mut tmp = s.to_string();
        tmp.retain(|c| c != ':');
        let bytes = proxmox::tools::hex_to_digest(&tmp)?;
        Ok(Fingerprint::new(bytes))
    }
}

/// Encryption Configuration with secret key
///
/// This structure stores the secret key and provides helpers for
/// authenticated encryption.
pub struct CryptConfig {
    // the Cipher
    cipher: Cipher,
    // A secrect key use to provide the chunk digest name space.
    id_key: [u8; 32],
    // Openssl hmac PKey of id_key
    id_pkey: openssl::pkey::PKey<openssl::pkey::Private>,
    // The private key used by the cipher.
    enc_key: [u8; 32],
}

impl CryptConfig {

    /// Create a new instance.
    ///
    /// We compute a derived 32 byte key using pbkdf2_hmac. This second
    /// key is used in compute_digest.
    pub fn new(enc_key: [u8; 32]) -> Result<Self, Error> {

        let mut id_key = [0u8; 32];

        pbkdf2_hmac(
            &enc_key,
            b"_id_key",
            10,
            MessageDigest::sha256(),
            &mut id_key)?;

        let id_pkey = openssl::pkey::PKey::hmac(&id_key).unwrap();

        Ok(Self { id_key, id_pkey, enc_key, cipher: Cipher::aes_256_gcm() })
    }

    /// Expose Cipher
    pub fn cipher(&self) -> &Cipher {
        &self.cipher
    }

    /// Compute a chunk digest using a secret name space.
    ///
    /// Computes an SHA256 checksum over some secret data (derived
    /// from the secret key) and the provided data. This ensures that
    /// chunk digest values do not clash with values computed for
    /// other sectret keys.
    pub fn compute_digest(&self, data: &[u8]) -> [u8; 32] {
        let mut hasher = openssl::sha::Sha256::new();
        hasher.update(data);
        hasher.update(&self.id_key); // at the end, to avoid length extensions attacks
        hasher.finish()
    }

    pub fn data_signer(&self) -> openssl::sign::Signer {
        openssl::sign::Signer::new(MessageDigest::sha256(), &self.id_pkey).unwrap()
    }

    /// Compute authentication tag (hmac/sha256)
    ///
    /// Computes an SHA256 HMAC using some secret data (derived
    /// from the secret key) and the provided data.
    pub fn compute_auth_tag(&self, data: &[u8]) -> [u8; 32] {
        let mut signer = self.data_signer();
        signer.update(data).unwrap();
        let mut tag = [0u8; 32];
        signer.sign(&mut tag).unwrap();
        tag
    }

    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::new(self.compute_digest(&FINGERPRINT_INPUT))
    }

    pub fn data_crypter(&self, iv: &[u8; 16], mode: Mode) -> Result<Crypter, Error>  {
        let mut crypter = openssl::symm::Crypter::new(self.cipher, mode, &self.enc_key, Some(iv))?;
        crypter.aad_update(b"")?; //??
        Ok(crypter)
    }

    /// Encrypt data using a random 16 byte IV.
    ///
    /// Writes encrypted data to ``output``, Return the used IV and computed MAC.
    pub fn encrypt_to<W: Write>(
        &self,
        data: &[u8],
        mut output: W,
    ) -> Result<([u8;16], [u8;16]), Error> {

        let mut iv = [0u8; 16];
        proxmox::sys::linux::fill_with_random_data(&mut iv)?;

        let mut tag = [0u8; 16];

        let mut c = self.data_crypter(&iv, Mode::Encrypt)?;

        const BUFFER_SIZE: usize = 32*1024;

        let mut encr_buf = [0u8; BUFFER_SIZE];
        let max_encoder_input = BUFFER_SIZE - self.cipher.block_size();

        let mut start = 0;
        loop {
            let mut end = start + max_encoder_input;
            if end > data.len() { end = data.len(); }
            if end > start {
                let count = c.update(&data[start..end], &mut encr_buf)?;
                output.write_all(&encr_buf[..count])?;
                start = end;
            } else {
                break;
            }
        }

        let rest = c.finalize(&mut encr_buf)?;
        if rest > 0 { output.write_all(&encr_buf[..rest])?; }

        output.flush()?;

        c.get_tag(&mut tag)?;

        Ok((iv, tag))
    }

    /// Decompress and decrypt data, verify MAC.
    pub fn decode_compressed_chunk(
        &self,
        data: &[u8],
        iv: &[u8; 16],
        tag: &[u8; 16],
    ) -> Result<Vec<u8>, Error> {

        let dec = Vec::with_capacity(1024*1024);

        let mut decompressor = zstd::stream::write::Decoder::new(dec)?;

        let mut c = self.data_crypter(iv, Mode::Decrypt)?;

        const BUFFER_SIZE: usize = 32*1024;

        let mut decr_buf = [0u8; BUFFER_SIZE];
        let max_decoder_input = BUFFER_SIZE - self.cipher.block_size();

        let mut start = 0;
        loop {
            let mut end = start + max_decoder_input;
            if end > data.len() { end = data.len(); }
            if end > start {
                let count = c.update(&data[start..end], &mut decr_buf)?;
                decompressor.write_all(&decr_buf[0..count])?;
                start = end;
            } else {
                break;
            }
        }

        c.set_tag(tag)?;
        let rest = c.finalize(&mut decr_buf)?;
        if rest > 0 { decompressor.write_all(&decr_buf[..rest])?; }

        decompressor.flush()?;

        Ok(decompressor.into_inner())
    }

    /// Decrypt data, verify tag.
    pub fn decode_uncompressed_chunk(
        &self,
        data: &[u8],
        iv: &[u8; 16],
        tag: &[u8; 16],
    ) -> Result<Vec<u8>, Error> {

        let decr_data = decrypt_aead(
            self.cipher,
            &self.enc_key,
            Some(iv),
            b"", //??
            data,
            tag,
        )?;

        Ok(decr_data)
    }
}
