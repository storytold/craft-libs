//! Stories of `Select`.

use super::Select;
use crate::contract::{Bind, Entry, Props as _, Role, Show as _, ShowContainer as _, Validity};
use crate::contract::{LabelAlign, SideLabel};
use crate::form::{Field, FieldGroup};
use crate::primitives::IconSpec;
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{IconId, Mixed};
use craft_theme::ControlSize;
use egui::Ui;

fn modes() -> [Entry<u8>; 3] {
    [Entry::new(0, "Normal"), Entry::new(1, "Multiply"), Entry::new(2, "Screen")]
}

const UNITS: [&str; 4] = ["Pixels", "Inches", "Centimetres", "Points"];

fn units() -> Vec<Entry<u8>> {
    UNITS.iter().enumerate().map(|(at, name)| Entry::new(at as u8, *name)).collect()
}

fn resampling() -> Vec<Entry<u8>> {
    vec![
        Entry::new(0, "Automatic"),
        Entry::separator(),
        Entry::new(1, "Bicubic"),
        Entry::new(2, "Bilinear"),
        Entry::new(3, "Nearest neighbour"),
        Entry::new(4, "Preserve details (AI)").enabled(false),
    ]
}

fn tools() -> Vec<Entry<u8>> {
    ["brush", "eraser", "eyedropper"]
        .into_iter()
        .zip(["Brush", "Eraser", "Eyedropper"])
        .enumerate()
        .map(|(at, (icon, name))| Entry::new(at as u8, name).icon(IconSpec::svg(IconId::new(icon))))
        .collect()
}

/// A row of the states story: the label column starts at the left edge, as the form list does.
fn row(label: &'static str) -> Field {
    Field::new(label).side(SideLabel::new().align(LabelAlign::Start)).id_salt(label)
}

#[story(Select<'static, u8>, covers = [Default], drive = Drive::Pick { row: 1 })]
fn select(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &[("Sm", ControlSize::Sm), ("Md", ControlSize::Md)]);
    let disabled = *data.value("disabled", false);
    let units = units();
    let unit = data.choice("unit", 0u8, &[("Pixels", 0), ("Inches", 1), ("Centimetres", 2), ("Points", 3)]);
    let out = FieldGroup::new("Select states").show(ui, |ui, _| {
        let first = row("Units").show(ui, |ui, _| Select::new("Units", Bind::bound(unit), &units).size(size).enabled(!disabled).show(ui));
        row("Resampling").show(ui, |ui, _| {
            Select::new("Resampling", Bind::bound(&mut 1u8), &resampling()).size(size).enabled(!disabled).show(ui);
        });
        row("Tool").show(ui, |ui, _| {
            Select::new("Tool", Bind::bound(&mut 0u8), &tools()).size(size).enabled(!disabled).show(ui);
        });
        let profiles = [Entry::new(0u8, "sRGB"), Entry::new(1, "Display P3"), Entry::new(2, "Adobe RGB")];
        let mut none = u8::MAX;
        row("No choice").show(ui, |ui, _| {
            Select::new("Profile", Bind::bound(&mut none), &profiles).placeholder("Choose a profile").size(size).enabled(!disabled).show(ui);
        });
        row("Disabled").show(ui, |ui, _| {
            Select::new("Locked units", Bind::bound(&mut 2u8), &units).size(size).enabled(false).show(ui);
        });
        first.inner
    });
    out.inner.flatten().map(StoryOutput::from).unwrap_or_default()
}

#[story(Select<'static, u8>, covers = [Mixed], drive = Drive::Never)]
fn select_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = modes();
    Select::new("Blend mode", Bind::bound_mixed(data.value("mixed", Mixed::<u8>::Mixed)), &entries).show(ui).into()
}

#[story(Select<'static, u8>, covers = [Invalid], drive = Drive::Never)]
fn select_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = modes();
    let bind = Bind::bound(data.value("invalid", 2u8))
        .check(|mode| if *mode == 2 { Validity::Refuse(Message::new("Screen is not available here")) } else { Validity::Ok });
    Select::new("Blend mode", bind, &entries).show(ui).into()
}

fn open_listbox(fixture: &mut Fixture) {
    if let Some(field) = fixture.by_role(Role::COMBO_BOX).first().copied() {
        fixture.click(&field);
    }
}

#[story(Select<'static, u8>, covers = [Open], drive = Drive::Custom(open_listbox))]
fn select_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = modes();
    Select::new("Blend mode", Bind::bound(data.value("open", 1u8)), &entries).show(ui).into()
}

#[story(Select<'static, u8>, covers = [], disabled_text = "Pick a layer first")]
fn select_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = modes();
    let tooltip = data.tooltip("Blend mode of the layer");
    Select::new("Blend mode", Bind::bound(data.value("mode", 0u8)), &entries).tooltip(tooltip).show(ui).into()
}
