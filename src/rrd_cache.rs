use anyhow::{format_err, Error};
use once_cell::sync::OnceCell;

use proxmox::tools::fs::CreateOptions;
use proxmox_rrd::RRDCache;
use proxmox_rrd::rrd::{DST, CF};

use pbs_api_types::{RRDMode, RRDTimeFrame};

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
    timeframe: RRDTimeFrame,
    mode: RRDMode,
) ->  Result<Option<(u64, u64, Vec<Option<f64>>)>, Error> {

    let end = proxmox_time::epoch_f64() as u64;

    let (start, resolution) = match timeframe {
        RRDTimeFrame::Hour => (end - 3600, 60),
        RRDTimeFrame::Day => (end - 3600*24, 60),
        RRDTimeFrame::Week => (end - 3600*24*7, 30*60),
        RRDTimeFrame::Month => (end - 3600*24*30, 30*60),
        RRDTimeFrame::Year => (end - 3600*24*365, 6*60*60),
        RRDTimeFrame::Decade => (end - 10*3600*24*366, 7*86400),
    };

    let cf = match mode {
        RRDMode::Max => CF::Maximum,
        RRDMode::Average => CF::Average,
    };

    let rrd_cache = get_rrd_cache()?;

    rrd_cache.extract_cached_data(basedir, name, cf, resolution, Some(start), Some(end))
}

/// Update RRD Gauge values
pub fn rrd_update_gauge(name: &str, value: f64) {
    if let Ok(rrd_cache) = get_rrd_cache() {
        if let Err(err) = rrd_cache.update_value(name, value, DST::Gauge) {
            log::error!("rrd::update_value '{}' failed - {}", name, err);
        }
    }
}

/// Update RRD Derive values
pub fn rrd_update_derive(name: &str, value: f64) {
    if let Ok(rrd_cache) = get_rrd_cache() {
        if let Err(err) = rrd_cache.update_value(name, value, DST::Derive) {
            log::error!("rrd::update_value '{}' failed - {}", name, err);
        }
    }
}
