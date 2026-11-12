//! Property values: dates and times, durations and escaped text.
//!
//! Times are `chrono` values. An instant is an `i64` of seconds since 1970-01-01T00:00:00Z, and an
//! offset of a zone is minutes east of UTC where this crate takes one from a caller and seconds
//! where a `VTIMEZONE` gives one. Every date and time this crate makes lies in the years 1 to 9999,
//! the years iCalendar can write.

use chrono::{DateTime, NaiveDate, NaiveDateTime};

/// Seconds in a day.
pub const DAY: i64 = 86_400;

/// Longest duration read, in seconds (some 10 000 years): sums of it can't overflow.
const MAX_DURATION: i64 = 10_000 * 366 * DAY;

/// The first and the last second iCalendar can write, as seconds since 1970 on the same clock.
const MIN_CIVIL: i64 = -62_135_596_800;
const MAX_CIVIL: i64 = 253_402_300_799;

/// Seconds since 1970-01-01T00:00:00 on the same wall clock (no zone applied).
pub fn civil_seconds(t: NaiveDateTime) -> i64 {
    t.and_utc().timestamp()
}

/// The wall-clock time `secs` seconds after 1970-01-01T00:00:00, kept in the years 1 to 9999.
pub fn from_civil_seconds(secs: i64) -> NaiveDateTime {
    DateTime::from_timestamp(secs.clamp(MIN_CIVIL, MAX_CIVIL), 0).map(|t| t.naive_utc()).unwrap_or_default()
}

/// The instant a wall clock shows in a zone `offset_minutes` east of UTC.
pub fn to_timestamp(t: NaiveDateTime, offset_minutes: i32) -> i64 {
    civil_seconds(t).saturating_sub(i64::from(offset_minutes).saturating_mul(60))
}

/// The wall clock at the instant `ts` in a zone `offset_minutes` east of UTC.
pub fn from_timestamp(ts: i64, offset_minutes: i32) -> NaiveDateTime {
    from_civil_seconds(ts.saturating_add(i64::from(offset_minutes).saturating_mul(60)))
}

/// Midnight at the start of `d`.
pub fn at_midnight(d: NaiveDate) -> NaiveDateTime {
    d.and_time(chrono::NaiveTime::MIN)
}

/// A date and time in the years iCalendar can write: one outside them is moved to the nearest end.
pub(crate) fn valid_time(t: NaiveDateTime) -> NaiveDateTime {
    from_civil_seconds(civil_seconds(t))
}

/// A date from `YYYY-MM-DD` or `YYYYMMDD`, years 1 to 9999.
fn parse_iso_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    let digits: String = s.chars().filter(char::is_ascii_digit).collect();
    if digits.len() != 8 || s.chars().any(|c| !c.is_ascii_digit() && c != '-') {
        return None;
    }
    let year: i32 = digits.get(0..4)?.parse().ok()?;
    if year < 1 {
        return None;
    }
    NaiveDate::from_ymd_opt(year, digits.get(4..6)?.parse().ok()?, digits.get(6..8)?.parse().ok()?)
}

/// A DATE or DATE-TIME value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    /// `20261012`: a whole day.
    Date(NaiveDate),
    /// `20261012T090000`: a wall-clock time, floating or in a named zone.
    Local(NaiveDateTime),
    /// `20261012T090000Z`.
    Utc(NaiveDateTime),
}

impl When {
    /// From `YYYYMMDD`, `YYYYMMDDTHHMMSS` or `YYYYMMDDTHHMMSSZ`; the seconds may be missing, and
    /// the ISO 8601 punctuation some writers add (`2026-10-12T09:00:00Z`) is let through.
    pub fn parse(s: &str) -> Option<When> {
        let s = s.trim();
        if s.len() > 32 || !s.is_ascii() {
            return None;
        }
        let (date, time) = match s.find(['T', 't']) {
            Some(i) => (s.get(..i)?, Some(s.get(i.saturating_add(1)..)?)),
            None => (s, None),
        };
        let date = parse_iso_date(date)?;
        let Some(time) = time else { return Some(When::Date(date)) };
        let (clock, utc) = match time.strip_suffix(['Z', 'z']) {
            Some(clock) => (clock, true),
            None => (time, false),
        };
        let digits: String = clock.chars().filter(|c| *c != ':').collect();
        if !matches!(digits.len(), 4 | 6) || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let part = |at: usize| digits.get(at..at.saturating_add(2)).and_then(|d| d.parse::<u32>().ok());
        // A leap second (60) is read as the last second of the minute.
        let second = if digits.len() == 6 { part(4)?.min(59) } else { 0 };
        let t = date.and_hms_opt(part(0)?, part(2)?, second)?;
        Some(if utc { When::Utc(t) } else { When::Local(t) })
    }

