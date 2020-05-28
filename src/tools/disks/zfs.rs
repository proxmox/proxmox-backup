use anyhow::{bail, Error};

use std::path::PathBuf;

use super::*;

pub fn zfs_pool_stats(pool: &OsStr) -> Result<Option<BlockDevStat>, Error> {

    let mut path = PathBuf::from("/proc/spl/kstat/zfs");
    path.push(pool);
    path.push("io");

    let text = match proxmox::tools::fs::file_read_optional_string(&path)? {
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

    let ticks = stat[4]/1_000_000; // convert to milisec

    let stat = BlockDevStat {
        read_sectors: stat[0]>>9,
        write_sectors: stat[1]>>9,
        read_ios: stat[2],
        write_ios: stat[3],
        read_merges: 0, // there is no such info
        write_merges: 0, // there is no such info
        write_ticks: ticks,
        read_ticks: ticks,
    };

    Ok(Some(stat))
}
