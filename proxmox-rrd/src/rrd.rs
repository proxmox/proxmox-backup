use std::io::Read;
use std::path::Path;

use anyhow::Error;

use proxmox::tools::{fs::replace_file, fs::CreateOptions};

use proxmox_rrd_api_types::{RRDMode, RRDTimeFrameResolution};

/// The number of data entries per RRA
pub const RRD_DATA_ENTRIES: usize = 70;

/// Proxmox RRD file magic number
// openssl::sha::sha256(b"Proxmox Round Robin Database file v1.0")[0..8];
pub const PROXMOX_RRD_MAGIC_1_0: [u8; 8] =  [206, 46, 26, 212, 172, 158, 5, 186];

use bitflags::bitflags;

bitflags!{
    struct RRAFlags: u64 {
        // Data Source Types
        const DST_GAUGE  = 1;
        const DST_DERIVE = 2;
        const DST_COUNTER = 4;
        const DST_MASK   = 255; // first 8 bits

        // Consolidation Functions
        const CF_AVERAGE = 1 << 8;
        const CF_MAX     = 2 << 8;
        const CF_MASK    = 255 << 8;
    }
}

/// RRD data source tyoe
pub enum DST {
    Gauge,
    Derive,
}

#[repr(C)]
struct RRA {
    flags: RRAFlags,
    resolution: u64,
    last_update: f64,
    last_count: u64,
    counter_value: f64, // used for derive/counters
    data: [f64; RRD_DATA_ENTRIES],
}

impl RRA {
    fn new(flags: RRAFlags, resolution: u64) -> Self {
        Self {
            flags, resolution,
            last_update: 0.0,
            last_count: 0,
            counter_value: f64::NAN,
            data: [f64::NAN; RRD_DATA_ENTRIES],
        }
    }

    fn delete_old(&mut self, time: f64) {
        let epoch = time as u64;
        let last_update = self.last_update as u64;
        let reso = self.resolution;

        let min_time = epoch - (RRD_DATA_ENTRIES as u64)*reso;
        let min_time = (min_time/reso + 1)*reso;
        let mut t = last_update.saturating_sub((RRD_DATA_ENTRIES as u64)*reso);
        let mut index = ((t/reso) % (RRD_DATA_ENTRIES as u64)) as usize;
        for _ in 0..RRD_DATA_ENTRIES {
            t += reso; index = (index + 1) % RRD_DATA_ENTRIES;
            if t < min_time {
                self.data[index] = f64::NAN;
            } else {
                break;
            }
        }
    }

    fn compute_new_value(&mut self, time: f64, value: f64) {
        let epoch = time as u64;
        let last_update = self.last_update as u64;
        let reso = self.resolution;

        let index = ((epoch/reso) % (RRD_DATA_ENTRIES as u64)) as usize;
        let last_index = ((last_update/reso) % (RRD_DATA_ENTRIES as u64)) as usize;

        if (epoch - (last_update as u64)) > reso || index != last_index {
            self.last_count = 0;
        }

        let last_value = self.data[index];
        if last_value.is_nan() {
            self.last_count = 0;
        }

        let new_count = if self.last_count < u64::MAX {
            self.last_count + 1
        } else {
            u64::MAX // should never happen
        };

        if self.last_count == 0 {
            self.data[index] = value;
            self.last_count = 1;
        } else {
            let new_value = if self.flags.contains(RRAFlags::CF_MAX) {
                if last_value > value { last_value } else { value }
            } else if self.flags.contains(RRAFlags::CF_AVERAGE) {
                (last_value*(self.last_count as f64))/(new_count as f64)
                    + value/(new_count as f64)
            } else {
                eprintln!("rrdb update failed - unknown CF");
                return;
            };
            self.data[index] = new_value;
            self.last_count = new_count;
        }
        self.last_update = time;
    }

    fn update(&mut self, time: f64, mut value: f64) {

        if time <= self.last_update {
            eprintln!("rrdb update failed - time in past ({} < {})", time, self.last_update);
        }

        if value.is_nan() {
            eprintln!("rrdb update failed - new value is NAN");
            return;
        }

        // derive counter value
        if self.flags.intersects(RRAFlags::DST_DERIVE | RRAFlags::DST_COUNTER) {
            let time_diff = time - self.last_update;
            let is_counter = self.flags.contains(RRAFlags::DST_COUNTER);

            let diff = if self.counter_value.is_nan() {
                0.0
            } else if is_counter && value < 0.0 {
                eprintln!("rrdb update failed - got negative value for counter");
                return;
            } else if is_counter && value < self.counter_value {
                // Note: We do not try automatic overflow corrections
                self.counter_value = value;
                eprintln!("rrdb update failed - conter overflow/reset detected");
                return;
            } else {
                value - self.counter_value
            };
            self.counter_value = value;
            value = diff/time_diff;
        }

        self.delete_old(time);
        self.compute_new_value(time, value);
    }
}

#[repr(C)]
// Note: Avoid alignment problems by using 8byte types only
pub struct RRD {
    magic: [u8; 8],
    hour_avg: RRA,
    hour_max: RRA,
    day_avg: RRA,
    day_max: RRA,
    week_avg: RRA,
    week_max: RRA,
    month_avg: RRA,
    month_max: RRA,
    year_avg: RRA,
    year_max: RRA,
}

impl RRD {

