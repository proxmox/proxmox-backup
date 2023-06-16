use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Error};
use lazy_static::lazy_static;

use proxmox_schema::const_regex;

use super::*;

lazy_static! {
    static ref ZFS_UUIDS: HashSet<&'static str> = {
        let mut set = HashSet::new();
        set.insert("6a898cc3-1dd2-11b2-99a6-080020736631"); // apple
        set.insert("516e7cba-6ecf-11d6-8ff8-00022d09712b"); // bsd
        set
    };
}

fn get_pool_from_dataset(dataset: &str) -> &str {
    if let Some(idx) = dataset.find('/') {
        dataset[0..idx].as_ref()
    } else {
        dataset
    }
}

/// Returns kernel IO-stats for zfs pools
pub fn zfs_pool_stats(pool: &OsStr) -> Result<Option<BlockDevStat>, Error> {
    let mut path = PathBuf::from("/proc/spl/kstat/zfs");
    path.push(pool);
    path.push("io");

    let text = match proxmox_sys::fs::file_read_optional_string(&path)? {
        Some(text) => text,
        None => {
            return Ok(None);
        }
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
    let stat: Vec<u64> = lines[2]
        .split_ascii_whitespace()
        .map(|s| s.parse().unwrap_or_default())
        .collect();

    let ticks = (stat[4] + stat[7]) / 1_000_000; // convert to milisec

    let stat = BlockDevStat {
        read_sectors: stat[0] >> 9,
        write_sectors: stat[1] >> 9,
        read_ios: stat[2],
        write_ios: stat[3],
        io_ticks: ticks,
    };

    Ok(Some(stat))
}

/// Get set of devices used by zfs (or a specific zfs pool)
///
/// The set is indexed by using the unix raw device number (dev_t is u64)
pub fn zfs_devices(lsblk_info: &[LsblkInfo], pool: Option<String>) -> Result<HashSet<u64>, Error> {
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

const ZFS_KSTAT_BASE_PATH: &str = "/proc/spl/kstat/zfs";
const_regex! {
    OBJSET_REGEX = r"^objset-0x[a-fA-F0-9]+$";
}

lazy_static::lazy_static! {
    pub static ref ZFS_DATASET_OBJSET_MAP: Arc<Mutex<HashMap<String, (String, String)>>> =
        Arc::new(Mutex::new(HashMap::new()));
}

// parses /proc/spl/kstat/zfs/POOL/objset-ID files
// they have the following format:
//
// 0 0 0x00 0 0000 00000000000 000000000000000000
// name                            type data
// dataset_name                    7    pool/dataset
// writes                          4    0
// nwritten                        4    0
// reads                           4    0
// nread                           4    0
// nunlinks                        4    0
// nunlinked                       4    0
//
// we are only interested in the dataset_name, writes, nwrites, reads and nread
fn parse_objset_stat(pool: &str, objset_id: &str) -> Result<(String, BlockDevStat), Error> {
    let path = PathBuf::from(format!("{}/{}/{}", ZFS_KSTAT_BASE_PATH, pool, objset_id));

    let text = match proxmox_sys::fs::file_read_optional_string(path)? {
        Some(text) => text,
        None => bail!("could not parse '{}' stat file", objset_id),
    };

    let mut dataset_name = String::new();
    let mut stat = BlockDevStat {
        read_sectors: 0,
        write_sectors: 0,
        read_ios: 0,
        write_ios: 0,
        io_ticks: 0,
    };

    for (i, line) in text.lines().enumerate() {
        if i < 2 {
            continue;
        }

        let mut parts = line.split_ascii_whitespace();
        let name = parts.next();
        parts.next(); // discard type
        let value = parts.next().ok_or_else(|| format_err!("no value found"))?;
        match name {
            Some("dataset_name") => dataset_name = value.to_string(),
            Some("writes") => stat.write_ios = value.parse().unwrap_or_default(),
            Some("nwritten") => stat.write_sectors = value.parse::<u64>().unwrap_or_default() / 512,
            Some("reads") => stat.read_ios = value.parse().unwrap_or_default(),
            Some("nread") => stat.read_sectors = value.parse::<u64>().unwrap_or_default() / 512,
            _ => {}
        }
    }

    Ok((dataset_name, stat))
}

fn get_mapping(dataset: &str) -> Option<(String, String)> {
    ZFS_DATASET_OBJSET_MAP
        .lock()
        .unwrap()
        .get(dataset)
        .map(|c| c.to_owned())
}

/// Updates the dataset <-> objset_map
pub(crate) fn update_zfs_objset_map(pool: &str) -> Result<(), Error> {
    let mut map = ZFS_DATASET_OBJSET_MAP.lock().unwrap();
    map.clear();
    let path = PathBuf::from(format!("{}/{}", ZFS_KSTAT_BASE_PATH, pool));

    proxmox_sys::fs::scandir(
        libc::AT_FDCWD,
        &path,
        &OBJSET_REGEX,
        |_l2_fd, filename, _type| {
            let (name, _) = parse_objset_stat(pool, filename)?;
            map.insert(name, (pool.to_string(), filename.to_string()));
            Ok(())
        },
    )?;

    Ok(())
}

/// Gets io stats for the dataset from /proc/spl/kstat/zfs/POOL/objset-ID
pub fn zfs_dataset_stats(dataset: &str) -> Result<BlockDevStat, Error> {
    let mut mapping = get_mapping(dataset);
    if mapping.is_none() {
        let pool = get_pool_from_dataset(dataset);
        update_zfs_objset_map(pool)?;
        mapping = get_mapping(dataset);
    }
    let (pool, objset_id) =
        mapping.ok_or_else(|| format_err!("could not find objset id for dataset"))?;

    match parse_objset_stat(&pool, &objset_id) {
        Ok((_, stat)) => Ok(stat),
        Err(err) => {
            // on error remove dataset from map, it probably vanished or the
            // mapping was incorrect
            ZFS_DATASET_OBJSET_MAP.lock().unwrap().remove(dataset);
            Err(err)
        }
    }
}
