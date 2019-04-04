use failure::*;

use crate::tools;
use lazy_static::lazy_static;
use regex::Regex;

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
