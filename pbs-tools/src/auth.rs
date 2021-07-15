//! Helpers for authentication used by both client and server.

use anyhow::Error;
use lazy_static::lazy_static;
use openssl::pkey::{PKey, Private};
use openssl::rsa::Rsa;

use proxmox::tools::fs::file_get_contents;

use pbs_buildcfg::configdir;

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
