//! Recurrence rules (RFC 5545 §3.3.10): the common subset a calendar keeps.

use chrono::{Datelike, NaiveDate, NaiveDateTime, Weekday};

use crate::lines::prefix;
use crate::values::{When, format_date, valid_time};

type Timestamp = i64;

/// How often a rule repeats. `SECONDLY`, `MINUTELY` and `HOURLY` are not held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Freq {
    Daily,
    #[default]
    Weekly,
    Monthly,
    Yearly,
}

impl Freq {
    /// From `DAILY`, `weekly`…: any letter case.
    pub fn parse(s: &str) -> Option<Freq> {
        match s.trim().to_ascii_uppercase().as_str() {
            "DAILY" => Some(Freq::Daily),
            "WEEKLY" => Some(Freq::Weekly),
            "MONTHLY" => Some(Freq::Monthly),
            "YEARLY" => Some(Freq::Yearly),
            _ => None,
        }
    }

    /// `DAILY`, `WEEKLY`, `MONTHLY`, `YEARLY`.
    pub fn as_str(self) -> &'static str {
        match self {
            Freq::Daily => "DAILY",
            Freq::Weekly => "WEEKLY",
            Freq::Monthly => "MONTHLY",
            Freq::Yearly => "YEARLY",
        }
    }
}

/// How an event repeats: the subset of RRULE that calendars show and edit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recurrence {
    pub freq: Freq,
    /// Every `interval` days/weeks/months/years; 0 reads as 1.
    pub interval: u32,
    /// Weekly: the days of the week (none: the start's own). Daily: only on these days. Monthly and
    /// yearly with `nth`: the weekday(s) counted.
    pub by_day: Vec<Weekday>,
    /// Monthly and yearly: the `nth` `by_day` weekday of the month (1–5, or -1 for the last)
    /// instead of the start's day of the month.
    pub nth: Option<i8>,
    /// Ends after this many occurrences.
    pub count: Option<u32>,
    /// Ends on this day (inclusive).
    pub until: Option<NaiveDate>,
}

impl Recurrence {
    /// Every day, week, month or year.
    pub fn every(freq: Freq) -> Recurrence {
        Recurrence { freq, interval: 1, ..Default::default() }
    }
}

/// The days of the week, Monday first.
pub const WEEKDAYS: [Weekday; 7] = [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun];

/// `MO`, `TU`… as iCalendar writes a weekday.
pub fn weekday_code(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "MO",
        Weekday::Tue => "TU",
        Weekday::Wed => "WE",
        Weekday::Thu => "TH",
        Weekday::Fri => "FR",
        Weekday::Sat => "SA",
        Weekday::Sun => "SU",
    }
}

/// The weekday of `MO`, `tu`…; `None` for anything else.
pub fn weekday_of_code(s: &str) -> Option<Weekday> {
    WEEKDAYS.into_iter().find(|d| weekday_code(*d).eq_ignore_ascii_case(s.trim()))
}

/// Longest rule read, in bytes, and most parts (and most `BYDAY` days) of one rule.
const MAX_RULE: usize = 4096;
const MAX_PARTS: usize = 64;

/// `FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE` as a [`Recurrence`]; `None` for a rule the model can't hold
/// (`SECONDLY`, `MINUTELY`, `HOURLY`, junk).
///
/// `UNTIL` gives its date, whether it is written as a date or a date-time. `BYDAY=2TU`, `BYDAY=-1FR`
/// and `BYDAY=TU;BYSETPOS=2` give `nth`. Parts that only restate the start (`BYMONTH`,
/// `BYMONTHDAY`, `WKST`) and parts the model has no place for are ignored.
pub fn parse_rrule(rule: &str) -> Option<Recurrence> {
    parse_with(rule, None)
}

