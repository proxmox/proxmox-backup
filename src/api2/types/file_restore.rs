use serde::{Deserialize, Serialize};
use proxmox::api::api;

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// General status information about a running VM file-restore daemon
pub struct RestoreDaemonStatus {
    /// VM uptime in seconds
    pub uptime: i64,
}

