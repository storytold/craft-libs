//! `DatePicker`: a text field that reads and writes a [`PickedDate`], with a calendar button that opens a popover of
//! days, months or years (10.4, 15.4, 15.5).
//!
//! The text part commits on Enter or focus loss at the precisions the picker offers. Alt+Down or a click on the
//! button opens the calendar; the day under the pointer is proposed while it is open and a pick commits it. Escape
//! or a press outside closes it and restores the value.

mod calendar;
mod codec;
#[cfg(feature = "testing")]
mod stories;

pub use calendar::{Calendar, CalendarOutput, CalendarStyle, CalendarView, local_month_grid, month_grid};
pub use codec::DateCodec;

use super::words::PickerWords;
use crate::buttons::{IconButton, Link};
use crate::choice::Segmented;
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Entry, Input, MeasureCx, Overlay, OverlayUi, Overlays as _, PaintUi,
    PartsCx, Props as _, Remember as _, Role, Show as _, WordSetCx as _, clicked,
};
use crate::primitives::{IconSpec, TextKey, TextOptions, TextRun};
use crate::runtime::env::Label;
use crate::text::{Adornment, FieldStyle, place, reserve};
use craft_behavior::{CancelReason, GesturePhase, HitLevel, HitPriority, IconId, PartKey, Sense};
use craft_i18n::{CalendarDate, DateFormat};
use egui::{Rect, Vec2};
use std::sync::Arc;

/// The text part.
const TEXT: PartKey = PartKey::new("text");
/// The button that opens the calendar.
const CALENDAR: PartKey = PartKey::new("calendar");
/// The button's icon.
const CALENDAR_ICON: &str = "calendar";
/// The last year that can be picked (four digits).
const LAST_YEAR: i32 = 9999;

fn text_key() -> TextKey<Option<PickedDate>> {
    TextKey::new(TEXT)
}

/// How much of a date a value names.
///
/// A picker offering only [`DatePrecision::Day`] shows the region's calendar on every page: its Month and Year pages
/// only move the day grid (the title goes up a level), so they list the calendar's own months and years (Persian
/// ones, say). A picker offering Month or Year as well picks stored values, which are Gregorian partial dates, so its
/// Month and Year pages hold Gregorian months and years (shifted by the year offset where the calendar's months are
/// the Gregorian ones, as in the Buddhist calendar).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum DatePrecision {
    /// A whole year.
    Year,
    /// A month of a year.
    Month,
    /// One day.
    Day,
}

impl DatePrecision {
    /// Every precision, from coarsest to finest.
    pub const ALL: [DatePrecision; 3] = [DatePrecision::Year, DatePrecision::Month, DatePrecision::Day];
}

/// A year, a month of it or a day of that month: `2026`, `2026-08`, `2026-08-14`. A value keeps its precision: `2026`
/// means the whole year. A day-precision value is one calendar date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PickedDate {
    /// The year, 0 to 9999.
    pub year: i32,
    /// 1 to 12.
    pub month: Option<u8>,
    /// 1 to the month's last day; only with a month.
    pub day: Option<u8>,
}

impl PickedDate {
    /// A value naming the whole `year`.
    pub fn year(year: i32) -> Self {
        Self { year, month: None, day: None }
    }

    /// A value naming `month` (1 to 12) of `year`.
    pub fn month(year: i32, month: u8) -> Self {
        Self { year, month: Some(month), day: None }
    }

    /// A value naming one day; `month` is 1 to 12 and `day` is 1 to the month's last day.
    pub fn day(year: i32, month: u8, day: u8) -> Self {
        Self { year, month: Some(month), day: Some(day) }
    }

