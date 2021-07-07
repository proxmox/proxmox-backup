use std::convert::TryInto;

use anyhow::Error;
use bitflags::bitflags;

use proxmox::tools::time::TmEditor;

pub use super::parse_time::*;

bitflags!{
    #[derive(Default)]
    pub struct WeekDays: u8 {
        const MONDAY = 1;
        const TUESDAY = 2;
        const WEDNESDAY = 4;
        const THURSDAY = 8;
        const FRIDAY = 16;
        const SATURDAY = 32;
        const SUNDAY = 64;
    }
}

#[derive(Debug, Clone)]
pub enum DateTimeValue {
    Single(u32),
    Range(u32, u32),
    Repeated(u32, u32),
}

impl DateTimeValue {
    // Test if the entry contains the value
    pub fn contains(&self, value: u32) -> bool {
        match self {
            DateTimeValue::Single(v) => *v == value,
            DateTimeValue::Range(start, end) => value >= *start && value <= *end,
            DateTimeValue::Repeated(start, repetition) => {
                if value >= *start {
                    if *repetition > 0 {
                        let offset = value - start;
                        offset % repetition == 0
                    } else {
                        *start == value
                    }
                } else {
                    false
                }
            }
        }
    }

    pub fn list_contains(list: &[DateTimeValue], value: u32) -> bool {
        list.iter().any(|spec| spec.contains(value))
    }

    // Find an return an entry greater than value
    pub fn find_next(list: &[DateTimeValue], value: u32) -> Option<u32> {
        let mut next: Option<u32> = None;
        let mut set_next = |v: u32| {
            if let Some(n) = next {
                if v < n { next = Some(v); }
            } else {
                next = Some(v);
            }
        };
        for spec in list {
            match spec {
                DateTimeValue::Single(v) => {
                    if *v > value { set_next(*v); }
                }
                DateTimeValue::Range(start, end) => {
                    if value < *start {
                        set_next(*start);
                    } else {
                        let n = value + 1;
                        if n >= *start && n <= *end {
                            set_next(n);
                        }
                    }
                }
                DateTimeValue::Repeated(start, repetition) => {
                    if value < *start {
                        set_next(*start);
                    } else if *repetition > 0 {
                        set_next(start + ((value - start + repetition) / repetition) * repetition);
                    }
                }
            }
        }

        next
    }
}

/// Calendar events may be used to refer to one or more points in time in a
/// single expression. They are designed after the systemd.time Calendar Events
/// specification, but are not guaranteed to be 100% compatible.
#[derive(Default, Clone, Debug)]
pub struct CalendarEvent {
    /// the days in a week this event should trigger
    pub days: WeekDays,
    /// the second(s) this event should trigger
    pub second: Vec<DateTimeValue>, // todo: support float values
    /// the minute(s) this event should trigger
    pub minute: Vec<DateTimeValue>,
    /// the hour(s) this event should trigger
    pub hour: Vec<DateTimeValue>,
    /// the day(s) in a month this event should trigger
    pub day: Vec<DateTimeValue>,
    /// the month(s) in a year this event should trigger
    pub month: Vec<DateTimeValue>,
    /// the years(s) this event should trigger
    pub year: Vec<DateTimeValue>,
}

#[derive(Default, Clone, Debug)]
pub struct TimeSpan {
    pub nsec: u64,
    pub usec: u64,
    pub msec: u64,
    pub seconds: u64,
    pub minutes: u64,
    pub hours: u64,
    pub days: u64,
    pub weeks: u64,
    pub months: u64,
    pub years: u64,
}

impl From<TimeSpan> for f64 {
    fn from(ts: TimeSpan) -> Self {
        (ts.seconds as f64) +
            ((ts.nsec as f64) / 1_000_000_000.0)  +
            ((ts.usec as f64) / 1_000_000.0)  +
            ((ts.msec as f64) / 1_000.0)  +
            ((ts.minutes as f64) * 60.0)  +
            ((ts.hours as f64) * 3600.0)  +
            ((ts.days as f64) * 3600.0 * 24.0)  +
            ((ts.weeks as f64) * 3600.0 * 24.0 * 7.0)  +
            ((ts.months as f64) * 3600.0 * 24.0 * 30.44)  +
            ((ts.years as f64) * 3600.0 * 24.0 * 365.25)
    }
}

