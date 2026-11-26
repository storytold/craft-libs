//! Days in the region's display calendar: [`LocalDate`] and the conversions a calendar grid needs.
//! Stored dates stay Gregorian ([`CalendarDate`]); these map to and from the calendar
//! [`DateFormat::format`] writes.

use icu_calendar::types::{Month, YearInput};
use icu_calendar::{AnyCalendar, AnyCalendarKind, Date, Gregorian, Ref};
use icu_datetime::DateTimeFormatter;
use icu_datetime::fieldsets::{M, YM};

use super::{CalendarDate, DateFormat, DateLength, Formatter, NameLength, Spliced, to_icu};
use crate::data;
use writeable::Writeable as _;

/// The calendar a [`DateFormat`] writes dates in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CalendarKind {
    #[allow(missing_docs)]
    Gregorian,
    /// The Gregorian months and days with the year counted from 543 BC.
    Buddhist,
    #[allow(missing_docs)]
    Persian,
    /// A calendar the data bundles without a name here.
    Other,
}

/// A day written in the display calendar. `month` is the month's number in its year (1 to 12 in
/// the Gregorian, Buddhist and Persian calendars); `year` is the calendar's own year number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalDate {
    #[allow(missing_docs)]
    pub year: i32,
    #[allow(missing_docs)]
    pub month: u8,
    #[allow(missing_docs)]
    pub day: u8,
}

impl DateFormat {
    fn local_calendar(&self) -> Option<&AnyCalendar> {
        self.cache
            .calendar
            .get_or_init(|| match self.load_formatter(Some(DateLength::Short), None, false, false)? {
                Formatter::Date(f) => Some(f.calendar().0.clone()),
                _ => None,
            })
            .as_ref()
    }

    /// The calendar dates are written in: the locale's `-u-ca-` keyword, else the region's default
    /// (Buddhist in Thailand, Persian in Iran and Afghanistan, Gregorian elsewhere), and Gregorian
    /// where the data for a named calendar is not bundled.
    pub fn calendar_kind(&self) -> CalendarKind {
        match self.local_calendar().map(AnyCalendar::kind) {
            Some(AnyCalendarKind::Buddhist) => CalendarKind::Buddhist,
            Some(AnyCalendarKind::Persian) => CalendarKind::Persian,
            Some(AnyCalendarKind::Gregorian) | None => CalendarKind::Gregorian,
            Some(_) => CalendarKind::Other,
        }
    }

    /// `date` in the display calendar; `None` past what ICU4X holds.
    pub fn to_local(&self, date: CalendarDate) -> Option<LocalDate> {
        let calendar = self.local_calendar()?;
        let gregorian = Date::try_new_gregorian(i32::try_from(date.year).ok()?, date.month, date.day).ok()?;
        let local = gregorian.to_calendar(Ref(calendar));
        Some(LocalDate { year: local.year().extended_year(), month: local.month().ordinal, day: local.day_of_month().0 })
    }

    /// The Gregorian date of a day in the display calendar; `None` when the day does not exist.
    pub fn from_local(&self, date: LocalDate) -> Option<CalendarDate> {
        let calendar = self.local_calendar()?;
        let local = Date::try_new(YearInput::Extended(date.year), Month::new(date.month), date.day, Ref(calendar)).ok()?;
        let gregorian = local.to_calendar(Gregorian);
        Some(CalendarDate { year: gregorian.year().extended_year().into(), month: gregorian.month().ordinal, day: gregorian.day_of_month().0 })
    }

    /// 28 to 31; 0 for a month that does not exist.
    pub fn local_days_in_month(&self, year: i32, month: u8) -> u8 {
        let Some(calendar) = self.local_calendar() else { return 0 };
        Date::try_new(YearInput::Extended(year), Month::new(month), 1, Ref(calendar)).map_or(0, |d| d.days_in_month())
    }

    /// The number of months in `year`; 12 in the calendars bundled.
    pub fn local_months_in_year(&self, year: i32) -> u8 {
        let Some(calendar) = self.local_calendar() else { return 0 };
        Date::try_new(YearInput::Extended(year), Month::new(1), 1, Ref(calendar)).map_or(0, |d| d.months_in_year())
    }

