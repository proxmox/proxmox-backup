use failure::*;

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader};
use crate::tools;
use lazy_static::lazy_static;
use regex::Regex;
use libc;

/// POSIX sysconf call
pub fn sysconf(name: i32) -> i64 {
    extern { fn sysconf(name: i32) -> i64; }
    unsafe { sysconf(name) }
}

lazy_static! {
    static ref CLOCK_TICKS: f64 = sysconf(libc::_SC_CLK_TCK) as f64;
}

pub struct ProcFsPidStat {
    pub status: u8,
    pub utime: u64,
    pub stime: u64,
    pub starttime: u64,
    pub vsize: u64,
    pub rss: i64,
}

pub fn read_proc_pid_stat(pid: libc::pid_t) -> Result<ProcFsPidStat, Error> {

    let statstr = tools::file_read_firstline(format!("/proc/{}/stat", pid))?;

    lazy_static! {
        static ref REGEX: Regex = Regex::new(concat!(
            r"^(?P<pid>\d+) \(.*\) (?P<status>\S) -?\d+ -?\d+ -?\d+ -?\d+ -?\d+ \d+ \d+ \d+ \d+ \d+ ",
            r"(?P<utime>\d+) (?P<stime>\d+) -?\d+ -?\d+ -?\d+ -?\d+ -?\d+ 0 ",
            r"(?P<starttime>\d+) (?P<vsize>\d+) (?P<rss>-?\d+) ",
            r"\d+ \d+ \d+ \d+ \d+ \d+ \d+ \d+ \d+ \d+ \d+ \d+ \d+ -?\d+ -?\d+ \d+ \d+ \d+"
        )).unwrap();
    }

    if let Some(cap) = REGEX.captures(&statstr) {
        if pid != cap["pid"].parse::<i32>().unwrap() {
            bail!("unable to read pid stat for process '{}' - got wrong pid", pid);
        }

	return Ok(ProcFsPidStat {
	    status: cap["status"].as_bytes()[0],
	    utime: cap["utime"].parse::<u64>().unwrap(),
	    stime: cap["stime"].parse::<u64>().unwrap(),
	    starttime: cap["starttime"].parse::<u64>().unwrap(),
	    vsize: cap["vsize"].parse::<u64>().unwrap(),
	    rss: cap["rss"].parse::<i64>().unwrap() * 4096,
	});

    }

    bail!("unable to read pid stat for process '{}'", pid);
}

pub fn read_proc_starttime(pid: libc::pid_t) -> Result<u64, Error> {

    let info = read_proc_pid_stat(pid)?;

    Ok(info.starttime)
}

pub fn check_process_running(pid: libc::pid_t) -> Option<ProcFsPidStat> {
    if let Ok(info) = read_proc_pid_stat(pid) {
	if info.status != 'Z' as u8 {
	    return Some(info);
	}
    }
    None
}

pub fn check_process_running_pstart(pid: libc::pid_t, pstart: u64) -> Option<ProcFsPidStat> {
    if let Some(info) = check_process_running(pid) {
	if info.starttime == pstart {
	    return Some(info);
	}
    }
    None
}

pub fn read_proc_uptime() -> Result<(f64, f64), Error> {
    let file = "/proc/uptime";
    let line = tools::file_read_firstline(&file)?;
    let mut values = line.split_whitespace().map(|v| v.parse::<f64>());

    match (values.next(), values.next()) {
	(Some(Ok(up)), Some(Ok(idle))) => return Ok((up, idle)),
	_ => bail!("Error while parsing '{}'", file),
    }
}

pub fn read_proc_uptime_ticks() -> Result<(u64, u64), Error> {
    let (mut up, mut idle) = read_proc_uptime()?;
    up *= *CLOCK_TICKS;
    idle *= *CLOCK_TICKS;
    Ok((up as u64, idle as u64))
}

#[derive(Debug)]
pub struct ProcFsMemStat {
    pub memtotal: u64,
    pub memfree: u64,
    pub memused: u64,
    pub memshared: u64,
    pub swaptotal: u64,
    pub swapfree: u64,
    pub swapused: u64,
}

pub fn read_meminfo() -> Result<ProcFsMemStat, Error> {
    let path = "/proc/meminfo";
    let file = OpenOptions::new().read(true).open(&path)?;

    let mut meminfo = ProcFsMemStat {
	memtotal: 0,
	memfree: 0,
	memused: 0,
	memshared: 0,
	swaptotal: 0,
	swapfree: 0,
	swapused: 0,
    };

    let (mut buffers, mut cached) = (0, 0);
    for line in BufReader::new(&file).lines() {
	let content = line?;
	let mut content_iter = content.split_whitespace();
	if let (Some(key), Some(value)) = (content_iter.next(), content_iter.next()) {
	    match key {
		"MemTotal:" => meminfo.memtotal = value.parse::<u64>()? * 1024,
		"MemFree:" => meminfo.memfree = value.parse::<u64>()? * 1024,
		"SwapTotal:" => meminfo.swaptotal = value.parse::<u64>()? * 1024,
		"SwapFree:" => meminfo.swapfree = value.parse::<u64>()? * 1024,
		"Buffers:" => buffers = value.parse::<u64>()? * 1024,
		"Cached:" => cached = value.parse::<u64>()? * 1024,
		_ => continue,
	    }
	}
    }

    meminfo.memfree += buffers + cached;
    meminfo.memused = meminfo.memtotal - meminfo.memfree;

    meminfo.swapused = meminfo.swaptotal - meminfo.swapfree;

    let spages_line = tools::file_read_firstline("/sys/kernel/mm/ksm/pages_sharing")?;
    meminfo.memshared = spages_line.trim_end().parse::<u64>()? * 4096;

    Ok(meminfo)
}
