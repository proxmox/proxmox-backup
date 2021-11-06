use std::collections::HashMap;

use anyhow::{bail, Error};
use lazy_static::lazy_static;

use super::time::*;
use super::daily_duration::*;

use nom::{
    error::{context, ParseError, VerboseError},
    bytes::complete::{tag, take_while1},
    combinator::{map_res, all_consuming, opt, recognize},
    sequence::{pair, preceded, tuple},
    character::complete::{alpha1, space0, digit1},
    multi::separated_nonempty_list,
};

type IResult<I, O, E = VerboseError<I>> = Result<(I, O), nom::Err<E>>;

fn parse_error<'a>(i: &'a str, context: &'static str) -> nom::Err<VerboseError<&'a str>> {
    let err = VerboseError { errors: Vec::new() };
    let err = VerboseError::add_context(i, context, err);
    nom::Err::Error(err)
}

// Parse a 64 bit unsigned integer
fn parse_u64(i: &str) -> IResult<&str, u64> {
    map_res(recognize(digit1), str::parse)(i)
}

// Parse complete input, generate simple error message (use this for sinple line input).
fn parse_complete_line<'a, F, O>(what: &str, i: &'a str, parser: F) -> Result<O, Error>
    where F: Fn(&'a str) -> IResult<&'a str, O>,
{
    match all_consuming(parser)(i) {
        Err(nom::Err::Error(VerboseError { errors })) |
        Err(nom::Err::Failure(VerboseError { errors })) => {
            if errors.is_empty() {
                bail!("unable to parse {}", what);
            } else {
                bail!("unable to parse {} at '{}' - {:?}", what, errors[0].0, errors[0].1);
            }
        }
        Err(err) => {
            bail!("unable to parse {} - {}", what, err);
        }
        Ok((_, data)) => Ok(data),
    }
}

lazy_static! {
    pub static ref TIME_SPAN_UNITS: HashMap<&'static str, f64> = {
        let mut map = HashMap::new();

        let second = 1.0;

        map.insert("seconds", second);
        map.insert("second", second);
        map.insert("sec", second);
        map.insert("s", second);

        let msec = second / 1000.0;

        map.insert("msec", msec);
        map.insert("ms", msec);

        let usec = msec / 1000.0;

        map.insert("usec", usec);
        map.insert("us", usec);
        map.insert("µs", usec);

        let nsec = usec / 1000.0;

        map.insert("nsec", nsec);
        map.insert("ns", nsec);

        let minute = second * 60.0;

        map.insert("minutes", minute);
        map.insert("minute", minute);
        map.insert("min", minute);
        map.insert("m", minute);

        let hour = minute * 60.0;

        map.insert("hours", hour);
        map.insert("hour", hour);
        map.insert("hr", hour);
        map.insert("h", hour);

        let day = hour * 24.0 ;

        map.insert("days", day);
        map.insert("day", day);
        map.insert("d", day);

        let week = day * 7.0;

        map.insert("weeks", week);
        map.insert("week", week);
        map.insert("w", week);

        let month = 30.44 * day;

        map.insert("months", month);
        map.insert("month", month);
        map.insert("M", month);

        let year = 365.25 * day;

        map.insert("years", year);
        map.insert("year", year);
        map.insert("y", year);

        map
    };
}

struct TimeSpec {
    hour: Vec<DateTimeValue>,
    minute: Vec<DateTimeValue>,
    second: Vec<DateTimeValue>,
}

struct DateSpec {
    year: Vec<DateTimeValue>,
    month: Vec<DateTimeValue>,
    day: Vec<DateTimeValue>,
}

fn parse_time_comp(max: usize) -> impl Fn(&str) -> IResult<&str, u32> {
    move |i: &str| {
        let (i, v) = map_res(recognize(digit1), str::parse)(i)?;
        if (v as usize) >= max {
            return Err(parse_error(i, "time value too large"));
        }
        Ok((i, v))
    }
}

