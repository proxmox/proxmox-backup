use serde::{Deserialize, Serialize};

use proxmox_schema::api;

#[api]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// General status information about a running VM file-restore daemon
pub struct RestoreDaemonStatus {
    /// VM uptime in seconds
    pub uptime: i64,
    /// time left until auto-shutdown, keep in mind that this is useless when 'keep-timeout' is
    /// not set, as then the status call will have reset the timer before returning the value
    pub timeout: i64,
}

#[api]
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
/// The desired format of the result.
pub enum FileRestoreFormat {
    /// Plain file (only works for single files)
    Plain,
    /// PXAR archive
    Pxar,
    /// ZIP archive
    Zip,
    /// TAR archive
    Tar,
}