    /// Reads `2026`, `2026-08` or `2026-08-14`; anything after a day's `T` or a space (a time) is read past. `None`
    /// for anything else or a date that doesn't exist (`2026-02-30`).
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let number =
            |from: usize, to: usize| text.get(from..to).filter(|t| t.bytes().all(|c| c.is_ascii_digit())).and_then(|t| t.parse::<u32>().ok());
        let year = i32::try_from(number(0, 4)?).ok()?;
        if text.len() == 4 {
            return Some(Self::year(year));
        }
        let month = u8::try_from(number(5, 7)?).ok().filter(|m| (1..=12).contains(m))?;
        if text.get(4..5) != Some("-") {
            return None;
        }
        if text.len() == 7 {
            return Some(Self::month(year, month));
        }
        let day = u8::try_from(number(8, 10)?).ok().filter(|d| (1..=CalendarDate::days_in_month(i64::from(year), month)).contains(d))?;
        // A time after the day is read past, but it must be one: T10, T10:00 or T10:00:00, the T or a space.
        let time_ok = match text.len() {
            10 => true,
            13 | 16 | 19 => {
                matches!(text.get(10..11), Some("T" | " "))
                    && text.char_indices().skip(11).all(|(i, c)| if i == 13 || i == 16 { c == ':' } else { c.is_ascii_digit() })
            }
            _ => false,
        };
        (text.get(7..8) == Some("-") && time_ok).then_some(Self::day(year, month, day))
    }

    /// `2026`, `2026-08` or `2026-08-14`, years padded to four digits.
    pub fn iso(&self) -> String {
        match (self.month, self.day) {
            (Some(month), Some(day)) => format!("{:04}-{month:02}-{day:02}", self.year),
            (Some(month), None) => format!("{:04}-{month:02}", self.year),
            _ => format!("{:04}", self.year),
        }
    }

    /// How much of a date the value names: year, month or day.
    pub fn precision(&self) -> DatePrecision {
        match (self.month, self.day) {
            (Some(_), Some(_)) => DatePrecision::Day,
            (Some(_), None) => DatePrecision::Month,
            _ => DatePrecision::Year,
        }
    }

    /// The first day it covers, at day precision.
    pub fn first_day(&self) -> PickedDate {
        Self::day(self.year, self.month.unwrap_or(1), self.day.unwrap_or(1))
    }

    /// The last day it covers, at day precision.
    pub fn last_day(&self) -> PickedDate {
        let month = self.month.unwrap_or(12);
        Self::day(self.year, month, self.day.unwrap_or_else(|| CalendarDate::days_in_month(i64::from(self.year), month)))
    }

    /// The calendar date of its first day, for `craft_i18n::DateFormat::format`.
    pub fn calendar_date(&self) -> Option<CalendarDate> {
        let first = self.first_day();
        CalendarDate::new(i64::from(first.year), first.month?, first.day?)
    }

    /// The day-precision value of `date`; `None` for a year outside the four digits.
    pub fn from_calendar_date(date: CalendarDate) -> Option<Self> {
        let year = i32::try_from(date.year).ok().filter(|y| (0..=LAST_YEAR).contains(y))?;
        Some(Self::day(year, date.month, date.day))
    }

    fn day_key(&self) -> (i32, u8, u8) {
        (self.year, self.month.unwrap_or(1), self.day.unwrap_or(1))
    }

    /// Whether any day of it lies between `min` and `max`, each optional and inclusive.
    pub fn in_bounds(&self, min: Option<&PickedDate>, max: Option<&PickedDate>) -> bool {
        min.is_none_or(|m| self.last_day().day_key() >= m.first_day().day_key())
            && max.is_none_or(|m| self.first_day().day_key() <= m.last_day().day_key())
    }
}

/// Whether the hover preview of an open calendar has proposed a value that closing must cancel.
#[derive(Clone, Default)]
struct Previewing(bool);

/// What the calendar's body reported this pass.
struct Chosen {
    hovered: Option<PickedDate>,
    pick: Option<Option<PickedDate>>,
}

/// A text field for a date with a calendar popover. Typing a date commits on Enter or focus loss: ISO (`2026`,
/// `2026-08`, `2026-08-14`) or, at day precision, the region's short date. The calendar button and Alt+Down open the
/// popover on the value's page. Arrows move the active day, Page Up and Page Down page by month (by year on a month
/// page), Home and End go to the week's ends, Enter picks and Escape closes without a change.
///
/// ```ignore
/// DatePicker::new("From", Bind::bound(&mut rule.from)).precisions(&[DatePrecision::Day]).show(ui);
/// ```
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(overlay)]
pub struct DatePicker<'a> {
    #[common]
    common: Common,
    value: Bind<'a, Option<PickedDate>>,
    /// The precisions offered; the first is the popup's page when the value has none of them.
    #[prop]
    precisions: &'a [DatePrecision],
    /// The earliest value allowed: a value is allowed when any of its days is in range.
    #[prop]
    min: Option<PickedDate>,
    /// The latest value allowed: a value is allowed when any of its days is in range.
    #[prop]
    max: Option<PickedDate>,
    /// What today is: the calendar rings its cell and the Today link picks it. Without it, the day the host supplies
    /// (`RootEnv::today`); without either neither shows.
    #[prop]
    today: Option<PickedDate>,
}

impl<'a> DatePicker<'a> {
    /// A date picker named `label` over `value`, at every precision.
    pub fn new(label: impl Into<Label>, value: Bind<'a, Option<PickedDate>>) -> Self {
        DatePicker { common: Common::new(), value, precisions: &DatePrecision::ALL, min: None, max: None, today: None }.label(label)
    }

    fn offered(&self) -> &'a [DatePrecision] {
        if self.precisions.is_empty() { &DatePrecision::ALL } else { self.precisions }
    }

    fn codec(&self, dates: Arc<DateFormat>, mixed: bool) -> DateCodec {
        DateCodec::new(dates, self.offered()).bounded(self.min, self.max).mixed(mixed)
    }

    fn trailing(name: Label) -> [Adornment; 1] {
        [Adornment::button(CALENDAR, IconSpec::svg(IconId::new(CALENDAR_ICON)), name)]
    }
}

