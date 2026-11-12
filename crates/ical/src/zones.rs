//! Time zones in iCalendar text (RFC 5545 §3.6.5): the `VTIMEZONE` definitions a text brings with
//! it, read ([`TimeZones`], [`VTimezone`]) and written ([`write_vtimezone`]).
//!
//! This crate has no time-zone database. A definition read from a text answers for itself (the
//! offset at an instant, the instant of a wall-clock time); what a `TZID` means beyond the text
//! (an IANA name, a Windows name, which database zone keeps the same clock) is the caller's. To
//! write a definition, the caller hands over its zone's rules through [`ZoneRules`].
//!
//! Definitions are hostile text: offsets of more than eighteen hours are not offsets, and the
//! zones, their parts and their dates are all capped. [`TimeZones::spend`] keeps a budget of the
//! work a caller does with them.

use std::cell::Cell;

use chrono::{Datelike, NaiveDate, NaiveDateTime, Weekday};

use crate::lines::{ContentLine, Lines, fold, prefix};
use crate::rrule::{weekday_code, weekday_of_code};
use crate::values::{When, at_midnight, civil_seconds, format_local, from_civil_seconds, from_timestamp, to_timestamp};

/// Most zones one text defines, most parts (`STANDARD`, `DAYLIGHT`) of one zone, and most dates
/// (`RDATE`) of one part.
pub const MAX_ZONES: usize = 64;
pub const MAX_PARTS: usize = 64;
pub const MAX_DATES: usize = 64;
/// The work one text's times may cost, in units of one look at one part or one date of a
/// definition ([`VTimezone::cost`]). Past it, [`TimeZones::spend`] says no.
pub const WORK_BUDGET: u64 = 20_000_000;
/// The furthest from UTC an offset may be, in seconds.
const MAX_OFFSET: i32 = 18 * 3600;
/// Longest `TZID` (and `X-LIC-LOCATION`) kept, in bytes.
pub const MAX_TZID: usize = 128;

/// On which day of each year a part of a zone begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Yearly {
    /// The `nth` (1–5, or negative from the end) `weekday` of `month`.
    Nth { month: u32, nth: i8, weekday: Weekday },
    /// Day `day` of `month`.
    Day { month: u32, day: u32 },
}

/// The `nth` (1–5, or negative from the end) `weekday` of a month, when the month has one.
fn nth_weekday(year: i32, month: u32, weekday: Weekday, nth: i8) -> Option<NaiveDate> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    if nth > 0 {
        let offset = (i64::from(weekday.num_days_from_monday()) - i64::from(first.weekday().num_days_from_monday())).rem_euclid(7);
        NaiveDate::from_ymd_opt(year, month, u32::try_from(1 + offset + 7 * (i64::from(nth) - 1)).ok()?)
    } else {
        let last = first.checked_add_months(chrono::Months::new(1))?.pred_opt()?;
        let back = (i64::from(last.weekday().num_days_from_monday()) - i64::from(weekday.num_days_from_monday())).rem_euclid(7);
        let day = last.checked_sub_days(chrono::Days::new(u64::try_from(back + 7 * (i64::from(nth.saturating_neg()) - 1)).ok()?))?;
        (day.month() == month).then_some(day)
    }
}

impl Yearly {
    fn date(self, year: i32) -> Option<NaiveDate> {
        match self {
            Yearly::Day { month, day } => NaiveDate::from_ymd_opt(year, month, day),
            Yearly::Nth { month, nth, weekday } => nth_weekday(year, month, weekday, nth),
        }
    }
}

/// One `STANDARD` or `DAYLIGHT` part of a definition: from `start` on (a wall-clock time by the
/// offset `from`), and then again every year or on the listed dates, the offset is `to`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Part {
    start: NaiveDateTime,
    /// Seconds east of UTC before and after.
    from: i32,
    to: i32,
    yearly: Option<Yearly>,
    /// The rule holds up to this instant, and for this many beginnings.
    until: Option<i64>,
    count: Option<u32>,
    dates: Vec<NaiveDateTime>,
}