impl From<std::time::Duration> for TimeSpan {
    fn from(duration: std::time::Duration) -> Self {
        let mut duration = duration.as_nanos();
        let nsec = (duration % 1000) as u64;
        duration /= 1000;
        let usec = (duration % 1000) as u64;
        duration /= 1000;
        let msec = (duration % 1000) as u64;
        duration /= 1000;
        let seconds = (duration % 60) as u64;
        duration /= 60;
        let minutes = (duration % 60) as u64;
        duration /= 60;
        let hours = (duration % 24) as u64;
        duration /= 24;
        let years = (duration as f64 / 365.25) as u64;
        let ydays = (duration as f64 % 365.25) as u64;
        let months = (ydays as f64 / 30.44) as u64;
        let mdays = (ydays as f64 % 30.44) as u64;
        let weeks = mdays / 7;
        let days = mdays % 7;
        Self {
            nsec,
            usec,
            msec,
            seconds,
            minutes,
            hours,
            days,
            weeks,
            months,
            years,
        }
    }
}

impl std::fmt::Display for TimeSpan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        let mut first = true;
        { // block scope for mutable borrows
            let mut do_write = |v: u64, unit: &str| -> Result<(), std::fmt::Error> {
                if !first {
                    write!(f, " ")?;
                }
                first = false;
                write!(f, "{}{}", v, unit)
            };
            if self.years > 0 {
                do_write(self.years, "y")?;
            }
            if self.months > 0 {
                do_write(self.months, "m")?;
            }
            if self.weeks > 0 {
                do_write(self.weeks, "w")?;
            }
            if self.days > 0 {
                do_write(self.days, "d")?;
            }
            if self.hours > 0 {
                do_write(self.hours, "h")?;
            }
            if self.minutes > 0 {
                do_write(self.minutes, "min")?;
            }
        }
        if !first {
            write!(f, " ")?;
        }
        let seconds = self.seconds as f64 + (self.msec as f64 / 1000.0);
        if seconds >= 0.1 {
            if seconds >= 1.0 || !first {
                write!(f, "{:.0}s", seconds)?;
            } else {
                write!(f, "{:.1}s", seconds)?;
            }
        } else if first {
            write!(f, "<0.1s")?;
        }
        Ok(())
    }
}

pub fn verify_time_span(i: &str) -> Result<(), Error> {
    parse_time_span(i)?;
    Ok(())
}

pub fn verify_calendar_event(i: &str) -> Result<(), Error> {
    parse_calendar_event(i)?;
    Ok(())
}

