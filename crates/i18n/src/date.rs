//! Dates and times written for a formatting [`Region`] (N11.02), and fixed UTC stamps for logs
//! and file names.
//!
//! - [`DateFormat`]: dates, times, month and weekday names and the first day of the week in the
//!   region's time locale ([`Region::time`]) and clock ([`Region::hour_cycle`]), from CLDR data
//!   (ICU4X `icu_datetime`, data in `data/dates.postcard` from `data/generate.sh`). Build it once
//!   per region and keep it; [`I18n::dates`] holds one, as [`I18n::format`] does for numbers.
//! - Relative times ("3 minutes ago") are [`relative_age`](crate::relative_age) and
//!   [`I18n::relative_age`]: their words are catalog rows, so they follow the interface language.
//!   A list that shows the date after a week ([`AGE_SHOWN_AS_DATE`](crate::AGE_SHOWN_AS_DATE))
//!   writes it with [`DateFormat`], which follows the region.
//! - [`utc_stamp`]: `2026-10-08 14:05` (UTC), the same in every language: render-queue logs,
//!   export file names, report headers.
//!
//! Stored dates ([`CalendarDate`], [`DateTime`]) are proleptic Gregorian, and so is everything a
//! calendar grid needs ([`DateFormat::month_name`], [`DateFormat::year_month`],
//! [`DateFormat::first_weekday`], [`DateFormat::parse_date`]). [`DateFormat::format`] writes a date
//! in the region's default calendar, which CLDR's calendar preferences give as Gregorian for every
//! region except Thailand (Buddhist, the Gregorian year plus 543), and Iran and Afghanistan
//! (Persian). Saudi Arabia, the other Gulf states and Israel are Gregorian by default. A locale
//! with a `-u-ca-` keyword (`th-TH-u-ca-gregory`, `ar-SA-u-ca-islamic-umalqura`) writes that
//! calendar when its data is bundled (Buddhist and Persian) and Gregorian otherwise.
//!
//! Years outside ±9999 are kept, written with as many digits as they need (EffectCraft's and
//! PdfCraft's 400-year-cycle rule), never collapsed to the epoch.

mod local;

pub use local::{CalendarKind, LocalDate};

use std::fmt;
use std::ops::Range;
use std::sync::{Arc, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Local, NaiveDate, Offset, TimeZone as _};
use icu_calendar::week::WeekInformation;
use icu_calendar::{AnyCalendarKind, Date, Gregorian};
use icu_datetime::fieldsets::{T, YM, YMD, YMDE, YMDET, YMDT};
use icu_datetime::options::{Length, TimePrecision as IcuPrecision};
use icu_datetime::parts;
use icu_datetime::pattern::{DateTimePattern, FixedCalendarDateTimeNames};
use icu_datetime::{DateTimeFormatter, FixedCalendarDateTimeFormatter};
use icu_locale_core::Locale;
use icu_locale_core::extensions::unicode::{key, value};
use icu_provider::buf::AsDeserializingBufferProvider;
use icu_time::Time;
use writeable::{Part, PartsWrite, TryWriteable, Writeable};

use crate::{Digits, HourCycle, I18n, Region, data};

/// Days in a 400-year Gregorian cycle, after which the calendar (and the weekday) repeats.
const CYCLE_DAYS: i64 = 146_097;
const CYCLE_SECS: i64 = CYCLE_DAYS * 86_400;
/// `NaiveDate::num_days_from_ce` of 1970-01-01.
const EPOCH_FROM_CE: i32 = 719_163;

/// A day of the Gregorian calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CalendarDate {
    /// The year; negative for years before 1 CE (proleptic Gregorian).
    pub year: i64,
    /// 1 to 12.
    pub month: u8,
    /// 1 to the month's length.
    pub day: u8,
}

impl CalendarDate {
    /// `None` when `month` or `day` is out of range for that year (`2026-02-29`).
    pub fn new(year: i64, month: u8, day: u8) -> Option<CalendarDate> {
        // Whole 400-year cycles leave month lengths, leap days and weekdays as they were.
        NaiveDate::from_ymd_opt(year.rem_euclid(400) as i32, month.into(), day.into())?;
        Some(CalendarDate { year, month, day })
    }

    /// The date `days` after 1970-01-01 (negative before it).
    // pdfcraft: crates/ui-egui/src/workflows/combine_ui/parts.rs:279 civil_from_days
    pub fn from_days(days: i64) -> CalendarDate {
        let cycles = days.div_euclid(CYCLE_DAYS);
        let in_cycle = days.rem_euclid(CYCLE_DAYS) as i32;
        let date = NaiveDate::from_num_days_from_ce_opt(in_cycle + EPOCH_FROM_CE).unwrap_or_default();
        CalendarDate { year: i64::from(date.year()).saturating_add(cycles.saturating_mul(400)), month: date.month() as u8, day: date.day() as u8 }
    }