fn parse_weekday(i: &str) -> IResult<&str, WeekDays> {
    let (i, text) = alpha1(i)?;

    match text.to_ascii_lowercase().as_str() {
        "monday" | "mon" => Ok((i, WeekDays::MONDAY)),
        "tuesday" | "tue" => Ok((i, WeekDays::TUESDAY)),
        "wednesday" | "wed" => Ok((i, WeekDays::WEDNESDAY)),
        "thursday" | "thu" => Ok((i, WeekDays::THURSDAY)),
        "friday" | "fri" => Ok((i, WeekDays::FRIDAY)),
        "saturday" | "sat" => Ok((i, WeekDays::SATURDAY)),
        "sunday" | "sun" => Ok((i, WeekDays::SUNDAY)),
        _ => return Err(parse_error(text, "weekday")),
    }
}

fn parse_weekdays_range(i: &str) -> IResult<&str, WeekDays> {
    let (i, startday) = parse_weekday(i)?;

    let generate_range = |start, end| {
        let mut res = 0;
        let mut pos = start;
        loop {
            res |= pos;
            if pos >= end { break; }
            pos <<= 1;
        }
        WeekDays::from_bits(res).unwrap()
    };

    if let (i, Some((_, endday))) = opt(pair(tag(".."),parse_weekday))(i)? {
        let start = startday.bits();
        let end = endday.bits();
        if start > end {
            let set1 = generate_range(start, WeekDays::SUNDAY.bits());
            let set2 = generate_range(WeekDays::MONDAY.bits(), end);
            Ok((i, set1 | set2))
        } else {
            Ok((i, generate_range(start, end)))
        }
    } else {
        Ok((i, startday))
    }
}

fn parse_date_time_comp(max: usize) -> impl Fn(&str) -> IResult<&str, DateTimeValue> {
    move |i: &str| {
        let (i, value) = parse_time_comp(max)(i)?;

        if let (i, Some(end)) = opt(preceded(tag(".."), parse_time_comp(max)))(i)? {
            if value > end {
                return Err(parse_error(i, "range start is bigger than end"));
            }
            return Ok((i, DateTimeValue::Range(value, end)))
        }

        if let Some(time) = i.strip_prefix('/') {
            let (time, repeat) = parse_time_comp(max)(time)?;
            Ok((time, DateTimeValue::Repeated(value, repeat)))
        } else {
            Ok((i, DateTimeValue::Single(value)))
        }
    }
}

fn parse_date_time_comp_list(start: u32, max: usize) -> impl Fn(&str) -> IResult<&str, Vec<DateTimeValue>> {
    move |i: &str| {
        if let Some(rest) = i.strip_prefix('*') {
            if let Some(time) = rest.strip_prefix('/') {
                let (n, repeat) = parse_time_comp(max)(time)?;
                if repeat > 0 {
                    return Ok((n, vec![DateTimeValue::Repeated(start, repeat)]));
                }
            }
            return Ok((rest, Vec::new()));
        }

        separated_nonempty_list(tag(","), parse_date_time_comp(max))(i)
    }
}

fn parse_time_spec(i: &str) -> IResult<&str, TimeSpec> {

    let (i, (hour, minute, opt_second)) = tuple((
        parse_date_time_comp_list(0, 24),
        preceded(tag(":"), parse_date_time_comp_list(0, 60)),
        opt(preceded(tag(":"), parse_date_time_comp_list(0, 60))),
    ))(i)?;

    if let Some(second) = opt_second {
        Ok((i, TimeSpec { hour, minute, second }))
    } else {
        Ok((i, TimeSpec { hour, minute, second: vec![DateTimeValue::Single(0)] }))
    }
}

fn parse_date_spec(i: &str) -> IResult<&str, DateSpec> {

    // TODO: implement ~ for days (man systemd.time)
    if let Ok((i, (year, month, day))) = tuple((
        parse_date_time_comp_list(0, 2200), // the upper limit for systemd, stay compatible
        preceded(tag("-"), parse_date_time_comp_list(1, 13)),
        preceded(tag("-"), parse_date_time_comp_list(1, 32)),
    ))(i) {
        Ok((i, DateSpec { year, month, day }))
    } else if let Ok((i, (month, day))) = tuple((
        parse_date_time_comp_list(1, 13),
        preceded(tag("-"), parse_date_time_comp_list(1, 32)),
    ))(i) {
        Ok((i, DateSpec { year: Vec::new(), month, day }))
    } else {
        Err(parse_error(i, "invalid date spec"))
    }
}