impl Part {
    /// The wall-clock second from which `to` holds for a beginning written as `start`: a time
    /// the clocks skip is still by the offset before, and so is the first of an hour that comes
    /// twice (RFC 5545 §3.3.5).
    fn wall_from(&self, start: NaiveDateTime) -> i64 {
        civil_seconds(start).saturating_add(i64::from(self.to.saturating_sub(self.from).max(0)))
    }

    fn instant_of(&self, start: NaiveDateTime) -> i64 {
        civil_seconds(start).saturating_sub(i64::from(self.from))
    }

    /// The beginning of the rule's year `year`, when the rule has one then.
    fn in_year(&self, year: i32) -> Option<NaiveDateTime> {
        let rule = self.yearly?;
        let first = self.start.year();
        if year < first || self.count.is_some_and(|count| i64::from(year) - i64::from(first) >= i64::from(count)) {
            return None;
        }
        let begins = rule.date(year)?.and_time(self.start.time());
        (begins >= self.start && self.until.is_none_or(|until| self.instant_of(begins) <= until)).then_some(begins)
    }

    /// The last beginning at or before `limit`, as `key` measures beginnings.
    fn latest(&self, limit: i64, year: i32, key: &dyn Fn(&Part, NaiveDateTime) -> i64) -> Option<i64> {
        let mut best: Option<i64> = None;
        let mut take = |begins: NaiveDateTime| {
            let at = key(self, begins);
            if at <= limit && best.is_none_or(|have| at > have) {
                best = Some(at);
            }
        };
        take(self.start);
        self.dates.iter().copied().for_each(&mut take);
        // The year of the time asked about and the years beside it; and where the rule ends.
        let last = self.until.map(|until| from_timestamp(until, 0).year());
        let counted = self.count.map(|count| self.start.year().saturating_add(i32::try_from(count).unwrap_or(i32::MAX)).saturating_sub(1));
        for year in [Some(year.saturating_add(1)), Some(year), Some(year.saturating_sub(1)), last, last.map(|y| y.saturating_sub(1)), counted]
            .into_iter()
            .flatten()
        {
            if let Some(begins) = self.in_year(year) {
                take(begins);
            }
        }
        best
    }
}

/// A zone as a text defines it (`VTIMEZONE`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VTimezone {
    tzid: String,
    /// `X-LIC-LOCATION`: the place the writer says the zone is (an IANA name, as a rule).
    place: String,
    parts: Vec<Part>,
}

impl VTimezone {
    /// The `TZID`, at most [`MAX_TZID`] bytes.
    pub fn tzid(&self) -> &str {
        &self.tzid
    }

    /// `X-LIC-LOCATION`, empty when the text gives none.
    pub fn place(&self) -> &str {
        &self.place
    }

    /// What one look at the definition costs, in units of [`WORK_BUDGET`].
    pub fn cost(&self) -> u64 {
        self.parts.iter().fold(0u64, |sum, part| sum.saturating_add(8).saturating_add(part.dates.len() as u64))
    }

    fn offset_by(&self, limit: i64, year: i32, key: &dyn Fn(&Part, NaiveDateTime) -> i64) -> i32 {
        let mut best: Option<(i64, i32)> = None;
        for part in &self.parts {
            if let Some(at) = part.latest(limit, year, key)
                && best.is_none_or(|(have, _)| at >= have)
            {
                best = Some((at, part.to));
            }
        }
        match best {
            Some((_, to)) => to,
            // Before the first beginning: the offset the first one changes from.
            None => self.parts.iter().min_by_key(|part| part.start).map_or(0, |part| part.from),
        }
    }