    /// Days since 1970-01-01; the inverse of [`from_days`](Self::from_days).
    pub fn days(self) -> i64 {
        let cycles = self.year.div_euclid(400);
        let year = self.year.rem_euclid(400) as i32;
        let date = NaiveDate::from_ymd_opt(year, self.month.into(), self.day.into()).unwrap_or_default();
        i64::from(date.num_days_from_ce() - EPOCH_FROM_CE).saturating_add(cycles.saturating_mul(CYCLE_DAYS))
    }

    /// The day of the week.
    pub fn weekday(self) -> Weekday {
        // Day 0, 1970-01-01, was a Thursday.
        Weekday::ALL[((self.days().rem_euclid(7) + 3) % 7) as usize]
    }

    /// 28 to 31; 0 for a month outside 1 to 12.
    pub fn days_in_month(year: i64, month: u8) -> u8 {
        (28..=31).rev().find(|&day| CalendarDate::new(year, month, day).is_some()).unwrap_or(0)
    }

    /// `n` days later (earlier when negative).
    pub fn add_days(self, n: i64) -> CalendarDate {
        CalendarDate::from_days(self.days() + n)
    }

    /// `n` months later, the day clamped to the new month's length (Jan 31 + 1 month is Feb 28
    /// or 29), as calendar pickers step months.
    pub fn add_months(self, n: i64) -> CalendarDate {
        let index = i128::from(self.year) * 12 + i128::from(self.month - 1) + i128::from(n);
        let year = index.div_euclid(12).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
        let month = index.rem_euclid(12) as u8 + 1;
        CalendarDate { year, month, day: self.day.min(CalendarDate::days_in_month(year, month)) }
    }
}

/// A day of the week.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weekday {
    #[allow(missing_docs)]
    Monday,
    #[allow(missing_docs)]
    Tuesday,
    #[allow(missing_docs)]
    Wednesday,
    #[allow(missing_docs)]
    Thursday,
    #[allow(missing_docs)]
    Friday,
    #[allow(missing_docs)]
    Saturday,
    #[allow(missing_docs)]
    Sunday,
}

impl Weekday {
    /// Monday first (ISO 8601).
    pub const ALL: [Weekday; 7] =
        [Weekday::Monday, Weekday::Tuesday, Weekday::Wednesday, Weekday::Thursday, Weekday::Friday, Weekday::Saturday, Weekday::Sunday];
}

/// Which clock a moment is read on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TimeZone {
    /// The system's offset at that moment (daylight saving included).
    #[default]
    Local,
    /// Coordinated Universal Time.
    Utc,
}

/// A date with a time of day, on one clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DateTime {
    /// The day.
    pub date: CalendarDate,
    /// 0 to 23.
    pub hour: u8,
    /// 0 to 59.
    pub minute: u8,
    /// 0 to 59.
    pub second: u8,
}

impl DateTime {
    /// The moment `unix` seconds after 1970-01-01T00:00Z, on `zone`'s clock. The local offset
    /// is the system's (chrono's `Local`; the browser's `Date` on wasm).
    // effectcraft: crates/ui-egui/src/panels/home.rs:74 utc_parts
    pub fn from_unix(unix: i64, zone: TimeZone) -> DateTime {
        let offset = match zone {
            TimeZone::Utc => 0,
            TimeZone::Local => local_offset(unix),
        };
        let on_clock = unix.saturating_add(offset);
        let seconds = on_clock.rem_euclid(86_400);
        DateTime {
            date: CalendarDate::from_days(on_clock.div_euclid(86_400)),
            hour: (seconds / 3600) as u8,
            minute: (seconds % 3600 / 60) as u8,
            second: (seconds % 60) as u8,
        }
    }

    /// [`from_unix`](Self::from_unix) of a `SystemTime`; a time before 1970 counts back.
    pub fn from_system(time: SystemTime, zone: TimeZone) -> DateTime {
        let unix = match time.duration_since(UNIX_EPOCH) {
            Ok(after) => i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
            Err(before) => {
                let before = before.duration();
                // Whole seconds round toward the earlier moment.
                let secs = i64::try_from(before.as_secs()).unwrap_or(i64::MAX);
                let part = i64::from(before.subsec_nanos() > 0);
                -secs.saturating_add(part)
            }
        };
        DateTime::from_unix(unix, zone)
    }
}