pub fn parse_calendar_event(i: &str) -> Result<CalendarEvent, Error> {
    parse_complete_line("calendar event", i, parse_calendar_event_incomplete)
}

fn parse_calendar_event_incomplete(mut i: &str) -> IResult<&str, CalendarEvent> {

    let mut has_dayspec = false;
    let mut has_timespec = false;
    let mut has_datespec = false;

    let mut event = CalendarEvent::default();

    if i.starts_with(|c: char| char::is_ascii_alphabetic(&c)) {

        match i {
            "minutely" => {
                return Ok(("", CalendarEvent {
                    second: vec![DateTimeValue::Single(0)],
                    ..Default::default()
                }));
            }
            "hourly" => {
                return Ok(("", CalendarEvent {
                    minute: vec![DateTimeValue::Single(0)],
                    second: vec![DateTimeValue::Single(0)],
                    ..Default::default()
                }));
            }
            "daily" => {
                return Ok(("", CalendarEvent {
                    hour: vec![DateTimeValue::Single(0)],
                    minute: vec![DateTimeValue::Single(0)],
                    second: vec![DateTimeValue::Single(0)],
                    ..Default::default()
                }));
            }
            "weekly" => {
                return Ok(("", CalendarEvent {
                    hour: vec![DateTimeValue::Single(0)],
                    minute: vec![DateTimeValue::Single(0)],
                    second: vec![DateTimeValue::Single(0)],
                    days: WeekDays::MONDAY,
                    ..Default::default()
                }));
            }
            "monthly" => {
                return Ok(("", CalendarEvent {
                    hour: vec![DateTimeValue::Single(0)],
                    minute: vec![DateTimeValue::Single(0)],
                    second: vec![DateTimeValue::Single(0)],
                    day: vec![DateTimeValue::Single(1)],
                    ..Default::default()
                }));
            }
            "yearly" | "annually" => {
                return Ok(("", CalendarEvent {
                    hour: vec![DateTimeValue::Single(0)],
                    minute: vec![DateTimeValue::Single(0)],
                    second: vec![DateTimeValue::Single(0)],
                    day: vec![DateTimeValue::Single(1)],
                    month: vec![DateTimeValue::Single(1)],
                    ..Default::default()
                }));
            }
            "quarterly" => {
                return Ok(("", CalendarEvent {
                    hour: vec![DateTimeValue::Single(0)],
                    minute: vec![DateTimeValue::Single(0)],
                    second: vec![DateTimeValue::Single(0)],
                    day: vec![DateTimeValue::Single(1)],
                    month: vec![
                        DateTimeValue::Single(1),
                        DateTimeValue::Single(4),
                        DateTimeValue::Single(7),
                        DateTimeValue::Single(10),
                    ],
                    ..Default::default()
                }));
            }
            "semiannually" | "semi-annually" => {
                return Ok(("", CalendarEvent {
                    hour: vec![DateTimeValue::Single(0)],
                    minute: vec![DateTimeValue::Single(0)],
                    second: vec![DateTimeValue::Single(0)],
                    day: vec![DateTimeValue::Single(1)],
                    month: vec![
                        DateTimeValue::Single(1),
                        DateTimeValue::Single(7),
                    ],
                    ..Default::default()
                }));
            }
            _ => { /* continue */ }
        }

        let (n, range_list) =  context(
            "weekday range list",
            separated_nonempty_list(tag(","), parse_weekdays_range)
        )(i)?;

        has_dayspec = true;

        i = space0(n)?.0;

        for range in range_list  { event.days.insert(range); }
    }

    if let (n, Some(date)) = opt(parse_date_spec)(i)? {
        event.year = date.year;
        event.month = date.month;
        event.day = date.day;
        has_datespec = true;
        i = space0(n)?.0;
    }

    if let (n, Some(time)) = opt(parse_time_spec)(i)? {
        event.hour = time.hour;
        event.minute = time.minute;
        event.second = time.second;
        has_timespec = true;
        i = n;
    } else {
        event.hour = vec![DateTimeValue::Single(0)];
        event.minute = vec![DateTimeValue::Single(0)];
        event.second = vec![DateTimeValue::Single(0)];
    }

    if !(has_dayspec || has_timespec || has_datespec) {
        return Err(parse_error(i, "date or time specification"));
    }

    Ok((i, event))
}

