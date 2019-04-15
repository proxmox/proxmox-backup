use failure::*;

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader};
use std::collections::HashSet;
use std::process;
use std::net::Ipv4Addr;
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
    let path = "/proc/uptime";
    let line = tools::file_read_firstline(&path)?;
    let mut values = line.split_whitespace().map(|v| v.parse::<f64>());

    match (values.next(), values.next()) {
	(Some(Ok(up)), Some(Ok(idle))) => return Ok((up, idle)),
	_ => bail!("Error while parsing '{}'", path),
    }
}

pub fn read_proc_uptime_ticks() -> Result<(u64, u64), Error> {
    let (mut up, mut idle) = read_proc_uptime()?;
    up *= *CLOCK_TICKS;
    idle *= *CLOCK_TICKS;
    Ok((up as u64, idle as u64))
}

#[derive(Debug)]
pub struct ProcFsMemInfo {
    pub memtotal: u64,
    pub memfree: u64,
    pub memused: u64,
    pub memshared: u64,
    pub swaptotal: u64,
    pub swapfree: u64,
    pub swapused: u64,
}

pub fn read_meminfo() -> Result<ProcFsMemInfo, Error> {
    let path = "/proc/meminfo";
    let file = OpenOptions::new().read(true).open(&path)?;

    let mut meminfo = ProcFsMemInfo {
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

#[derive(Clone, Debug)]
pub struct ProcFsCPUInfo {
    pub user_hz: f64,
    pub mhz: f64,
    pub model: String,
    pub hvm: bool,
    pub sockets: usize,
    pub cpus: usize,
}

static CPU_INFO: Option<ProcFsCPUInfo> = None;

pub fn read_cpuinfo() -> Result<ProcFsCPUInfo, Error> {
    if let Some(cpu_info) = &CPU_INFO { return Ok(cpu_info.clone()); }

    let path = "/proc/cpuinfo";
    let file = OpenOptions::new().read(true).open(&path)?;

    let mut cpuinfo = ProcFsCPUInfo {
	user_hz: *CLOCK_TICKS,
	mhz: 0.0,
	model: String::new(),
	hvm: false,
	sockets: 0,
	cpus: 0,
    };

    let mut socket_ids = HashSet::new();
    for line in BufReader::new(&file).lines() {
	let content = line?;
	if content.is_empty() { continue; }
	let mut content_iter = content.split(":");
	match (content_iter.next(), content_iter.next()) {
	    (Some(key), Some(value)) => {
		match key.trim_end() {
		    "processor" => cpuinfo.cpus += 1,
		    "model name" => cpuinfo.model = value.trim().to_string(),
		    "cpu MHz" => cpuinfo.mhz = value.trim().parse::<f64>()?,
		    "flags" => cpuinfo.hvm = value.contains(" vmx ") || value.contains(" svm "),
		    "physical id" => {
			let id = value.trim().parse::<u8>()?;
			socket_ids.insert(id);
		    },
		    _ => continue,
		}
	    },
	    _ => bail!("Error while parsing '{}'", path),
	}
    }
    cpuinfo.sockets = socket_ids.len();

    Ok(cpuinfo)
}

#[derive(Debug)]
pub struct ProcFsMemUsage {
    pub size: u64,
    pub resident: u64,
    pub shared: u64,
}

pub fn read_memory_usage() -> Result<ProcFsMemUsage, Error> {
    let path = format!("/proc/{}/statm", process::id());
    let line = tools::file_read_firstline(&path)?;
    let mut values = line.split_whitespace().map(|v| v.parse::<u64>());

    let ps = 4096;
    match (values.next(), values.next(), values.next()) {
	(Some(Ok(size)), Some(Ok(resident)), Some(Ok(shared))) =>
	    Ok(ProcFsMemUsage {
		size: size * ps,
		resident: resident * ps,
		shared: shared * ps,
	    }),
	_ => bail!("Error while parsing '{}'", path),
    }
}

#[derive(Debug)]
pub struct ProcFsNetDev {
    pub device: String,
    pub receive: u64,
    pub send: u64,
}

pub fn read_proc_net_dev() -> Result<Vec<ProcFsNetDev>, Error> {
    let path = "/proc/net/dev";
    let file = OpenOptions::new().read(true).open(&path)?;

    let mut result = Vec::new();
    for line in BufReader::new(&file).lines().skip(2) {
	let content = line?;
	let mut iter = content.split_whitespace();
	match (iter.next(), iter.next(), iter.skip(7).next()) {
	    (Some(device), Some(receive), Some(send)) => {
		result.push(ProcFsNetDev {
		    device: device[..device.len()-1].to_string(),
		    receive: receive.parse::<u64>()?,
		    send: send.parse::<u64>()?,
		});
	    },
	    _ => bail!("Error while parsing '{}'", path),
	}
    }

    Ok(result)
}

fn hex_nibble(c: u8) -> Result<u8, Error> {
    Ok(match c {
	b'0'..=b'9' => c - b'0',
	b'a'..=b'f' => c - b'a' + 0xa,
	b'A'..=b'F' => c - b'A' + 0xa,
	_ => bail!("not a hex digit: {}", c as char),
    })
}

fn hexstr_to_ipv4addr<T: AsRef<[u8]>>(hex: T) -> Result<Ipv4Addr, Error> {
    let hex = hex.as_ref();
    if hex.len() != 8 {
	bail!("Error while converting hex string to IP address: unexpected string length");
    }

    let mut addr: [u8; 4] = unsafe { std::mem::uninitialized() };
    for i in 0..4 {
	addr[3 - i] = (hex_nibble(hex[i * 2])? << 4) + hex_nibble(hex[i * 2 + 1])?;
    }

    Ok(Ipv4Addr::from(addr))
}

#[derive(Debug)]
pub struct ProcFsNetRoute {
    pub dest: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub mask: Ipv4Addr,
    pub metric: u32,
    pub mtu: u32,
    pub iface: String,
}

pub fn read_proc_net_route() -> Result<Vec<ProcFsNetRoute>, Error> {
    let path = "/proc/net/route";
    let file = OpenOptions::new().read(true).open(&path)?;
    let err = "Error while parsing '/proc/net/route";

    let mut result = Vec::new();
    for line in BufReader::new(&file).lines().skip(1) {
	let content = line?;
	if content.is_empty() { continue; }
	let mut iter = content.split_whitespace();

	let iface = iter.next().ok_or(format_err!("{}", err))?;
	let dest = iter.next().ok_or(format_err!("{}", err))?;
	let gateway = iter.next().ok_or(format_err!("{}", err))?;
	for _ in 0..3 { iter.next(); }
	let metric = iter.next().ok_or(format_err!("{}", err))?;
	let mask = iter.next().ok_or(format_err!("{}", err))?;
	let mtu = iter.next().ok_or(format_err!("{}", err))?;

	result.push(ProcFsNetRoute {
	    dest: hexstr_to_ipv4addr(dest)?,
	    gateway: hexstr_to_ipv4addr(gateway)?,
	    mask: hexstr_to_ipv4addr(mask)?,
	    metric: metric.parse()?,
	    mtu: mtu.parse()?,
	    iface: iface.to_string(),
	});
    }

    Ok(result)
}
