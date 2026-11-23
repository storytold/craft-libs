//! `CalendarPopover`: the calendar of a `DatePicker` hung from any rect, and the calendar body the two share.

use super::super::words::PickerWords;
use super::{Calendar, CalendarView, DatePrecision, PickedDate, offered};
use crate::buttons::Link;
use crate::choice::Segmented;
use crate::contract::{
    Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, Entry, HeaderRegion, Mount, Overlay, POPUP_PAD, Props as _,
    Remember as _, Role, Show as _, ShowContainer as _, Trigger, WordSetCx as _, clicked,
};
use crate::layout::RowAlign;
use crate::layout_components::Row;
use crate::popover::PopoverOutput;
use crate::runtime::env::Label;
use crate::runtime::overlay::CloseReason;
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::{EditEvent, Metrics, Mixed, Sense};
use craft_i18n::{DateFormat, Weekday};
use egui::{Response, Ui};
use std::sync::Arc;

/// The style of a calendar popover: the surface comes from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CalendarPopoverStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a calendar body reads: the value, the precisions and bounds, and the words and dates of its scope.
#[derive(Clone, Copy)]
pub(super) struct CalendarAt<'a> {
    /// The value the page marks as selected.
    pub(super) current: Option<PickedDate>,
    /// The value the page follows: a change of it while the calendar is open moves the page to it.
    pub(super) settled: Option<PickedDate>,
    pub(super) precisions: &'a [DatePrecision],
    pub(super) min: Option<PickedDate>,
    pub(super) max: Option<PickedDate>,
    pub(super) today: Option<PickedDate>,
    /// The first weekday the host sets, for the page's grid.
    pub(super) first_weekday: Option<Weekday>,
    pub(super) dates: &'a DateFormat,
    pub(super) words: &'a PickerWords,
    pub(super) name: &'a Label,
}

/// What the calendar's body reported this pass.
pub(super) struct Chosen {
    /// The day under the pointer.
    pub(super) hovered: Option<PickedDate>,
    /// A picked value, `Some(None)` for Clear.
    pub(super) pick: Option<Option<PickedDate>>,
}

/// The body's memory: the page and the value it last followed. The arrows' page stays while that value does.
#[derive(Clone, Copy, Default)]
struct Followed {
    view: Option<CalendarView>,
    seen: Option<PickedDate>,
}

/// The calendar: a precision switch when more than one is offered, the page, and Today and Clear.
pub(super) fn calendar_body(ui: &mut Ui, body: &mut BodyCx<'_>, at: CalendarAt<'_>) -> Chosen {
    let CalendarAt { current, settled, precisions, min, max, today, first_weekday, dates, words, name } = at;
    let Followed { view: kept, seen } = *body.memory::<Followed>();
    let mut view = match kept {
        Some(view) if seen == settled => view,
        // `landed` moves only the active cell; a value on another page needs the page opened on it.
        _ => CalendarView::open(current, today, precisions, dates),
    };
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
            view = view.switched(precision, dates);
            view = view.landed(current, today, dates);
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
    if let Some(day) = first_weekday {
        calendar = calendar.first_weekday(day);
    }
    let page = calendar.show(ui);
    if let Some(next) = page.moved {
        view = next;
    }
    let mut pick = page.picked.map(Some);
    // Today and Clear are centred under the page, as production's; Clear shows once there is a value to clear.
    Row::new().fill_width().align(RowAlign::Center).show(ui, |ui, _| {
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
        if current.is_some() && clicked(&Link::new(words.date_clear.clone()).show(ui)) {
            pick = Some(None);
        }
    });
    *body.memory::<Followed>() = Followed { view: Some(view), seen: settled };
    Chosen { hovered: page.hovered, pick }
}

