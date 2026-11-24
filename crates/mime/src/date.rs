//! The `Date` header (RFC 5322 §3.3) as plain numbers.
//!
//! No date type of any library is in the public API: a caller turns a [`DateTime`] into whatever
//! it keeps time in. `chrono` is used inside, for the weekday and for whether a date exists.

use chrono::{Datelike, NaiveDate};

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// A wall-clock date and time and the zone it is said in. `2003-07-01 10:52:37 +0200` is
/// `offset_minutes: 120`; the instant is the wall clock minus the offset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DateTime {
    pub year: i32,
    /// 1 to 12.
    pub month: u8,
    /// 1 to 31.
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Minutes east of UTC.
    pub offset_minutes: i32,
}

impl DateTime {
    /// The date, when it exists and the clock is one.
    fn date(&self) -> Option<NaiveDate> {
        (self.hour < 24 && self.minute < 60 && self.second < 60)
            .then(|| NaiveDate::from_ymd_opt(self.year, u32::from(self.month), u32::from(self.day)))
            .flatten()
            .filter(|_| (0..=9999).contains(&self.year))
    }

    /// The short names of the weekday and the month.
    fn names(&self) -> Option<(&'static str, &'static str)> {
        let weekday = WEEKDAYS.get(self.date()?.weekday().num_days_from_monday() as usize)?;
        Some((weekday, MONTHS.get(usize::from(self.month).checked_sub(1)?)?))
    }

    /// `Tue Jul  1 08:52:37 2003`, as the separator line of an mbox file has it. `None` when
    /// the date doesn't exist.
    pub(crate) fn asctime(&self) -> Option<String> {
        let (weekday, month) = self.names()?;
        Some(format!("{weekday} {month} {:>2} {:02}:{:02}:{:02} {:04}", self.day, self.hour, self.minute, self.second, self.year))
    }
}

/// `Tue, 01 Jul 2003 10:52:37 +0200`. `None` when the date or the time doesn't exist (the year
/// must be 0 to 9999); the offset is kept within a day.
pub fn format_date(date: &DateTime) -> Option<String> {
    let (weekday, month) = date.names()?;
    let offset = date.offset_minutes.clamp(-24 * 60, 24 * 60);
    let sign = if offset < 0 { '-' } else { '+' };
    let a = offset.unsigned_abs();
    Some(format!(
        "{weekday}, {:02} {month} {:04} {:02}:{:02}:{:02} {sign}{:02}{:02}",
        date.day,
        date.year,
        date.hour,
        date.minute,
        date.second,
        a / 60,
        a % 60
    ))
}

/// Parse an RFC 5322 date, leniently: a missing weekday or seconds, two-digit years, obsolete
/// zone names, comments. `None` when it is no date, or one that doesn't exist. A leap second
/// (60) is read as the last second of the minute; a zone that can't be read is UTC.
pub fn parse_date(s: &str) -> Option<DateTime> {
    let s = strip_comments(s);
    // The weekday, when present, ends at the first comma.
    let s = match s.split_once(',') {
        Some((day, rest)) if is_weekday(day) || day.trim().is_empty() => rest,
        _ => s.as_str(),
    };
    let mut it = s.split_whitespace();
    let day: u8 = it.next()?.parse().ok()?;
    let month = it.next()?;
    let month = MONTHS.iter().position(|m| month.get(..3).is_some_and(|p| p.eq_ignore_ascii_case(m)))?;
    let year: i32 = match it.next()? {
        y if y.len() <= 2 => {
            // RFC 5322 §4.3: two-digit years 00–49 are 2000–2049, 50–99 are 1950–1999.
            let y: i32 = y.parse::<u8>().ok()?.into();
            if y < 50 { 2000 + y } else { 1900 + y }
        }
        y if y.len() == 3 => 1900 + i32::from(y.parse::<u16>().ok()?),
        y => y.parse::<u16>().ok()?.into(),
    };
    let mut clock = it.next()?.split(':');
    let hour: u8 = clock.next()?.parse().ok()?;
    let minute: u8 = clock.next()?.parse().ok()?;
    let second: u8 = clock.next().map_or(Some(0), |x| x.parse().ok())?.min(59);
    let offset_minutes = it.next().map_or(0, zone_offset);
    let date = DateTime { year, month: u8::try_from(month).ok()?.checked_add(1)?, day, hour, minute, second, offset_minutes };
    date.date().map(|_| date)
}

fn is_weekday(s: &str) -> bool {
    let s = s.trim();
    WEEKDAYS.iter().any(|w| s.get(..3).is_some_and(|p| p.eq_ignore_ascii_case(w)))
}

/// Minutes east of UTC for `+0200`, `-0500`, `Z`, `GMT`, `EST`…; unknown names are UTC.
fn zone_offset(z: &str) -> i32 {
    let z = z.trim();
    if let Some(digits) = z.strip_prefix(['+', '-']) {
        let digits: String = digits.chars().filter(char::is_ascii_digit).collect();
        let (h, m) = match digits.len() {
            4 => (digits.get(0..2), digits.get(2..4)),
            2 => (digits.get(0..2), Some("0")),
            _ => (None, None),
        };
        let (Some(h), Some(m)) = (h.and_then(|h| h.parse::<u8>().ok()), m.and_then(|m| m.parse::<u8>().ok())) else { return 0 };
        if h > 23 || m > 59 {
            return 0;
        }
        let total = i32::from(h) * 60 + i32::from(m);
        return if z.starts_with('-') { -total } else { total };
    }
    match z.to_ascii_uppercase().as_str() {
        "EDT" => -4 * 60,
        "EST" | "CDT" => -5 * 60,
        "CST" | "MDT" => -6 * 60,
        "MST" | "PDT" => -7 * 60,
        "PST" => -8 * 60,
        _ => 0,
    }
}