/// How much of a date [`DateFormat::format`] writes, in the region's order and words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DateLength {
    /// `10/31/26`, `31.10.26`, `2026/10/31`.
    Short,
    /// `Oct 31, 2026`, `31.10.2026`, `31 oct. 2026`.
    Medium,
    /// `October 31, 2026`, `31. Oktober 2026`.
    Long,
    /// `Saturday, October 31, 2026`.
    Full,
}

/// How much of a time [`DateFormat::format`] writes, on the region's clock (`2:05 PM`, `14:05`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimePrecision {
    /// Hours and minutes (`2:05 PM`).
    Minutes,
    /// Hours, minutes and seconds (`2:05:09 PM`).
    Seconds,
}

/// What [`DateFormat::format`] writes: a date, a time, or both joined as the locale joins them
/// (`Oct 31, 2026, 2:05 PM`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DateStyle {
    /// The date part and its length; `None` writes no date.
    pub date: Option<DateLength>,
    /// The time part and its precision; `None` writes no time.
    pub time: Option<TimePrecision>,
}

impl DateStyle {
    /// `Oct 31, 2026`: lists and file details.
    pub const DATE: DateStyle = DateStyle { date: Some(DateLength::Medium), time: None };
    /// `Oct 31, 2026, 2:05 PM`: "Modified" columns, history rows (LightCraft's short time).
    pub const DATE_TIME: DateStyle = DateStyle { date: Some(DateLength::Medium), time: Some(TimePrecision::Minutes) };
    /// `October 31, 2026 at 2:05:11 PM`: a photo's capture time (LightCraft's display time).
    pub const LONG_DATE_TIME: DateStyle = DateStyle { date: Some(DateLength::Long), time: Some(TimePrecision::Seconds) };
    /// `2:05 PM`.
    pub const TIME: DateStyle = DateStyle { date: None, time: Some(TimePrecision::Minutes) };
}

/// How long a month or weekday name is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NameLength {
    /// `S`, `O`: calendar column heads where room is short.
    Narrow,
    /// `Sat`, `Oct`.
    Abbreviated,
    /// `Saturday`, `October`.
    Wide,
}

/// How dates and times are written in a formatting region.
///
/// ```ignore
/// let dates = i18n.dates();
/// dates.format(DateTime::from_system(modified, TimeZone::Local), DateStyle::DATE_TIME); // "Oct 31, 2026, 2:05 PM"
/// dates.month_name(10, NameLength::Wide);                                               // "October", "Oktober"
/// dates.first_weekday();                                                                // Sunday in en-US, Monday in de-DE
/// ```
///
/// Dates are written in the region's default calendar (Buddhist in Thailand, Persian in Iran and
/// Afghanistan, Gregorian elsewhere), with the region's order, words, digits and clock. Month and
/// weekday names, the month title and the date parser stay Gregorian, as a date picker's grid is.
#[derive(Clone, Debug)]
pub struct DateFormat {
    /// The time locale, BCP 47.
    locale: String,
    /// The region's clock where the system sets one apart from the locale.
    hour_cycle: Option<crate::HourCycle>,
    /// Write `0`–`9` whatever the locale's digits are.
    latin_digits: bool,
    /// Formatters and names, built on first use and shared by clones.
    cache: Arc<Cache>,
}

/// One ICU formatter per field set the styles need.
#[derive(Debug)]
enum Formatter {
    Date(DateTimeFormatter<YMD>),
    DateWeekday(DateTimeFormatter<YMDE>),
    DateTime(DateTimeFormatter<YMDT>),
    DateTimeWeekday(DateTimeFormatter<YMDET>),
    Time(DateTimeFormatter<T>),
}

impl Formatter {
    /// Whether the formatter writes Gregorian years, which a date past ICU4X's range relies on.
    fn is_gregorian(&self) -> bool {
        let kind = match self {
            Formatter::Date(f) => f.calendar().kind(),
            Formatter::DateWeekday(f) => f.calendar().kind(),
            Formatter::DateTime(f) => f.calendar().kind(),
            Formatter::DateTimeWeekday(f) => f.calendar().kind(),
            Formatter::Time(f) => f.calendar().kind(),
        };
        kind == AnyCalendarKind::Gregorian
    }