    /// Seconds east of UTC for a wall-clock time written in the zone. A time the clocks skip is
    /// read by the offset before the change; a time that comes twice is the first of the two.
    pub fn offset_for_wall(&self, wall: NaiveDateTime) -> i32 {
        self.offset_by(civil_seconds(wall), wall.year(), &|part, begins| part.wall_from(begins))
    }

    /// Seconds east of UTC at an instant (seconds since 1970-01-01T00:00:00Z).
    pub fn offset_at(&self, instant: i64) -> i32 {
        self.offset_by(instant, from_timestamp(instant, 0).year(), &|part, begins| part.instant_of(begins))
    }

    /// The instant of a wall-clock time written in the zone.
    pub fn instant(&self, wall: NaiveDateTime) -> i64 {
        civil_seconds(wall).saturating_sub(i64::from(self.offset_for_wall(wall)))
    }
}

/// `+0100`, `-0500`, `+053000`: seconds east of UTC; `None` for anything else and for more than
/// an offset can be.
pub fn parse_utc_offset(text: &str) -> Option<i32> {
    let text = text.trim();
    let (sign, digits) = match (text.strip_prefix('+'), text.strip_prefix('-')) {
        (Some(rest), _) => (1, rest),
        (None, Some(rest)) => (-1, rest),
        (None, None) => return None,
    };
    let digits: String = digits.chars().filter(|c| *c != ':').collect();
    if !matches!(digits.len(), 4 | 6) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let part = |at: usize| digits.get(at..at.saturating_add(2)).and_then(|d| d.parse::<i32>().ok());
    let (hours, minutes, seconds) = (part(0)?, part(2)?, if digits.len() == 6 { part(4)? } else { 0 });
    let total = hours * 3600 + minutes * 60 + seconds;
    (minutes < 60 && seconds < 60 && total <= MAX_OFFSET).then_some(sign * total)
}

/// An offset in seconds as `+0100`, `-0930`, `+045602`.
pub fn format_utc_offset(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let a = seconds.unsigned_abs();
    if a.is_multiple_of(60) {
        format!("{sign}{:02}{:02}", a / 3600, a / 60 % 60)
    } else {
        format!("{sign}{:02}{:02}{:02}", a / 3600, a / 60 % 60, a % 60)
    }
}

/// The rule of a part: `FREQ=YEARLY;BYMONTH=3;BYDAY=2SU`, with the day of the month or the
/// part's own day when no weekday is named. `None` for a rule that isn't yearly.
fn yearly(rule: &str, start: NaiveDateTime) -> Option<(Yearly, Option<i64>, Option<u32>)> {
    let (mut yearly, mut month, mut by_day, mut month_day) = (false, None, None, None);
    let (mut until, mut count) = (None, None);
    for part in prefix(rule, 512).split(';').take(16) {
        let Some((key, value)) = part.split_once('=') else { continue };
        let value = value.trim();
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => yearly = value.eq_ignore_ascii_case("YEARLY"),
            "INTERVAL" if value != "1" => return None,
            "BYMONTH" => month = value.parse::<u32>().ok().filter(|m| (1..=12).contains(m)),
            "BYMONTHDAY" => month_day = value.parse::<u32>().ok().filter(|d| (1..=31).contains(d)),
            "BYDAY" => {
                let split = value.len().checked_sub(2)?;
                let weekday = weekday_of_code(value.get(split..)?)?;
                let nth = match value.get(..split)? {
                    "" => return None,
                    nth => nth.parse::<i8>().ok().filter(|n| (1..=5).contains(n) || (-5..=-1).contains(n))?,
                };
                by_day = Some((nth, weekday));
            }
            "UNTIL" => until = When::parse(value).map(|at| to_timestamp(at.wall(), 0)),
            "COUNT" => count = value.parse::<u32>().ok(),
            _ => {}
        }
    }
    if !yearly {
        return None;
    }
    let month = month.unwrap_or(start.month());
    let rule = match (by_day, month_day) {
        (Some((nth, weekday)), _) => Yearly::Nth { month, nth, weekday },
        (None, day) => Yearly::Day { month, day: day.unwrap_or(start.day()) },
    };
    Some((rule, until, count))
}

