use std::io::Read;

use anyhow::Error;
use bitflags::bitflags;

/// The number of data entries per RRA
pub const RRD_DATA_ENTRIES: usize = 70;

/// Proxmox RRD file magic number
// openssl::sha::sha256(b"Proxmox Round Robin Database file v1.0")[0..8];
pub const PROXMOX_RRD_MAGIC_1_0: [u8; 8] = [206, 46, 26, 212, 172, 158, 5, 186];

use crate::rrd::{DataSource, CF, DST, RRA, RRD};

bitflags! {
    /// Flags to specify the data source type and consolidation function
    pub struct RRAFlags: u64 {
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

/// Round Robin Archive with [RRD_DATA_ENTRIES] data slots.
///
/// This data structure is used inside [RRD] and directly written to the
/// RRD files.
#[repr(C)]
pub struct RRAv1 {
    /// Defined the data source type and consolidation function
    pub flags: RRAFlags,
    /// Resolution (seconds)
    pub resolution: u64,
    /// Last update time (epoch)
    pub last_update: f64,
    /// Count values computed inside this update interval
    pub last_count: u64,
    /// Stores the last value, used to compute differential value for derive/counters
    pub counter_value: f64,
    /// Data slots
    pub data: [f64; RRD_DATA_ENTRIES],
}

impl RRAv1 {
    fn extract_data(&self) -> (u64, u64, Vec<Option<f64>>) {
        let reso = self.resolution;

        let mut list = Vec::new();

        let rra_end = reso * ((self.last_update as u64) / reso);
        let rra_start = rra_end - reso * (RRD_DATA_ENTRIES as u64);

        let mut t = rra_start;
        let mut index = ((t / reso) % (RRD_DATA_ENTRIES as u64)) as usize;
        for _ in 0..RRD_DATA_ENTRIES {
            let value = self.data[index];
            if value.is_nan() {
                list.push(None);
            } else {
                list.push(Some(value));
            }

            t += reso;
            index = (index + 1) % RRD_DATA_ENTRIES;
        }

        (rra_start, reso, list)
    }
}

/// Round Robin Database file format with fixed number of [RRA]s
#[repr(C)]
// Note: Avoid alignment problems by using 8byte types only
pub struct RRDv1 {
    /// The magic number to identify the file type
    pub magic: [u8; 8],
    /// Hourly data (average values)
    pub hour_avg: RRAv1,
    /// Hourly data (maximum values)
    pub hour_max: RRAv1,
    /// Dayly data (average values)
    pub day_avg: RRAv1,
    /// Dayly data (maximum values)
    pub day_max: RRAv1,
    /// Weekly data (average values)
    pub week_avg: RRAv1,
    /// Weekly data (maximum values)
    pub week_max: RRAv1,
    /// Monthly data (average values)
    pub month_avg: RRAv1,
    /// Monthly data (maximum values)
    pub month_max: RRAv1,
    /// Yearly data (average values)
    pub year_avg: RRAv1,
    /// Yearly data (maximum values)
    pub year_max: RRAv1,
}

impl RRDv1 {
    pub fn from_raw(mut raw: &[u8]) -> Result<Self, std::io::Error> {
        let expected_len = std::mem::size_of::<RRDv1>();

        if raw.len() != expected_len {
            let msg = format!("wrong data size ({} != {})", raw.len(), expected_len);
            return Err(std::io::Error::new(std::io::ErrorKind::Other, msg));
        }

        let mut rrd: RRDv1 = unsafe { std::mem::zeroed() };
        unsafe {
            let rrd_slice =
                std::slice::from_raw_parts_mut(&mut rrd as *mut _ as *mut u8, expected_len);
            raw.read_exact(rrd_slice)?;
        }

        if rrd.magic != PROXMOX_RRD_MAGIC_1_0 {
            let msg = "wrong magic number".to_string();
            return Err(std::io::Error::new(std::io::ErrorKind::Other, msg));
        }

        Ok(rrd)
    }

    pub fn to_rrd_v2(&self) -> Result<RRD, Error> {
        let mut rra_list = Vec::new();

        // old format v1:
        //
        // hour      1 min,   70 points
        // day      30 min,   70 points
        // week      3 hours, 70 points
        // month    12 hours, 70 points
        // year      1 week,  70 points
        //
        // new default for RRD v2:
        //
        // day      1 min,      1440 points
        // month   30 min,      1440 points
        // year   365 min (6h), 1440 points
        // decade   1 week,      570 points

        // Linear extrapolation
        fn extrapolate_data(
            start: u64,
            reso: u64,
            factor: u64,
            data: Vec<Option<f64>>,
        ) -> (u64, u64, Vec<Option<f64>>) {
            let mut new = Vec::new();

            for i in 0..data.len() {
                let mut next = i + 1;
                if next >= data.len() {
                    next = 0
                };
                let v = data[i];
                let v1 = data[next];
                match (v, v1) {
                    (Some(v), Some(v1)) => {
                        let diff = (v1 - v) / (factor as f64);
                        for j in 0..factor {
                            new.push(Some(v + diff * (j as f64)));
                        }
                    }
                    (Some(v), None) => {
                        new.push(Some(v));
                        for _ in 0..factor - 1 {
                            new.push(None);
                        }
                    }
                    (None, Some(v1)) => {
                        for _ in 0..factor - 1 {
                            new.push(None);
                        }
                        new.push(Some(v1));
                    }
                    (None, None) => {
                        for _ in 0..factor {
                            new.push(None);
                        }
                    }
                }
            }

            (start, reso / factor, new)
        }

        // Try to convert to new, higher capacity format

        // compute daily average (merge old self.day_avg and self.hour_avg
        let mut day_avg = RRA::new(CF::Average, 60, 1440);

        let (start, reso, data) = self.day_avg.extract_data();
        let (start, reso, data) = extrapolate_data(start, reso, 30, data);
        day_avg.insert_data(start, reso, data)?;

        let (start, reso, data) = self.hour_avg.extract_data();
        day_avg.insert_data(start, reso, data)?;

        // compute daily maximum (merge old self.day_max and self.hour_max
        let mut day_max = RRA::new(CF::Maximum, 60, 1440);

        let (start, reso, data) = self.day_max.extract_data();
        let (start, reso, data) = extrapolate_data(start, reso, 30, data);
        day_max.insert_data(start, reso, data)?;

        let (start, reso, data) = self.hour_max.extract_data();
        day_max.insert_data(start, reso, data)?;

        // compute monthly average (merge old self.month_avg,
        // self.week_avg and self.day_avg)
        let mut month_avg = RRA::new(CF::Average, 30 * 60, 1440);

        let (start, reso, data) = self.month_avg.extract_data();
        let (start, reso, data) = extrapolate_data(start, reso, 24, data);
        month_avg.insert_data(start, reso, data)?;

        let (start, reso, data) = self.week_avg.extract_data();
        let (start, reso, data) = extrapolate_data(start, reso, 6, data);
        month_avg.insert_data(start, reso, data)?;

        let (start, reso, data) = self.day_avg.extract_data();
        month_avg.insert_data(start, reso, data)?;

        // compute monthly maximum (merge old self.month_max,
        // self.week_max and self.day_max)
        let mut month_max = RRA::new(CF::Maximum, 30 * 60, 1440);

        let (start, reso, data) = self.month_max.extract_data();
        let (start, reso, data) = extrapolate_data(start, reso, 24, data);
        month_max.insert_data(start, reso, data)?;

        let (start, reso, data) = self.week_max.extract_data();
        let (start, reso, data) = extrapolate_data(start, reso, 6, data);
        month_max.insert_data(start, reso, data)?;

        let (start, reso, data) = self.day_max.extract_data();
        month_max.insert_data(start, reso, data)?;

        // compute yearly average (merge old self.year_avg)
        let mut year_avg = RRA::new(CF::Average, 6 * 3600, 1440);

        let (start, reso, data) = self.year_avg.extract_data();
        let (start, reso, data) = extrapolate_data(start, reso, 28, data);
        year_avg.insert_data(start, reso, data)?;

        // compute yearly maximum (merge old self.year_avg)
        let mut year_max = RRA::new(CF::Maximum, 6 * 3600, 1440);

        let (start, reso, data) = self.year_max.extract_data();
        let (start, reso, data) = extrapolate_data(start, reso, 28, data);
        year_max.insert_data(start, reso, data)?;

        // compute decade average (merge old self.year_avg)
        let mut decade_avg = RRA::new(CF::Average, 7 * 86400, 570);
        let (start, reso, data) = self.year_avg.extract_data();
        decade_avg.insert_data(start, reso, data)?;

        // compute decade maximum (merge old self.year_max)
        let mut decade_max = RRA::new(CF::Maximum, 7 * 86400, 570);
        let (start, reso, data) = self.year_max.extract_data();
        decade_max.insert_data(start, reso, data)?;

        rra_list.push(day_avg);
        rra_list.push(day_max);
        rra_list.push(month_avg);
        rra_list.push(month_max);
        rra_list.push(year_avg);
        rra_list.push(year_max);
        rra_list.push(decade_avg);
        rra_list.push(decade_max);

        // use values from hour_avg for source (all RRAv1 must have the same config)
        let dst = if self.hour_avg.flags.contains(RRAFlags::DST_COUNTER) {
            DST::Counter
        } else if self.hour_avg.flags.contains(RRAFlags::DST_DERIVE) {
            DST::Derive
        } else {
            DST::Gauge
        };

        let source = DataSource {
            dst,
            last_value: f64::NAN,
            last_update: self.hour_avg.last_update, // IMPORTANT!
        };
        Ok(RRD { source, rra_list })
    }
}
