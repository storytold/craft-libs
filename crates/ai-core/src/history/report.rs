//! Looking at the history: filtering, totals per day and month, and CSV export. Pure functions on
//! [`HistoryEntry`] rows, no I/O and no clock (the caller passes the time), so the egui widget and
//! any other front end share one set of numbers.
//!
//! Money is exact. Costs are summed in millionths per currency and never mixed across currencies,
//! and a job whose provider reported no cost is counted separately (`without_cost`), never priced
//! at zero or guessed.

use std::collections::BTreeMap;
use std::fmt;

use super::{HistoryEntry, Outcome};

const DAY_MS: i64 = 86_400_000;

/// A calendar day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Day {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl Day {
    /// Days since 1970-01-01 (negative before it).
    pub fn to_days(self) -> i64 {
        let (y, m, d) = (i64::from(self.year), i64::from(self.month), i64::from(self.day));
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    pub fn from_days(days: i64) -> Day {
        civil_from_days(days)
    }

    pub fn month(self) -> Month {
        Month { year: self.year, month: self.month }
    }
}

impl fmt::Display for Day {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// A calendar month.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Month {
    pub year: i32,
    pub month: u8,
}

impl fmt::Display for Month {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}", self.year, self.month)
    }
}

/// Days since 1970-01-01 to a calendar day (proleptic Gregorian).
fn civil_from_days(days: i64) -> Day {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Day { year: i32::try_from(y).unwrap_or(0), month: u8::try_from(m).unwrap_or(1), day: u8::try_from(d).unwrap_or(1) }
}

/// The calendar day of a Unix-millisecond time, at a fixed offset from UTC in minutes (east is
/// positive). Apps that know the local offset pass it; 0 groups by UTC day.
pub fn day_of(at_ms: u64, offset_minutes: i32) -> Day {
    let ms = i64::try_from(at_ms).unwrap_or(i64::MAX) + i64::from(offset_minutes) * 60_000;
    civil_from_days(ms.div_euclid(DAY_MS))
}

/// `2026-10-12T14:03:07Z`, always UTC so an exported file means the same thing anywhere.
pub fn iso_utc(at_ms: u64) -> String {
    let ms = i64::try_from(at_ms).unwrap_or(i64::MAX);
    let day = civil_from_days(ms.div_euclid(DAY_MS));
    let secs = ms.rem_euclid(DAY_MS) / 1000;
    format!("{day}T{:02}:{:02}:{:02}Z", secs / 3600, secs / 60 % 60, secs % 60)
}

/// `2025-10-09 08:53` at the given offset, for a list.
pub fn time_text(at_ms: u64, offset_minutes: i32) -> String {
    let ms = i64::try_from(at_ms).unwrap_or(i64::MAX) + i64::from(offset_minutes) * 60_000;
    let mins = ms.rem_euclid(DAY_MS) / 60_000;
    format!("{} {:02}:{:02}", day_of(at_ms, offset_minutes), mins / 60, mins % 60)
}

/// The Unix-millisecond time at which `now_ms`'s calendar day began, at the given offset.
pub fn start_of_day_ms(now_ms: u64, offset_minutes: i32) -> u64 {
    let offset = i64::from(offset_minutes) * 60_000;
    let now = i64::try_from(now_ms).unwrap_or(i64::MAX);
    let local = now.saturating_add(offset);
    let start = local - local.rem_euclid(DAY_MS) - offset;
    u64::try_from(start).unwrap_or(0)
}

/// `31_000` micros is `0.03`; under a cent it shows four places so small costs are not `0.00`.
pub fn amount_text(micros: u64) -> String {
    let whole = micros / 1_000_000;
    let frac = micros % 1_000_000;
    if micros < 10_000 { format!("{whole}.{:04}", frac / 100) } else { format!("{whole}.{:02}", frac / 10_000) }
}

/// Every digit, for a file: `31_000` micros is `0.031000`.
fn exact_amount(micros: u64) -> String {
    format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000)
}

/// Which entries to show. An empty filter shows everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryFilter {
    pub app: Option<String>,
    pub model: Option<String>,
    /// Only jobs that started at or after this Unix-millisecond time.
    pub since_ms: Option<u64>,
}