fn parse_time_unit(i: &str) ->  IResult<&str, &str> {
    let (n, text) = take_while1(|c: char| char::is_ascii_alphabetic(&c) || c == 'µ')(i)?;
    if TIME_SPAN_UNITS.contains_key(&text) {
        Ok((n, text))
    } else {
        Err(parse_error(text, "time unit"))
    }
}


pub fn parse_time_span(i: &str) -> Result<TimeSpan, Error> {
    parse_complete_line("time span", i, parse_time_span_incomplete)
}

fn parse_time_span_incomplete(mut i: &str) -> IResult<&str, TimeSpan> {

    let mut ts = TimeSpan::default();

    loop {
        i = space0(i)?.0;
        if i.is_empty() { break; }
        let (n, num) = parse_u64(i)?;
        i = space0(n)?.0;

        if let (n, Some(unit)) = opt(parse_time_unit)(i)? {
            i = n;
            match unit {
                "seconds" | "second" | "sec" | "s" => {
                    ts.seconds += num;
                }
                "msec" | "ms" => {
                    ts.msec += num;
                }
                "usec" | "us" | "µs" => {
                    ts.usec += num;
                }
                "nsec" | "ns" => {
                    ts.nsec += num;
                }
                "minutes" | "minute" | "min" | "m" => {
                    ts.minutes += num;
                }
                "hours" | "hour" | "hr" | "h" => {
                    ts.hours += num;
                }
                "days" | "day" | "d" => {
                    ts.days += num;
                }
                "weeks" | "week" | "w" => {
                    ts.weeks += num;
                }
                "months" | "month" | "M" => {
                    ts.months += num;
                }
                "years" | "year" | "y" => {
                    ts.years += num;
                }
                _ => return Err(parse_error(unit, "internal error")),
            }
        } else {
            ts.seconds += num;
        }
    }

    Ok((i, ts))
}

pub fn parse_daily_duration(i: &str) -> Result<DailyDuration, Error> {
    parse_complete_line("daily duration", i, parse_daily_duration_incomplete)
}

fn parse_daily_duration_incomplete(mut i: &str) -> IResult<&str, DailyDuration> {

    let mut duration = DailyDuration::default();

    if i.starts_with(|c: char| char::is_ascii_alphabetic(&c)) {

        let (n, range_list) =  context(
            "weekday range list",
            separated_nonempty_list(tag(","), parse_weekdays_range)
        )(i)?;

        i = space0(n)?.0;

        for range in range_list  { duration.days.insert(range); }
    }

    let (i, start) = parse_hm_time(i)?;

    let i = space0(i)?.0;

    let (i, _) = tag("-")(i)?;

    let i = space0(i)?.0;

    let end_time_start = i;

    let (i, end) = parse_hm_time(i)?;

    if start > end {
        return Err(parse_error(end_time_start, "end time before start time"));
    }

    duration.start = start;
    duration.end = end;

    Ok((i, duration))
}

fn parse_hm_time(i: &str) -> IResult<&str, HmTime> {

    let (i, (hour, opt_minute)) = tuple((
        parse_time_comp(24),
        opt(preceded(tag(":"), parse_time_comp(60))),
    ))(i)?;

    match opt_minute {
        Some(minute) => Ok((i, HmTime { hour, minute })),
        None => Ok((i, HmTime { hour, minute: 0})),
    }
}
