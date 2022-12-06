use std::fmt;

use anyhow::{format_err, Error};

use pbs_api_types::{Authid, Userid, BACKUP_REPO_URL_REGEX, IP_V6_REGEX};

/// Reference remote backup locations
///

#[derive(Debug)]
pub struct BackupRepository {
    /// The user name used for Authentication
    auth_id: Option<Authid>,
    /// The host name or IP address
    host: Option<String>,
    /// The port
    port: Option<u16>,
    /// The name of the datastore
    store: String,
}

impl BackupRepository {
    pub fn new(
        auth_id: Option<Authid>,
        host: Option<String>,
        port: Option<u16>,
        store: String,
    ) -> Self {
        let host = match host {
            Some(host) if (IP_V6_REGEX.regex_obj)().is_match(&host) => Some(format!("[{}]", host)),
            other => other,
        };
        Self {
            auth_id,
            host,
            port,
            store,
        }
    }

    pub fn auth_id(&self) -> &Authid {
        if let Some(ref auth_id) = self.auth_id {
            return auth_id;
        }

        Authid::root_auth_id()
    }

    pub fn user(&self) -> &Userid {
        if let Some(auth_id) = &self.auth_id {
            return auth_id.user();
        }

        Userid::root_userid()
    }

    pub fn host(&self) -> &str {
        if let Some(ref host) = self.host {
            return host;
        }
        "localhost"
    }

    pub fn port(&self) -> u16 {
        if let Some(port) = self.port {
            return port;
        }
        8007
    }

    pub fn store(&self) -> &str {
        &self.store
    }
}

impl fmt::Display for BackupRepository {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match (&self.auth_id, &self.host, self.port) {
            (Some(auth_id), _, _) => write!(
                f,
                "{}@{}:{}:{}",
                auth_id,
                self.host(),
                self.port(),
                self.store
            ),
            (None, Some(host), None) => write!(f, "{}:{}", host, self.store),
            (None, _, Some(port)) => write!(f, "{}:{}:{}", self.host(), port, self.store),
            (None, None, None) => write!(f, "{}", self.store),
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
        let cap = (BACKUP_REPO_URL_REGEX.regex_obj)()
            .captures(url)
            .ok_or_else(|| format_err!("unable to parse repository url '{}'", url))?;

        Ok(Self {
            auth_id: cap
                .get(1)
                .map(|m| Authid::try_from(m.as_str().to_owned()))
                .transpose()?,
            host: cap.get(2).map(|m| m.as_str().to_owned()),
            port: cap.get(3).map(|m| m.as_str().parse::<u16>()).transpose()?,
            store: cap[4].to_owned(),
        })
    }
}
