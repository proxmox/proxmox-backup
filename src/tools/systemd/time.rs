use anyhow::{bail, Error};
use bitflags::bitflags;

use proxmox::tools::time::*;

use super::parse_time::*;

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

#[derive(Debug)]
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

    // Find an return an entry greater than value
    pub fn find_next(list: &[DateTimeValue], value: u32) -> Option<u32> {
        let mut next: Option<u32> = None;
        let mut set_next = |v: u32| {
            if let Some(n) = next {
                if v > n { next = Some(v); }
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

#[derive(Default, Debug)]
pub struct CalendarEvent {
    pub days: WeekDays,
    pub second: Vec<DateTimeValue>, // todo: support float values
    pub minute: Vec<DateTimeValue>,
    pub hour: Vec<DateTimeValue>,
}


#[derive(Default)]
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

impl TimeSpan {

    pub fn  new() -> Self {
        Self::default()
    }
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


pub fn verify_time_span<'a>(i: &'a str) -> Result<(), Error> {
    parse_time_span(i)?;
    Ok(())
}

pub fn verify_calendar_event(i: &str) -> Result<(), Error> {
    parse_calendar_event(i)?;
    Ok(())
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

fn wrap_time(t: &mut libc::tm) {

    // sec: 0..59
    if t.tm_sec >= 60 {
        t.tm_min += t.tm_sec / 60;
        t.tm_sec %= 60;
    }

    // min: 0..59
    if t.tm_min >= 60 {
        t.tm_hour += t.tm_min / 60;
        t.tm_min %= 60;
    }

    // hour: 0..23
    if t.tm_hour >= 24 {
        t.tm_mday += t.tm_hour / 24;
        t.tm_wday += t.tm_hour / 24;
        t.tm_hour %= 24;
    }

    // Translate to 0..($days_in_mon-1)
    t.tm_mday -= 1;
    loop {
	let days_in_mon = days_in_month(t.tm_mon, t.tm_year);
	if t.tm_mday < days_in_mon { break; }
	// Wrap one month
	t.tm_mday -= days_in_mon;
        t.tm_wday += 7 - (days_in_mon % 7);
	t.tm_mon += 1;
    }

    // Translate back to 1..$days_in_mon
    t.tm_mday += 1;

    // mon: 0..11
    if t.tm_mon >= 12 {
        t.tm_year += t.tm_mon / 12;
        t.tm_mon %= 12;
    }

    t.tm_wday %= 7;
}

fn time_add_days(t: &mut libc::tm, days: libc::c_int) {
    t.tm_mday += days;
    t.tm_wday += days;
    wrap_time(t);
}

pub fn compute_next_event(
    event: &CalendarEvent,
    last: i64,
    utc: bool,
) -> Result<i64, Error> {

    let last = last + 60; // at least one minute later

    let all_days = event.days.is_empty() || event.days.is_all();

    let mut t = if utc { gmtime(last)? } else { localtime(last)? };
    t.tm_sec = 0; // we're not interested in seconds, actually
    t.tm_year += 1900; // real years for clarity

    let mut count = 0;

    loop {
        if count > 1000 { // should not happen
            bail!("unable to compute next calendar event");
        } else {
            count += 1;
        }

        if !all_days { // match day first
            // Note: tm_wday (0-6, Sunday = 0) => convert to Sunday = 6
            let day_num = (t.tm_wday + 6) % 7;
            let day = WeekDays::from_bits(1<<day_num).unwrap();
            if !event.days.contains(day) {
                if let Some(n) = (day_num+1..6)
                    .map(|d| WeekDays::from_bits(1<<d).unwrap())
                    .find(|d| event.days.contains(*d))
                {
                    // try next day
                    t.tm_sec = 0; t.tm_min = 0; t.tm_hour = 0;
                    time_add_days(&mut t, (n.bits() as i32) - day_num);
                    continue;
                } else {
                    // try next week
                    t.tm_sec = 0; t.tm_min = 0; t.tm_hour = 0;
                    time_add_days(&mut t, 7 - day_num);
                    continue;

                }
            }
        }

        // this day
        if !event.hour.is_empty() {
            let hour = t.tm_hour as u32;
            if event.hour.iter().find(|hspec| hspec.contains(hour)).is_none() {
                if let Some(n) = DateTimeValue::find_next(&event.hour, hour) {
                    // test next hour
                    t.tm_sec = 0; t.tm_min = 0; t.tm_hour = n as libc::c_int;
                    wrap_time(&mut t);
                    continue;
                } else {
                    // test next day
                    t.tm_sec = 0; t.tm_min = 0; t.tm_hour = 0;
                    time_add_days(&mut t, 1);
                    continue;
                }
            }
        }

        // this hour
        if !event.minute.is_empty() {
            let minute = t.tm_min as u32;
            if event.minute.iter().find(|hspec| hspec.contains(minute)).is_none() {
                if let Some(n) = DateTimeValue::find_next(&event.minute, minute) {
                    // test next minute
                    t.tm_sec = 0; t.tm_min = n as libc::c_int;
                    wrap_time(&mut t);
                    continue;
                } else {
                   // test next hour
                    t.tm_sec = 0; t.tm_min = 0; t.tm_hour += 1;
                    wrap_time(&mut t);
                    continue;
                }
            }
        }

        t.tm_year -= 1900;
        let next = if utc { timegm(t)? } else { timelocal(t)? };
        return Ok(next)
    }
}

#[cfg(test)]
mod test {

    use super::*;

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
                Ok(next) => {
                    if next == expect {
                        println!("next {:?} => {}", event, next);
                    } else {
                        bail!("next {:?} failed\nnext:  {:?}\nexpect: {:?}",
                              event, gmtime(next), gmtime(expect));
                    }
                }
                Err(err) => bail!("compute next for '{}' failed - {}", v, err),
            }

            Ok(expect)
        };

        const MIN: i64 = 60;
        const HOUR: i64 = 3600;
        const DAY: i64 = 3600*24;

        const THURSDAY_00_00: i64 = make_test_time(0, 0, 0);
        const THURSDAY_15_00: i64 = make_test_time(0, 15, 0);

        test_value("*:*", THURSDAY_00_00, THURSDAY_00_00 + MIN)?;

        test_value("mon *:*", THURSDAY_00_00, THURSDAY_00_00 + 4*DAY)?;
        test_value("mon 2:*", THURSDAY_00_00, THURSDAY_00_00 + 4*DAY + 2*HOUR)?;
        test_value("mon 2:50", THURSDAY_00_00, THURSDAY_00_00 + 4*DAY + 2*HOUR + 50*MIN)?;

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
