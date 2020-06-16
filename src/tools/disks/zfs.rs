use std::path::PathBuf;
use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt;

use anyhow::{bail, Error};
use lazy_static::lazy_static;

use nom::{
    error::VerboseError,
    bytes::complete::{take_while, take_while1, take_till, take_till1},
    combinator::{map_res, all_consuming, recognize, opt},
    sequence::{preceded, tuple},
    character::complete::{space1, digit1, char, line_ending},
    multi::{many0},
};

use super::*;

lazy_static!{
    static ref ZFS_UUIDS: HashSet<&'static str> = {
        let mut set = HashSet::new();
	set.insert("6a898cc3-1dd2-11b2-99a6-080020736631"); // apple
	set.insert("516e7cba-6ecf-11d6-8ff8-00022d09712b"); // bsd
        set
    };
}

type IResult<I, O, E = VerboseError<I>> = Result<(I, O), nom::Err<E>>;

#[derive(Debug)]
pub struct ZFSPoolUsage {
    pub size: u64,
    pub alloc: u64,
    pub free: u64,
    pub dedup: f64,
    pub frag: u64,
}

#[derive(Debug)]
pub struct ZFSPoolStatus {
    pub name: String,
    pub health: String,
    pub usage: Option<ZFSPoolUsage>,
    pub devices: Vec<String>,
}

/// Returns kernel IO-stats for zfs pools
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

/// Recognizes zero or more spaces and tabs (but not carage returns or line feeds)
fn multispace0(i: &str)  -> IResult<&str, &str> {
    take_while(|c| c == ' ' || c == '\t')(i)
}

/// Recognizes one or more spaces and tabs (but not carage returns or line feeds)
fn multispace1(i: &str)  -> IResult<&str, &str> {
    take_while1(|c| c == ' ' || c == '\t')(i)
}

/// Recognizes one or more non-whitespace-characters
fn notspace1(i: &str)  -> IResult<&str, &str> {
    take_while1(|c| !(c == ' ' || c == '\t' || c == '\n'))(i)
}

fn parse_optional_u64(i: &str) -> IResult<&str, Option<u64>> {
    if i.starts_with('-') {
        Ok((&i[1..], None))
    } else {
        let (i, value) = map_res(recognize(digit1), str::parse)(i)?;
        Ok((i, Some(value)))
    }
}

fn parse_optional_f64(i: &str) -> IResult<&str, Option<f64>> {
    if i.starts_with('-') {
        Ok((&i[1..], None))
    } else {
        let (i, value) = nom::number::complete::double(i)?;
        Ok((i, Some(value)))
    }
}

fn parse_pool_device(i: &str) -> IResult<&str, String> {
    let (i, (device, _, _rest)) = tuple((
        preceded(multispace1, take_till1(|c| c == ' ' || c == '\t')),
        multispace1,
        preceded(take_till(|c| c == '\n'), char('\n')),
    ))(i)?;

    Ok((i, device.to_string()))
}

fn parse_pool_header(i: &str) -> IResult<&str, ZFSPoolStatus> {
    // name, size, allocated, free, checkpoint, expandsize, fragmentation, capacity, dedupratio, health, altroot.

    let (i, (text, size, alloc, free, _, _,
             frag, _, dedup, health,
             _, _eol)) = tuple((
        take_while1(|c| char::is_alphanumeric(c)), // name
        preceded(multispace1, parse_optional_u64), // size
        preceded(multispace1, parse_optional_u64), // allocated
        preceded(multispace1, parse_optional_u64), // free
        preceded(multispace1, notspace1), // checkpoint
        preceded(multispace1, notspace1), // expandsize
        preceded(multispace1, parse_optional_u64), // fragmentation
        preceded(multispace1, notspace1), // capacity
        preceded(multispace1, parse_optional_f64), // dedup
        preceded(multispace1, notspace1), // health
        opt(preceded(space1, take_till(|c| c == '\n'))), // skip rest
        line_ending,
    ))(i)?;

    let status = if let (Some(size), Some(alloc), Some(free), Some(frag), Some(dedup)) = (size, alloc, free, frag, dedup)  {
        ZFSPoolStatus {
            name: text.into(),
            health: health.into(),
            usage: Some(ZFSPoolUsage { size, alloc, free, frag, dedup }),
            devices: Vec::new(),
        }
    } else {
         ZFSPoolStatus {
             name: text.into(),
             health: health.into(),
             usage: None,
             devices: Vec::new(),
         }
    };

    Ok((i, status))
}

fn parse_pool_status(i: &str) -> IResult<&str, ZFSPoolStatus> {

    let (i, mut stat) = parse_pool_header(i)?;
    let (i, devices) = many0(parse_pool_device)(i)?;

    for device_path in devices.into_iter().filter(|n| n.starts_with("/dev/")) {
        stat.devices.push(device_path);
    }

    let (i, _) = many0(tuple((multispace0, char('\n'))))(i)?; // skip empty lines

    Ok((i, stat))
}

/// Parse zpool list outout
///
/// Note: This does not reveal any details on how the pool uses the devices, because
/// the zpool list output format is not really defined...
pub fn parse_zfs_list(i: &str) -> Result<Vec<ZFSPoolStatus>, Error> {
    match all_consuming(many0(parse_pool_status))(i) {
        Err(nom::Err::Error(err)) |
        Err(nom::Err::Failure(err)) => {
            bail!("unable to parse zfs list output - {}", nom::error::convert_error(i, err));
        }
        Err(err) => {
            bail!("unable to parse calendar event: {}", err);
        }
        Ok((_, ce)) => Ok(ce),
    }
}

/// Get set of devices used by zfs (or a specific zfs pool)
///
/// The set is indexed by using the unix raw device number (dev_t is u64)
pub fn zfs_devices(
    partition_type_map: &HashMap<String, Vec<String>>,
    pool: Option<&OsStr>,
) -> Result<HashSet<u64>, Error> {

    // Note: zpools list  output can include entries for 'special', 'cache' and 'logs'
    // and maybe other things.

    let mut command = std::process::Command::new("/sbin/zpool");
    command.args(&["list", "-H", "-v", "-p", "-P"]);

    if let Some(pool) = pool { command.arg(pool); }

    let output = crate::tools::run_command(command, None)?;

    let list = parse_zfs_list(&output)?;

    let mut device_set = HashSet::new();
    for entry in list {
        for device in entry.devices {
            let meta = std::fs::metadata(device)?;
            device_set.insert(meta.rdev());
        }
    }

    for device_list in partition_type_map.iter()
        .filter_map(|(uuid, list)| if ZFS_UUIDS.contains(uuid.as_str()) { Some(list) } else { None })
    {
        for device in device_list {
            let meta = std::fs::metadata(device)?;
            device_set.insert(meta.rdev());
        }
    }

    Ok(device_set)
}
