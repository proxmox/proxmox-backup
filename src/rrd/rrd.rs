use std::io::Read;
use std::path::Path;

use anyhow::{bail, Error};

use crate::api2::types::{RRDMode, RRDTimeFrameResolution};

pub const RRD_DATA_ENTRIES: usize = 70;

#[repr(C)]
#[derive(Default, Copy, Clone)]
struct RRDEntry {
    max: f64,
    average: f64,
    count: u64,
}

#[repr(C)]
// Note: Avoid alignment problems by using 8byte types only
pub struct RRD {
    last_update: u64,
    hour: [RRDEntry; RRD_DATA_ENTRIES],
    day: [RRDEntry; RRD_DATA_ENTRIES],
    week: [RRDEntry; RRD_DATA_ENTRIES],
    month: [RRDEntry; RRD_DATA_ENTRIES],
    year: [RRDEntry; RRD_DATA_ENTRIES],
}

impl RRD {

    pub fn new() -> Self {
        Self {
            last_update: 0,
            hour: [RRDEntry::default(); RRD_DATA_ENTRIES],
            day: [RRDEntry::default(); RRD_DATA_ENTRIES],
            week: [RRDEntry::default(); RRD_DATA_ENTRIES],
            month: [RRDEntry::default(); RRD_DATA_ENTRIES],
            year: [RRDEntry::default(); RRD_DATA_ENTRIES],
        }
    }

    pub fn extract_data(
        &self,
        epoch: u64,
        timeframe: RRDTimeFrameResolution,
        mode: RRDMode,
    ) -> (u64, u64, Vec<Option<f64>>) {

        let reso = timeframe as u64;

        let end = reso*(epoch/reso + 1);
        let start = end - reso*(RRD_DATA_ENTRIES as u64);

        let rrd_end = reso*(self.last_update/reso);
        let rrd_start = rrd_end - reso*(RRD_DATA_ENTRIES as u64);

        let mut list = Vec::new();

        let data = match timeframe {
            RRDTimeFrameResolution::Hour => &self.hour,
            RRDTimeFrameResolution::Day => &self.day,
            RRDTimeFrameResolution::Week => &self.week,
            RRDTimeFrameResolution::Month => &self.month,
            RRDTimeFrameResolution::Year => &self.year,
        };

        let mut t = start;
        let mut index = ((t/reso) % (RRD_DATA_ENTRIES as u64)) as usize;
        for _ in 0..RRD_DATA_ENTRIES {
            if t < rrd_start || t > rrd_end {
                list.push(None);
            } else {
                let entry = data[index];
                if entry.count == 0 {
                    list.push(None);
                } else {
                    let value = match mode {
                        RRDMode::Max => entry.max,
                        RRDMode::Average => entry.average,
                    };
                    list.push(Some(value));
                }
            }
            t += reso; index = (index + 1) % RRD_DATA_ENTRIES;
        }

        (start, reso, list.into())
    }

    pub fn from_raw(mut raw: &[u8]) -> Result<Self, Error> {
        let expected_len = std::mem::size_of::<RRD>();
        if raw.len() != expected_len {
            bail!("RRD::from_raw failed - wrong data size ({} != {})", raw.len(), expected_len);
        }

        let mut rrd: RRD = unsafe { std::mem::zeroed() };
        unsafe {
            let rrd_slice = std::slice::from_raw_parts_mut(&mut rrd as *mut _ as *mut u8, expected_len);
            raw.read_exact(rrd_slice)?;
        }

        Ok(rrd)
    }

    pub fn load(filename: &Path) -> Result<Self, Error> {
        let raw = proxmox::tools::fs::file_get_contents(filename)?;
        Self::from_raw(&raw)
    }

    pub fn save(&self, filename: &Path) -> Result<(), Error> {
        use proxmox::tools::{fs::replace_file, fs::CreateOptions};

        let rrd_slice = unsafe {
            std::slice::from_raw_parts(self as *const _ as *const u8, std::mem::size_of::<RRD>())
        };

        let backup_user = crate::backup::backup_user()?;
        let mode = nix::sys::stat::Mode::from_bits_truncate(0o0644);
        // set the correct owner/group/permissions while saving file
        // owner(rw) = backup, group(r)= backup
        let options = CreateOptions::new()
            .perm(mode)
            .owner(backup_user.uid)
            .group(backup_user.gid);

        replace_file(filename, rrd_slice, options)?;

        Ok(())
    }

