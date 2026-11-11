//! Stories of `NumberField`.

use super::{NumberField, NumberPick};
use crate::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Validity};
use crate::contract::{LabelAlign, SideLabel};
use crate::form::{Field, FieldGroup};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{Mixed, NumberCodec, Reader};
use craft_theme::ControlSize;
use egui::{Key, Modifiers, Ui};

const SIZE: f64 = 12.5;
const HUGE: f64 = 123_456_789_012_345.0;
/// The number the Auto word reads as.
const AUTO: f64 = 0.0;
const RANGE: std::ops::RangeInclusive<f64> = 0.0..=100.0;
/// The range of the Invalid rows: the value 7 lies inside it.
const COLUMNS: std::ops::RangeInclusive<f64> = 1.0..=12.0;

/// A row of the states story: the label column starts at the left edge, as the form list does.
fn row(label: &'static str) -> Field {
    Field::new(label).side(SideLabel::new().align(LabelAlign::Start)).id_salt(label)
}

/// The zoom stops of the old PhotoCraft zoom field.
const ZOOM_STOPS: [f64; 11] = [12.5, 25.0, 50.0, 100.0, 200.0, 400.0, 800.0, 1600.0, 3200.0, 6400.0, 12_800.0];

/// The next zoom stop above `value` for a positive `direction`, the previous one below it otherwise.
fn zoom_step(value: f64, direction: i32) -> f64 {
    let mut stops = ZOOM_STOPS.into_iter();
    match direction > 0 {
        true => stops.find(|stop| *stop > value).unwrap_or(value),
        false => stops.rfind(|stop| *stop < value).unwrap_or(value),
    }
}

/// A check no value passes: the field reads as invalid whatever it holds.
fn refuse_all(_: &Option<f64>) -> Validity<Option<f64>> {
    Validity::Refuse(Message::new("Not accepted"))
}

#[story(NumberField<'static>, covers = [Default], drive = Drive::Type("42"))]
fn number_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("control_size", ControlSize::Md, &[("Sm", ControlSize::Sm), ("Md", ControlSize::Md)]);
    let out = FieldGroup::new("Number field states").show(ui, |ui, _| {
        let first = row("Plain").show(ui, |ui, _| NumberField::new("Plain", Bind::bound(data.value("plain", Some(42.0)))).size(size).show(ui).edit);
        row("With unit").show(ui, |ui, _| {
            NumberField::new("Gap", Bind::bound(data.value("gap", Some(SIZE)))).range(RANGE).step(0.5).unit("mm").size(size).show(ui);
        });
        row("Mixed").show(ui, |ui, _| {
            NumberField::new("Mixed size", Bind::bound_mixed(data.value("mixed", Mixed::<Option<f64>>::Mixed))).unit("pt").size(size).show(ui);
        });
        row("Invalid").show(ui, |ui, _| {
            NumberField::new("Columns", Bind::bound(data.value("invalid", Some(7.0))).check(refuse_all)).range(COLUMNS).size(size).show(ui);
        });
        row("Pop-up slider").show(ui, |ui, _| {
            NumberField::new("Opacity", Bind::bound(data.value("opacity", Some(100.0))))
                .range(RANGE)
                .unit("%")
                .popup_slider(RANGE)
                .size(size)
                .show(ui);
        });
        row("Zoom stops").show(ui, |ui, _| {
            NumberField::new("Zoom", Bind::bound(data.value("zoom", Some(100.0))))
                .range(0.1..=12_800.0)
                .unit("%")
                .stepper(zoom_step)
                .size(size)
                .show(ui);
        });
        row("Disabled").show(ui, |ui, _| {
            NumberField::new("Disabled", Bind::bound(&mut Some(64.0))).range(RANGE).step(0.5).unit("px").enabled(false).size(size).show(ui);
        });
        row("Not a number").show(ui, |ui, _| {
            // Held outside the story data: NaN is never equal to itself, so the suite would read it as changed.
            NumberField::new("Not a number", Bind::bound(&mut Some(f64::NAN))).size(size).show(ui);
        });
        first.inner
    });
    out.inner.flatten().map(StoryOutput::from).unwrap_or_default()
}

#[story(NumberField<'static>, covers = [LongText], drive = Drive::Never)]
fn number_field_long(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    NumberField::new("Size", Bind::bound(data.value("long", Some(HUGE)))).show(ui).edit.into()
}

#[story(NumberField<'static>, covers = [Mixed], drive = Drive::Never)]
fn number_field_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    NumberField::new("Size", Bind::bound_mixed(data.value("mixed", Mixed::<Option<f64>>::Mixed))).show(ui).edit.into()
}

#[story(NumberField<'static>, covers = [Invalid], drive = Drive::Never)]
fn number_field_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    NumberField::new("Size", Bind::bound(data.value("invalid", Some(7.0))).check(refuse_all)).range(COLUMNS).show(ui).edit.into()
}

/// Focuses the field and opens its list or slider with Alt+Down.
fn open_popup(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.idle(4);
}

#[story(NumberField<'static>, covers = [Open], drive = Drive::Custom(open_popup))]
fn number_field_presets(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let presets = [
        Entry::new(NumberPick::Choice(0), "Auto"),
        Entry::separator(),
        Entry::new(NumberPick::Number(8.0), "Small"),
        Entry::new(NumberPick::Number(24.0), "Large"),
    ];
    NumberField::new("Size", Bind::bound(data.value("presets", Some(SIZE)))).presets(&presets).show(ui).edit.into()
}

#[story(NumberField<'static>, covers = [], drive = Drive::Custom(open_popup))]
fn number_field_popup_slider(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    NumberField::new("Size", Bind::bound(data.value("slider", Some(SIZE)))).range(RANGE).popup_slider(RANGE).show(ui).edit.into()
}

#[story(NumberField<'static>, covers = [], drive = Drive::Type("Auto"))]
fn number_field_auto(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let codec = NumberCodec::new(Reader::new(), 1).or_parse(|text| text.trim().eq_ignore_ascii_case("auto").then_some(AUTO));
    NumberField::new("Size", Bind::bound(data.value("auto", Some(SIZE)))).codec(codec).range(RANGE).show(ui).edit.into()
}

#[story(NumberField<'static>, covers = [], drive = Drive::Type("7"))]
fn number_field_snapped(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    NumberField::new("Size", Bind::bound(data.value("snapped", Some(SIZE)))).range(RANGE).step(5.0).snap_typed(true).show(ui).edit.into()
}
