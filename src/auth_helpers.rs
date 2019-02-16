use crate::tools;

use failure::*;
use lazy_static::lazy_static;

use openssl::rsa::{Rsa};
use openssl::pkey::{PKey, Public, Private};
use openssl::sha;

use std::path::PathBuf;

pub fn assemble_csrf_prevention_token(
    secret: &[u8],
    username: &str,
) -> String {

    let epoch = std::time::SystemTime::now().duration_since(
        std::time::SystemTime::UNIX_EPOCH).unwrap().as_secs();

    let mut hasher = sha::Sha256::new();
    let data = format!("{:08X}:{}:", epoch, username);
    hasher.update(data.as_bytes());
    hasher.update(secret);

    let digest = base64::encode_config(&hasher.finish(), base64::STANDARD_NO_PAD);

    format!("{:08X}:{}", epoch, digest)
}

pub fn generate_csrf_key() -> Result<(), Error> {

    let path = PathBuf::from(configdir!("/csrf.key"));

    if path.exists() { return Ok(()); }

    let rsa = Rsa::generate(2048).unwrap();

    let pem = rsa.private_key_to_pem()?;

    use nix::sys::stat::Mode;

    tools::file_set_contents(
        &path, &pem, Some(Mode::from_bits_truncate(0o0640)))?;

    let (_, backup_gid) = tools::getpwnam_ugid("backup")?;

    nix::unistd::chown(&path, Some(nix::unistd::ROOT), Some(nix::unistd::Gid::from_raw(backup_gid)))?;

    Ok(())
}

pub fn generate_auth_key() -> Result<(), Error> {

    let priv_path = PathBuf::from(configdir!("/authkey.key"));

    let mut public_path = priv_path.clone();
    public_path.set_extension("pub");

    if priv_path.exists() && public_path.exists() { return Ok(()); }

    let rsa = Rsa::generate(4096).unwrap();

    let priv_pem = rsa.private_key_to_pem()?;

    use nix::sys::stat::Mode;

    tools::file_set_contents(
        &priv_path, &priv_pem, Some(Mode::from_bits_truncate(0o0600)))?;


    let public_pem = rsa.public_key_to_pem()?;

    tools::file_set_contents(&public_path, &public_pem, None)?;

    Ok(())
}

pub fn csrf_secret() -> &'static [u8] {

    lazy_static! {
        static ref SECRET: Vec<u8> =
            tools::file_get_contents(configdir!("/csrf.key")).unwrap();
    }

    &SECRET
}

fn load_private_auth_key() -> Result<PKey<Private>, Error> {

    let pem = tools::file_get_contents(configdir!("/authkey.key"))?;
    let rsa = Rsa::private_key_from_pem(&pem)?;
    let key = PKey::from_rsa(rsa)?;

    Ok(key)
}

pub fn private_auth_key() -> &'static PKey<Private> {

    lazy_static! {
        static ref KEY: PKey<Private> = load_private_auth_key().unwrap();
    }

    &KEY
}

fn load_public_auth_key() -> Result<PKey<Public>, Error> {

    let pem = tools::file_get_contents(configdir!("/authkey.pub"))?;
    let rsa = Rsa::public_key_from_pem(&pem)?;
    let key = PKey::from_rsa(rsa)?;

    Ok(key)
}

pub fn public_auth_key() -> &'static PKey<Public> {

    lazy_static! {
        static ref KEY: PKey<Public> = load_public_auth_key().unwrap();
    }

    &KEY
}
