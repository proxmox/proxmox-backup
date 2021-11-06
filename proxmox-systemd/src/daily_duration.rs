use std::cmp::{Ordering, PartialOrd};
use std::convert::{TryFrom, TryInto};

use anyhow::Error;

use proxmox_time::TmEditor;

use super::time::{WeekDays};

pub use super::parse_time::parse_daily_duration;

/// Time of Day (hour with minute)
#[derive(Default, PartialEq, Clone, Debug)]
pub struct HmTime {
    pub hour: u32,
    pub minute: u32,
}

impl PartialOrd for HmTime {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let mut order = self.hour.cmp(&other.hour);
        if order == Ordering::Equal {
            order =  self.minute.cmp(&other.minute);
        }
        Some(order)
    }
}

#[derive(Default, Clone, Debug)]
pub struct DailyDuration {
    /// the days in a week this duration should trigger
    pub days: WeekDays,
    pub start: HmTime,
    pub end: HmTime,
}

impl DailyDuration {

    /// Test it time is within this frame
    pub fn time_match(&self, epoch: i64, utc: bool) -> Result<bool, Error> {

        let t = TmEditor::with_epoch(epoch, utc)?;

        Ok(self.time_match_with_tm_editor(&t))
    }

    /// Like time_match, but use [TmEditor] to specify the time
    ///
    /// Note: This function returns bool (not Result<bool, Error>). It
    /// simply returns ''false' if passed time 't' contains invalid values.
    pub fn time_match_with_tm_editor(&self, t: &TmEditor) -> bool {
        let all_days = self.days.is_empty() || self.days.is_all();

        if !all_days { // match day first
            match u32::try_from(t.day_num()) {
                Ok(day_num) => {
                    match WeekDays::from_bits(1<<day_num) {
                        Some(day) => {
                            if !self.days.contains(day) {
                                return false;
                            }
                        }
                        None => return false,
                    }
                }
                Err(_) => return false,
            }
        }

        let hour = t.hour().try_into();
        let minute = t.min().try_into();

        match (hour, minute) {
            (Ok(hour), Ok(minute)) => {
                let ctime = HmTime { hour, minute };
                ctime >= self.start && ctime < self.end
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod test {

    use anyhow::{bail, Error};

    use super::*;

    fn test_parse(
        duration_str: &str,
        start_h: u32, start_m: u32,
        end_h: u32, end_m: u32,
        days: &[usize],
    ) -> Result<(), Error> {
        let mut day_bits = 0;
        for day in days { day_bits |= 1<<day; }
        let expected_days = WeekDays::from_bits(day_bits).unwrap();

        let duration = parse_daily_duration(duration_str)?;

        if duration.start.hour != start_h {
            bail!("start hour missmatch, extected {}, got {:?}", start_h, duration);
        }
        if duration.start.minute != start_m {
            bail!("start minute missmatch, extected {}, got {:?}", start_m, duration);
        }
        if duration.end.hour != end_h {
            bail!("end hour missmatch, extected {}, got {:?}", end_h, duration);
        }
        if duration.end.minute != end_m {
            bail!("end minute missmatch, extected {}, got {:?}", end_m, duration);
        }

        if duration.days != expected_days {
            bail!("weekday missmatch, extected {:?}, got {:?}", expected_days, duration);
        }

        Ok(())
    }

    const fn make_test_time(mday: i32, hour: i32, min: i32) -> i64 {
        (mday*3600*24 + hour*3600 + min*60) as i64
    }

    #[test]
    fn test_daily_duration_parser() -> Result<(), Error> {

        assert!(parse_daily_duration("").is_err());
        assert!(parse_daily_duration(" 8-12").is_err());
        assert!(parse_daily_duration("8:60-12").is_err());
        assert!(parse_daily_duration("8-25").is_err());
        assert!(parse_daily_duration("12-8").is_err());

        test_parse("8-12", 8, 0, 12, 0, &[])?;
        test_parse("8:0-12:0", 8, 0, 12, 0, &[])?;
        test_parse("8:00-12:00", 8, 0, 12, 0, &[])?;
        test_parse("8:05-12:20", 8, 5, 12, 20, &[])?;
        test_parse("8:05 - 12:20", 8, 5, 12, 20, &[])?;

        test_parse("mon 8-12", 8, 0, 12, 0, &[0])?;
        test_parse("tue..fri 8-12", 8, 0, 12, 0, &[1,2,3,4])?;
        test_parse("sat,tue..thu,fri 8-12", 8, 0, 12, 0, &[1,2,3,4,5])?;

        Ok(())
    }

    #[test]
    fn test_time_match() -> Result<(), Error> {
        const THURSDAY_80_00: i64 = make_test_time(0, 8, 0);
        const THURSDAY_12_00: i64 = make_test_time(0, 12, 0);
        const DAY: i64 = 3600*24;

        let duration = parse_daily_duration("thu..fri 8:05-12")?;

        assert!(!duration.time_match(THURSDAY_80_00, true)?);
        assert!(!duration.time_match(THURSDAY_80_00 + DAY, true)?);
        assert!(!duration.time_match(THURSDAY_80_00 + 2*DAY, true)?);

        assert!(duration.time_match(THURSDAY_80_00 + 5*60, true)?);
        assert!(duration.time_match(THURSDAY_80_00 + 5*60 + DAY, true)?);
        assert!(!duration.time_match(THURSDAY_80_00 + 5*60 + 2*DAY, true)?);

        assert!(duration.time_match(THURSDAY_12_00 - 1, true)?);
        assert!(duration.time_match(THURSDAY_12_00 - 1 + DAY, true)?);
        assert!(!duration.time_match(THURSDAY_12_00 - 1 + 2*DAY, true)?);

        assert!(!duration.time_match(THURSDAY_12_00, true)?);
        assert!(!duration.time_match(THURSDAY_12_00 + DAY, true)?);
        assert!(!duration.time_match(THURSDAY_12_00 + 2*DAY, true)?);

        Ok(())
    }
}
