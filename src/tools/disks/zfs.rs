use std::path::PathBuf;

use anyhow::{bail, Error};
use nom::{
    error::VerboseError,
    bytes::complete::{take_while, take_while1, take_till, take_till1},
    combinator::{map_res, all_consuming, recognize},
    sequence::{preceded, tuple},
    character::complete::{space1, digit1, char, line_ending},
    multi::{many0, many1},
};

use super::*;

type IResult<I, O, E = VerboseError<I>> = Result<(I, O), nom::Err<E>>;

#[derive(Debug)]
pub struct ZFSPoolUsage {
    total: u64,
    used: u64,
    free: u64,
}

#[derive(Debug)]
pub struct ZFSPoolStatus {
    name: String,
    usage: Option<ZFSPoolUsage>,
    devices: Vec<String>,
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

fn parse_optional_u64(i: &str) -> IResult<&str, Option<u64>> {
    if i.starts_with('-') {
        Ok((&i[1..], None))
    } else {
        let (i, value) = map_res(recognize(digit1), str::parse)(i)?;
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
    let (i, (text, total, used, free, _, _eol)) = tuple((
        take_while1(|c| char::is_alphanumeric(c)),
        preceded(multispace1, parse_optional_u64),
        preceded(multispace1, parse_optional_u64),
        preceded(multispace1, parse_optional_u64),
        preceded(space1, take_till(|c| c == '\n')),
        line_ending,
    ))(i)?;

    let status = if let (Some(total), Some(used), Some(free)) = (total, used, free)  {
        ZFSPoolStatus {
            name: text.into(),
            usage: Some(ZFSPoolUsage { total, used, free }),
            devices: Vec::new(),
        }
    } else {
         ZFSPoolStatus {
            name: text.into(), usage: None, devices: Vec::new(),
         }
    };

    Ok((i, status))
}

fn parse_pool_status(i: &str) -> IResult<&str, ZFSPoolStatus> {

    let (i, mut stat) = parse_pool_header(i)?;
    let (i, devices) = many1(parse_pool_device)(i)?;

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
    match all_consuming(many1(parse_pool_status))(i) {
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

/// List devices used by zfs (or a specific zfs pool)
pub fn zfs_devices(pool: Option<&OsStr>) -> Result<Vec<String>, Error> {

    // Note: zpools list  output can include entries for 'special', 'cache' and 'logs'
    // and maybe other things.

    let mut command = std::process::Command::new("/sbin/zpool");
    command.args(&["list", "-H", "-v", "-p", "-P"]);

    if let Some(pool) = pool { command.arg(pool); }

    let output = command.output()
        .map_err(|err| format_err!("failed to execute '/sbin/zpool' - {}", err))?;

    let output = crate::tools::command_output(output, None)
        .map_err(|err| format_err!("zpool list command failed: {}", err))?;

    let list = parse_zfs_list(&output)?;

    let mut done = std::collections::HashSet::new();

    let mut device_list = Vec::new();
    for entry in list {
        for device in entry.devices {
            if !done.contains(&device) {
                device_list.push(device.clone());
                done.insert(device);
            }
        }
    }

    Ok(device_list)
}