pub fn compute_next_event(
    event: &CalendarEvent,
    last: i64,
    utc: bool,
) -> Result<Option<i64>, Error> {

    let last = last + 1; // at least one second later

    let all_days = event.days.is_empty() || event.days.is_all();

    let mut t = TmEditor::with_epoch(last, utc)?;

    let mut count = 0;

    loop {
        // cancel after 1000 loops
        if count > 1000 {
            return Ok(None);
        } else {
            count += 1;
        }

        if !event.year.is_empty() {
            let year: u32 = t.year().try_into()?;
            if !DateTimeValue::list_contains(&event.year, year) {
                if let Some(n) = DateTimeValue::find_next(&event.year, year) {
                    t.add_years((n - year).try_into()?)?;
                    continue;
                } else {
                    // if we have no valid year, we cannot find a correct timestamp
                    return Ok(None);
                }
            }
        }

        if !event.month.is_empty() {
            let month: u32 = t.month().try_into()?;
            if !DateTimeValue::list_contains(&event.month, month) {
                if let Some(n) = DateTimeValue::find_next(&event.month, month) {
                    t.add_months((n - month).try_into()?)?;
                } else {
                    // if we could not find valid month, retry next year
                    t.add_years(1)?;
                }
                continue;
            }
        }

        if !event.day.is_empty() {
            let day: u32 = t.day().try_into()?;
            if !DateTimeValue::list_contains(&event.day, day) {
                if let Some(n) = DateTimeValue::find_next(&event.day, day) {
                    t.add_days((n - day).try_into()?)?;
                } else {
                    // if we could not find valid mday, retry next month
                    t.add_months(1)?;
                }
                continue;
            }
        }

        if !all_days { // match day first
            let day_num: u32 = t.day_num().try_into()?;
            let day = WeekDays::from_bits(1<<day_num).unwrap();
            if !event.days.contains(day) {
                if let Some(n) = ((day_num+1)..7)
                    .find(|d| event.days.contains(WeekDays::from_bits(1<<d).unwrap()))
                {
                    // try next day
                    t.add_days((n - day_num).try_into()?)?;
                } else {
                    // try next week
                    t.add_days((7 - day_num).try_into()?)?;
                }
                continue;
            }
        }

        // this day
        if !event.hour.is_empty() {
            let hour = t.hour().try_into()?;
            if !DateTimeValue::list_contains(&event.hour, hour) {
                if let Some(n) = DateTimeValue::find_next(&event.hour, hour) {
                    // test next hour
                    t.set_time(n.try_into()?, 0, 0)?;
                } else {
                    // test next day
                    t.add_days(1)?;
                }
                continue;
            }
        }

        // this hour
        if !event.minute.is_empty() {
            let minute = t.min().try_into()?;
            if !DateTimeValue::list_contains(&event.minute, minute) {
                if let Some(n) = DateTimeValue::find_next(&event.minute, minute) {
                    // test next minute
                    t.set_min_sec(n.try_into()?, 0)?;
                } else {
                    // test next hour
                    t.set_time(t.hour() + 1, 0, 0)?;
                }
                continue;
            }
        }

        // this minute
        if !event.second.is_empty() {
            let second = t.sec().try_into()?;
            if !DateTimeValue::list_contains(&event.second, second) {
                if let Some(n) = DateTimeValue::find_next(&event.second, second) {
                    // test next second
                    t.set_sec(n.try_into()?)?;
                } else {
                    // test next min
                    t.set_min_sec(t.min() + 1, 0)?;
                }
                continue;
            }
        }

        let next = t.into_epoch()?;
        return Ok(Some(next))
    }
}

#[cfg(test)]
mod test {

    use anyhow::bail;

    use super::*;
    use proxmox::tools::time::*;

    fn test_event(v: &'static str) -> Result<(), Error> {
        match parse_calendar_event(v) {
            Ok(event) => println!("CalendarEvent '{}' => {:?}", v, event),
            Err(err) => bail!("parsing '{}' failed - {}", v, err),
        }

        Ok(())
    }

    const fn make_test_time(mday: i32, hour: i32, min: i32) -> libc::time_t {
        (mday*3600*24 + hour*3600 + min*60) as libc::time_t
    }

    #[test]
    fn test_compute_next_event() -> Result<(), Error> {

        let test_value = |v: &'static str, last: i64, expect: i64| -> Result<i64, Error> {
            let event = match parse_calendar_event(v) {
                Ok(event) => event,
                Err(err) => bail!("parsing '{}' failed - {}", v, err),
            };

            match compute_next_event(&event, last, true) {
                Ok(Some(next)) => {
                    if next == expect {
                        println!("next {:?} => {}", event, next);
                    } else {
                        bail!("next {:?} failed\nnext:  {:?}\nexpect: {:?}",
                              event, gmtime(next), gmtime(expect));
                    }
                }
                Ok(None) => bail!("next {:?} failed to find a timestamp", event),
                Err(err) => bail!("compute next for '{}' failed - {}", v, err),
            }

            Ok(expect)
        };

        let test_never = |v: &'static str, last: i64| -> Result<(), Error> {
            let event = match parse_calendar_event(v) {
                Ok(event) => event,
                Err(err) => bail!("parsing '{}' failed - {}", v, err),
            };

