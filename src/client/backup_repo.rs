use std::convert::TryFrom;
use std::fmt;

use anyhow::{format_err, Error};

use proxmox::api::schema::*;

use crate::api2::types::*;

/// API schema format definition for repository URLs
pub const BACKUP_REPO_URL: ApiStringFormat = ApiStringFormat::Pattern(&BACKUP_REPO_URL_REGEX);

/// Reference remote backup locations
///

#[derive(Debug)]
pub struct BackupRepository {
    /// The user name used for Authentication
    user: Option<Userid>,
    /// The host name or IP address
    host: Option<String>,
    /// The name of the datastore
    store: String,
}

impl BackupRepository {

    pub fn new(user: Option<Userid>, host: Option<String>, store: String) -> Self {
        Self { user, host, store }
    }

    pub fn user(&self) -> &Userid {
        if let Some(ref user) = self.user {
            return &user;
        }
        Userid::root_userid()
    }

    pub fn host(&self) -> &str {
        if let Some(ref host) = self.host {
            return host;
        }
        "localhost"
    }

    pub fn store(&self) -> &str {
        &self.store
    }
}

impl fmt::Display for BackupRepository {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
       if let Some(ref user) = self.user {
           write!(f, "{}@{}:{}", user, self.host(), self.store)
       } else if let Some(ref host) = self.host {
           write!(f, "{}:{}", host, self.store)
       } else {
           write!(f, "{}", self.store)
       }
    }
}

impl std::str::FromStr for BackupRepository {
    type Err = Error;

    /// Parse a repository URL.
    ///
    /// This parses strings like `user@host:datastore`. The `user` and
    /// `host` parts are optional, where `host` defaults to the local
    /// host, and `user` defaults to `root@pam`.
    fn from_str(url: &str) -> Result<Self, Self::Err> {

        let cap = (BACKUP_REPO_URL_REGEX.regex_obj)().captures(url)
            .ok_or_else(|| format_err!("unable to parse repository url '{}'", url))?;

        Ok(Self {
            user: cap.get(1).map(|m| Userid::try_from(m.as_str().to_owned())).transpose()?,
            host: cap.get(2).map(|m| m.as_str().to_owned()),
            store: cap[3].to_owned(),
        })
    }
}