/// Remove `(comments)`, which may nest.
fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0u32;
    for c in s.chars() {
        match c {
            '(' => depth = depth.saturating_add(1),
            ')' if depth > 0 => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8, offset_minutes: i32) -> DateTime {
        DateTime { year, month, day, hour, minute, second, offset_minutes }
    }

    #[test]
    fn reads_the_usual_and_the_obsolete_forms() {
        assert_eq!(parse_date("Tue, 1 Jul 2003 10:52:37 +0200"), Some(at(2003, 7, 1, 10, 52, 37, 120)));
        assert_eq!(parse_date("1 Jul 2003 08:52:37 GMT"), Some(at(2003, 7, 1, 8, 52, 37, 0)));
        assert_eq!(parse_date("  tuesday,  01 JULY 2003 08:52 -0500 (EST)"), Some(at(2003, 7, 1, 8, 52, 0, -300)));
        assert_eq!(parse_date("(a (nested) comment) 1 Jul 03 08:52:60 PST"), Some(at(2003, 7, 1, 8, 52, 59, -480)));
        assert_eq!(parse_date("1 Jul 99 00:00:00 Z").map(|d| d.year), Some(1999));
        assert_eq!(parse_date("1 Jul 103 00:00:00").map(|d| d.year), Some(2003));
        assert_eq!(parse_date("29 Feb 2024 12:00:00 +0530"), Some(at(2024, 2, 29, 12, 0, 0, 330)));
        // A zone that can't be read is UTC.
        for zone in ["+9999", "-12", "+123", "Mars", "+2460"] {
            let want = if zone == "-12" { -720 } else { 0 };
            assert_eq!(parse_date(&format!("1 Jul 2003 08:52:37 {zone}")).map(|d| d.offset_minutes), Some(want), "{zone}");
        }
    }

    #[test]
    fn what_is_no_date_is_none() {
        for bad in [
            "",
            "next Tuesday",
            "99 Abc 99999 99:99",
            "31 Feb 2003 10:00:00",
            "29 Feb 2023 10:00:00",
            "1 Jul 2003",
            "1 Jul 2003 24:00:00",
            "1 Jul 2003 10:60",
            "0 Jul 2003 10:00",
            "1 Jul 99999 10:00",
            "Foo, 1 Jul 2003 10:00",
            "1 J 2003 10:00",
            "1 Jül 2003 10:00",
        ] {
            assert_eq!(parse_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn writes_what_it_reads() {
        assert_eq!(format_date(&at(2003, 7, 1, 10, 52, 37, 120)).as_deref(), Some("Tue, 01 Jul 2003 10:52:37 +0200"));
        assert_eq!(format_date(&at(1970, 1, 1, 0, 0, 0, 0)).as_deref(), Some("Thu, 01 Jan 1970 00:00:00 +0000"));
        assert_eq!(format_date(&at(2024, 2, 29, 23, 59, 59, -570)).as_deref(), Some("Thu, 29 Feb 2024 23:59:59 -0930"));
        assert_eq!(format_date(&at(2003, 7, 1, 0, 0, 0, i32::MIN)).as_deref(), Some("Tue, 01 Jul 2003 00:00:00 -2400"));
        for bad in [
            at(2003, 2, 30, 0, 0, 0, 0),
            at(2003, 0, 1, 0, 0, 0, 0),
            at(2003, 13, 1, 0, 0, 0, 0),
            at(2003, 7, 1, 24, 0, 0, 0),
            at(10_000, 1, 1, 0, 0, 0, 0),
            at(-1, 1, 1, 0, 0, 0, 0),
        ] {
            assert_eq!(format_date(&bad), None, "{bad:?}");
        }
        assert_eq!(at(2003, 7, 1, 8, 52, 37, 0).asctime().as_deref(), Some("Tue Jul  1 08:52:37 2003"));
    }

    proptest::proptest! {
        #[test]
        fn any_text_parses(s in proptest::prelude::any::<String>()) {
            let _ = parse_date(&s);
        }

        #[test]
        fn date_soup_parses(s in "[0-9a-zA-Z,:+() -]{0,40}") {
            if let Some(date) = parse_date(&s) {
                proptest::prop_assert!(format_date(&date).is_some());
            }
        }

        #[test]
        fn any_numbers_format_and_read_back(y in -5i32..10_005, mo in 0u8..14, d in 0u8..33, h in 0u8..26, mi in 0u8..62, s in 0u8..62, off in -1500i32..1500) {
            let date = at(y, mo, d, h, mi, s, off);
            if let Some(text) = format_date(&date) {
                // Offsets are written whole within a day, and a day's worth of hours is no zone.
                if off.abs() < 24 * 60 && y >= 1000 {
                    proptest::prop_assert_eq!(parse_date(&text), Some(date));
                }
            }
        }
    }
}