            match compute_next_event(&event, last, true)? {
                None => Ok(()),
                Some(next) => bail!("compute next for '{}' succeeded, but expected fail - result {}", v, next),
            }
        };

        const MIN: i64 = 60;
        const HOUR: i64 = 3600;
        const DAY: i64 = 3600*24;

        const THURSDAY_00_00: i64 = make_test_time(0, 0, 0);
        const THURSDAY_15_00: i64 = make_test_time(0, 15, 0);

        const JUL_31_2020: i64 = 1596153600; // Friday, 2020-07-31 00:00:00
        const DEC_31_2020: i64 = 1609372800; // Thursday, 2020-12-31 00:00:00

        test_value("*:0", THURSDAY_00_00, THURSDAY_00_00 + HOUR)?;
        test_value("*:*", THURSDAY_00_00, THURSDAY_00_00 + MIN)?;
        test_value("*:*:*", THURSDAY_00_00, THURSDAY_00_00 + 1)?;
        test_value("*:3:5", THURSDAY_00_00, THURSDAY_00_00 + 3*MIN + 5)?;

        test_value("mon *:*", THURSDAY_00_00, THURSDAY_00_00 + 4*DAY)?;
        test_value("mon 2:*", THURSDAY_00_00, THURSDAY_00_00 + 4*DAY + 2*HOUR)?;
        test_value("mon 2:50", THURSDAY_00_00, THURSDAY_00_00 + 4*DAY + 2*HOUR + 50*MIN)?;

        test_value("tue", THURSDAY_00_00, THURSDAY_00_00 + 5*DAY)?;
        test_value("wed", THURSDAY_00_00, THURSDAY_00_00 + 6*DAY)?;
        test_value("thu", THURSDAY_00_00, THURSDAY_00_00 + 7*DAY)?;
        test_value("fri", THURSDAY_00_00, THURSDAY_00_00 + 1*DAY)?;
        test_value("sat", THURSDAY_00_00, THURSDAY_00_00 + 2*DAY)?;
        test_value("sun", THURSDAY_00_00, THURSDAY_00_00 + 3*DAY)?;

        // test multiple values for a single field
        // and test that the order does not matter
        test_value("5,10:4,8", THURSDAY_00_00, THURSDAY_00_00 + 5*HOUR + 4*MIN)?;
        test_value("10,5:8,4", THURSDAY_00_00, THURSDAY_00_00 + 5*HOUR + 4*MIN)?;
        test_value("6,4..10:23,5/5", THURSDAY_00_00, THURSDAY_00_00 + 4*HOUR + 5*MIN)?;
        test_value("4..10,6:5/5,23", THURSDAY_00_00, THURSDAY_00_00 + 4*HOUR + 5*MIN)?;

        // test month wrapping
        test_value("sat", JUL_31_2020, JUL_31_2020 + 1*DAY)?;
        test_value("sun", JUL_31_2020, JUL_31_2020 + 2*DAY)?;
        test_value("mon", JUL_31_2020, JUL_31_2020 + 3*DAY)?;
        test_value("tue", JUL_31_2020, JUL_31_2020 + 4*DAY)?;
        test_value("wed", JUL_31_2020, JUL_31_2020 + 5*DAY)?;
        test_value("thu", JUL_31_2020, JUL_31_2020 + 6*DAY)?;
        test_value("fri", JUL_31_2020, JUL_31_2020 + 7*DAY)?;

        // test year wrapping
        test_value("fri", DEC_31_2020, DEC_31_2020 + 1*DAY)?;
        test_value("sat", DEC_31_2020, DEC_31_2020 + 2*DAY)?;
        test_value("sun", DEC_31_2020, DEC_31_2020 + 3*DAY)?;
        test_value("mon", DEC_31_2020, DEC_31_2020 + 4*DAY)?;
        test_value("tue", DEC_31_2020, DEC_31_2020 + 5*DAY)?;
        test_value("wed", DEC_31_2020, DEC_31_2020 + 6*DAY)?;
        test_value("thu", DEC_31_2020, DEC_31_2020 + 7*DAY)?;

        test_value("daily", THURSDAY_00_00, THURSDAY_00_00 + DAY)?;
        test_value("daily", THURSDAY_00_00+1, THURSDAY_00_00 + DAY)?;

        let n = test_value("5/2:0", THURSDAY_00_00, THURSDAY_00_00 + 5*HOUR)?;
        let n = test_value("5/2:0", n, THURSDAY_00_00 + 7*HOUR)?;
        let n = test_value("5/2:0", n, THURSDAY_00_00 + 9*HOUR)?;
        test_value("5/2:0", n, THURSDAY_00_00 + 11*HOUR)?;

        let mut n = test_value("*:*", THURSDAY_00_00, THURSDAY_00_00 + MIN)?;
        for i in 2..100 {
            n = test_value("*:*", n, THURSDAY_00_00 + i*MIN)?;
        }

        let mut n = test_value("*:0", THURSDAY_00_00, THURSDAY_00_00 + HOUR)?;
        for i in 2..100 {
            n = test_value("*:0", n, THURSDAY_00_00 + i*HOUR)?;
        }

        let mut n = test_value("1:0", THURSDAY_15_00, THURSDAY_00_00 + DAY + HOUR)?;
        for i in 2..100 {
            n = test_value("1:0", n, THURSDAY_00_00 + i*DAY + HOUR)?;
        }

        // test date functionality

        test_value("2020-07-31", 0, JUL_31_2020)?;
        test_value("02-28", 0, (31+27)*DAY)?;
        test_value("02-29", 0, 2*365*DAY + (31+28)*DAY)?; // 1972-02-29
        test_value("1965/5-01-01", -1, THURSDAY_00_00)?;
        test_value("2020-7..9-2/2", JUL_31_2020, JUL_31_2020 + 2*DAY)?;
        test_value("2020,2021-12-31", JUL_31_2020, DEC_31_2020)?;

        test_value("monthly", 0, 31*DAY)?;
        test_value("quarterly", 0, (31+28+31)*DAY)?;
        test_value("semiannually", 0, (31+28+31+30+31+30)*DAY)?;
        test_value("yearly", 0, (365)*DAY)?;

        test_never("2021-02-29", 0)?;
        test_never("02-30", 0)?;

        Ok(())
    }

    #[test]
    fn test_calendar_event_weekday() -> Result<(), Error> {
        test_event("mon,wed..fri")?;
        test_event("fri..mon")?;

        test_event("mon")?;
        test_event("MON")?;
        test_event("monDay")?;
        test_event("tue")?;
        test_event("Tuesday")?;
        test_event("wed")?;
        test_event("wednesday")?;
        test_event("thu")?;
        test_event("thursday")?;
        test_event("fri")?;
        test_event("friday")?;
        test_event("sat")?;
        test_event("saturday")?;
        test_event("sun")?;
        test_event("sunday")?;

        test_event("mon..fri")?;
        test_event("mon,tue,fri")?;
        test_event("mon,tue..wednesday,fri..sat")?;

        Ok(())
    }

    #[test]
    fn test_time_span_parser() -> Result<(), Error> {

        let test_value = |ts_str: &str, expect: f64| -> Result<(), Error> {
            let ts = parse_time_span(ts_str)?;
            assert_eq!(f64::from(ts), expect, "{}", ts_str);
            Ok(())
        };

        test_value("2", 2.0)?;
        test_value("2s", 2.0)?;
        test_value("2sec", 2.0)?;
        test_value("2second", 2.0)?;
        test_value("2seconds", 2.0)?;

        test_value(" 2s 2 s 2", 6.0)?;

        test_value("1msec 1ms", 0.002)?;
        test_value("1usec 1us 1µs", 0.000_003)?;
        test_value("1nsec 1ns", 0.000_000_002)?;
        test_value("1minutes 1minute 1min 1m", 4.0*60.0)?;
        test_value("1hours 1hour 1hr 1h", 4.0*3600.0)?;
        test_value("1days 1day 1d", 3.0*86400.0)?;
        test_value("1weeks 1 week 1w", 3.0*86400.0*7.0)?;
        test_value("1months 1month 1M", 3.0*86400.0*30.44)?;
        test_value("1years 1year 1y", 3.0*86400.0*365.25)?;

        test_value("2h", 7200.0)?;
        test_value(" 2 h", 7200.0)?;
        test_value("2hours", 7200.0)?;
        test_value("48hr", 48.0*3600.0)?;
        test_value("1y 12month", 365.25*24.0*3600.0 + 12.0*30.44*24.0*3600.0)?;
        test_value("55s500ms", 55.5)?;
        test_value("300ms20s 5day", 5.0*24.0*3600.0 + 20.0 + 0.3)?;

        Ok(())
    }
}
