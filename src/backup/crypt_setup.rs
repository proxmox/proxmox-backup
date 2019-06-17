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
use openssl::symm::{decrypt_aead, Cipher, Crypter, Mode};
use std::io::Write;

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

pub struct SCryptConfig {
    pub n: u64,
    pub r: u64,
    pub p: u64,
    pub salt: Vec<u8>,
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
    pub fn derive_key_from_password(password: &[u8], scrypt_config: &SCryptConfig) -> Result<[u8; 32], Error> {

        let mut key = [0u8; 32];

        // estimated scrypt memory usage is 128*r*n*p

        scrypt(
            password,
            &scrypt_config.salt,
            scrypt_config.n, scrypt_config.r, scrypt_config.p, 1025*1024*1024,
            &mut key)?;

        Ok(key)
    }

    /// Compute a chunk digest using a secret name space.
    ///
    /// Computes an SHA256 checksum over some secret data (derived
    /// from the secret key) and the provided data. This ensures that
    /// chunk digest values do not clash with values computed for
    /// other sectret keys.
    pub fn compute_digest(&self, data: &[u8]) -> [u8; 32] {
        // FIXME: use HMAC-SHA256 instead??
        let mut hasher = openssl::sha::Sha256::new();
        hasher.update(&self.id_key);
        hasher.update(data);
        let digest = hasher.finish();
        digest
    }

    /// Compress and encrypt data using a random 16 byte IV.
    ///
    /// Return the encrypted data, including IV and MAC (MAGIC || IV || MAC || ENC_DATA).
    pub fn encode_chunk(&self, data: &[u8], compress: bool) -> Result<Vec<u8>, Error> {

        let iv = proxmox::sys::linux::random_data(16)?;
        let mut c = Crypter::new(self.cipher, Mode::Encrypt, &self.enc_key, Some(&iv))?;
        c.aad_update(b"")?; //??

        if compress {
            let compr_data =  zstd::block::compress(data, 1)?;
            // Note: We only use compression if result is shorter
            if compr_data.len() < data.len() {
                let mut enc = vec![0; compr_data.len()+40+self.cipher.block_size()];

                enc[0..8].copy_from_slice(&super::ENCR_COMPR_CHUNK_MAGIC_1_0);
                enc[8..24].copy_from_slice(&iv);

                let count = c.update(&compr_data, &mut enc[40..])?;
                let rest = c.finalize(&mut enc[(40+count)..])?;
                enc.truncate(40 + count + rest);

                c.get_tag(&mut enc[24..40])?;

                return Ok(enc)
            }
        }

        let mut enc = vec![0; data.len()+40+self.cipher.block_size()];

        enc[0..8].copy_from_slice(&super::ENCRYPTED_CHUNK_MAGIC_1_0);
        enc[8..24].copy_from_slice(&iv);

        let count = c.update(data, &mut enc[40..])?;
        let rest = c.finalize(&mut enc[(40+count)..])?;
        enc.truncate(40 + count + rest);

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

        if magic == super::ENCR_COMPR_CHUNK_MAGIC_1_0 {

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

            return Ok(decompressor.into_inner());

        } else if magic == super::ENCRYPTED_CHUNK_MAGIC_1_0 {
            let decr_data = decrypt_aead(
                self.cipher,
                &self.enc_key,
                Some(iv),
                b"", //??
                &data[40..],
                mac,
            )?;
            return Ok(decr_data);
        } else {
            bail!("Invalid magic number (expected encrypted chunk).");
        }
    }
}