    pub fn new(dst: DST) -> Self {
        let flags = match dst {
            DST::Gauge => RRAFlags::DST_GAUGE,
            DST::Derive => RRAFlags::DST_DERIVE,
        };

        Self {
            magic: PROXMOX_RRD_MAGIC_1_0,
            hour_avg: RRA::new(
                flags | RRAFlags::CF_AVERAGE,
                RRDTimeFrameResolution::Hour as u64,
            ),
            hour_max: RRA::new(
                flags |  RRAFlags::CF_MAX,
                RRDTimeFrameResolution::Hour as u64,
            ),
            day_avg: RRA::new(
                flags |  RRAFlags::CF_AVERAGE,
                RRDTimeFrameResolution::Day as u64,
            ),
            day_max: RRA::new(
                flags |  RRAFlags::CF_MAX,
                RRDTimeFrameResolution::Day as u64,
            ),
            week_avg: RRA::new(
                flags |  RRAFlags::CF_AVERAGE,
                RRDTimeFrameResolution::Week as u64,
            ),
            week_max: RRA::new(
                flags |  RRAFlags::CF_MAX,
                RRDTimeFrameResolution::Week as u64,
            ),
            month_avg: RRA::new(
                flags |  RRAFlags::CF_AVERAGE,
                RRDTimeFrameResolution::Month as u64,
            ),
            month_max: RRA::new(
                flags |  RRAFlags::CF_MAX,
                RRDTimeFrameResolution::Month as u64,
            ),
            year_avg: RRA::new(
                flags |  RRAFlags::CF_AVERAGE,
                RRDTimeFrameResolution::Year as u64,
            ),
            year_max: RRA::new(
                flags |  RRAFlags::CF_MAX,
                RRDTimeFrameResolution::Year as u64,
            ),
        }
    }

    pub fn extract_data(
        &self,
        time: f64,
        timeframe: RRDTimeFrameResolution,
        mode: RRDMode,
    ) -> (u64, u64, Vec<Option<f64>>) {

        let epoch = time as u64;
        let reso = timeframe as u64;

        let end = reso*(epoch/reso + 1);
        let start = end - reso*(RRD_DATA_ENTRIES as u64);

        let mut list = Vec::new();

        let raa = match (mode, timeframe) {
            (RRDMode::Average, RRDTimeFrameResolution::Hour) => &self.hour_avg,
            (RRDMode::Max, RRDTimeFrameResolution::Hour) => &self.hour_max,
            (RRDMode::Average, RRDTimeFrameResolution::Day) => &self.day_avg,
            (RRDMode::Max, RRDTimeFrameResolution::Day) => &self.day_max,
            (RRDMode::Average, RRDTimeFrameResolution::Week) => &self.week_avg,
            (RRDMode::Max, RRDTimeFrameResolution::Week) => &self.week_max,
            (RRDMode::Average, RRDTimeFrameResolution::Month) => &self.month_avg,
            (RRDMode::Max, RRDTimeFrameResolution::Month) => &self.month_max,
            (RRDMode::Average, RRDTimeFrameResolution::Year) => &self.year_avg,
            (RRDMode::Max, RRDTimeFrameResolution::Year) => &self.year_max,
        };

        let rrd_end = reso*((raa.last_update as u64)/reso);
        let rrd_start = rrd_end - reso*(RRD_DATA_ENTRIES as u64);

        let mut t = start;
        let mut index = ((t/reso) % (RRD_DATA_ENTRIES as u64)) as usize;
        for _ in 0..RRD_DATA_ENTRIES {
            if t < rrd_start || t > rrd_end {
                list.push(None);
            } else {
                let value = raa.data[index];
                if value.is_nan() {
                    list.push(None);
                } else {
                    list.push(Some(value));
                }
            }
            t += reso; index = (index + 1) % RRD_DATA_ENTRIES;
        }

        (start, reso, list)
    }

    pub fn from_raw(mut raw: &[u8]) -> Result<Self, std::io::Error> {
        let expected_len = std::mem::size_of::<RRD>();
        if raw.len() != expected_len {
            let msg = format!("wrong data size ({} != {})", raw.len(), expected_len);
            return Err(std::io::Error::new(std::io::ErrorKind::Other, msg));
        }

        let mut rrd: RRD = unsafe { std::mem::zeroed() };
        unsafe {
            let rrd_slice = std::slice::from_raw_parts_mut(&mut rrd as *mut _ as *mut u8, expected_len);
            raw.read_exact(rrd_slice)?;
        }

        if rrd.magic != PROXMOX_RRD_MAGIC_1_0 {
            let msg = "wrong magic number".to_string();
            return Err(std::io::Error::new(std::io::ErrorKind::Other, msg));
        }

        Ok(rrd)
    }

    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        let raw = std::fs::read(path)?;
        Self::from_raw(&raw)
    }

    pub fn save(&self, filename: &Path, options: CreateOptions) -> Result<(), Error> {
        let rrd_slice = unsafe {
            std::slice::from_raw_parts(self as *const _ as *const u8, std::mem::size_of::<RRD>())
        };
        replace_file(filename, rrd_slice, options)
    }


    pub fn update(&mut self, time: f64, value: f64) {
        self.hour_avg.update(time, value);
        self.hour_max.update(time, value);

        self.day_avg.update(time, value);
        self.day_max.update(time, value);

        self.week_avg.update(time, value);
        self.week_max.update(time, value);

        self.month_avg.update(time, value);
        self.month_max.update(time, value);

        self.year_avg.update(time, value);
        self.year_max.update(time, value);
    }
}
