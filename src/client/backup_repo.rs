use failure::*;

use crate::api_schema::*;

use std::sync::Arc;
use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    /// Regular expression to parse repository URLs
    pub static ref BACKUP_REPO_URL_REGEX: Regex =
        Regex::new(r"^(?:(?:([\w@]+)@)?([\w\-_.]+):)?(\w+)$").unwrap();

    /// API schema format definition for repository URLs
    pub static ref BACKUP_REPO_URL: Arc<ApiStringFormat> =
        ApiStringFormat::Pattern(&BACKUP_REPO_URL_REGEX).into();
}

/// Reference remote backup locations
///

#[derive(Debug)]
pub struct BackupRepository {
    /// The user name used for Authentication
    pub user: String,
    /// The host name or IP address
    pub host: String,
    /// The name of the datastore
    pub store: String,
}

impl BackupRepository {

    /// Parse a repository URL.
    ///
    /// This parses strings like `user@host:datastore`. The `user` and
    /// `host` parts are optional, where `host` defaults to the local
    /// host, and `user` defaults to `root@pam`.
    pub fn parse(url: &str) -> Result<Self, Error> {

        let cap = BACKUP_REPO_URL_REGEX.captures(url)
            .ok_or_else(|| format_err!("unable to parse repository url '{}'", url))?;

        Ok(BackupRepository {
            user: cap.get(1).map_or("root@pam", |m| m.as_str()).to_owned(),
            host: cap.get(2).map_or("localhost", |m| m.as_str()).to_owned(),
            store: cap[3].to_owned(),
        })
    }
}
