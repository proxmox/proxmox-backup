use std::time::{SystemTime, UNIX_EPOCH};
use std::path::PathBuf;
use std::collections::HashMap;
use std::sync::{RwLock};

use anyhow::{format_err, Error};
use lazy_static::lazy_static;
use serde_json::Value;

use proxmox::tools::fs::{create_path, CreateOptions};

use crate::api2::types::{RRDMode, RRDTimeFrameResolution};

use super::*;

const PBS_RRD_BASEDIR: &str = "/var/lib/proxmox-backup/rrdb";

lazy_static!{
    static ref RRD_CACHE: RwLock<HashMap<String, RRD>> = {
        RwLock::new(HashMap::new())
    };
}

/// Create rrdd stat dir with correct permission
pub fn create_rrdb_dir() -> Result<(), Error> {

    let backup_user = crate::backup::backup_user()?;
    let opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);

    create_path(PBS_RRD_BASEDIR, None, Some(opts))
        .map_err(|err: Error| format_err!("unable to create rrdb stat dir - {}", err))?;

    Ok(())
}

fn now() -> Result<u64, Error> {
    let epoch = SystemTime::now().duration_since(UNIX_EPOCH)?;
    Ok(epoch.as_secs())
}

pub fn update_value(rel_path: &str, value: f64) -> Result<(), Error> {

    let mut path = PathBuf::from(PBS_RRD_BASEDIR);
    path.push(rel_path);

    std::fs::create_dir_all(path.parent().unwrap())?;

    let mut map = RRD_CACHE.write().unwrap();
    let now = now()?;

    if let Some(rrd) = map.get_mut(rel_path) {
        rrd.update(now, value);
        rrd.save(&path)?;
    } else {
        let mut rrd = match RRD::load(&path) {
            Ok(rrd) => rrd,
            Err(_) => RRD::new(),
        };
        rrd.update(now, value);
        rrd.save(&path)?;
        map.insert(rel_path.into(), rrd);
    }

    Ok(())
}

pub fn extract_data(
    rel_path: &str,
    timeframe: RRDTimeFrameResolution,
    mode: RRDMode,
) -> Result<Value, Error> {

    let now = now()?;

    let map = RRD_CACHE.read().unwrap();

    if let Some(rrd) = map.get(rel_path) {
        Ok(rrd.extract_data(now, timeframe, mode))
    } else {
        Ok(RRD::new().extract_data(now, timeframe, mode))
    }
}


pub fn extract_data_list(
    base: &str,
    items: &[&str],
    timeframe: RRDTimeFrameResolution,
    mode: RRDMode,
) -> Result<Value, Error> {

    let now = now()?;

    let map = RRD_CACHE.read().unwrap();

    let mut list: Vec<(&str, &RRD)> = Vec::new();

    let empty_rrd = RRD::new();

    for name in items.iter() {
        if let Some(rrd) = map.get(&format!("{}/{}", base, name)) {
            list.push((name, rrd));
        } else {
            list.push((name, &empty_rrd));
        }
    }

    Ok(extract_rrd_data(&list, now, timeframe, mode))
}