    /// The date and time as written, whatever the zone.
    pub fn wall(self) -> NaiveDateTime {
        match self {
            When::Date(d) => at_midnight(d),
            When::Local(t) | When::Utc(t) => t,
        }
    }

    /// The local wall-clock time: a UTC time is converted, anything else is taken as written.
    /// `offset_at(instant)` is minutes east of UTC.
    pub fn local(self, offset_at: &dyn Fn(i64) -> i32) -> NaiveDateTime {
        match self {
            When::Utc(t) => {
                let ts = to_timestamp(t, 0);
                from_timestamp(ts, offset_at(ts))
            }
            other => other.wall(),
        }
    }

    /// The instant: anything but a UTC time is read in the local zone.
    pub fn timestamp(self, offset_at: &dyn Fn(i64) -> i32) -> i64 {
        match self {
            When::Utc(t) => to_timestamp(t, 0),
            other => {
                let t = other.wall();
                // Asked twice, so a time near a daylight-saving change still gets its own offset.
                let guess = to_timestamp(t, offset_at(to_timestamp(t, 0)));
                to_timestamp(t, offset_at(guess))
            }
        }
    }
}

/// A duration in seconds from `PT15M`, `-PT1H30M`, `P1D`, `P1DT12H`, `P2W`; `None` for junk.
pub fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    let (negative, body) = match s.strip_prefix('-') {
        Some(body) => (true, body),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let body = body.strip_prefix(['P', 'p'])?;
    let (mut total, mut number, mut in_time, mut any) = (0i64, None::<i64>, false, false);
    for c in body.chars() {
        match c.to_ascii_uppercase() {
            '0'..='9' => {
                let digit = i64::from(c.to_digit(10)?);
                number = Some(number.unwrap_or(0).checked_mul(10)?.checked_add(digit)?);
            }
            'T' if number.is_none() => in_time = true,
            unit => {
                let seconds = match (unit, in_time) {
                    ('W', false) => 7 * DAY,
                    ('D', false) => DAY,
                    ('H', true) => 3600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
                total = total.checked_add(number.take()?.checked_mul(seconds)?)?;
                any = true;
            }
        }
    }
    if number.is_some() || !any || total > MAX_DURATION {
        return None;
    }
    Some(if negative { -total } else { total })
}

/// `20261012`.
pub fn format_date(d: NaiveDate) -> String {
    valid_time(at_midnight(d)).format("%Y%m%d").to_string()
}

/// `20261012T090000` (floating).
pub fn format_local(t: NaiveDateTime) -> String {
    valid_time(t).format("%Y%m%dT%H%M%S").to_string()
}

/// `20261012T130000Z`.
pub fn format_utc(ts: i64) -> String {
    format!("{}Z", format_local(from_timestamp(ts, 0)))
}

/// A TEXT value ready to write: `\`, `;`, `,` and line breaks escaped (`\r\n` and `\r` count as
/// one line break), other control characters except tab dropped.
pub fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len().saturating_add(8));
    let mut after_cr = false;
    for c in s.chars() {
        let cr = c == '\r';
        match c {
            '\n' if after_cr => {}
            '\r' | '\n' => out.push_str("\\n"),
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
        after_cr = cr;
    }
    out
}

/// Undo [`escape_text`]: `\n` and `\N` are line breaks, `\\`, `\;`, `\,` (and the `\:` and `\"`
/// some writers add) their second character; any other backslash is kept. Control characters
/// except tab are dropped.
pub fn unescape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(x @ ('\\' | ';' | ',' | ':' | '"')) => out.push(x),
                Some(x) => {
                    out.push('\\');
                    if !x.is_control() || x == '\t' {
                        out.push(x);
                    }
                }
                None => out.push('\\'),
            },
            '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// The first `max` items of a list value, split on its unescaped commas; the items are still