    fn write(&self, at: &icu_time::DateTime<Gregorian>, sink: &mut Spliced) -> fmt::Result {
        match self {
            Formatter::Date(f) => f.format(at).write_to_parts(sink),
            Formatter::DateWeekday(f) => f.format(at).write_to_parts(sink),
            Formatter::DateTime(f) => f.format(at).write_to_parts(sink),
            Formatter::DateTimeWeekday(f) => f.format(at).write_to_parts(sink),
            Formatter::Time(f) => f.format(at).write_to_parts(sink),
        }
    }
}

/// Standalone month and weekday names, indexed by [`NameLength`] (narrow, abbreviated, wide)
/// and then by month (January first) or weekday (Monday first).
#[derive(Debug)]
struct Names {
    months: [[String; 12]; 3],
    weekdays: [[String; 7]; 3],
}

/// Which field comes first, second and third in a numeric date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Year,
    Month,
    Day,
}

#[derive(Debug, Default)]
struct Cache {
    /// By date length (none, short, medium, long, full) times time precision (none, minutes,
    /// seconds); the slot with neither is unused.
    formatters: [OnceLock<Option<Formatter>>; 15],
    names: OnceLock<Option<Names>>,
    year_month: OnceLock<Option<FixedCalendarDateTimeFormatter<Gregorian, YM>>>,
    first_weekday: OnceLock<Option<Weekday>>,
    calendar: OnceLock<Option<icu_calendar::AnyCalendar>>,
    year_offset: OnceLock<Option<i32>>,
    local_year_month: OnceLock<Option<DateTimeFormatter<YM>>>,
    local_months: OnceLock<Option<[[String; 12]; 2]>>,
    order: OnceLock<[Field; 3]>,
}

/// The text a formatter writes, with the byte range of its year: a year past what ICU4X holds is
/// formatted four hundred years' cycles away and written back in place.
#[derive(Default)]
struct Spliced {
    text: String,
    year: Option<Range<usize>>,
}

impl fmt::Write for Spliced {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.text.push_str(s);
        Ok(())
    }
}

impl PartsWrite for Spliced {
    type SubPartsWrite = Self;

    fn with_part(&mut self, part: Part, mut f: impl FnMut(&mut Self) -> fmt::Result) -> fmt::Result {
        let start = self.text.len();
        let result = f(self);
        if part == parts::YEAR && self.year.is_none() {
            self.year = Some(start..self.text.len());
        }
        result
    }
}

impl DateFormat {
    /// The format for `region`'s time locale and clock.
    pub fn new(region: &Region) -> DateFormat {
        Self { locale: region.time.clone(), hour_cycle: region.hour_cycle, latin_digits: false, cache: Arc::default() }
    }

    /// This format writing `0`–`9` for [`Digits::Latin`], else the locale's own digits. Formatters
    /// are rebuilt on first use.
    pub fn with_digits(self, digits: Digits) -> DateFormat {
        DateFormat { latin_digits: digits == Digits::Latin, cache: Arc::default(), ..self }
    }

    /// The format for a locale tag (`de-CH`), with the locale's clock.
    pub fn from_tag(tag: &str) -> DateFormat {
        Self::new(&Region::from_tag(tag))
    }

    /// The time locale with the region's clock as a `-u-hc-` keyword.
    fn icu_locale(&self, latin_digits: bool) -> Locale {
        self.icu_locale_in(latin_digits, false)
    }

    /// As [`Self::icu_locale`], with the calendar forced to Gregorian when `gregorian` is set.
    fn icu_locale_in(&self, latin_digits: bool, gregorian: bool) -> Locale {
        let mut locale = Locale::try_from_str(&self.locale).unwrap_or(Locale::UNKNOWN);
        match self.hour_cycle {
            Some(HourCycle::H12) => locale.extensions.unicode.keywords.set(key!("hc"), value!("h12")),
            Some(HourCycle::H23) => locale.extensions.unicode.keywords.set(key!("hc"), value!("h23")),
            None => None,
        };
        if latin_digits {
            locale.extensions.unicode.keywords.set(key!("nu"), value!("latn"));
        }
        if gregorian {
            locale.extensions.unicode.keywords.set(key!("ca"), value!("gregory"));
        }
        locale
    }

    /// The formatter in the locale's calendar, or in the Gregorian one when `gregorian` is set or
    /// the locale names a calendar whose data isn't bundled.
    fn load_formatter(&self, date: Option<DateLength>, time: Option<TimePrecision>, latin_digits: bool, gregorian: bool) -> Option<Formatter> {
        let own = (!gregorian).then(|| self.load_formatter_in(date, time, self.icu_locale_in(latin_digits, false)));
        own.flatten().or_else(|| self.load_formatter_in(date, time, self.icu_locale_in(latin_digits, true)))
    }