    fn compute_new_value(
        data: &[RRDEntry; RRD_DATA_ENTRIES],
        index: usize,
        value: f64,
    ) -> RRDEntry {
        let RRDEntry { max, average, count } = data[index];
        let new_count = count + 1; // fixme: check overflow?
        if count == 0 {
            RRDEntry { max: value, average: value,  count: 1 }
        } else {
            let new_max = if max > value { max } else { value };
            //let new_average = (average*(count as f64) + value)/(new_count as f64);
            // Note: Try to avoid numeric errors
            let new_average = average*((count as f64)/(new_count as f64))
                + value/(new_count as f64);
            RRDEntry { max: new_max, average: new_average, count: new_count }
        }
    }

    pub fn update(&mut self, epoch: u64, value: f64) {
        // fixme: check time progress (epoch  last)
        let last = self.last_update;

        let reso = RRDTimeFrameResolution::Hour as u64;

        let min_time = epoch - (RRD_DATA_ENTRIES as u64)*reso;
        let mut t = last;
        let mut index = ((t/reso) % (RRD_DATA_ENTRIES as u64)) as usize;
        for _ in 0..RRD_DATA_ENTRIES {
            if t < min_time { self.hour[index] = RRDEntry::default(); }
            t += reso; index = (index + 1) % RRD_DATA_ENTRIES;
        }
        let index = ((epoch/reso) % (RRD_DATA_ENTRIES as u64)) as usize;
        self.hour[index] = Self::compute_new_value(&self.hour, index, value);

        let reso = RRDTimeFrameResolution::Day as u64;
        let min_time = epoch - (RRD_DATA_ENTRIES as u64)*reso;
        let mut t = last;
        let mut index = ((t/reso) % (RRD_DATA_ENTRIES as u64)) as usize;
        for _ in 0..RRD_DATA_ENTRIES {
            if t < min_time { self.day[index] = RRDEntry::default(); }
            t += reso; index = (index + 1) % RRD_DATA_ENTRIES;
        }
        let index = ((epoch/reso) % (RRD_DATA_ENTRIES as u64)) as usize;
        self.day[index] = Self::compute_new_value(&self.day, index, value);

        let reso = RRDTimeFrameResolution::Week as u64;
        let min_time = epoch - (RRD_DATA_ENTRIES as u64)*reso;
        let mut t = last;
        let mut index = ((t/reso) % (RRD_DATA_ENTRIES as u64)) as usize;
        for _ in 0..RRD_DATA_ENTRIES {
            if t < min_time { self.week[index] = RRDEntry::default(); }
            t += reso; index = (index + 1) % RRD_DATA_ENTRIES;
        }
        let index = ((epoch/reso) % (RRD_DATA_ENTRIES as u64)) as usize;
        self.week[index] = Self::compute_new_value(&self.week, index, value);

        let reso = RRDTimeFrameResolution::Month as u64;
        let min_time = epoch - (RRD_DATA_ENTRIES as u64)*reso;
        let mut t = last;
        let mut index = ((t/reso) % (RRD_DATA_ENTRIES as u64)) as usize;
        for _ in 0..RRD_DATA_ENTRIES {
            if t < min_time { self.month[index] = RRDEntry::default(); }
            t += reso; index = (index + 1) % RRD_DATA_ENTRIES;
        }
        let index = ((epoch/reso) % (RRD_DATA_ENTRIES as u64)) as usize;
        self.month[index] = Self::compute_new_value(&self.month, index, value);

        let reso = RRDTimeFrameResolution::Year as u64;
        let min_time = epoch - (RRD_DATA_ENTRIES as u64)*reso;
        let mut t = last;
        let mut index = ((t/reso) % (RRD_DATA_ENTRIES as u64)) as usize;
        for _ in 0..RRD_DATA_ENTRIES {
            if t < min_time { self.year[index] = RRDEntry::default(); }
            t += reso; index = (index + 1) % RRD_DATA_ENTRIES;
        }
        let index = ((epoch/reso) % (RRD_DATA_ENTRIES as u64)) as usize;
        self.year[index] = Self::compute_new_value(&self.year, index, value);

        self.last_update = epoch;
    }
}
