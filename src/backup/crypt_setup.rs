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
use openssl::symm::{Cipher, Crypter, Mode};
use std::io::{Read, Write};

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

    /// Compress and encrypt data using a random 16 byte IV.
    ///
    /// Return the encrypted data, including IV and MAC (MAGIC || IV || MAC || ENC_DATA).
    pub fn encode_chunk(&self, data: &[u8]) -> Result<Vec<u8>, Error> {

        let iv = proxmox::sys::linux::random_data(16)?;

        let mut enc = Vec::with_capacity(data.len()+40+self.cipher.block_size());

        enc.write_all(&super::ENCRYPTED_CHUNK_MAGIC_1_0)?;
        enc.write_all(&iv)?;
        enc.write_all(&[0u8;16])?; // dummy tag, update later

        let mut zstream = zstd::stream::read::Encoder::new(data, 1)?;

        let mut c = Crypter::new(self.cipher, Mode::Encrypt, &self.enc_key, Some(&iv))?;
        c.aad_update(b"")?; //??

        const BUFFER_SIZE: usize = 32*1024;

        let mut comp_buf = [0u8; BUFFER_SIZE];
        let mut encr_buf = [0u8; BUFFER_SIZE];

        loop {
            let bytes = zstream.read(&mut comp_buf)?;
            if bytes == 0 { break; }

            let count = c.update(&comp_buf[..bytes], &mut encr_buf)?;
            enc.write_all(&encr_buf[..count])?;
        }

        let rest = c.finalize(&mut encr_buf)?;
        if rest > 0 {  enc.write_all(&encr_buf[..rest])?; }

        c.get_tag(&mut enc[24..40])?;

        Ok(enc)
    }

    /// Decompress and decrypt chunk, verify MAC.
    ///
    /// Binrary ``data`` is expected to be in format returned by encode_chunk.
    pub fn decode_chunk(&self, data: &[u8]) -> Result<Vec<u8>, Error> {

        if data.len() < 40 {
            bail!("Invalid chunk len (<40)");
        }


        let magic = &data[0..8];
        let iv = &data[8..24];
        let mac = &data[24..40];

        if magic != super::ENCRYPTED_CHUNK_MAGIC_1_0 {
            bail!("Invalid magic number (expected encrypted chunk).");
        }

        let dec = Vec::with_capacity(1024*1024);

        let mut decompressor = zstd::stream::write::Decoder::new(dec)?;

        let mut c = Crypter::new(self.cipher, Mode::Decrypt, &self.enc_key, Some(iv))?;
        c.aad_update(b"")?; //??

        const BUFFER_SIZE: usize = 32*1024;

        let mut decr_buf = [0u8; BUFFER_SIZE];
        let max_decoder_input = BUFFER_SIZE - self.cipher.block_size();

        let mut start = 40;
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

        c.set_tag(mac)?;
        let rest = c.finalize(&mut decr_buf)?;
        if rest > 0 { decompressor.write_all(&decr_buf[..rest])?; }

        decompressor.flush()?;

        Ok(decompressor.into_inner())
    }
}