    fn load_formatter_in(&self, date: Option<DateLength>, time: Option<TimePrecision>, locale: Locale) -> Option<Formatter> {
        let provider = data::dates_provider()?;
        let prefs = (&locale).into();
        let length = |l: DateLength| match l {
            DateLength::Short => Length::Short,
            DateLength::Medium => Length::Medium,
            DateLength::Long | DateLength::Full => Length::Long,
        };
        let precision = |t: TimePrecision| match t {
            TimePrecision::Minutes => IcuPrecision::Minute,
            TimePrecision::Seconds => IcuPrecision::Second,
        };
        match (date, time) {
            (None, None) => None,
            (None, Some(t)) => {
                let set = match t {
                    TimePrecision::Minutes => T::hm(),
                    TimePrecision::Seconds => T::hms(),
                };
                DateTimeFormatter::try_new_with_buffer_provider(provider, prefs, set).ok().map(Formatter::Time)
            }
            (Some(DateLength::Full), None) => {
                DateTimeFormatter::try_new_with_buffer_provider(provider, prefs, YMDE::long()).ok().map(Formatter::DateWeekday)
            }
            (Some(d), None) => {
                DateTimeFormatter::try_new_with_buffer_provider(provider, prefs, YMD::medium().with_length(length(d))).ok().map(Formatter::Date)
            }
            (Some(DateLength::Full), Some(t)) => {
                DateTimeFormatter::try_new_with_buffer_provider(provider, prefs, YMDE::long().with_time(precision(t)))
                    .ok()
                    .map(Formatter::DateTimeWeekday)
            }
            (Some(d), Some(t)) => {
                DateTimeFormatter::try_new_with_buffer_provider(provider, prefs, YMD::medium().with_length(length(d)).with_time(precision(t)))
                    .ok()
                    .map(Formatter::DateTime)
            }
        }
    }

    /// `at` written as `style` says. A style with neither part is empty.
    // new: ICU4X DateTimeFormatter with the region's hour cycle; the apps write English only (lightcraft: crates/ui-egui/src/i18n.rs:445 display_time)
    pub fn format(&self, at: DateTime, style: DateStyle) -> String {
        if style.date.is_none() && style.time.is_none() {
            return String::new();
        }
        let slot = style.date.map_or(0, |d| d as usize + 1) * 3 + style.time.map_or(0, |t| t as usize + 1);
        let formatter = self
            .cache
            .formatters
            .get(slot)
            .and_then(|cell| cell.get_or_init(|| self.load_formatter(style.date, style.time, self.latin_digits, false)).as_ref());
        let Some(((icu, true_year), formatter)) = to_icu(at).zip(formatter) else {
            return plain(at, style);
        };
        // A year moved by whole cycles is written back as a Gregorian year, which only a Gregorian
        // formatter can do.
        if true_year.is_some() && !formatter.is_gregorian() {
            return plain(at, style);
        }
        let mut out = Spliced::default();
        if formatter.write(&icu, &mut out).is_err() {
            return plain(at, style);
        }
        if let (Some(year), Some(true_year)) = (out.year, true_year) {
            out.text.replace_range(year, &true_year.to_string());
        }
        out.text
    }

    fn names(&self) -> Option<&Names> {
        self.cache
            .names
            .get_or_init(|| {
                let provider = data::dates_provider()?;
                let provider = provider.as_deserializing();
                let locale = self.icu_locale(false);
                // One names object per pattern: ICU4X refuses two lengths of one field in one.
                let name = |pattern: &str, date: CalendarDate| -> Option<String> {
                    let pattern: DateTimePattern = pattern.parse().ok()?;
                    let at = to_icu(DateTime { date, hour: 0, minute: 0, second: 0 })?.0;
                    let mut names = FixedCalendarDateTimeNames::<Gregorian>::try_new_unstable(&provider, (&locale).into()).ok()?;
                    let formatted = names.load_for_pattern(&provider, &pattern).ok()?.format(&at);
                    Some(formatted.try_write_to_string().map_or_else(|(_, text)| text.into_owned(), |text| text.into_owned()))
                };
                let months = ["LLLLL", "LLL", "LLLL"].map(|pattern| {
                    std::array::from_fn(|i| name(pattern, CalendarDate { year: 2026, month: i as u8 + 1, day: 1 }).unwrap_or_default())
                });
                // 2026-10-26 is a Monday.
                let weekdays = ["ccccc", "ccc", "cccc"].map(|pattern| {
                    std::array::from_fn(|i| name(pattern, CalendarDate { year: 2026, month: 10, day: 26 + i as u8 }).unwrap_or_default())
                });
                Some(Names { months, weekdays })
            })
            .as_ref()
    }

