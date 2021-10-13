//! See the different modules for documentation on their usage.
//!
//! The [backup](backup/index.html) module contains some detailed information
//! on the inner workings of the backup server regarding data storage.

use std::path::PathBuf;

use once_cell::sync::OnceCell;
use anyhow::{format_err, Error};

use proxmox::tools::fs::CreateOptions;

use pbs_api_types::{RRDMode, RRDTimeFrameResolution};
use pbs_buildcfg::configdir;
use pbs_tools::cert::CertInfo;
use proxmox_rrd::{rrd::CF, RRDCache};

#[macro_use]
pub mod tools;

#[macro_use]
pub mod server;

#[macro_use]
pub mod backup;

pub mod config;

pub mod api2;

pub mod auth_helpers;

pub mod auth;

pub mod tape;

pub mod acme;

pub mod client_helpers;

/// Get the server's certificate info (from `proxy.pem`).
pub fn cert_info() -> Result<CertInfo, anyhow::Error> {
    CertInfo::from_path(PathBuf::from(configdir!("/proxy.pem")))
}

pub static RRD_CACHE: OnceCell<RRDCache> = OnceCell::new();

/// Get the RRD cache instance
pub fn get_rrd_cache() -> Result<&'static RRDCache, Error> {
    RRD_CACHE.get().ok_or_else(|| format_err!("RRD cache not initialized!"))
}

/// Initialize the RRD cache instance
///
/// Note: Only a single process must do this (proxmox-backup-proxy)
pub fn initialize_rrd_cache() -> Result<&'static RRDCache, Error> {

    let backup_user = pbs_config::backup_user()?;

    let file_options = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);

    let dir_options = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);

    let apply_interval = 30.0*60.0; // 30 minutes

    let cache = RRDCache::new(
        "/var/lib/proxmox-backup/rrdb",
        Some(file_options),
        Some(dir_options),
        apply_interval,
    )?;

    RRD_CACHE.set(cache)
        .map_err(|_| format_err!("RRD cache already initialized!"))?;

    Ok(RRD_CACHE.get().unwrap())
}

/// Extracts data for the specified time frame from from RRD cache
pub fn extract_rrd_data(
    basedir: &str,
    name: &str,
    timeframe: RRDTimeFrameResolution,
    mode: RRDMode,
) ->  Result<Option<(u64, u64, Vec<Option<f64>>)>, Error> {

    let end = proxmox_time::epoch_f64() as u64;

    let (start, resolution) = match timeframe {
        RRDTimeFrameResolution::Hour => (end - 3600, 60),
        RRDTimeFrameResolution::Day => (end - 3600*24, 60),
        RRDTimeFrameResolution::Week => (end - 3600*24*7, 30*60),
        RRDTimeFrameResolution::Month => (end - 3600*24*30, 30*60),
        RRDTimeFrameResolution::Year => (end - 3600*24*365, 6*60*60),
        RRDTimeFrameResolution::Decade => (end - 10*3600*24*366, 7*86400),
    };

    let cf = match mode {
        RRDMode::Max => CF::Maximum,
        RRDMode::Average => CF::Average,
    };

    let rrd_cache = get_rrd_cache()?;

    rrd_cache.extract_cached_data(basedir, name, cf, resolution, Some(start), Some(end))
}
