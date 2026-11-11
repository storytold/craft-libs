//! Stories of `DatePicker` and `Calendar`.

use super::{Calendar, CalendarView, DatePicker, DatePrecision, PickedDate};
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use craft_theme::ControlSize;
use egui::{Key, Modifiers, Ui};

const TODAY: (i32, u8, u8) = (2026, 8, 14);
const EARLIEST: (i32, u8, u8) = (2026, 8, 20);

fn today() -> PickedDate {
    PickedDate::day(TODAY.0, TODAY.1, TODAY.2)
}

#[story(DatePicker<'static>, covers = [Default], drive = Drive::Type("2026-09-01"))]
fn date_picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &[("Sm", ControlSize::Sm), ("Md", ControlSize::Md)]);
    let disabled = *data.value("disabled", false);
    DatePicker::new("From", Bind::bound(data.value("from", Some(today())))).today(today()).size(size).enabled(!disabled).show(ui).into()
}

#[story(DatePicker<'static>, covers = [LongText], drive = Drive::Never)]
fn date_picker_long(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    DatePicker::new("From", Bind::bound(data.value("long", Some(PickedDate::day(9999, 12, 28))))).show(ui).into()
}

#[story(DatePicker<'static>, covers = [Mixed], drive = Drive::Never)]
fn date_picker_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    DatePicker::new("From", Bind::bound_mixed(data.value("mixed", Mixed::<Option<PickedDate>>::Mixed))).show(ui).into()
}

#[story(DatePicker<'static>, covers = [Invalid], drive = Drive::Never)]
fn date_picker_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_past = |date: &Option<PickedDate>| match date {
        Some(date) if date.in_bounds(None, Some(&PickedDate::day(EARLIEST.0, EARLIEST.1, EARLIEST.2 - 1))) => {
            Validity::Refuse(Message::new("The date is too early"))
        }
        _ => Validity::Ok,
    };
    DatePicker::new("From", Bind::bound(data.value("early", Some(today()))).check(refuse_past)).show(ui).into()
}

/// Focuses the field and opens the calendar with Alt+Down.
fn open_calendar(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.idle(4);
}

#[story(DatePicker<'static>, covers = [Open], drive = Drive::Custom(open_calendar))]
fn date_picker_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    DatePicker::new("From", Bind::bound(data.value("open", Some(today())))).today(today()).show(ui).into()
}

#[story(DatePicker<'static>, covers = [], drive = Drive::Custom(open_calendar))]
fn date_picker_days_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let min = PickedDate::day(EARLIEST.0, EARLIEST.1, EARLIEST.2);
    DatePicker::new("From", Bind::bound(data.value("days", None))).precisions(&[DatePrecision::Day]).min(min).today(today()).show(ui).into()
}

#[story(DatePicker<'static>, covers = [], disabled_text = "The date is fixed by the template")]
fn date_picker_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tooltip = data.tooltip("The date the photo was taken");
    DatePicker::new("Captured", Bind::bound(data.value("captured", Some(today())))).tooltip(tooltip).show(ui).into()
}

#[story(Calendar, covers = [Default], drive = Drive::Never)]
fn calendar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let dates = crate::runtime::env::Env::of(ui).dates();
    let view = *data.value("view", CalendarView::open(Some(today()), Some(today()), &[DatePrecision::Day], &dates));
    let out = Calendar::new("Pick a day", view, Some(today())).today(today()).show(ui);
    if let Some(next) = out.moved {
        *data.value("view", view) = next;
    }
    out.into()
}
