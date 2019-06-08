//! Wrappers for OpenSSL crypto functions
//!
//! We use this to encrypt and decryprt data chunks. Cipher is
//! AES_256_GCM, which is fast and provides authenticated encryption.
//!
//! See the Wikipedia Artikel for [Authenticated
//! encryption](https://en.wikipedia.org/wiki/Authenticated_encryption)
//! for a short introduction.
use failure::*;
use proxmox::tools;
use openssl::pkcs5::{pbkdf2_hmac, scrypt};
use openssl::hash::MessageDigest;
use openssl::symm::{encrypt_aead, decrypt_aead, Cipher};

/// Store data required for authenticated enryption
pub struct CryptData {
    /// A 16 byte IV
    pub iv: [u8; 16],
    /// A 16 byte message authentication code (MAC)
    pub mac: [u8; 16],
}

/// Encryption Configuration with secret key
///
/// This structure stores the secret key and provides helpers for
/// authenticated encryption.
pub struct CryptConfig {
    // the Cipher
    cipher: Cipher,
    // A secrect key use to provide the chunk digest name space.
    id_key: Vec<u8>,
    // The private key used by the cipher.
    enc_key: [u8; 32],
}

impl CryptConfig {

    /// Create a new instance.
    ///
    /// We compute a derived 32 byte key using pbkdf2_hmac. This second
    /// key is used in compute_digest.
    pub fn new(enc_key: [u8; 32]) -> Result<Self, Error> {

        let mut id_key = tools::vec::undefined(32);

        pbkdf2_hmac(
            &enc_key,
            b"_id_key",
            10,
            MessageDigest::sha256(),
            &mut id_key)?;

        Ok(Self { id_key, enc_key, cipher: Cipher::aes_256_gcm() })
    }

    /// A simple key derivation function using scrypt
    fn derive_key_from_password(password: &[u8]) -> Result<[u8; 32], Error> {

        let mut key = [0u8; 32];

        // estimated scrypt memory usage is N*2r*64
        let n = 65536;
        let r = 8;
        let p = 1;

        let salt = b""; // Salt??

        scrypt(
            password,
            salt,
            n, r, p, 128*1024*1024,
            &mut key)?;

        Ok(key)
    }

    /// Create a new instance, but derive key from password using scrypt.
    pub fn with_password(password: &[u8]) -> Result<Self, Error> {

        let enc_key = Self::derive_key_from_password(password)?;

        Self::new(enc_key)
    }

    /// Compute a chunk digest using a secret name space.
    ///
    /// Computes an SHA256 checksum over some secret data (derived
    /// from the secret key) and the provided data. This ensures that
    /// chunk digest values do not clash with values computed for
    /// other sectret keys.
    pub fn compute_digest(&self, data: &[u8]) -> [u8; 32] {
        let mut hasher = openssl::sha::Sha256::new();
        hasher.update(&self.id_key);
        hasher.update(data);
        let digest = hasher.finish();
        digest
    }

    /// Encrypt data using a random 16 byte IV.
    ///
    /// Return the encrypted data, IV and MAC.
    pub fn encrypt(&self, data: &[u8]) -> Result<(Vec<u8>, CryptData), Error> {

        let mac = [0u8; 16];
        let mut iv = [0u8; 16];

        proxmox::sys::linux::fill_with_random_data(&mut iv)?;

        let mut crypt_data = CryptData { mac: mac, iv: iv  };

        let enc_data = encrypt_aead(
            self.cipher,
            &self.enc_key,
            Some(&iv),
            b"", // no additional data
            &data,
            &mut crypt_data.mac)?;

        Ok((enc_data, crypt_data))
    }

    /// Decrypt data, verify authentication.
    ///
    /// You need to pass the IV and MAC from the entryption step in ``crypt_data``.
    pub fn decrypt(&self, data: &[u8], crypt_data: &CryptData) -> Result<Vec<u8>, Error> {

        let decrypt_result = decrypt_aead(
            self.cipher,
            &self.enc_key,
            Some(&crypt_data.iv),
            b"", // no additional data
            data,
            &crypt_data.mac);

        let raw_data = match decrypt_result {
            Ok(data) => data,
            Err(err) => {
                // for unknown reason, openssl does not return useful errors (just empty array)
                if err.errors().len() == 0 {
                    bail!("unable to decyrpt chunk data");
                }
                bail!("unable to decyrpt data - {}", err);
            }
        };

        Ok(raw_data)
    }
}