impl Component for DatePicker<'_> {
    const ROLE: Role = Role::TEXT_INPUT;
    const INPUT: Input = Input::Text;
    type Style = FieldStyle;
    type Output = EditResponse<Option<PickedDate>>;

    fn style(cx: &crate::style::StyleCx<'_>, _: DatePickerStyleKey) -> FieldStyle {
        FieldStyle::at(cx)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    /// The widest typical date and the room of the calendar button.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        use craft_behavior::Codec as _;
        let codec = self.codec(cx.env.dates(), false);
        let sample = codec.sample(&None);
        let text = cx.layout(&TextRun::new(sample.as_ref())).size();
        let name = cx.words::<PickerWords>().date_pick.clone();
        Vec2::new(text.x + reserve(cx.style, &[], &Self::trailing(name)), text.y)
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let name = cx.words::<PickerWords>().date_pick.clone();
        let trailing = Self::trailing(name);
        let placed = place(cx.content_rect, cx.style, &[], &trailing, cx.flow.is_rtl());
        let shown = cx.value();
        let codec = self.codec(cx.env.dates(), shown.is_none());
        cx.text(text_key(), placed.text, &codec, shown.unwrap_or(&None), &TextOptions::new());
        if let Some(at) = placed.trailing.first() {
            cx.part(CALENDAR, *at)
                .sense(Sense::CLICK | Sense::KEEP_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .pointer_only("Alt+Down opens the calendar")
                .toggles("calendar");
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, Option<PickedDate>>> {
        Some(self.value.reborrow())
    }

    /// A hover preview that the calendar closed on without a pick is cancelled.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("calendar");
        if popup.is_open(cx) || !cx.memory::<Previewing>().0 {
            return;
        }
        cx.memory::<Previewing>().0 = false;
        if matches!(cx.gesture(), Some(GesturePhase::Begin | GesturePhase::Update)) {
            cx.cancel(CancelReason::Dismissed);
        }
    }

    /// The calendar: a precision switch when more than one is offered, the page, and Today and Clear. The day under
    /// the pointer is proposed; a pick proposes and commits, and closes the popover.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("calendar");
        let dates = cx.env.dates();
        let words = cx.words::<PickerWords>();
        let current = cx.value().copied().flatten();
        let precisions = self.offered();
        let (min, max) = (self.min, self.max);
        let today = self.today.or_else(|| cx.env.today().and_then(PickedDate::from_calendar_date));
        let name = cx.label();
        let chosen = ui.show(popup, Overlay::popover(), |ui, body| {
            let mut view = (*body.memory::<Option<CalendarView>>()).unwrap_or_else(|| CalendarView::open(current, today, precisions, &dates));
            if precisions.len() > 1 {
                let entries: Vec<Entry<DatePrecision>> = precisions
                    .iter()
                    .map(|precision| {
                        let label = match precision {
                            DatePrecision::Year => &words.date_year,
                            DatePrecision::Month => &words.date_month,
                            _ => &words.date_day,
                        };
                        Entry::new(*precision, label.clone())
                    })
                    .collect();
                let mut precision = view.precision;
                Segmented::new(words.date_precision.clone(), Bind::bound(&mut precision), &entries).show(ui);
                if precision != view.precision {
                    view = view.switched(precision, &dates);
                    view = view.landed(current, today, &dates);
                }
            }
            let mut calendar = Calendar::new(name.clone(), view, current);
            if let Some(min) = min {
                calendar = calendar.min(min);
            }
            if let Some(max) = max {
                calendar = calendar.max(max);
            }
            if let Some(today) = today {
                calendar = calendar.today(today);
            }
            let page = calendar.show(ui);
            if let Some(next) = page.moved {
                view = next;
            }
            let mut pick = page.picked.map(Some);
            ui.horizontal(|ui| {
                let at_precision = today.map(|today| match view.precision {
                    DatePrecision::Day => today,
                    DatePrecision::Month => PickedDate::month(today.year, today.month.unwrap_or(1)),
                    _ => PickedDate::year(today.year),
                });
                if let Some(today) = at_precision {
                    let allowed = today.in_bounds(min.as_ref(), max.as_ref());
                    if clicked(&Link::new(words.date_today.clone()).enabled(allowed).show(ui)) {
                        pick = Some(Some(today));
                    }
                }
                if clicked(&Link::new(words.date_clear.clone()).enabled(current.is_some()).show(ui)) {
                    pick = Some(None);
                }
            });
            *body.memory::<Option<CalendarView>>() = Some(view);
            Chosen { hovered: page.hovered, pick }
        });
        let Some(chosen) = chosen else { return };
        match chosen.pick {
            Some(date) => {
                cx.propose(date);
                cx.commit();
                popup.close(cx);
            }
            None => {
                if let Some(hovered) = chosen.hovered.filter(|hovered| Some(*hovered) != current) {
                    cx.propose(Some(hovered));
                    cx.memory::<Previewing>().0 = true;
                }
            }
        }
    }

    /// The calendar button's node, which reports whether the calendar is open.
    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.node(CALENDAR, Role::BUTTON);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let name = cx.words::<PickerWords>().date_pick.clone();
        ui.face(CALENDAR, IconButton::new(IconId::new(CALENDAR_ICON)).label(name));
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<Option<PickedDate>> {
        cx.edit_response()
    }
}
