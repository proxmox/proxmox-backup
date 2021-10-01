//! Authentication via a static ticket file
use std::fs::File;
use std::io::prelude::*;
use std::future::Future;
use std::pin::Pin;

use anyhow::{bail, format_err, Error};

use proxmox::api::UserInformation;

use proxmox_rest_server::{ApiAuth, AuthError};

const TICKET_FILE: &str = "/ticket";

struct SimpleUserInformation {}

impl UserInformation for SimpleUserInformation {
    fn is_superuser(&self, userid: &str) -> bool {
        userid == "root@pam"
    }
    fn is_group_member(&self, _userid: &str, _group: &str) -> bool { false }
    fn lookup_privs(&self, _userid: &str, _path: &[&str]) -> u64 { 0 }
}

pub struct StaticAuth {
    ticket: String,
}

impl ApiAuth for StaticAuth {
    fn check_auth<'a>(
        &'a self,
        headers: &'a http::HeaderMap,
        _method: &'a hyper::Method,
    ) -> Pin<Box<dyn Future<Output = Result<(String, Box<dyn UserInformation + Sync + Send>), AuthError>> + Send + 'a>> {
        Box::pin(async move {

            match headers.get(hyper::header::AUTHORIZATION) {
                Some(header) if header.to_str().unwrap_or("") == &self.ticket => {
                    let user_info: Box<dyn UserInformation + Send + Sync> = Box::new(SimpleUserInformation {});
                    Ok((String::from("root@pam"), user_info))
                }
                _ => {
                    return Err(AuthError::Generic(format_err!(
                        "invalid file restore ticket provided"
                    )));
                }
            }
        })
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