/// A calendar hung from any rect, with `DatePicker`'s calendar body: the precision switch, the page, Today and
/// Clear. A pick writes the value and closes the popover with `CloseReason::Activated`; a value set from outside
/// while it is open moves the page to it. It has no text field, so the day under the pointer is not previewed.
///
/// The calendar is the container's own content: show it with an empty body.
///
/// ```ignore
/// CalendarPopover::new("Due date", Bind::bound(&mut open), Bind::bound(&mut due)).opened_by(&button).show(ui, |_, _| {});
/// ```
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct CalendarPopover<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    open: Bind<'a, bool>,
    closed: Option<CloseReason>,
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
    /// The day the calendar's weeks start on; the locale's when not set.
    #[prop]
    first_weekday: Option<Weekday>,
}

impl<'a> CalendarPopover<'a> {
    /// A calendar popover named `label`, open while `open` is true, reading and writing `value` at every precision.
    ///
    /// `value` is a bound or kept binding: a given binding is never written, and the output carries no edit to report.
    pub fn new(label: impl Into<Label>, open: Bind<'a, bool>, value: Bind<'a, Option<PickedDate>>) -> Self {
        let props =
            ContainerProps::new().surface(SurfaceKind::Popup).pad(POPUP_PAD).header(HeaderRegion::new()).mount(Mount::Layer(Overlay::popover()));
        CalendarPopover {
            common: Common::new(),
            props,
            open,
            closed: None,
            value,
            precisions: &DatePrecision::ALL,
            min: None,
            max: None,
            today: None,
            first_weekday: None,
        }
        .label(label)
    }

    /// The control the calendar opens from. The popover closes when the opener is no longer shown; it hangs from the
    /// opener unless [`overlay`](CalendarPopover::overlay) names another anchor with `Overlay::opener_anchor`.
    #[must_use]
    pub fn opened_by(self, opener: &Response) -> Self {
        self.overlay(|overlay| overlay.opened_by(opener, Trigger::Manual))
    }

    /// Changes the layer the popover shows in, as `Popover::overlay` does: the rect it hangs from, say.
    #[must_use]
    pub fn overlay(mut self, change: impl FnOnce(Overlay) -> Overlay) -> Self {
        let overlay = match std::mem::take(&mut self.props.mount) {
            Mount::Layer(overlay) => overlay,
            _ => Overlay::popover(),
        };
        self.props = self.props.mount(Mount::Layer(change(overlay)));
        self
    }
}

impl Component for CalendarPopover<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = CalendarPopoverStyle;
    type Output = PopoverOutput;

    fn style(_: &StyleCx<'_>, _: CalendarPopoverStyleKey) -> CalendarPopoverStyle {
        CalendarPopoverStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> PopoverOutput {
        PopoverOutput::of(cx, self.closed, self.open)
    }
}

impl Container for CalendarPopover<'_> {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        Some(self.open.reborrow())
    }

    /// The calendar. Its output's value is `NoValue`, so a pick goes to the binding as a set and closes the popover.
    fn header(&mut self, ui: &mut Ui, body: &mut BodyCx<'_>) {
        let dates = match self.first_weekday {
            Some(day) => Arc::new((*body.env.dates()).clone().with_first_weekday(day)),
            None => body.env.dates(),
        };
        let words = body.words::<PickerWords>();
        let name = body.label();
        let read = self.value.reborrow().read(body);
        let current = match &read {
            Some(Mixed::Same(value)) => *value,
            _ => None,
        };
        let today = self.today.or_else(|| body.env.today().and_then(PickedDate::from_calendar_date));
        let at = CalendarAt {
            current,
            settled: current,
            precisions: offered(self.precisions),
            min: self.min,
            max: self.max,
            today,
            first_weekday: self.first_weekday,
            dates: &dates,
            words: &words,
            name: &name,
        };
        let chosen = calendar_body(ui, body, at);
        if let Some(date) = chosen.pick {
            let before = read.unwrap_or(Mixed::Mixed);
            let _ = self.value.reborrow().write(body, &EditEvent::set(before, date, Arc::from(name.as_str()), None));
            body.request_close_as(CloseReason::Activated);
        }
    }

    fn closed(&mut self, why: CloseReason) {
        self.closed.get_or_insert(why);
    }
}
