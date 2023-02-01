//! See the different modules for documentation on their usage.
//!
//! The [backup](backup/index.html) module contains some detailed information
//! on the inner workings of the backup server regarding data storage.

use std::path::PathBuf;

use pbs_buildcfg::configdir;
use pbs_tools::cert::CertInfo;

#[macro_use]
pub mod tools;

#[macro_use]
pub mod server;

#[macro_use]
pub mod backup;

pub mod config;

pub mod api2;

pub mod auth_helpers;

pub(crate) mod auth;

pub mod tape;

pub mod acme;

pub mod client_helpers;

pub mod rrd_cache;

pub mod traffic_control_cache;

/// Get the server's certificate info (from `proxy.pem`).
pub fn cert_info() -> Result<CertInfo, anyhow::Error> {
    CertInfo::from_path(PathBuf::from(configdir!("/proxy.pem")))
}