/// [`parse_rrule`] for a rule read from a file: with `offset_at` (minutes east of UTC at an
/// instant), a UTC `UNTIL` gives its local day.
pub fn parse_with(rule: &str, offset_at: Option<&dyn Fn(Timestamp) -> i32>) -> Option<Recurrence> {
    let rule = prefix(rule.trim(), MAX_RULE);
    let rule = match rule.get(..6) {
        Some(name) if name.eq_ignore_ascii_case("RRULE:") => rule.get(6..).unwrap_or(""),
        _ => rule,
    };
    let mut freq = None;
    let mut interval = 1u32;
    let (mut count, mut until, mut set_pos) = (None, None, None);
    let mut days: Vec<(Option<i32>, Weekday)> = Vec::new();
    // Parts that pick days in a way the model can't: with them, a bare BYDAY means something else.
    let mut limited = false;
    for part in rule.split(';').take(MAX_PARTS) {
        let Some((key, value)) = part.split_once('=') else { continue };
        let value = value.trim();
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => freq = Some(Freq::parse(value)?),
            "INTERVAL" => interval = value.parse().ok().filter(|n| *n > 0).unwrap_or(1),
            "COUNT" => count = value.parse::<u32>().ok(),
            "UNTIL" => {
                until = When::parse(value).map(|w| match offset_at {
                    Some(offset_at) => w.local(offset_at).date(),
                    None => w.wall().date(),
                })
            }
            "BYDAY" => days = value.split(',').take(MAX_PARTS).filter_map(by_day).collect(),
            "BYSETPOS" => set_pos = value.parse::<i32>().ok(),
            "BYMONTH" | "BYMONTHDAY" | "BYYEARDAY" | "BYWEEKNO" => limited = true,
            _ => {}
        }
    }
    let mut freq = freq?;
    let by_month = matches!(freq, Freq::Monthly | Freq::Yearly);
    // `1MO,3MO` isn't one position: the first one is kept.
    let ordinal = days.iter().find_map(|(ordinal, _)| *ordinal);
    if by_month && ordinal.is_some() {
        days.retain(|(o, _)| *o == ordinal);
    }
    let mut by_day: Vec<Weekday> = Vec::new();
    for (_, day) in &days {
        if !by_day.contains(day) {
            by_day.push(*day);
        }
    }
    let mut nth = None;
    if by_month {
        nth = match (ordinal, set_pos) {
            (Some(ordinal), _) => position(ordinal),
            // A position in a set of several weekdays ("the last working day") isn't a weekday's position.
            (None, Some(pos)) if by_day.len() == 1 || (by_day.is_empty() && !limited) => position(pos),
            _ => None,
        };
        if nth.is_none() {
            if freq == Freq::Monthly && interval == 1 && !by_day.is_empty() && ordinal.is_none() && set_pos.is_none() && !limited {
                // Every Monday of every month is every Monday.
                freq = Freq::Weekly;
            } else {
                by_day.clear();
            }
        }
    }
    Some(Recurrence { freq, interval, by_day, nth, count, until })
}

/// `MO`, `2TU`, `-1FR`: the position, when there is one, and the day.
fn by_day(item: &str) -> Option<(Option<i32>, Weekday)> {
    let item = item.trim();
    let split = item.len().checked_sub(2)?;
    let day = weekday_of_code(item.get(split..)?)?;
    let ordinal = item.get(..split)?;
    Some((if ordinal.is_empty() { None } else { Some(ordinal.parse::<i32>().ok()?) }, day))
}

/// A position the model holds: the first to the fifth, or the last.
fn position(n: i32) -> Option<i8> {
    if (1..=5).contains(&n) || n == -1 { i8::try_from(n).ok() } else { None }
}

/// A [`Recurrence`] as an RRULE value: `FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE`.
///
/// An `nth` without a `by_day` means "the start's weekday", which a rule alone can't say: it is
/// written as a bare `BYSETPOS`, which [`parse_rrule`] reads back but other programs may not.
/// [`format_for`] knows the start and writes the weekday (and the month of a yearly rule) out. For monthly and yearly rules without an `nth` the model ignores `by_day`, so it isn't
/// written.
pub fn format_rrule(r: &Recurrence) -> String {
    format(r, None, None)
}

