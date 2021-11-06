use std::cmp::{Ordering, PartialOrd};

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
}