    /// The display calendar's years that hold Gregorian years 0 to 9999: the range a picker offers.
    pub fn local_year_range(&self) -> Option<(i32, i32)> {
        let first = self.to_local(CalendarDate { year: 0, month: 1, day: 1 })?;
        let last = self.to_local(CalendarDate { year: 9999, month: 12, day: 31 })?;
        Some((first.year, last.year))
    }

    /// The difference between the display calendar's year and the Gregorian year when every
    /// month of the one is a month of the other (Gregorian 0, Buddhist 543); `None` when its months
    /// start on other days (Persian), so a Gregorian month or year has no month or year of its own
    /// there.
    pub fn local_year_offset(&self) -> Option<i32> {
        *self.cache.year_offset.get_or_init(|| {
            let first = |year: i64, month: u8| self.to_local(CalendarDate { year, month, day: 1 });
            let offset = first(2026, 1)?.year - 2026;
            for year in 2024..=2028 {
                for month in 1..=12 {
                    let local = first(year, month)?;
                    if (i64::from(local.year), local.month, local.day) != (year + i64::from(offset), month, 1) {
                        return None;
                    }
                }
            }
            Some(offset)
        })
    }

    /// The month and year of a month of the display calendar as a title writes them: `October
    /// 2026`, `พฤศจิกายน 2569`, `آبان ۱۴۰۵`.
    pub fn local_year_month(&self, year: i32, month: u8) -> String {
        let fallback = || format!("{year:04}-{month:02}");
        if self.calendar_kind() == CalendarKind::Gregorian {
            return self.year_month(year.into(), month);
        }
        let formatter = self.cache.local_year_month.get_or_init(|| {
            let provider = data::dates_provider()?;
            let locale = self.icu_locale(self.latin_digits);
            DateTimeFormatter::try_new_with_buffer_provider(provider, (&locale).into(), YM::long()).ok()
        });
        let Some(date) = self.from_local(LocalDate { year, month, day: 1 }) else { return fallback() };
        let at = super::DateTime { date, hour: 0, minute: 0, second: 0 };
        let (Some((icu, None)), Some(formatter)) = (to_icu(at), formatter.as_ref()) else { return fallback() };
        let mut out = Spliced::default();
        if formatter.format(&icu).write_to_parts(&mut out).is_err() {
            return fallback();
        }
        out.text
    }

    /// The name of `month` (1 to 12) of the display calendar as it stands alone: `October`,
    /// `آبان`. Narrow names are the abbreviated ones.
    pub fn local_month_name(&self, month: u8, length: NameLength) -> String {
        if matches!(self.calendar_kind(), CalendarKind::Gregorian | CalendarKind::Buddhist) {
            return self.month_name(month, length);
        }
        let index = usize::from(month.clamp(1, 12)) - 1;
        let names = self.cache.local_months.get_or_init(|| {
            let provider = data::dates_provider()?;
            let locale = self.icu_locale(false);
            let year = self.to_local(CalendarDate { year: 2026, month: 1, day: 1 })?.year;
            let build = |abbreviated: bool| -> Option<[String; 12]> {
                let set = if abbreviated { M::medium() } else { M::long() };
                let formatter = DateTimeFormatter::try_new_with_buffer_provider(provider, (&locale).into(), set).ok()?;
                let mut names: [String; 12] = Default::default();
                for (i, name) in names.iter_mut().enumerate() {
                    let date = self.from_local(LocalDate { year, month: i as u8 + 1, day: 1 })?;
                    let (icu, _) = to_icu(super::DateTime { date, hour: 0, minute: 0, second: 0 })?;
                    *name = formatter.format(&icu).write_to_string().into_owned();
                }
                Some(names)
            };
            Some([build(true)?, build(false)?])
        });
        match names {
            Some(names) => names[usize::from(length == NameLength::Wide)][index].clone(),
            None => month.to_string(),
        }
    }
}