/// The rule of an event that starts at `start`: the start's weekday and month are spelled out
/// where the rule leans on them, and `UNTIL` has the start's value type.
pub fn format_for(r: &Recurrence, start: NaiveDateTime, all_day: bool) -> String {
    format(r, Some((valid_time(start), all_day)), None)
}

/// The rule of a timed event whose start is written in a zone: as [`format_for`], with `UNTIL`
/// as the instant `until` in UTC, which is how RFC 5545 has it beside such a start.
pub fn format_zoned(r: &Recurrence, start: NaiveDateTime, until: Option<Timestamp>) -> String {
    format(r, Some((valid_time(start), false)), until)
}

fn format(r: &Recurrence, start: Option<(NaiveDateTime, bool)>, until_at: Option<Timestamp>) -> String {
    let mut out = format!("FREQ={}", r.freq.as_str());
    if r.interval > 1 {
        out.push_str(&format!(";INTERVAL={}", r.interval));
    }
    let mut days: Vec<Weekday> = Vec::new();
    for day in &r.by_day {
        if !days.contains(day) {
            days.push(*day);
        }
    }
    let list = |days: &[Weekday], position: &str| days.iter().map(|d| format!("{position}{}", weekday_code(*d))).collect::<Vec<_>>().join(",");
    match (r.freq, r.nth.filter(|n| *n != 0)) {
        (Freq::Daily | Freq::Weekly, _) => {
            if !days.is_empty() {
                out.push_str(&format!(";BYDAY={}", list(&days, "")));
            }
        }
        (_, None) => {}
        (freq, Some(nth)) => {
            // Any negative position is "the last" to the model.
            let nth = nth.max(-1);
            if let Some((start, _)) = start {
                if days.is_empty() {
                    days.push(start.date().weekday());
                }
                if freq == Freq::Yearly {
                    out.push_str(&format!(";BYMONTH={}", start.date().month()));
                }
            }
            if days.is_empty() {
                out.push_str(&format!(";BYSETPOS={nth}"));
            } else {
                out.push_str(&format!(";BYDAY={}", list(&days, &nth.to_string())));
            }
        }
    }
    if let Some(count) = r.count {
        out.push_str(&format!(";COUNT={count}"));
    }
    if let (Some(_), Some(at)) = (r.until, until_at) {
        out.push_str(&format!(";UNTIL={}", crate::values::format_utc(at)));
    } else if let Some(until) = r.until {
        out.push_str(&format!(";UNTIL={}", format_date(until)));
        // The whole last day of a timed series.
        if matches!(start, Some((_, false))) {
            out.push_str("T235959");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::values::at_midnight;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn rule(freq: Freq) -> Recurrence {
        Recurrence::every(freq)
    }

    #[test]
    fn parses_the_common_rules() {
        assert_eq!(parse_rrule("FREQ=DAILY"), Some(rule(Freq::Daily)));
        assert_eq!(
            parse_rrule("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE"),
            Some(Recurrence { interval: 2, by_day: vec![Weekday::Mon, Weekday::Wed], ..rule(Freq::Weekly) })
        );
        assert_eq!(
            parse_rrule("freq=weekly;byday=fr,mo;count=10;wkst=su"),
            Some(Recurrence { by_day: vec![Weekday::Fri, Weekday::Mon], count: Some(10), ..rule(Freq::Weekly) })
        );
        assert_eq!(
            parse_rrule("FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR"),
            Some(Recurrence { by_day: vec![Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri], ..rule(Freq::Daily) })
        );
        assert_eq!(parse_rrule("FREQ=MONTHLY;INTERVAL=3"), Some(Recurrence { interval: 3, ..rule(Freq::Monthly) }));
        assert_eq!(parse_rrule("FREQ=YEARLY"), Some(rule(Freq::Yearly)));
        // The property name and stray spaces are let through.
        assert_eq!(parse_rrule(" RRULE:FREQ=DAILY; INTERVAL = 4 ;"), Some(Recurrence { interval: 4, ..rule(Freq::Daily) }));
    }

    #[test]
    fn positions_come_from_ordinals_or_bysetpos() {
        let second_tue = Recurrence { by_day: vec![Weekday::Tue], nth: Some(2), ..rule(Freq::Monthly) };
        assert_eq!(parse_rrule("FREQ=MONTHLY;BYDAY=2TU"), Some(second_tue.clone()));
        assert_eq!(parse_rrule("FREQ=MONTHLY;BYDAY=+2TU"), Some(second_tue.clone()));
        assert_eq!(parse_rrule("FREQ=MONTHLY;BYDAY=TU;BYSETPOS=2"), Some(second_tue));
        let last_fri = Recurrence { by_day: vec![Weekday::Fri], nth: Some(-1), ..rule(Freq::Monthly) };
        assert_eq!(parse_rrule("FREQ=MONTHLY;BYDAY=-1FR"), Some(last_fri.clone()));
        assert_eq!(parse_rrule("FREQ=MONTHLY;BYSETPOS=-1;BYDAY=FR"), Some(last_fri));
        // The fourth Thursday of November: the month restates the start.
        assert_eq!(
            parse_rrule("FREQ=YEARLY;BYMONTH=11;BYDAY=4TH"),
            Some(Recurrence { by_day: vec![Weekday::Thu], nth: Some(4), ..rule(Freq::Yearly) })
        );
        assert_eq!(
            parse_rrule("FREQ=MONTHLY;BYDAY=1MO,1FR"),
            Some(Recurrence { by_day: vec![Weekday::Mon, Weekday::Fri], nth: Some(1), ..rule(Freq::Monthly) })
        );
        // Positions mean nothing in a weekly rule.
        assert_eq!(parse_rrule("FREQ=WEEKLY;BYDAY=2TU"), Some(Recurrence { by_day: vec![Weekday::Tue], ..rule(Freq::Weekly) }));
    }

    #[test]
    fn rules_the_model_cannot_hold_fall_back() {
        // Two positions: the first is kept.
        assert_eq!(
            parse_rrule("FREQ=MONTHLY;BYDAY=1MO,3MO,1WE"),
            Some(Recurrence { by_day: vec![Weekday::Mon, Weekday::Wed], nth: Some(1), ..rule(Freq::Monthly) })
        );
        // The last working day, the second-to-last Friday, Friday the 13th and the like: plain monthly.
        for r in [
            "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
            "FREQ=MONTHLY;BYDAY=-2FR",
            "FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=13",
            "FREQ=MONTHLY;BYMONTHDAY=15",
            "FREQ=MONTHLY;BYMONTHDAY=1,2,3;BYSETPOS=1",
        ] {
            assert_eq!(parse_rrule(r), Some(rule(Freq::Monthly)), "{r}");
        }
        // The 20th Monday of the year.
        assert_eq!(parse_rrule("FREQ=YEARLY;BYDAY=20MO"), Some(rule(Freq::Yearly)));
        // Every Monday and Thursday of every month is weekly; every other month isn't.
        assert_eq!(parse_rrule("FREQ=MONTHLY;BYDAY=MO,TH"), Some(Recurrence { by_day: vec![Weekday::Mon, Weekday::Thu], ..rule(Freq::Weekly) }));
        assert_eq!(parse_rrule("FREQ=MONTHLY;INTERVAL=2;BYDAY=MO"), Some(Recurrence { interval: 2, ..rule(Freq::Monthly) }));
        assert_eq!(parse_rrule("FREQ=YEARLY;BYMONTH=3,6,9"), Some(rule(Freq::Yearly)));
    }

    #[test]
    fn until_is_a_date_whatever_its_form() {
        let until = |r: &str| parse_rrule(r).and_then(|r| r.until);
        assert_eq!(until("FREQ=DAILY;UNTIL=20261231"), Some(d(2026, 12, 31)));
        assert_eq!(until("FREQ=DAILY;UNTIL=20261231T235959"), Some(d(2026, 12, 31)));
        assert_eq!(until("FREQ=DAILY;UNTIL=20261231T030000Z"), Some(d(2026, 12, 31)));
        assert_eq!(until("FREQ=DAILY;UNTIL=someday"), None);
        // Read from a file, a UTC time is a local day.
        let west = parse_with("FREQ=DAILY;UNTIL=20261231T030000Z", Some(&|_| -300)).unwrap();
        assert_eq!(west.until, Some(d(2026, 12, 30)));
        let east = parse_with("FREQ=DAILY;UNTIL=20261230T220000Z", Some(&|_| 600)).unwrap();
        assert_eq!(east.until, Some(d(2026, 12, 31)));
        assert_eq!(parse_with("FREQ=DAILY;UNTIL=20261231", Some(&|_| -300)).unwrap().until, Some(d(2026, 12, 31)));
    }

    #[test]
    fn junk_and_fast_rules_are_none() {
        for bad in [
            "",
            "FREQ=",
            "FREQ=HOURLY",
            "FREQ=MINUTELY;INTERVAL=5",
            "FREQ=SECONDLY",
            "FREQ=SOMETIMES",
            "INTERVAL=2;BYDAY=MO",
            "weekly",
            ";;;===",
            "RRULE:",
        ] {
            assert_eq!(parse_rrule(bad), None, "{bad}");
        }
    }

    #[test]
    fn junk_parts_are_ignored() {
        let got = parse_rrule("FREQ=WEEKLY;INTERVAL=0;COUNT=many;BYDAY=MO,XX,,7,MO,é;BYSETPOS=x;X-THING=1;novalue").unwrap();
        assert_eq!(got, Recurrence { by_day: vec![Weekday::Mon], ..rule(Freq::Weekly) });
        assert_eq!(parse_rrule("FREQ=DAILY;INTERVAL=99999999999;COUNT=99999999999").unwrap(), rule(Freq::Daily));
        assert_eq!(parse_rrule("FREQ=DAILY;COUNT=0").unwrap().count, Some(0));
        let long = format!("FREQ=WEEKLY;BYDAY={}", "MO,".repeat(100_000));
        assert_eq!(parse_rrule(&long).unwrap().by_day, [Weekday::Mon]);
    }

    #[test]
    fn formats_rules() {
        assert_eq!(format_rrule(&rule(Freq::Daily)), "FREQ=DAILY");
        assert_eq!(
            format_rrule(&Recurrence { interval: 2, by_day: vec![Weekday::Mon, Weekday::Wed, Weekday::Mon], ..rule(Freq::Weekly) }),
            "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE"
        );
        assert_eq!(format_rrule(&Recurrence { count: Some(10), ..rule(Freq::Yearly) }), "FREQ=YEARLY;COUNT=10");
        assert_eq!(format_rrule(&Recurrence { until: Some(d(2026, 12, 31)), ..rule(Freq::Monthly) }), "FREQ=MONTHLY;UNTIL=20261231");
        assert_eq!(format_rrule(&Recurrence { by_day: vec![Weekday::Tue], nth: Some(2), ..rule(Freq::Monthly) }), "FREQ=MONTHLY;BYDAY=2TU");
        assert_eq!(
            format_rrule(&Recurrence { by_day: vec![Weekday::Mon, Weekday::Fri], nth: Some(-1), ..rule(Freq::Monthly) }),
            "FREQ=MONTHLY;BYDAY=-1MO,-1FR"
        );
        assert_eq!(format_rrule(&Recurrence { nth: Some(3), ..rule(Freq::Monthly) }), "FREQ=MONTHLY;BYSETPOS=3");
        // Interval 0 reads as 1, and the model ignores the days of a monthly rule without a position.
        assert_eq!(format_rrule(&Recurrence { interval: 0, by_day: vec![Weekday::Mon], nth: Some(0), ..rule(Freq::Monthly) }), "FREQ=MONTHLY");
    }

    #[test]
    fn an_event_s_rule_spells_out_what_the_start_implies() {
        let tuesday = d(2026, 10, 13).and_hms_opt(9, 0, 0).unwrap();
        let r = Recurrence { nth: Some(2), until: Some(d(2027, 6, 30)), ..rule(Freq::Monthly) };
        assert_eq!(format_for(&r, tuesday, false), "FREQ=MONTHLY;BYDAY=2TU;UNTIL=20270630T235959");
        assert_eq!(format_for(&r, tuesday, true), "FREQ=MONTHLY;BYDAY=2TU;UNTIL=20270630");
        let thanksgiving = Recurrence { by_day: vec![Weekday::Thu], nth: Some(4), ..rule(Freq::Yearly) };
        assert_eq!(format_for(&thanksgiving, at_midnight(d(2026, 11, 26)), true), "FREQ=YEARLY;BYMONTH=11;BYDAY=4TH");
        assert_eq!(format_for(&Recurrence { nth: Some(i8::MIN), ..rule(Freq::Yearly) }, tuesday, false), "FREQ=YEARLY;BYMONTH=10;BYDAY=-1TU");
        // What it writes reads back as the same series.
        assert_eq!(parse_rrule("FREQ=YEARLY;BYMONTH=11;BYDAY=4TH"), Some(thanksgiving));
        // A start in a year iCalendar can't write is moved into the years it can.
        let written = format_for(&Recurrence { nth: Some(1), until: Some(NaiveDate::MAX), ..rule(Freq::Yearly) }, NaiveDateTime::MAX, true);
        assert_eq!(written, "FREQ=YEARLY;BYMONTH=12;BYDAY=1FR;UNTIL=99991231");
    }

    /// Every shape the model gives a meaning: weekly and daily rules with any days, monthly and
    /// yearly rules with a position (and then any days) or without.
    fn shapes() -> Vec<Recurrence> {
        let day_sets: [&[Weekday]; 5] = [&[], &[Weekday::Tue], &[Weekday::Sun, Weekday::Mon], &[Weekday::Fri, Weekday::Mon, Weekday::Wed], &WEEKDAYS];
        let mut out = Vec::new();
        for freq in [Freq::Daily, Freq::Weekly, Freq::Monthly, Freq::Yearly] {
            for interval in [1, 2, 13, u32::MAX] {
                for count in [None, Some(0), Some(1), Some(u32::MAX)] {
                    for until in [None, Some(d(2030, 1, 31)), Some(d(1, 1, 1)), Some(d(9999, 12, 31))] {
                        for days in day_sets {
                            let positions: &[Option<i8>] = if matches!(freq, Freq::Monthly | Freq::Yearly) {
                                &[None, Some(1), Some(2), Some(3), Some(4), Some(5), Some(-1)]
                            } else {
                                &[None]
                            };
                            for nth in positions {
                                if nth.is_none() && !days.is_empty() && matches!(freq, Freq::Monthly | Freq::Yearly) {
                                    continue;
                                }
                                out.push(Recurrence { freq, interval, by_day: days.to_vec(), nth: *nth, count, until });
                            }
                        }
                    }
                }
            }
        }
        out
    }

    #[test]
    fn every_shape_the_model_supports_round_trips() {
        let shapes = shapes();
        assert!(shapes.len() > 1000);
        for r in shapes {
            let text = format_rrule(&r);
            assert_eq!(parse_rrule(&text), Some(r.clone()), "{text}");
            // And as an event writes it: the start is a Tuesday, so a missing day reads back as Tuesday.
            for all_day in [false, true] {
                let text = format_for(&r, d(2026, 10, 13).and_hms_opt(9, 30, 0).unwrap(), all_day);
                let mut want = r.clone();
                if want.nth.is_some() && want.by_day.is_empty() {
                    want.by_day = vec![Weekday::Tue];
                }
                assert_eq!(parse_rrule(&text), Some(want), "{text}");
            }
        }
    }

    #[test]
    fn shapes_without_a_meaning_are_normalised() {
        let normal = |r: &Recurrence| parse_rrule(&format_rrule(r)).unwrap();
        // Interval 0 is 1, duplicate days are one, position 0 is none, any negative position is the last.
        assert_eq!(normal(&Recurrence { interval: 0, ..rule(Freq::Weekly) }).interval, 1);
        assert_eq!(normal(&Recurrence { by_day: vec![Weekday::Mon; 100], ..rule(Freq::Weekly) }).by_day, [Weekday::Mon]);
        assert_eq!(normal(&Recurrence { nth: Some(0), by_day: vec![Weekday::Mon], ..rule(Freq::Monthly) }), rule(Freq::Monthly));
        assert_eq!(normal(&Recurrence { nth: Some(-7), by_day: vec![Weekday::Mon], ..rule(Freq::Monthly) }).nth, Some(-1));
        // A position no month has isn't read back.
        assert_eq!(normal(&Recurrence { nth: Some(9), by_day: vec![Weekday::Mon], ..rule(Freq::Monthly) }), rule(Freq::Monthly));
        // A position means nothing in a weekly rule.
        assert_eq!(normal(&Recurrence { nth: Some(2), ..rule(Freq::Weekly) }), rule(Freq::Weekly));
    }

    fn recurrence() -> impl Strategy<Value = Recurrence> {
        let freq = prop_oneof![Just(Freq::Daily), Just(Freq::Weekly), Just(Freq::Monthly), Just(Freq::Yearly)];
        let days = proptest::sample::subsequence(WEEKDAYS.to_vec(), 0..=7).prop_shuffle();
        let nth = proptest::option::of(prop_oneof![1i8..=5, Just(-1i8)]);
        let until = proptest::option::of((1i32..=9999, 1u32..=12, 1u32..=28).prop_map(|(y, m, day)| d(y, m, day)));
        (freq, 1u32.., days, nth, proptest::option::of(any::<u32>()), until).prop_map(|(freq, interval, by_day, nth, count, until)| {
            let monthly = matches!(freq, Freq::Monthly | Freq::Yearly);
            let nth = if monthly { nth } else { None };
            let by_day = if monthly && nth.is_none() { Vec::new() } else { by_day };
            Recurrence { freq, interval, by_day, nth, count, until }
        })
    }

    proptest! {
        #[test]
        fn any_supported_rule_round_trips(r in recurrence()) {
            prop_assert_eq!(parse_rrule(&format_rrule(&r)), Some(r));
        }

        #[test]
        fn any_rule_at_all_formats_and_reads_back_stably(freq in 0u8..4, interval in any::<u32>(), days in proptest::collection::vec(0usize..7, 0..12), nth in proptest::option::of(any::<i8>()), count in proptest::option::of(any::<u32>())) {
            let freq = [Freq::Daily, Freq::Weekly, Freq::Monthly, Freq::Yearly][usize::from(freq)];
            let r = Recurrence { freq, interval, by_day: days.into_iter().filter_map(|i| WEEKDAYS.get(i).copied()).collect(), nth, count, until: None };
            let text = format_rrule(&r);
            let back = parse_rrule(&text).unwrap();
            // Reading normalises; a normal rule then round-trips.
            prop_assert_eq!(parse_rrule(&format_rrule(&back)), Some(back));
        }

        #[test]
        fn rule_text_never_panics(s in any::<String>(), parts in proptest::collection::vec("(FREQ|INTERVAL|COUNT|UNTIL|BYDAY|BYSETPOS|BYMONTH|X)=[-+A-Z0-9,]{0,12}", 0..8)) {
            let _ = parse_rrule(&s);
            let _ = parse_rrule(&parts.join(";"));
            let _ = parse_with(&parts.join(";"), Some(&|_| i32::MIN));
        }
    }
}
