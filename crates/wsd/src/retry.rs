//! Retry vocabulary shared by every WSD backend: whether repeating a failed request is safe, and
//! the `Retry-After` header.
//!
//! The crate never retries anything itself. It only reports what it knows: [`RetrySafety`] says
//! whether a repeat could do the work twice (see [`crate::Error::retry_safety`]), and
//! [`parse_retry_after`] turns a header value, however hostile, into a bounded wait.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The longest wait a `Retry-After` header can ask for: one hour. A device (or whatever sits in
/// front of it) that asks for more is read as asking for this much; the application decides how
/// much of it to honour.
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(3600);

/// Header values longer than this are not even looked at.
const MAX_FIELD: usize = 64;

/// What repeating a failed request would do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RetrySafety {
    /// Nothing was delivered or accepted (refused by policy, connect failed, or the device said it
    /// is busy): repeating cannot do the work twice.
    Safe,
    /// The request may have been delivered and acted on, and repeating it is not known to be
    /// harmless: a non-idempotent submit must be reconciled first, never blindly resent.
    OutcomeUnknown,
    /// The request may have been delivered, but the caller says repeating it is harmless (a query
    /// or a cancel).
    Idempotent,
}

impl RetrySafety {
    /// May a backend repeat the request without asking a person or reconciling first?
    pub const fn allows_automatic_retry(self) -> bool {
        !matches!(self, Self::OutcomeUnknown)
    }

    /// A stable lowercase name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::Idempotent => "idempotent",
        }
    }
}

/// Read a `Retry-After` value (RFC 9110 section 10.2.3): delta-seconds, or an HTTP-date in any of
/// its three forms, measured from `now`.
///
/// * The result never exceeds [`MAX_RETRY_AFTER`]; a number too large for 64 bits is the maximum.
/// * A date that is not in the future is a zero wait.
/// * Anything else (empty, signs, fractions, units, lists, control characters, non-ASCII, more
///   than 64 bytes, impossible dates, dates before 1970) is `None`.
/// * Two-digit years (the obsolete RFC 850 form) are read as 1970 to 2069.
pub fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let v = value.trim_matches([' ', '\t']);
    if v.is_empty() || v.len() > MAX_FIELD || !v.bytes().all(|b| b == b' ' || b.is_ascii_graphic()) {
        return None;
    }
    if v.bytes().all(|b| b.is_ascii_digit()) {
        let secs = v.parse::<u64>().unwrap_or(u64::MAX);
        return Some(Duration::from_secs(secs).min(MAX_RETRY_AFTER));
    }
    let when = http_date(v)?;
    let now = now.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(Duration::from_secs(when.saturating_sub(now)).min(MAX_RETRY_AFTER))
}

