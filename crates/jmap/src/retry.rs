//! `Retry-After` (RFC 9110 §10.2.3) and the HTTP-date forms it may use (RFC 9110 §5.6.7).
//!
//! The crate reads no clock: a date is kept as it was given, and turned into a delay against a "now" the
//! caller passes to [`RetryAfter::delay_from`]. Every delay, whichever form it came in, is clamped to
//! [`MAX_RETRY_DELAY`], so that a hostile server cannot silence the client for days.

use std::time::Duration;

/// The longest delay a server can impose through `Retry-After`. Defined here and nowhere else.
pub const MAX_RETRY_DELAY: Duration = Duration::from_secs(60 * 60);

const SECONDS_PER_DAY: u64 = 86_400;
const LAST_YEAR: u64 = 9999;

/// A point in time, in whole seconds since 1970-01-01T00:00:00Z, supplied by the caller's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnixTime(u64);

impl UnixTime {
    /// The time `secs` seconds after the Unix epoch.
    pub const fn from_secs(secs: u64) -> Self {
        Self(secs)
    }

    /// Seconds since the Unix epoch.
    pub const fn as_secs(self) -> u64 {
        self.0
    }
}

/// An HTTP-date in any of its three forms (RFC 9110 §5.6.7), not yet tied to a calendar century when it came
/// in the two-digit RFC 850 form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HttpDate {
    year: u16,
    two_digit_year: bool,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

/// What a `Retry-After` field asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryAfter {
    /// A number of seconds, already clamped to [`MAX_RETRY_DELAY`].
    Delay(Duration),
    /// A date; see [`RetryAfter::delay_from`].
    At(HttpDate),
}

const DAY_NAMES: [&[u8]; 7] = [b"Mon", b"Tue", b"Wed", b"Thu", b"Fri", b"Sat", b"Sun"];
const LONG_DAY_NAMES: [&[u8]; 7] = [b"Monday", b"Tuesday", b"Wednesday", b"Thursday", b"Friday", b"Saturday", b"Sunday"];
const MONTHS: [&[u8]; 12] = [b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct", b"Nov", b"Dec"];

struct Cursor<'a> {
    rest: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn new(text: &'a str) -> Self {
        Self { rest: text.as_bytes() }
    }

    fn literal(&mut self, expected: &[u8]) -> Option<()> {
        self.rest = self.rest.strip_prefix(expected)?;
        Some(())
    }

    fn one_of(&mut self, options: &[&[u8]]) -> Option<usize> {
        let (index, rest) = options.iter().enumerate().find_map(|(index, option)| Some((index, self.rest.strip_prefix(*option)?)))?;
        self.rest = rest;
        Some(index)
    }

    fn digits(&mut self, count: usize) -> Option<u16> {
        let (head, rest) = self.rest.split_at_checked(count)?;
        let mut value: u16 = 0;
        for &b in head {
            if !b.is_ascii_digit() {
                return None;
            }
            value = value.checked_mul(10)?.checked_add(u16::from(b.saturating_sub(b'0')))?;
        }
        self.rest = rest;
        Some(value)
    }

    fn time(&mut self) -> Option<(u8, u8, u8)> {
        let hour = self.digits(2)?;
        self.literal(b":")?;
        let minute = self.digits(2)?;
        self.literal(b":")?;
        let second = self.digits(2)?;
        if hour > 23 || minute > 59 || second > 60 {
            return None;
        }
        Some((u8::try_from(hour).ok()?, u8::try_from(minute).ok()?, u8::try_from(second).ok()?))
    }

    fn month(&mut self) -> Option<u8> {
        let index = self.one_of(&MONTHS)?;
        u8::try_from(index).ok()?.checked_add(1)
    }

    fn finished(&self) -> bool {
        self.rest.is_empty()
    }
}

/// Whether `year` has a 29 February (Gregorian calendar, as RFC 5322 §3.3 dates use): a century year only
/// when divisible by 400, any other year when divisible by 4.
fn has_leap_day(year: u64) -> bool {
    if year.is_multiple_of(100) { year.is_multiple_of(400) } else { year.is_multiple_of(4) }
}

/// The number of days of `month` (1 to 12) in `year`, 0 for anything else.
///
/// Apart from February, months alternate 31 and 30 days from January, and the alternation starts again at
/// August (July and August both have 31). Adding `month / 8`, which is 1 from August on, shifts the parity
/// there: `30 + (month + month / 8) % 2`.
fn month_length(year: u64, month: u8) -> u8 {
    match month {
        2 => 28u8.saturating_add(u8::from(has_leap_day(year))),
        1..=12 => 30u8.saturating_add(month.saturating_add(month / 8) % 2),
        _ => 0,
    }
}

/// Leap years in `1..=year`.
fn leaps_through(year: u64) -> u64 {
    (year / 4).saturating_sub(year / 100).saturating_add(year / 400)
}

/// Days from 1970-01-01 to January 1st of `year` (`year` ≥ 1970).
fn days_before_year(year: u64) -> Option<u64> {
    let years = year.checked_sub(1970)?;
    let leaps = leaps_through(year.checked_sub(1)?).checked_sub(leaps_through(1969))?;
    years.checked_mul(365)?.checked_add(leaps)
}

/// The calendar year holding the given Unix time, capped at year 9999.
fn year_of(now: UnixTime) -> u64 {
    let days = now.as_secs() / SECONDS_PER_DAY;
    let mut year = 1970u64.saturating_add(days / 366).min(LAST_YEAR);
    while year < LAST_YEAR {
        match days_before_year(year.saturating_add(1)) {
            Some(start) if start <= days => year = year.saturating_add(1),
            _ => break,
        }
    }
    year
}