/// escaped.
pub fn split_list(s: &str, max: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut escaped) = (0usize, false);
    for (i, c) in s.char_indices() {
        if out.len() >= max {
            return out;
        }
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            ',' => {
                out.push(s.get(start..i).unwrap_or(""));
                start = i.saturating_add(1);
            }
            _ => {}
        }
    }
    if out.len() < max {
        out.push(s.get(start..).unwrap_or(""));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn at(y: i32, m: u32, day: u32, h: u32, min: u32, s: u32) -> NaiveDateTime {
        d(y, m, day).and_hms_opt(h, min, s).unwrap()
    }

    #[test]
    fn dates_and_times() {
        assert_eq!(When::parse("20261012"), Some(When::Date(d(2026, 10, 12))));
        assert_eq!(When::parse("20261012T090000"), Some(When::Local(at(2026, 10, 12, 9, 0, 0))));
        assert_eq!(When::parse(" 20261012T133015Z "), Some(When::Utc(at(2026, 10, 12, 13, 30, 15))));
        assert_eq!(When::parse("20261012t0900z"), Some(When::Utc(at(2026, 10, 12, 9, 0, 0))));
        assert_eq!(When::parse("2026-10-12T09:00:00Z"), Some(When::Utc(at(2026, 10, 12, 9, 0, 0))));
        // A leap second.
        assert_eq!(When::parse("20161231T235960Z"), Some(When::Utc(at(2016, 12, 31, 23, 59, 59))));
    }

    #[test]
    fn junk_dates_are_none() {
        for bad in [
            "",
            "tomorrow",
            "2026101",
            "202610123",
            "20261312",
            "20260230",
            "00000101",
            "20261012T",
            "20261012T25",
            "20261012T240000",
            "20261012T0960",
            "20261012T090000+0200",
            "２０２６１０１２",
            "20261012T09000000000000000000000000000000000000",
        ] {
            assert_eq!(When::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn utc_times_become_local_and_local_times_become_instants() {
        let utc = When::parse("20261012T130000Z").unwrap();
        assert_eq!(utc.local(&|_| -240), at(2026, 10, 12, 9, 0, 0));
        assert_eq!(utc.local(&|_| 330), at(2026, 10, 12, 18, 30, 0));
        assert_eq!(utc.wall(), at(2026, 10, 12, 13, 0, 0));
        // Across midnight.
        assert_eq!(When::parse("20261012T020000Z").unwrap().local(&|_| -300).date(), d(2026, 10, 11));
        let local = When::parse("20261012T090000").unwrap();
        assert_eq!(local.local(&|_| -240), at(2026, 10, 12, 9, 0, 0));
        assert_eq!(local.timestamp(&|_| -240), utc.timestamp(&|_| 600));
        assert_eq!(When::Date(d(1970, 1, 2)).timestamp(&|_| 60), DAY - 3600);
        // A zone that changes its offset at an instant: each side gets its own.
        let change = to_timestamp(at(2026, 11, 1, 6, 0, 0), 0);
        let zone = |ts: i64| if ts < change { -240 } else { -300 };
        assert_eq!(When::parse("20261101T000000").unwrap().timestamp(&zone), to_timestamp(at(2026, 11, 1, 4, 0, 0), 0));
        assert_eq!(When::parse("20261101T120000").unwrap().timestamp(&zone), to_timestamp(at(2026, 11, 1, 17, 0, 0), 0));
        // A hostile offset is survived.
        let _ = utc.local(&|_| i32::MIN);
        let _ = local.timestamp(&|_| i32::MAX);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT15M"), Some(900));
        assert_eq!(parse_duration("-PT15M"), Some(-900));
        assert_eq!(parse_duration("+PT1H30M"), Some(5400));
        assert_eq!(parse_duration("P1D"), Some(DAY));
        assert_eq!(parse_duration("P1DT12H"), Some(DAY + 12 * 3600));
        assert_eq!(parse_duration("P2W"), Some(14 * DAY));
        assert_eq!(parse_duration("pt0s"), Some(0));
        assert_eq!(parse_duration("-PT0M"), Some(0));
        assert_eq!(parse_duration("PT1H2M3S"), Some(3723));
        for bad in ["", "P", "PT", "15M", "PT15", "P15M", "PT1D", "PTM", "P1X", "P-1D", "P99999999999999999999D", "P9999999999W", "PT1H T"] {
            assert_eq!(parse_duration(bad), None, "{bad}");
        }
    }

    #[test]
    fn formats_and_keeps_dates_in_writable_years() {
        assert_eq!(format_date(d(2026, 1, 2)), "20260102");
        assert_eq!(format_date(d(987, 6, 5)), "09870605");
        assert_eq!(format_local(at(2026, 10, 12, 9, 5, 0)), "20261012T090500");
        assert_eq!(format_utc(0), "19700101T000000Z");
        assert_eq!(format_utc(1_760_082_785), "20251010T075305Z");
        assert_eq!(format_utc(i64::MAX), "99991231T235959Z");
        assert_eq!(format_utc(i64::MIN), "00010101T000000Z");
        // Years iCalendar can't write are moved to the nearest it can.
        for hostile in [NaiveDate::MIN, NaiveDate::MAX, d(0, 1, 1), d(-4, 2, 29), d(10_000, 2, 29)] {
            assert!(When::parse(&format_date(hostile)).is_some(), "{hostile:?}");
            assert!(When::parse(&format_local(at_midnight(hostile))).is_some(), "{hostile:?}");
        }
        assert_eq!(format_date(NaiveDate::MAX), "99991231");
        assert_eq!(format_local(NaiveDateTime::MIN), "00010101T000000");
        // Civil seconds are the same clock read as UTC, and are kept in the years 1 to 9999.
        assert_eq!(civil_seconds(at(1970, 1, 2, 0, 0, 1)), DAY + 1);
        assert_eq!(from_civil_seconds(DAY + 1), at(1970, 1, 2, 0, 0, 1));
        assert_eq!(from_civil_seconds(i64::MAX), at(9999, 12, 31, 23, 59, 59));
        assert_eq!(from_civil_seconds(i64::MIN), at(1, 1, 1, 0, 0, 0));
        assert_eq!(from_timestamp(0, -300), at(1969, 12, 31, 19, 0, 0));
        assert_eq!(to_timestamp(at(1969, 12, 31, 19, 0, 0), -300), 0);
    }

    #[test]
    fn text_escapes() {
        assert_eq!(escape_text("Budget, phase two; \\ done\nNext"), "Budget\\, phase two\\; \\\\ done\\nNext");
        assert_eq!(escape_text("a\r\nb\rc\n\nd\u{0}\u{7f}\te"), "a\\nb\\nc\\n\\nd\te");
        assert_eq!(unescape_text("Budget\\, phase two\\; \\\\ done\\nNext\\NLast"), "Budget, phase two; \\ done\nNext\nLast");
        // What other writers escape without being asked to, and what isn't an escape at all.
        assert_eq!(unescape_text("10\\:30 \\\"quoted\\\" C:\\Users\\x tail\\"), "10:30 \"quoted\" C:\\Users\\x tail\\");
        assert_eq!(unescape_text("a\u{0}b\u{1b}c\td"), "abc\td");
    }

    #[test]
    fn lists_split_on_unescaped_commas() {
        assert_eq!(split_list("Work,Travel\\, abroad,,Home\\\\,x", 10), ["Work", "Travel\\, abroad", "", "Home\\\\", "x"]);
        assert_eq!(split_list("", 10), [""]);
        assert_eq!(split_list("a,b,c,d", 2), ["a", "b"]);
        assert_eq!(split_list("a,b", 2), ["a", "b"]);
        assert!(split_list("a,b", 0).is_empty());
    }

    proptest::proptest! {
        #[test]
        fn clean_text_survives_escaping(s in "(\\PC|[\n\t;,\\\\:\"]){0,80}") {
            proptest::prop_assert_eq!(unescape_text(&escape_text(&s)), s);
        }

        #[test]
        fn any_text_escapes_to_one_clean_line(s in proptest::prelude::any::<String>()) {
            let escaped = escape_text(&s);
            proptest::prop_assert!(!escaped.contains(['\r', '\n']));
            // Escaping what was read gives the same line again.
            proptest::prop_assert_eq!(escape_text(&unescape_text(&escaped)), escaped);
        }

        #[test]
        fn values_never_panic(s in proptest::prelude::any::<String>(), max in 0usize..5) {
            let _ = When::parse(&s);
            let _ = parse_duration(&s);
            let _ = unescape_text(&s);
            let _ = split_list(&s, max);
        }

        #[test]
        fn any_civil_second_reads_back_in_writable_years(secs in proptest::prelude::any::<i64>()) {
            let t = from_civil_seconds(secs);
            proptest::prop_assert_eq!(civil_seconds(t), secs.clamp(MIN_CIVIL, MAX_CIVIL));
            proptest::prop_assert!(When::parse(&format_local(t)).is_some());
        }
    }
}
