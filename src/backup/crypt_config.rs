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
use openssl::pkcs5::pbkdf2_hmac;
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

        let mut c = Crypter::new(self.cipher, Mode::Encrypt, &self.enc_key, Some(&iv))?;
        c.aad_update(b"")?; //??

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

    /// Decompress and decrypt date, verify MAC.
    pub fn decode_compressed_chunk(
        &self,
        data: &[u8],
        iv: &[u8; 16],
        tag: &[u8; 16],
    ) -> Result<Vec<u8>, Error> {

        let dec = Vec::with_capacity(1024*1024);

        let mut decompressor = zstd::stream::write::Decoder::new(dec)?;

        let mut c = Crypter::new(self.cipher, Mode::Decrypt, &self.enc_key, Some(iv))?;
        c.aad_update(b"")?; //??

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

    pub fn generate_rsa_encoded_key(
        &self,
        rsa: openssl::rsa::Rsa<openssl::pkey::Public>,
    ) -> Result<Vec<u8>, Error> {

        let mut buffer = vec![0u8; rsa.size() as usize];
        let len = rsa.public_encrypt(&self.enc_key, &mut buffer, openssl::rsa::Padding::PKCS1)?;
        if len != buffer.len() {
            bail!("got unexpected length from rsa.public_encrypt().");
        }
        Ok(buffer)
    }
}
