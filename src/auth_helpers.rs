use anyhow::{bail, format_err, Error};
use lazy_static::lazy_static;

use openssl::rsa::{Rsa};
use openssl::pkey::{PKey, Public, Private};
use openssl::sha;

use std::path::PathBuf;

use proxmox::tools::fs::{file_get_contents, replace_file, CreateOptions};
use proxmox::try_block;

use pbs_buildcfg::configdir;

use crate::api2::types::Userid;

fn compute_csrf_secret_digest(
    timestamp: i64,
    secret: &[u8],
    userid: &Userid,
) -> String {

    let mut hasher = sha::Sha256::new();
    let data = format!("{:08X}:{}:", timestamp, userid);
    hasher.update(data.as_bytes());
    hasher.update(secret);

    base64::encode_config(&hasher.finish(), base64::STANDARD_NO_PAD)
}

pub fn assemble_csrf_prevention_token(
    secret: &[u8],
    userid: &Userid,
) -> String {

    let epoch = proxmox::tools::time::epoch_i64();

    let digest = compute_csrf_secret_digest(epoch, secret, userid);

    format!("{:08X}:{}", epoch, digest)
}

pub fn verify_csrf_prevention_token(
    secret: &[u8],
    userid: &Userid,
    token: &str,
    min_age: i64,
    max_age: i64,
) -> Result<i64, Error> {

    use std::collections::VecDeque;

    let mut parts: VecDeque<&str> = token.split(':').collect();

    try_block!({

        if parts.len() != 2 {
            bail!("format error - wrong number of parts.");
        }

        let timestamp = parts.pop_front().unwrap();
        let sig = parts.pop_front().unwrap();

        let ttime = i64::from_str_radix(timestamp, 16).
            map_err(|err| format_err!("timestamp format error - {}", err))?;

        let digest = compute_csrf_secret_digest(ttime, secret, userid);

        if digest != sig {
            bail!("invalid signature.");
        }

        let now = proxmox::tools::time::epoch_i64();

        let age = now - ttime;
        if age < min_age {
            bail!("timestamp newer than expected.");
        }

        if age > max_age {
            bail!("timestamp too old.");
        }

        Ok(age)
    }).map_err(|err| format_err!("invalid csrf token - {}", err))
}

pub fn generate_csrf_key() -> Result<(), Error> {

    let path = PathBuf::from(configdir!("/csrf.key"));

    if path.exists() { return Ok(()); }

    let rsa = Rsa::generate(2048).unwrap();

    let pem = rsa.private_key_to_pem()?;

    use nix::sys::stat::Mode;

    let backup_user = crate::backup::backup_user()?;

    replace_file(
        &path,
        &pem,
        CreateOptions::new()
            .perm(Mode::from_bits_truncate(0o0640))
            .owner(nix::unistd::ROOT)
            .group(backup_user.gid),
    )?;

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

    replace_file(
        &priv_path, &priv_pem, CreateOptions::new().perm(Mode::from_bits_truncate(0o0600)))?;

    let public_pem = rsa.public_key_to_pem()?;

    let backup_user = crate::backup::backup_user()?;

    replace_file(
        &public_path,
        &public_pem,
        CreateOptions::new()
            .perm(Mode::from_bits_truncate(0o0640))
            .owner(nix::unistd::ROOT)
            .group(backup_user.gid),
    )?;

    Ok(())
}

pub fn csrf_secret() -> &'static [u8] {

    lazy_static! {
        static ref SECRET: Vec<u8> =
            file_get_contents(configdir!("/csrf.key")).unwrap();
    }

    &SECRET
}

fn load_private_auth_key() -> Result<PKey<Private>, Error> {

    let pem = file_get_contents(configdir!("/authkey.key"))?;
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

    let pem = file_get_contents(configdir!("/authkey.pub"))?;
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
