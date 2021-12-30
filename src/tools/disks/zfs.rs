use std::path::PathBuf;
use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;

use anyhow::{bail, Error};
use lazy_static::lazy_static;

use super::*;

lazy_static!{
    static ref ZFS_UUIDS: HashSet<&'static str> = {
        let mut set = HashSet::new();
        set.insert("6a898cc3-1dd2-11b2-99a6-080020736631"); // apple
        set.insert("516e7cba-6ecf-11d6-8ff8-00022d09712b"); // bsd
        set
    };
}

/// returns pool from dataset path of the form 'rpool/ROOT/pbs-1'
pub fn get_pool_from_dataset(dataset: &OsStr) -> Option<&OsStr> {
    if let Some(dataset) = dataset.to_str() {
        if let Some(idx) = dataset.find('/') {
            return Some(dataset[0..idx].as_ref());
        }
    }

    None
}

/// Returns kernel IO-stats for zfs pools
pub fn zfs_pool_stats(pool: &OsStr) -> Result<Option<BlockDevStat>, Error> {

    let mut path = PathBuf::from("/proc/spl/kstat/zfs");
    path.push(pool);
    path.push("io");

    let text = match proxmox_sys::fs::file_read_optional_string(&path)? {
        Some(text) => text,
        None => { return Ok(None); }
    };

    let lines: Vec<&str> = text.lines().collect();

    if lines.len() < 3 {
        bail!("unable to parse {:?} - got less than 3 lines", path);
    }

    // https://github.com/openzfs/zfs/blob/master/lib/libspl/include/sys/kstat.h#L578
    // nread    nwritten reads    writes   wtime    wlentime wupdate  rtime    rlentime rupdate  wcnt     rcnt
    // Note: w -> wait (wtime -> wait time)
    // Note: r -> run  (rtime -> run time)
    // All times are nanoseconds
    let stat: Vec<u64> = lines[2].split_ascii_whitespace().map(|s| {
        u64::from_str_radix(s, 10).unwrap_or(0)
    }).collect();

    let ticks = (stat[4] + stat[7])/1_000_000; // convert to milisec

    let stat = BlockDevStat {
        read_sectors: stat[0]>>9,
        write_sectors: stat[1]>>9,
        read_ios: stat[2],
        write_ios: stat[3],
        io_ticks: ticks,
    };

    Ok(Some(stat))
}

/// Get set of devices used by zfs (or a specific zfs pool)
///
/// The set is indexed by using the unix raw device number (dev_t is u64)
pub fn zfs_devices(
    lsblk_info: &[LsblkInfo],
    pool: Option<String>,
) -> Result<HashSet<u64>, Error> {

    let list = zpool_list(pool, true)?;

    let mut device_set = HashSet::new();
    for entry in list {
        for device in entry.devices {
            let meta = std::fs::metadata(device)?;
            device_set.insert(meta.rdev());
        }
    }

    for info in lsblk_info.iter() {
        if let Some(partition_type) = &info.partition_type {
            if ZFS_UUIDS.contains(partition_type.as_str()) {
                let meta = std::fs::metadata(&info.path)?;
                device_set.insert(meta.rdev());
            }
        }
    }

    Ok(device_set)
}