const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const LONG_DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn digits(text: &str, min: usize, max: usize) -> Option<u32> {
    if text.len() < min || text.len() > max || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn month(text: &str) -> Option<u32> {
    MONTHS.iter().position(|m| m.eq_ignore_ascii_case(text)).and_then(|i| u32::try_from(i + 1).ok())
}

fn weekday_ok(text: &str, names: &[&str]) -> bool {
    names.iter().any(|d| d.eq_ignore_ascii_case(text))
}

/// `hh:mm:ss` as seconds into the day (a leap second 60 is accepted and clamped to 59).
fn clock(text: &str) -> Option<u32> {
    let mut it = text.split(':');
    let (h, m, s) = (digits(it.next()?, 2, 2)?, digits(it.next()?, 2, 2)?, digits(it.next()?, 2, 2)?);
    if it.next().is_some() || h > 23 || m > 59 || s > 60 {
        return None;
    }
    Some(h * 3600 + m * 60 + s.min(59))
}

/// Seconds since the Unix epoch of an HTTP-date (IMF-fixdate, RFC 850 or asctime), or `None`.
fn http_date(text: &str) -> Option<u64> {
    let parts: Vec<&str> = text.split_ascii_whitespace().collect();
    let (day, mon, year, time) = match parts.as_slice() {
        // Sun, 06 Nov 1994 08:49:37 GMT
        [wd, day, mon, year, time, "GMT"] if wd.strip_suffix(',').is_some_and(|w| weekday_ok(w, &DAYS)) => {
            (digits(day, 1, 2)?, month(mon)?, i64::from(digits(year, 4, 4)?), clock(time)?)
        }
        // Sunday, 06-Nov-94 08:49:37 GMT
        [wd, dmy, time, "GMT"] if wd.strip_suffix(',').is_some_and(|w| weekday_ok(w, &LONG_DAYS)) => {
            let mut it = dmy.split('-');
            let (d, m, y) = (it.next()?, it.next()?, it.next()?);
            if it.next().is_some() {
                return None;
            }
            let yy = i64::from(digits(y, 2, 2)?);
            (digits(d, 1, 2)?, month(m)?, if yy < 70 { 2000 + yy } else { 1900 + yy }, clock(time)?)
        }
        // Sun Nov  6 08:49:37 1994
        [wd, mon, day, time, year] if weekday_ok(wd, &DAYS) => (digits(day, 1, 2)?, month(mon)?, i64::from(digits(year, 4, 4)?), clock(time)?),
        _ => return None,
    };
    let days = days_from_civil(year, mon, day)?;
    u64::try_from(days.checked_mul(86_400)?.checked_add(i64::from(time))?).ok()
}

/// Days since 1970-01-01 of a proleptic Gregorian date, `None` for an impossible date or one
/// before 1970.
fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    if year < 1970 || !(1..=12).contains(&month) || day == 0 {
        return None;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let in_month = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day > in_month {
        return None;
    }
    // The well-known days-from-civil algorithm, months counted from March.
    let (m, d) = (i64::from(month), i64::from(day));
    let y = if m <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1994-11-06 08:49:37 UTC, the RFC's example instant.
    const T0: u64 = 784_111_777;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn after(value: &str, now: u64) -> Option<u64> {
        parse_retry_after(value, at(now)).map(|d| d.as_secs())
    }

    #[test]
    fn delta_seconds() {
        assert_eq!(after("0", 0), Some(0));
        assert_eq!(after("120", 0), Some(120));
        assert_eq!(after("  7\t", 0), Some(7), "optional whitespace around the value");
        assert_eq!(after("0005", 0), Some(5));
        assert_eq!(after("3600", 0), Some(3600));
        assert_eq!(after("3601", 0), Some(3600), "capped");
    }

    #[test]
    fn the_three_date_forms_are_the_same_instant() {
        for form in ["Sun, 06 Nov 1994 08:49:37 GMT", "Sunday, 06-Nov-94 08:49:37 GMT", "Sun Nov  6 08:49:37 1994", "Sun Nov 6 08:49:37 1994"] {
            assert_eq!(after(form, T0 - 90), Some(90), "{form}");
            assert_eq!(after(form, T0), Some(0), "{form}");
            assert_eq!(after(form, T0 + 5), Some(0), "a past date is no wait: {form}");
            assert_eq!(after(form, T0 - 100_000), Some(3600), "a far date is capped: {form}");
        }
    }

    #[test]
    fn dates_are_validated() {
        let now = T0 - 10;
        for ok in ["Mon, 29 Feb 2088 00:00:00 GMT", "Sun, 06 Nov 1994 23:59:60 GMT"] {
            assert!(after(ok, now).is_some(), "{ok}");
        }
        for bad in [
            "Mon, 29 Feb 2087 00:00:00 GMT",
            "Sun, 31 Apr 1994 08:49:37 GMT",
            "Sun, 00 Nov 1994 08:49:37 GMT",
            "Sun, 06 Nov 1994 24:00:00 GMT",
            "Sun, 06 Nov 1994 08:60:00 GMT",
            "Sun, 06 Nov 1994 08:49:61 GMT",
            "Sun, 06 Nov 1969 08:49:37 GMT",
            "Sun, 06 Nov 94 08:49:37 GMT",
            "Sun, 06 Nov 1994 08:49:37 UTC",
            "Sun, 06 Nov 1994 08:49:37",
            "Sun, 06 Nov 1994 8:49:37 GMT",
            "Sun, 06 Nov 1994 08:49 GMT",
            "Sun, 06 Foo 1994 08:49:37 GMT",
            "Fun, 06 Nov 1994 08:49:37 GMT",
            "Sun 06 Nov 1994 08:49:37 GMT",
            "Sunday, 06-Nov-94-1 08:49:37 GMT",
            "Sun Nov 6 08:49:37 94",
            "Sun, 06 Nov 99999 08:49:37 GMT",
            "Sun, 06 Nov 1994 08:49:37 GMT extra",
        ] {
            assert_eq!(after(bad, now), None, "{bad}");
        }
    }

    #[test]
    fn hostile_values_are_none_or_capped_and_never_panic() {
        let huge = "9".repeat(60);
        assert_eq!(after(&huge, 0), Some(3600), "a number past u64 is the cap, not an error");
        assert_eq!(after("18446744073709551615", 0), Some(3600));
        assert_eq!(after("18446744073709551616", 0), Some(3600));
        for bad in [
            "", " ", "-1", "+5", "1.5", "1e3", "5s", "5 s", "0x10", "1,2", "5, 10", "\u{0663}", "1\u{0}", "5\n", "5\r\n6", "abc", "NaN", "inf",
            "GMT", ",", "::::", "Sun,", "0 0",
        ] {
            assert_eq!(after(bad, 0), None, "{bad:?}");
        }
        assert_eq!(after(&"1".repeat(65), 0), None, "longer than the field limit");
        assert_eq!(after(&" ".repeat(70), 0), None);
        // A clock before the epoch cannot measure a date, but a number needs no clock.
        let before_epoch = UNIX_EPOCH - Duration::from_secs(5);
        assert_eq!(parse_retry_after("Sun, 06 Nov 1994 08:49:37 GMT", before_epoch), None);
        assert_eq!(parse_retry_after("9", before_epoch), Some(Duration::from_secs(9)));
        // Every prefix of a real date, and a spread of junk, is handled without panicking.
        let date = "Sun, 06 Nov 1994 08:49:37 GMT";
        for i in 0..=date.len() {
            let _ = parse_retry_after(date.get(..i).unwrap_or(""), at(T0));
        }
        for junk in ["\u{ffff}", "Sun, 06 Nov 1994 08:49:37 G\u{301}MT", "Sun, 06 Nov 9999 23:59:59 GMT", "Mon, 31 Dec 9999 23:59:60 GMT"] {
            let _ = parse_retry_after(junk, at(T0));
        }
    }

    #[test]
    fn two_digit_years_pivot_at_1970() {
        // 70 is 1970 (long past, so zero); 69 is 2069 (far future, so the cap).
        assert_eq!(after("Thursday, 01-Jan-70 00:00:00 GMT", T0), Some(0));
        assert_eq!(after("Tuesday, 01-Jan-69 00:00:00 GMT", T0), Some(3600));
    }

    #[test]
    fn days_from_civil_matches_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        assert_eq!(days_from_civil(2000, 3, 1), Some(11_017));
        assert_eq!(days_from_civil(2024, 2, 29), Some(19_782));
        assert_eq!(days_from_civil(1994, 11, 6), Some(i64::try_from(T0 / 86_400).unwrap_or(-1)));
        assert_eq!(days_from_civil(2100, 2, 29), None, "2100 is not a leap year");
    }

    #[test]
    fn safety_names_and_automatic_retry() {
        assert!(RetrySafety::Safe.allows_automatic_retry() && RetrySafety::Idempotent.allows_automatic_retry());
        assert!(!RetrySafety::OutcomeUnknown.allows_automatic_retry());
        assert_eq!(
            [RetrySafety::Safe, RetrySafety::OutcomeUnknown, RetrySafety::Idempotent].map(RetrySafety::name),
            ["safe", "outcome_unknown", "idempotent"]
        );
    }
}