    /// The name of `month` (1 to 12) as it stands alone (a calendar's title): `October`,
    /// `październik`.
    pub fn month_name(&self, month: u8, length: NameLength) -> String {
        let index = usize::from(month.clamp(1, 12)) - 1;
        match self.names() {
            Some(names) => names.months[length as usize][index].clone(),
            None => month.to_string(),
        }
    }

    /// The month and year as a calendar's title writes them, in the region's order and digits:
    /// `October 2026`, `2026年10月`. A year past 9999 keeps its digits. Without locale data,
    /// `2026-10`.
    pub fn year_month(&self, year: i64, month: u8) -> String {
        let month = month.clamp(1, 12);
        let formatter = self.cache.year_month.get_or_init(|| {
            let provider = data::dates_provider()?;
            let locale = self.icu_locale(self.latin_digits);
            FixedCalendarDateTimeFormatter::try_new_with_buffer_provider(provider, (&locale).into(), YM::long()).ok()
        });
        let at = DateTime { date: CalendarDate { year, month, day: 1 }, hour: 0, minute: 0, second: 0 };
        let Some(((icu, true_year), formatter)) = to_icu(at).zip(formatter.as_ref()) else {
            return format!("{year:04}-{month:02}");
        };
        let mut out = Spliced::default();
        if formatter.format(&icu).write_to_parts(&mut out).is_err() {
            return format!("{year:04}-{month:02}");
        }
        if let (Some(range), Some(true_year)) = (out.year, true_year) {
            out.text.replace_range(range, &true_year.to_string());
        }
        out.text
    }

    /// The name of `day` as it stands alone: `Saturday`, `Sat`, `S`.
    pub fn weekday_name(&self, day: Weekday, length: NameLength) -> String {
        match self.names() {
            Some(names) => names.weekdays[length as usize][day as usize].clone(),
            None => (day as u8 + 1).to_string(),
        }
    }

    /// The day a week starts on in the region: Sunday in the United States, Monday in most of
    /// Europe, Saturday in parts of the Middle East.
    pub fn first_weekday(&self) -> Weekday {
        let first = self.cache.first_weekday.get_or_init(|| {
            let provider = data::dates_provider()?;
            let locale = self.icu_locale(false);
            let info = WeekInformation::try_new_with_buffer_provider(provider, (&locale).into()).ok()?;
            Some(from_icu_weekday(info.first_weekday))
        });
        first.unwrap_or(Weekday::Monday)
    }

    /// The order of the numeric fields in the region's short date, read off a date written in it.
    fn field_order(&self) -> [Field; 3] {
        *self.cache.order.get_or_init(|| {
            // The year ends in 33 and the month and day are 11 and 22, in the display calendar,
            // since that is the order a typed date follows.
            let probe_year = self.century_start() + 33;
            let numeric = |length: DateLength| -> Option<[Field; 3]> {
                let date = self.from_local(LocalDate { year: probe_year, month: 11, day: 22 })?;
                let (icu, _) = to_icu(DateTime { date, hour: 0, minute: 0, second: 0 })?;
                let formatter = self.load_formatter(Some(length), None, true, false)?;
                let mut out = Spliced::default();
                formatter.write(&icu, &mut out).ok()?;
                let runs = digit_runs(&out.text);
                let [a, b, c] = runs.as_slice() else { return None };
                let field = |run: &str| match run.parse::<i32>().ok()? {
                    11 => Some(Field::Month),
                    22 => Some(Field::Day),
                    33 => Some(Field::Year),
                    y if y == probe_year => Some(Field::Year),
                    _ => None,
                };
                Some([field(a)?, field(b)?, field(c)?])
            };
            numeric(DateLength::Short).or_else(|| numeric(DateLength::Medium)).unwrap_or([Field::Month, Field::Day, Field::Year])
        })
    }

    /// The first year of the display calendar's century that holds 2026, where two-digit years
    /// are read: 2000, 2500 in the Buddhist calendar, 1400 in the Persian.
    fn century_start(&self) -> i32 {
        self.to_local(CalendarDate { year: 2026, month: 1, day: 1 }).map_or(2000, |l| l.year.div_euclid(100) * 100)
    }

