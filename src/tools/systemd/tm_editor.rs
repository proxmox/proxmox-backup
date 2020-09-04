use anyhow::Error;

use proxmox::tools::time::*;

pub struct TmEditor {
    utc: bool,
    t: libc::tm,
}

impl TmEditor {

    pub fn new(epoch: i64, utc: bool) -> Result<Self, Error> {
        let mut t = if utc { gmtime(epoch)? } else { localtime(epoch)? };
        t.tm_year += 1900; // real years for clarity
        Ok(Self { utc, t })
    }

    pub fn into_epoch(mut self) -> Result<i64, Error> {
        self.t.tm_year -= 1900;
        let epoch = if self.utc { timegm(&mut self.t)? } else { timelocal(&mut self.t)? };
        Ok(epoch)
    }

    pub fn add_days(&mut self, days: libc::c_int, reset_time: bool) -> Result<(), Error> {
        if days == 0 { return Ok(()); }
        if reset_time {
            self.t.tm_hour = 0;
            self.t.tm_min = 0;
            self.t.tm_sec = 0;
        }
        self.t.tm_mday += days;
        self.t.tm_wday += days;
        self.normalize_time()
    }

    pub fn hour(&self) -> libc::c_int { self.t.tm_hour }
    pub fn min(&self) -> libc::c_int { self.t.tm_min }
    pub fn sec(&self) -> libc::c_int { self.t.tm_sec }

    // Note: tm_wday (0-6, Sunday = 0) => convert to Sunday = 6
    pub fn day_num(&self) -> libc::c_int {
        (self.t.tm_wday + 6) % 7
    }

    pub fn set_time(&mut self, hour: libc::c_int, min: libc::c_int, sec: libc::c_int) -> Result<(), Error> {
        self.t.tm_hour = hour;
        self.t.tm_min = min;
        self.t.tm_sec = sec;
        self.normalize_time()
    }

    pub fn set_min_sec(&mut self, min: libc::c_int, sec: libc::c_int) -> Result<(), Error> {
        self.t.tm_min = min;
        self.t.tm_sec = sec;
        self.normalize_time()
    }

    fn normalize_time(&mut self) -> Result<(), Error> {
        // libc normalizes it for us
        if self.utc {
            timegm(&mut self.t)?;
        } else {
            timelocal(&mut self.t)?;
        }
        Ok(())
    }

    pub fn set_sec(&mut self, v: libc::c_int) -> Result<(), Error> {
        self.t.tm_sec = v;
        self.normalize_time()
    }

    pub fn set_min(&mut self, v: libc::c_int) -> Result<(), Error> {
        self.t.tm_min = v;
        self.normalize_time()
    }

    pub fn set_hour(&mut self, v: libc::c_int) -> Result<(), Error> {
        self.t.tm_hour = v;
        self.normalize_time()
    }

    pub fn set_mday(&mut self, v: libc::c_int) -> Result<(), Error> {
        self.t.tm_mday = v;
        self.normalize_time()
    }

    pub fn set_mon(&mut self, v: libc::c_int) -> Result<(), Error> {
        self.t.tm_mon = v;
        self.normalize_time()
    }

    pub fn set_year(&mut self, v: libc::c_int) -> Result<(), Error> {
        self.t.tm_year = v;
        self.normalize_time()
    }
}