impl HistoryFilter {
    pub fn matches(&self, entry: &HistoryEntry) -> bool {
        self.app.as_ref().is_none_or(|a| *a == entry.start.app)
            && self.model.as_ref().is_none_or(|m| *m == entry.start.model)
            && self.since_ms.is_none_or(|t| entry.start.at_ms >= t)
    }

    pub fn apply<'a>(&self, entries: &'a [HistoryEntry]) -> Vec<&'a HistoryEntry> {
        entries.iter().filter(|e| self.matches(e)).collect()
    }
}

/// The distinct apps in the log, sorted, for a filter list.
pub fn apps(entries: &[HistoryEntry]) -> Vec<String> {
    distinct(entries.iter().map(|e| e.start.app.as_str()))
}

/// The distinct models in the log, sorted.
pub fn models(entries: &[HistoryEntry]) -> Vec<String> {
    distinct(entries.iter().map(|e| e.start.model.as_str()))
}

fn distinct<'a>(items: impl Iterator<Item = &'a str>) -> Vec<String> {
    let set: std::collections::BTreeSet<&str> = items.collect();
    set.into_iter().map(str::to_owned).collect()
}

/// A count of jobs and the money the providers reported for them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub jobs: usize,
    pub done: usize,
    pub failed: usize,
    pub cancelled: usize,
    /// Started, but the app closed before the end was seen.
    pub unfinished: usize,
    /// Reported cost per currency, in millionths.
    pub cost_micros: BTreeMap<String, u64>,
    /// Jobs that finished successfully with no cost reported (free, local, or unknown).
    pub without_cost: usize,
    /// Total time of the successful jobs that have a duration, in milliseconds.
    pub duration_ms: u64,
    /// How many successful jobs went into `duration_ms`.
    pub timed: usize,
}

impl Totals {
    pub fn add(&mut self, entry: &HistoryEntry) {
        self.jobs += 1;
        if matches!(entry.outcome(), Some(Outcome::Done))
            && let Some(ms) = entry.duration_ms()
        {
            self.duration_ms = self.duration_ms.saturating_add(ms);
            self.timed += 1;
        }
        match entry.outcome() {
            Some(Outcome::Done) => self.done += 1,
            Some(Outcome::Failed { .. }) => self.failed += 1,
            Some(Outcome::Cancelled) => self.cancelled += 1,
            None => self.unfinished += 1,
        }
        match entry.reported_cost() {
            Some((micros, currency)) => {
                let slot = self.cost_micros.entry(currency.to_owned()).or_insert(0);
                *slot = slot.saturating_add(micros);
            }
            None if matches!(entry.outcome(), Some(Outcome::Done)) => self.without_cost += 1,
            None => {}
        }
    }
}

impl Totals {
    /// Average time of the successful jobs that have one.
    pub fn average_ms(&self) -> Option<u64> {
        self.duration_ms.checked_div(u64::try_from(self.timed).ok()?)
    }
}

pub fn totals<'a>(entries: impl IntoIterator<Item = &'a HistoryEntry>) -> Totals {
    let mut t = Totals::default();
    for e in entries {
        t.add(e);
    }
    t
}

/// Totals per calendar day, newest day first.
pub fn by_day(entries: &[&HistoryEntry], offset_minutes: i32) -> Vec<(Day, Totals)> {
    let mut map: BTreeMap<Day, Totals> = BTreeMap::new();
    for e in entries {
        map.entry(day_of(e.start.at_ms, offset_minutes)).or_default().add(e);
    }
    map.into_iter().rev().collect()
}

/// Totals per calendar month, newest month first.
pub fn by_month(entries: &[&HistoryEntry], offset_minutes: i32) -> Vec<(Month, Totals)> {
    let mut map: BTreeMap<Month, Totals> = BTreeMap::new();
    for e in entries {
        map.entry(day_of(e.start.at_ms, offset_minutes).month()).or_default().add(e);
    }
    map.into_iter().rev().collect()
}

/// One entry per calendar day, oldest first, with days that had no jobs included as empty totals,
/// for the most recent `max_days` days from the first to the last day that has a job. For a chart.
pub fn daily(entries: &[&HistoryEntry], offset_minutes: i32, max_days: usize) -> Vec<(Day, Totals)> {
    let mut map: BTreeMap<i64, Totals> = BTreeMap::new();
    for e in entries {
        map.entry(day_of(e.start.at_ms, offset_minutes).to_days()).or_default().add(e);
    }
    let (Some((&first, _)), Some((&last, _))) = (map.first_key_value(), map.last_key_value()) else {
        return Vec::new();
    };
    let span = usize::try_from(last - first + 1).unwrap_or(usize::MAX);
    let start = if span > max_days { last - i64::try_from(max_days).unwrap_or(0) + 1 } else { first };
    (start..=last).map(|d| (Day::from_days(d), map.remove(&d).unwrap_or_default())).collect()
}