/// A `STANDARD` or `DAYLIGHT` part being read.
#[derive(Default)]
struct PartBuilder {
    start: Option<NaiveDateTime>,
    from: Option<i32>,
    to: Option<i32>,
    rule: Option<String>,
    dates: Vec<NaiveDateTime>,
}

impl PartBuilder {
    fn finish(self) -> Option<Part> {
        let (start, from, to) = (self.start?, self.from?, self.to?);
        let (yearly, until, count) = match self.rule.as_deref().and_then(|rule| yearly(rule, start)) {
            Some((rule, until, count)) => (Some(rule), until, count),
            None => (None, None, None),
        };
        Some(Part { start, from, to, yearly, until, count, dates: self.dates })
    }
}

/// The zones a text defines, and the budget of work spent on them.
#[derive(Debug, Default)]
pub struct TimeZones {
    zones: Vec<VTimezone>,
    /// Work spent so far (see [`WORK_BUDGET`]), spends granted so far, and whether the budget ran out.
    spent: Cell<u64>,
    granted: Cell<u32>,
    stopped: Cell<bool>,
}

impl TimeZones {
    /// Read the `VTIMEZONE`s of a text: at most [`MAX_ZONES`], the first of each `TZID`. A zone
    /// without a `TZID` or without a part that has a start and both offsets is not one.
    pub fn collect(text: &str) -> TimeZones {
        let mut out = TimeZones::default();
        // Nothing to look for in most texts.
        let named = text.as_bytes().windows(9).any(|bytes| bytes.eq_ignore_ascii_case(b"VTIMEZONE"));
        if !named {
            return out;
        }
        let mut zone: Option<VTimezone> = None;
        let mut part: Option<PartBuilder> = None;
        for raw in Lines::new(text) {
            let Some(line) = ContentLine::parse(&raw) else { continue };
            let value = prefix(line.value.trim(), MAX_TZID);
            match (line.name.as_str(), value.to_ascii_uppercase().as_str()) {
                ("BEGIN", "VTIMEZONE") => {
                    if out.zones.len() >= MAX_ZONES {
                        break;
                    }
                    zone = Some(VTimezone::default());
                    part = None;
                }
                ("END", "VTIMEZONE") => {
                    part = None;
                    if let Some(definition) = zone.take()
                        && !definition.tzid.is_empty()
                        && !definition.parts.is_empty()
                        && !out.zones.iter().any(|have| have.tzid == definition.tzid)
                    {
                        out.zones.push(definition);
                    }
                }
                ("BEGIN", "STANDARD" | "DAYLIGHT") if zone.is_some() => part = Some(PartBuilder::default()),
                ("END", "STANDARD" | "DAYLIGHT") => {
                    if let (Some(definition), Some(built)) = (&mut zone, part.take().and_then(PartBuilder::finish))
                        && definition.parts.len() < MAX_PARTS
                    {
                        definition.parts.push(built);
                    }
                }
                // Anything else that begins inside a zone is not a part of it.
                ("BEGIN", _) => part = None,
                (name, _) => match (&mut zone, &mut part) {
                    (Some(_), Some(part)) => match name {
                        "DTSTART" => part.start = part.start.or_else(|| When::parse(&line.value).map(When::wall)),
                        "TZOFFSETFROM" => part.from = part.from.or_else(|| parse_utc_offset(&line.value)),
                        "TZOFFSETTO" => part.to = part.to.or_else(|| parse_utc_offset(&line.value)),
                        "RRULE" if part.rule.is_none() => part.rule = Some(prefix(&line.value, 512).to_string()),
                        "RDATE" => {
                            let room = MAX_DATES.saturating_sub(part.dates.len());
                            part.dates.extend(line.value.split(',').take(room).filter_map(When::parse).map(When::wall));
                        }
                        _ => {}
                    },
                    (Some(definition), None) => match name {
                        "TZID" if definition.tzid.is_empty() => definition.tzid = value.to_string(),
                        "X-LIC-LOCATION" if definition.place.is_empty() => definition.place = value.to_string(),
                        _ => {}
                    },
                    _ => {}
                },
            }
        }
        out
    }

