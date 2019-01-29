use crate::tools;

use failure::*;

use openssl::rsa::{Rsa};
use std::path::PathBuf;

pub fn generate_csrf_key() -> Result<(), Error> {

    let path = PathBuf::from("/etc/proxmox-backup/csrf.key");

    if path.exists() { return Ok(()); }

    let rsa = Rsa::generate(2048).unwrap();

    let pem = rsa.private_key_to_pem()?;

    use nix::sys::stat::Mode;

    tools::file_set_contents(
        &path, &pem, Some(Mode::from_bits_truncate(0o0640)))?;

    nix::unistd::chown(&path, Some(nix::unistd::ROOT), Some(nix::unistd::Gid::from_raw(33)))?;

    Ok(())
}

pub fn generate_auth_key() -> Result<(), Error> {

    let priv_path = PathBuf::from("/etc/proxmox-backup/authkey.key");

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