    /// A date typed in the region's short or medium form (`10/31/2026`, `31.10.2026`), for a
    /// date field, in the display calendar, or as Gregorian ISO (`2026-10-31`, whatever the
    /// calendar): a Thai `31/10/2569` and a Persian
    /// `۱۴۰۵/۰۸/۰۹` are the Gregorian 2026-10-31. Two-digit years are in the display calendar's
    /// current century (2000 to 2099, 1400 to 1499 in Persian). Digits are any script's.
    /// `None` when the text isn't a valid date.
    pub fn parse_date(&self, text: &str) -> Option<CalendarDate> {
        let text: String = text.chars().map(|c| ascii_digit(c).unwrap_or(c)).collect();
        if text.chars().any(|c| c.is_alphanumeric() && !c.is_ascii_digit()) {
            return None;
        }
        let runs = digit_runs(&text);
        let [a, b, c] = runs.as_slice() else { return None };
        // `2026-10-31`, the form `plain` writes, is Gregorian ISO in every locale.
        let iso = a.len() == 4 && text.trim().split('-').map(str::len).eq([4, b.len(), c.len()]);
        if iso {
            return CalendarDate::new(a.parse().ok()?, b.parse().ok()?, c.parse().ok()?);
        }
        let order = if a.len() == 4 { [Field::Year, Field::Month, Field::Day] } else { self.field_order() };
        let (mut year, mut month, mut day) = (None, None, None);
        for (field, run) in order.into_iter().zip([a, b, c]) {
            let value: i64 = run.parse().ok()?;
            match field {
                Field::Year => year = Some(if run.len() <= 2 { i64::from(self.century_start()) + value } else { value }),
                Field::Month => month = Some(u8::try_from(value).ok()?),
                Field::Day => day = Some(u8::try_from(value).ok()?),
            }
        }
        let (year, month, day) = (year?, month?, day?);
        if self.calendar_kind() == CalendarKind::Gregorian {
            return CalendarDate::new(year, month, day);
        }
        self.from_local(LocalDate { year: i32::try_from(year).ok()?, month, day })
    }
}

/// The value of a decimal digit of any script that writes its ten digits in a row.
fn ascii_digit(c: char) -> Option<char> {
    const ZEROS: [u32; 18] =
        [0x660, 0x6F0, 0x7C0, 0x966, 0x9E6, 0xA66, 0xAE6, 0xB66, 0xBE6, 0xC66, 0xCE6, 0xD66, 0xE50, 0xED0, 0xF20, 0x1040, 0x17E0, 0xFF10];
    ZEROS.iter().find_map(|&zero| (c as u32).checked_sub(zero).filter(|d| *d < 10)).and_then(|d| char::from_digit(d, 10))
}

/// The maximal runs of `0`–`9` in `text`.
fn digit_runs(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_ascii_digit()).filter(|run| !run.is_empty()).collect()
}

/// `at` as ICU4X dates, with the true year when it had to be moved four hundred years' cycles to
/// fit ICU4X's range.
fn to_icu(at: DateTime) -> Option<(icu_time::DateTime<Gregorian>, Option<i64>)> {
    let direct = i32::try_from(at.date.year).ok().and_then(|y| Date::try_new_gregorian(y, at.date.month, at.date.day).ok());
    let (date, true_year) = match direct {
        Some(date) => (date, None),
        None => {
            let year = 2000 + (at.date.year - 2000).rem_euclid(400) as i32;
            (Date::try_new_gregorian(year, at.date.month, at.date.day).ok()?, Some(at.date.year))
        }
    };
    let time = Time::try_new(at.hour, at.minute, at.second, 0).ok()?;
    Some((icu_time::DateTime { date, time }, true_year))
}

/// `2026-10-31 14:05`: what a style writes when no locale data loads.
fn plain(at: DateTime, style: DateStyle) -> String {
    let CalendarDate { year, month, day } = at.date;
    let mut parts = Vec::new();
    if style.date.is_some() {
        parts.push(format!("{year:04}-{month:02}-{day:02}"));
    }
    match style.time {
        Some(TimePrecision::Minutes) => parts.push(format!("{:02}:{:02}", at.hour, at.minute)),
        Some(TimePrecision::Seconds) => parts.push(format!("{:02}:{:02}:{:02}", at.hour, at.minute, at.second)),
        None => {}
    }
    parts.join(" ")
}

fn from_icu_weekday(day: icu_calendar::types::Weekday) -> Weekday {
    use icu_calendar::types::Weekday as W;
    match day {
        W::Monday => Weekday::Monday,
        W::Tuesday => Weekday::Tuesday,
        W::Wednesday => Weekday::Wednesday,
        W::Thursday => Weekday::Thursday,
        W::Friday => Weekday::Friday,
        W::Saturday => Weekday::Saturday,
        W::Sunday => Weekday::Sunday,
    }
}

