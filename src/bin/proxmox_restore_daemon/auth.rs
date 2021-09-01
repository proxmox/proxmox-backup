//! Authentication via a static ticket file
use std::fs::File;
use std::io::prelude::*;

use anyhow::{bail, format_err, Error};

use pbs_api_types::Authid;

use proxmox_backup::config::cached_user_info::CachedUserInfo;
use proxmox_backup::server::auth::{ApiAuth, AuthError};

const TICKET_FILE: &str = "/ticket";

pub struct StaticAuth {
    ticket: String,
}

impl ApiAuth for StaticAuth {
    fn check_auth(
        &self,
        headers: &http::HeaderMap,
        _method: &hyper::Method,
        _user_info: &CachedUserInfo,
    ) -> Result<Authid, AuthError> {
        match headers.get(hyper::header::AUTHORIZATION) {
            Some(header) if header.to_str().unwrap_or("") == &self.ticket => {
                Ok(Authid::root_auth_id().to_owned())
            }
            _ => {
                return Err(AuthError::Generic(format_err!(
                    "invalid file restore ticket provided"
                )));
            }
        }
    }
}

pub fn ticket_auth() -> Result<StaticAuth, Error> {
    let mut ticket_file = File::open(TICKET_FILE)?;
    let mut ticket = String::new();
    let len = ticket_file.read_to_string(&mut ticket)?;
    if len <= 0 {
        bail!("invalid ticket: cannot be empty");
    }
    Ok(StaticAuth { ticket })
}
