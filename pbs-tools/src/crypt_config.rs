//! Wrappers for OpenSSL crypto functions
//!
//! We use this to encrypt and decrypt data chunks. Cipher is
//! AES_256_GCM, which is fast and provides authenticated encryption.
//!
//! See the Wikipedia Artikel for [Authenticated
//! encryption](https://en.wikipedia.org/wiki/Authenticated_encryption)
//! for a short introduction.

use anyhow::Error;
use openssl::hash::MessageDigest;
use openssl::pkcs5::pbkdf2_hmac;
use openssl::symm::{Cipher, Crypter, Mode};

// openssl::sha::sha256(b"Proxmox Backup Encryption Key Fingerprint")
/// This constant is used to compute fingerprints.
const FINGERPRINT_INPUT: [u8; 32] = [
    110, 208, 239, 119, 71, 31, 255, 77, 85, 199, 168, 254, 74, 157, 182, 33, 97, 64, 127, 19, 76,
    114, 93, 223, 48, 153, 45, 37, 236, 69, 237, 38,
];

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
            &mut id_key,
        )?;

        let id_pkey = openssl::pkey::PKey::hmac(&id_key).unwrap();

        Ok(Self {
            id_key,
            id_pkey,
            enc_key,
            cipher: Cipher::aes_256_gcm(),
        })
    }

    /// Expose Cipher (AES_256_GCM)
    pub fn cipher(&self) -> &Cipher {
        &self.cipher
    }

    /// Expose encryption key
    pub fn enc_key(&self) -> &[u8; 32] {
        &self.enc_key
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

    /// Returns an openssl Signer using SHA256
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

    /// Computes a fingerprint for the secret key.
    ///
    /// This computes a digest using the derived key (id_key) in order
    /// to hinder brute force attacks.
    pub fn fingerprint(&self) -> [u8; 32] {
        self.compute_digest(&FINGERPRINT_INPUT)
    }

    /// Returns an openssl Crypter using AES_256_GCM,
    pub fn data_crypter(&self, iv: &[u8; 16], mode: Mode) -> Result<Crypter, Error> {
        let mut crypter = openssl::symm::Crypter::new(self.cipher, mode, &self.enc_key, Some(iv))?;
        crypter.aad_update(b"")?; //??
        Ok(crypter)
    }
}