    /// The definition of `tzid`: the one of that name, else one whose name differs only in letter
    /// case. The name is looked at to [`MAX_TZID`] bytes.
    pub fn get(&self, tzid: &str) -> Option<&VTimezone> {
        let tzid = prefix(tzid.trim(), MAX_TZID);
        self.zones.iter().find(|zone| zone.tzid == tzid).or_else(|| self.zones.iter().find(|zone| zone.tzid.eq_ignore_ascii_case(tzid)))
    }

    /// Every definition, in the order of the text.
    pub fn iter(&self) -> impl Iterator<Item = &VTimezone> {
        self.zones.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }

    /// Spend `units` of the text's work budget; `false` (and the budget is out for good) when
    /// there is not enough left.
    pub fn spend(&self, units: u64) -> bool {
        let total = self.spent.get().saturating_add(units);
        if self.stopped.get() || total > WORK_BUDGET {
            self.stopped.set(true);
            return false;
        }
        self.spent.set(total);
        self.granted.set(self.granted.get().saturating_add(1));
        true
    }

    /// When the budget ran out: how many spends were granted before it did.
    pub fn stopped_after(&self) -> Option<u32> {
        self.stopped.get().then(|| self.granted.get())
    }
}

// ---- writing ----

/// A change of a zone's offset: at the instant `at` (seconds since 1970-01-01T00:00:00Z), from
/// `before` to `after` seconds east of UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transition {
    pub at: i64,
    pub before: i32,
    pub after: i32,
}

/// What [`write_vtimezone`] needs to know of a zone: the caller's time-zone database.
pub trait ZoneRules {
    /// Seconds east of UTC at an instant.
    fn offset_at(&self, instant: i64) -> i32;
    /// The changes of offset from `from` up to `to` (instants), in order.
    fn transitions(&self, from: i64, to: i64) -> Vec<Transition>;
}

/// The rule a change of clocks follows when it falls on the same weekday of the same month each
/// year at the same time of day: the `nth` weekday (the last, when it is in the month's last week
/// in each of the years).
fn yearly_rule(changes: &[(NaiveDateTime, i32, i32)]) -> Option<Yearly> {
    let (first, first_from, first_to) = changes.first().copied()?;
    let alike = changes.iter().all(|(at, from, to)| {
        at.month() == first.month() && at.weekday() == first.weekday() && at.time() == first.time() && (*from, *to) == (first_from, first_to)
    });
    if !alike {
        return None;
    }
    let nth = |at: &NaiveDateTime| (at.day() - 1) / 7 + 1;
    let last = |at: &NaiveDateTime| at.date().checked_add_days(chrono::Days::new(7)).is_none_or(|week_on| week_on.month() != at.month());
    let (month, weekday) = (first.month(), first.weekday());
    if changes.iter().all(|(at, _, _)| nth(at) == nth(&first)) && nth(&first) < 5 && !changes.iter().all(|(at, _, _)| last(at)) {
        Some(Yearly::Nth { month, nth: i8::try_from(nth(&first)).unwrap_or(1), weekday })
    } else if changes.iter().all(|(at, _, _)| last(at)) {
        Some(Yearly::Nth { month, nth: -1, weekday })
    } else {
        None
    }
}