/// The system's offset from UTC in seconds at `unix`. A moment beyond chrono's range takes the
/// offset of the same moment in the 400-year cycle that fits.
fn local_offset(unix: i64) -> i64 {
    let at = Local.timestamp_opt(unix, 0).single().or_else(|| Local.timestamp_opt(unix.rem_euclid(CYCLE_SECS), 0).single());
    at.map_or(0, |t| i64::from(t.offset().fix().local_minus_utc()))
}

impl I18n {
    /// How dates are written: [`Region::system`]'s until [`I18n::set_dates`], and again after
    /// [`I18n::reset_dates`].
    // new: the counterpart of I18n::format for numbers
    pub fn dates(&self) -> Arc<DateFormat> {
        if let Some(d) = self.dates.read().unwrap_or_else(PoisonError::into_inner).as_ref() {
            return Arc::clone(d);
        }
        let mut slot = self.dates.write().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(slot.get_or_insert_with(|| Arc::new(DateFormat::new(&Region::system()))))
    }

    /// Write dates as `dates` does from now on: the user's region setting.
    pub fn set_dates(&self, dates: DateFormat) {
        *self.dates.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(dates));
    }

    /// Write dates as the system region does again.
    pub fn reset_dates(&self) {
        *self.dates.write().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// `unix` seconds as `YYYY-MM-DD hh:mm` (or `hh:mm:ss` with [`TimePrecision::Seconds`]), UTC,
/// with no zone suffix: the same in every language. Years past 9999 keep all their digits
/// (`{y:04}`, never `%Y`'s `+`).
// effectcraft: crates/ui-egui/src/panels/home.rs:92 format_utc, crates/ui-egui/src/panels/render_queue.rs:62 utc_stamp
pub fn utc_stamp(unix: i64, precision: TimePrecision) -> String {
    let at = DateTime::from_unix(unix, TimeZone::Utc);
    let style = DateStyle { date: Some(DateLength::Short), time: Some(precision) };
    plain(at, style)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1970-01-01 is day 0 and a Thursday; 2000-02-29 exists and 2100-02-29 doesn't; from_days
    // and days round-trip across ±400-year cycles and years past 9999; Jan 31 + 1 month is Feb 29
    // in 2028.
    #[test]
    fn calendar_dates_round_trip_and_step() {
        let epoch = CalendarDate::new(1970, 1, 1).unwrap();
        assert_eq!((epoch.days(), epoch.weekday()), (0, Weekday::Thursday));
        assert_eq!(CalendarDate::from_days(-1), CalendarDate { year: 1969, month: 12, day: 31 });
        assert!(CalendarDate::new(2000, 2, 29).is_some());
        assert!(CalendarDate::new(2100, 2, 29).is_none());
        assert!(CalendarDate::new(2026, 4, 31).is_none());
        assert!(CalendarDate::new(2026, 13, 1).is_none());
        assert_eq!(CalendarDate::days_in_month(2024, 2), 29);
        assert_eq!(CalendarDate::days_in_month(1900, 2), 28);
        assert_eq!(CalendarDate::days_in_month(2026, 12), 31);
        for days in [-1_000_000_000_000, -146_097 * 3 - 5, -719_528, -1, 0, 20_392, 146_097, 3_000_000, 4_000_000_000_000] {
            let date = CalendarDate::from_days(days);
            assert_eq!(date.days(), days, "{date:?}");
            assert_eq!(CalendarDate::new(date.year, date.month, date.day), Some(date));
        }
        // The weekday repeats every 400 years.
        let saturday = CalendarDate::new(2026, 10, 31).unwrap();
        assert_eq!(saturday.weekday(), Weekday::Saturday);
        assert_eq!(CalendarDate::new(2026 + 400 * 1000, 10, 31).unwrap().weekday(), Weekday::Saturday);
        assert_eq!(CalendarDate::new(12_026, 10, 31).unwrap().days() - saturday.days(), 10_000 * 365 + 2425);
        let jan31 = CalendarDate::new(2028, 1, 31).unwrap();
        assert_eq!(jan31.add_months(1), CalendarDate::new(2028, 2, 29).unwrap());
        assert_eq!(jan31.add_months(13), CalendarDate::new(2029, 2, 28).unwrap());
        assert_eq!(jan31.add_months(-2), CalendarDate::new(2027, 11, 30).unwrap());
        assert_eq!(saturday.add_days(1), CalendarDate::new(2026, 11, 1).unwrap());
    }
}