fn grouped_by<'a>(entries: &[&'a HistoryEntry], key: impl Fn(&'a HistoryEntry) -> &'a str) -> Vec<(String, Totals)> {
    let mut map: BTreeMap<&str, Totals> = BTreeMap::new();
    for e in entries {
        map.entry(key(e)).or_default().add(e);
    }
    let mut rows: Vec<(String, Totals)> = map.into_iter().map(|(k, t)| (k.to_owned(), t)).collect();
    // Busiest first; ties keep the alphabetical order the map gave.
    rows.sort_by_key(|(_, t)| std::cmp::Reverse(t.jobs));
    rows
}

/// Totals per model, busiest first.
pub fn by_model(entries: &[&HistoryEntry]) -> Vec<(String, Totals)> {
    grouped_by(entries, |e| e.start.model.as_str())
}

/// Totals per app, busiest first.
pub fn by_app(entries: &[&HistoryEntry]) -> Vec<(String, Totals)> {
    grouped_by(entries, |e| e.start.app.as_str())
}

const CSV_HEADER: [&str; 17] = [
    "started_utc",
    "app",
    "feature",
    "capability",
    "connection",
    "model",
    "where",
    "outcome",
    "error_kind",
    "seconds",
    "cost",
    "currency",
    "estimate",
    "estimate_currency",
    "provider_job",
    "sent",
    "prompt",
];

/// One CSV cell. Quoted when it holds a comma, quote or line break. A cell that starts with a
/// character a spreadsheet reads as a formula gets a leading apostrophe, so an exported prompt or
/// label cannot run anything when the file is opened.
fn cell(value: &str) -> String {
    let mut text = value.to_owned();
    if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        text.insert(0, '\'');
    }
    if text.contains([',', '"', '\n', '\r']) { format!("\"{}\"", text.replace('"', "\"\"")) } else { text }
}

fn row(entry: &HistoryEntry) -> Vec<String> {
    let s = &entry.start;
    let (outcome, kind) = match entry.outcome() {
        Some(Outcome::Done) => ("done", String::new()),
        Some(Outcome::Failed { kind }) => ("failed", serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()),
        Some(Outcome::Cancelled) => ("cancelled", String::new()),
        None => ("unfinished", String::new()),
    };
    let (cost, currency) = match entry.reported_cost() {
        Some((micros, c)) => (exact_amount(micros), c.to_owned()),
        None => (String::new(), String::new()),
    };
    let (estimate, estimate_currency) = match &s.estimate {
        Some(e) => (exact_amount(e.micros), e.currency.clone()),
        None => (String::new(), String::new()),
    };
    let sent: Vec<String> = s
        .sent
        .iter()
        .map(|i| match i.bytes {
            Some(b) => format!("{:?} {} {b} bytes", i.kind, i.role).to_lowercase(),
            None => format!("{:?} {}", i.kind, i.role).to_lowercase(),
        })
        .collect();
    vec![
        iso_utc(s.at_ms),
        s.app.clone(),
        s.role_label.clone(),
        s.capability.clone(),
        s.connection_label.clone(),
        s.model.clone(),
        match s.locality {
            crate::Locality::Local => "local".to_owned(),
            crate::Locality::Remote => "remote".to_owned(),
        },
        outcome.to_owned(),
        kind,
        entry.duration_ms().map(|ms| format!("{}.{:03}", ms / 1000, ms % 1000)).unwrap_or_default(),
        cost,
        currency,
        estimate,
        estimate_currency,
        s.provider_job.clone().unwrap_or_default(),
        sent.join("; "),
        s.prompt.clone().unwrap_or_default(),
    ]
}

/// The entries as CSV text, header first, CRLF line ends. UTC times and exact amounts.
pub fn to_csv(entries: &[&HistoryEntry]) -> String {
    let mut out = CSV_HEADER.join(",");
    out.push_str("\r\n");
    for entry in entries {
        let cells: Vec<String> = row(entry).iter().map(|c| cell(c)).collect();
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests;