fn write_part(out: &mut String, kind: &str, start: NaiveDateTime, from: i32, to: i32, rule: Option<Yearly>) {
    fold(out, &format!("BEGIN:{kind}"));
    fold(out, &format!("DTSTART:{}", format_local(start)));
    fold(out, &format!("TZOFFSETFROM:{}", format_utc_offset(from)));
    fold(out, &format!("TZOFFSETTO:{}", format_utc_offset(to)));
    if let Some(Yearly::Nth { month, nth, weekday }) = rule {
        fold(out, &format!("RRULE:FREQ=YEARLY;BYMONTH={month};BYDAY={nth}{}", weekday_code(weekday)));
    }
    fold(out, &format!("END:{kind}"));
}

/// Write the definition of a zone named `tzid` (control characters dropped, at most [`MAX_TZID`]
/// bytes) for times in the years `from`–`to` (at most 120
/// years are written; the years are kept in 2 to 9990).
///
/// A zone that changes its clocks twice a year by one rule is written as most programs write it,
/// and all of them read it: one `STANDARD` and one `DAYLIGHT` part, each with its yearly rule. A
/// zone that doesn't change is one `STANDARD` part. Any other (a year in which the rule changed,
/// a zone that follows the moon) is written out change by change for those years.
pub fn write_vtimezone(out: &mut String, tzid: &str, zone: &dyn ZoneRules, from: i32, to: i32) {
    let (from, to) = (from.clamp(2, 9_990), to.clamp(2, 9_990).max(from.clamp(2, 9_990)));
    // A few hundred changes at most are looked at.
    let to = to.min(from.saturating_add(120));
    let instant = |year: i32| NaiveDate::from_ymd_opt(year, 1, 1).map_or(0, |day| to_timestamp(at_midnight(day), 0));
    let before = zone.offset_at(instant(from - 1));
    // Each change as it is written: the wall-clock time by the offset before it.
    let changes: Vec<(NaiveDateTime, i32, i32)> = zone
        .transitions(instant(from - 1), instant(to + 1))
        .into_iter()
        .map(|change| (from_civil_seconds(change.at.saturating_add(i64::from(change.before))), change.before, change.after))
        .collect();
    let midnight = |year: i32| NaiveDate::from_ymd_opt(year, 1, 1).map(at_midnight).unwrap_or_default();
    fold(out, "BEGIN:VTIMEZONE");
    // A name can't start another line.
    let tzid: String = prefix(tzid, MAX_TZID).chars().filter(|c| !c.is_control()).collect();
    fold(out, &format!("TZID:{tzid}"));
    if changes.is_empty() {
        write_part(out, "STANDARD", midnight(1601), before, before, None);
        fold(out, "END:VTIMEZONE");
        return;
    }
    // Clocks forward is the beginning of daylight time, clocks back the end of it.
    let (forward, back): (Vec<_>, Vec<_>) = changes.iter().copied().partition(|(_, from, to)| to > from);
    let years = usize::try_from(i64::from(to) - i64::from(from) + 2).unwrap_or(0);
    let regular = forward.len() == years && back.len() == years;
    match (yearly_rule(&forward).filter(|_| regular), yearly_rule(&back).filter(|_| regular)) {
        (Some(spring), Some(autumn)) => {
            for (kind, list, rule) in [("DAYLIGHT", &forward, spring), ("STANDARD", &back, autumn)] {
                if let Some((start, from, to)) = list.first().copied() {
                    write_part(out, kind, start, from, to, Some(rule));
                }
            }
        }
        _ => {
            // What holds before the first change, then each change.
            let first = changes.first().map_or(NaiveDateTime::default(), |(at, _, _)| *at);
            write_part(out, "STANDARD", midnight(first.year().saturating_sub(1).max(1)), before, before, None);
            for (start, from, to) in changes {
                write_part(out, if to > from { "DAYLIGHT" } else { "STANDARD" }, start, from, to, None);
            }
        }
    }
    fold(out, "END:VTIMEZONE");
}

#[cfg(test)]
#[path = "zones_tests.rs"]
mod tests;
