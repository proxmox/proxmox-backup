//! Authentication via a static ticket file
use std::fs::File;
use std::future::Future;
use std::io::prelude::*;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{bail, format_err, Error};
use http::HeaderMap;
use hyper::{Body, Method, Response, StatusCode};

use proxmox_router::UserInformation;

use proxmox_rest_server::AuthError;

const TICKET_FILE: &str = "/ticket";

struct SimpleUserInformation {}

impl UserInformation for SimpleUserInformation {
    fn is_superuser(&self, userid: &str) -> bool {
        userid == "root@pam"
    }
    fn is_group_member(&self, _userid: &str, _group: &str) -> bool {
        false
    }
    fn lookup_privs(&self, _userid: &str, _path: &[&str]) -> u64 {
        0
    }
}

pub fn read_ticket() -> Result<Arc<str>, Error> {
    let mut ticket_file = File::open(TICKET_FILE)?;
    let mut ticket = String::new();
    let len = ticket_file.read_to_string(&mut ticket)?;
    if len == 0 {
        bail!("invalid ticket: cannot be empty");
    }
    Ok(ticket.into())
}

pub fn check_auth<'a>(
    ticket: Arc<str>,
    headers: &'a HeaderMap,
    _method: &'a Method,
) -> Pin<
    Box<
        dyn Future<Output = Result<(String, Box<dyn UserInformation + Sync + Send>), AuthError>>
            + Send
            + 'a,
    >,
> {
    Box::pin(async move {
        match headers.get(hyper::header::AUTHORIZATION) {
            Some(header) if header.to_str().unwrap_or("") == &*ticket => {
                let user_info: Box<dyn UserInformation + Send + Sync> =
                    Box::new(SimpleUserInformation {});
                Ok((String::from("root@pam"), user_info))
            }
            _ => Err(AuthError::Generic(format_err!(
                "invalid file restore ticket provided"
            ))),
        }
    })
}

pub fn get_index() -> Pin<Box<dyn Future<Output = http::Response<Body>> + Send>> {
    Box::pin(async move {
        let index = "<center><h1>Proxmox Backup Restore Daemon/h1></center>";

        Response::builder()
            .status(StatusCode::OK)
            .header(hyper::header::CONTENT_TYPE, "text/html")
            .body(index.into())
            .unwrap()
    })
}
