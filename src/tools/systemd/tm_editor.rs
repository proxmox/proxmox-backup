use anyhow::Error;
use bitflags::bitflags;

use proxmox::tools::time::*;

bitflags!{
    #[derive(Default)]
    pub struct TMChanges: u8 {
        const SEC = 1;
        const MIN = 2;
        const HOUR = 4;
        const MDAY = 8;
        const MON = 16;
        const YEAR = 32;
        const WDAY = 64;
    }
}

pub struct TmEditor {
    utc: bool,
    t: libc::tm,
    changes: TMChanges,
}

fn is_leap_year(year: libc::c_int) -> bool {
    if year % 4 != 0  { return false; }
    if year % 100 != 0 { return true; }
    if year % 400 != 0  { return false; }
    return true;
}

fn days_in_month(mon: libc::c_int, year: libc::c_int) -> libc::c_int {

    let mon = mon % 12;

    static MAP: &[libc::c_int] = &[31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    if mon == 1 && is_leap_year(year) { return 29; }

    MAP[mon as usize]
}

impl TmEditor {

    pub fn new(epoch: i64, utc: bool) -> Result<Self, Error> {
        let mut t = if utc { gmtime(epoch)? } else { localtime(epoch)? };
        t.tm_sec = 0; // we're not interested in seconds, actually
        t.tm_year += 1900; // real years for clarity
        Ok(Self { utc, t, changes: TMChanges::all() })
    }

    pub fn into_epoch(mut self) -> Result<i64, Error> {
        self.t.tm_year -= 1900;
        let epoch = if self.utc { timegm(self.t)? } else { timelocal(self.t)? };
        Ok(epoch)
    }

    pub fn add_days(&mut self, days: libc::c_int, reset_time: bool) {
        if days == 0 { return; }
        if reset_time {
            self.t.tm_hour = 0;
            self.t.tm_min = 0;
            self.t.tm_sec = 0;
            self.changes.insert(TMChanges::HOUR|TMChanges::MIN|TMChanges::SEC);
        }
        self.t.tm_mday += days;
        self.t.tm_wday += days;
        self.changes.insert(TMChanges::MDAY|TMChanges::WDAY);
        self.wrap_time();
    }

    pub fn hour(&self) -> libc::c_int { self.t.tm_hour }
    pub fn min(&self) -> libc::c_int { self.t.tm_min }
    pub fn sec(&self) -> libc::c_int { self.t.tm_sec }

    // Note: tm_wday (0-6, Sunday = 0) => convert to Sunday = 6
    pub fn day_num(&self) -> libc::c_int {
        (self.t.tm_wday + 6) % 7
    }

    pub fn set_time(&mut self, hour: libc::c_int, min: libc::c_int, sec: libc::c_int) {
        self.t.tm_hour = hour;
        self.t.tm_min = min;
        self.t.tm_sec = sec;
        self.changes.insert(TMChanges::HOUR|TMChanges::MIN|TMChanges::SEC);
        self.wrap_time();
    }

    pub fn set_min_sec(&mut self, min: libc::c_int, sec: libc::c_int) {
        self.t.tm_min = min;
        self.t.tm_sec = sec;
        self.changes.insert(TMChanges::MIN|TMChanges::SEC);
        self.wrap_time();
    }

    fn wrap_time(&mut self) {

        // sec: 0..59
        if self.t.tm_sec >= 60 {
            self.t.tm_min += self.t.tm_sec / 60;
            self.t.tm_sec %= 60;
            self.changes.insert(TMChanges::SEC|TMChanges::MIN);
        }

        // min: 0..59
        if self.t.tm_min >= 60 {
            self.t.tm_hour += self.t.tm_min / 60;
            self.t.tm_min %= 60;
            self.changes.insert(TMChanges::MIN|TMChanges::HOUR);
       }

        // hour: 0..23
        if self.t.tm_hour >= 24 {
            self.t.tm_mday += self.t.tm_hour / 24;
            self.t.tm_wday += self.t.tm_hour / 24;
            self.t.tm_hour %= 24;
            self.changes.insert(TMChanges::HOUR|TMChanges::MDAY|TMChanges::WDAY);
        }

        // Translate to 0..($days_in_mon-1)
        self.t.tm_mday -= 1;
        loop {
	    let days_in_mon = days_in_month(self.t.tm_mon, self.t.tm_year);
	    if self.t.tm_mday < days_in_mon { break; }
	    // Wrap one month
	    self.t.tm_mday -= days_in_mon;
            self.t.tm_wday += 7 - (days_in_mon % 7);
	    self.t.tm_mon += 1;
            self.changes.insert(TMChanges::MDAY|TMChanges::WDAY|TMChanges::MON);
        }

        // Translate back to 1..$days_in_mon
        self.t.tm_mday += 1;

        // mon: 0..11
        if self.t.tm_mon >= 12 {
            self.t.tm_year += self.t.tm_mon / 12;
            self.t.tm_mon %= 12;
            self.changes.insert(TMChanges::MON|TMChanges::YEAR);
        }

        self.t.tm_wday %= 7;
    }

    pub fn set_sec(&mut self, v: libc::c_int) {
        self.t.tm_sec = v;
        self.changes.insert(TMChanges::SEC);
        self.wrap_time();
    }

    pub fn set_min(&mut self, v: libc::c_int) {
        self.t.tm_min = v;
        self.changes.insert(TMChanges::MIN);
        self.wrap_time();
    }

    pub fn set_hour(&mut self, v: libc::c_int) {
        self.t.tm_hour = v;
        self.changes.insert(TMChanges::HOUR);
        self.wrap_time();
    }

    pub fn set_mday(&mut self, v: libc::c_int) {
        self.t.tm_mday = v;
        self.changes.insert(TMChanges::MDAY);
        self.wrap_time();
    }

    pub fn set_mon(&mut self, v: libc::c_int) {
        self.t.tm_mon = v;
        self.changes.insert(TMChanges::MON);
        self.wrap_time();
    }

    pub fn set_year(&mut self, v: libc::c_int) {
        self.t.tm_year = v;
        self.changes.insert(TMChanges::YEAR);
        self.wrap_time();
    }

    pub fn set_wday(&mut self, v: libc::c_int) {
        self.t.tm_wday = v;
        self.changes.insert(TMChanges::WDAY);
        self.wrap_time();
    }

}
