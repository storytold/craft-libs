//! `DateCodec`: the text form of an optional [`PickedDate`].

use super::{DatePrecision, PickedDate};
use craft_behavior::{Codec, NotAValue};
use craft_i18n::{DateFormat, DateLength, DateStyle, DateTime};
use std::borrow::Cow;
use std::sync::Arc;

/// The sample the field is sized by: a date with the widest day and month numbers.
const SAMPLE: (i32, u8, u8) = (2026, 12, 28);

/// Writes a day as the region's short date and a month or year as ISO (`2026-08`, `2026`); reads ISO, and at day
/// precision the region's short or medium numeric date, at the precisions the picker offers and inside its bounds.
/// Empty text is no date.
#[derive(Clone)]
#[non_exhaustive]
pub struct DateCodec {
    dates: Arc<DateFormat>,
    precisions: Vec<DatePrecision>,
    min: Option<PickedDate>,
    max: Option<PickedDate>,
    mixed: bool,
}

impl DateCodec {
    /// A codec that writes with `dates` and reads values of `precisions`.
    pub fn new(dates: Arc<DateFormat>, precisions: &[DatePrecision]) -> Self {
        DateCodec { dates, precisions: precisions.to_vec(), min: None, max: None, mixed: false }
    }

    /// Values outside `min` and `max` are not dates of this codec: a value is inside when any of its days is.
    #[must_use]
    pub fn bounded(mut self, min: Option<PickedDate>, max: Option<PickedDate>) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    /// The field shows no value (a mixed binding): empty text is not a date, so a draft cannot clear every target.
    #[must_use]
    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    fn day_text(&self, date: &PickedDate) -> Option<String> {
        let day = date.calendar_date()?;
        Some(self.dates.format(DateTime { date: day, hour: 0, minute: 0, second: 0 }, DateStyle { date: Some(DateLength::Short), time: None }))
    }
}

impl Codec for DateCodec {
    type Value = Option<PickedDate>;

    fn format(&self, v: &Option<PickedDate>) -> String {
        let Some(date) = v else { return String::new() };
        match date.precision() {
            DatePrecision::Day => self.day_text(date).unwrap_or_else(|| date.iso()),
            _ => date.iso(),
        }
    }

    fn parse(&self, text: &str, _: &Option<PickedDate>) -> Result<Option<PickedDate>, NotAValue> {
        let text = text.trim();
        if text.is_empty() {
            return if self.mixed { Err(NotAValue) } else { Ok(None) };
        }
        let date = PickedDate::parse(text)
            .or_else(|| self.dates.parse_date(text).and_then(PickedDate::from_calendar_date))
            .filter(|date| self.precisions.contains(&date.precision()) && date.in_bounds(self.min.as_ref(), self.max.as_ref()))
            .ok_or(NotAValue)?;
        Ok(Some(date))
    }

    fn sample(&self, _: &Option<PickedDate>) -> Cow<'_, str> {
        let (year, month, day) = SAMPLE;
        let widest = if self.precisions.contains(&DatePrecision::Day) { PickedDate::day(year, month, day) } else { PickedDate::month(year, month) };
        Cow::Owned(self.format(&Some(widest)))
    }
}