fn valid_day(year: u64, month: u8, day: u8, two_digit_year: bool) -> bool {
    // For a two-digit year the century, and so the leap year, is not known yet: allow 29 February until then.
    let last = if two_digit_year && month == 2 { 29 } else { month_length(year, month) };
    day >= 1 && day <= last
}

impl HttpDate {
    /// Parses an IMF-fixdate, an RFC 850 date or an asctime date (RFC 9110 §5.6.7). The forms are case
    /// sensitive, as the grammar says; the day name is checked for spelling but not against the date.
    pub fn parse(text: &str) -> Option<Self> {
        Self::imf_fixdate(text).or_else(|| Self::rfc850(text)).or_else(|| Self::asctime(text))
    }

    fn build(year: u16, two_digit_year: bool, month: u8, day: u16, (hour, minute, second): (u8, u8, u8)) -> Option<Self> {
        let day = u8::try_from(day).ok()?;
        if !valid_day(u64::from(year), month, day, two_digit_year) {
            return None;
        }
        Some(Self { year, two_digit_year, month, day, hour, minute, second })
    }

    /// `Sat, 29 Oct 1994 19:43:31 GMT`
    fn imf_fixdate(text: &str) -> Option<Self> {
        let mut c = Cursor::new(text);
        c.one_of(&DAY_NAMES)?;
        c.literal(b", ")?;
        let day = c.digits(2)?;
        c.literal(b" ")?;
        let month = c.month()?;
        c.literal(b" ")?;
        let year = c.digits(4)?;
        c.literal(b" ")?;
        let time = c.time()?;
        c.literal(b" GMT")?;
        if !c.finished() {
            return None;
        }
        Self::build(year, false, month, day, time)
    }

    /// `Saturday, 29-Oct-94 19:43:31 GMT`
    fn rfc850(text: &str) -> Option<Self> {
        let mut c = Cursor::new(text);
        c.one_of(&LONG_DAY_NAMES)?;
        c.literal(b", ")?;
        let day = c.digits(2)?;
        c.literal(b"-")?;
        let month = c.month()?;
        c.literal(b"-")?;
        let year = c.digits(2)?;
        c.literal(b" ")?;
        let time = c.time()?;
        c.literal(b" GMT")?;
        if !c.finished() {
            return None;
        }
        Self::build(year, true, month, day, time)
    }

    /// `Wed Jul  1 00:00:00 2009`
    fn asctime(text: &str) -> Option<Self> {
        let mut c = Cursor::new(text);
        c.one_of(&DAY_NAMES)?;
        c.literal(b" ")?;
        let month = c.month()?;
        c.literal(b" ")?;
        let day = if c.literal(b" ").is_some() { c.digits(1)? } else { c.digits(2)? };
        c.literal(b" ")?;
        let time = c.time()?;
        c.literal(b" ")?;
        let year = c.digits(4)?;
        if !c.finished() {
            return None;
        }
        Self::build(year, false, month, day, time)
    }

    /// The full year, resolving a two-digit RFC 850 year against `now` (RFC 9110 §5.6.7: a year that appears
    /// to be more than 50 years in the future is the most recent past year with the same last two digits).
    pub fn year(&self, now: UnixTime) -> u64 {
        if !self.two_digit_year {
            return u64::from(self.year);
        }
        let current = year_of(now);
        let candidate = (current / 100).saturating_mul(100).saturating_add(u64::from(self.year));
        if candidate > current.saturating_add(50) { candidate.saturating_sub(100) } else { candidate }
    }

    /// The date as a Unix time, `now` serving only to place a two-digit year. A date before 1970 is the
    /// epoch; a 29 February that turns out not to exist gives `None`.
    pub fn to_unix(&self, now: UnixTime) -> Option<UnixTime> {
        let year = self.year(now);
        if !valid_day(year, self.month, self.day, false) {
            return None;
        }
        if year < 1970 {
            return Some(UnixTime(0));
        }
        let mut days = days_before_year(year)?;
        for month in 1..self.month {
            days = days.checked_add(u64::from(month_length(year, month)))?;
        }
        days = days.checked_add(u64::from(self.day).checked_sub(1)?)?;
        let seconds = days
            .checked_mul(SECONDS_PER_DAY)?
            .checked_add(u64::from(self.hour).checked_mul(3600)?)?
            .checked_add(u64::from(self.minute).checked_mul(60)?)?
            .checked_add(u64::from(self.second))?;
        Some(UnixTime(seconds))
    }
}

impl RetryAfter {
    /// Parses a `Retry-After` field value: `delay-seconds` or an HTTP-date (RFC 9110 §10.2.3). Surrounding
    /// spaces and tabs are ignored; anything else unexpected gives `None`, as if the field were absent.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim_matches([' ', '\t']);
        if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
            let mut seconds: u64 = 0;
            for b in value.bytes() {
                seconds = seconds.saturating_mul(10).saturating_add(u64::from(b.saturating_sub(b'0')));
            }
            return Some(Self::Delay(Duration::from_secs(seconds).min(MAX_RETRY_DELAY)));
        }
        HttpDate::parse(value).map(Self::At)
    }

    /// How long to wait from `now`: never negative, never more than [`MAX_RETRY_DELAY`].
    pub fn delay_from(&self, now: UnixTime) -> Duration {
        match self {
            Self::Delay(delay) => (*delay).min(MAX_RETRY_DELAY),
            Self::At(date) => match date.to_unix(now) {
                Some(at) => Duration::from_secs(at.as_secs().saturating_sub(now.as_secs())).min(MAX_RETRY_DELAY),
                None => Duration::ZERO,
            },
        }
    }
}

#[cfg(test)]
mod tests;
