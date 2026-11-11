//! Stories of the buttons.

use super::{Button, IconButton, Link, SplitButton, ToggleButton};
use crate::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Validity};
use crate::display::{Shade, Text};
use crate::layout_components::{Grid, Row};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{ColumnWidth, IconId, Mixed, Proposal};
use craft_theme::{ControlSize, Variant};
use egui::{Key, Modifiers, Ui};

const SIZES: [(&str, ControlSize); 2] = [("Sm", ControlSize::Sm), ("Md", ControlSize::Md)];

/// Every variant at each size (rows, each under its name) at rest, disabled and loading (columns). Where the full grid
/// is wider than the room, the loading column is left out.
#[story(Button<'static>, covers = [], drive = Drive::Never, group = "Variants", title = "Variants")]
fn button_variants(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    const ALL: &[ColumnWidth] = &[ColumnWidth::Content; 4];
    const STATES: [(&str, bool, bool); 3] = [("rest", true, false), ("disabled", false, false), ("loading", true, true)];
    let variants = [("Primary", Variant::PRIMARY), ("Secondary", Variant::SECONDARY), ("Ghost", Variant::GHOST), ("Danger", Variant::DANGER)];
    let label = data.value("label", String::from("Button")).clone();
    let rows = |ui: &mut Ui, states: &[(&str, bool, bool)]| {
        for (name, variant) in variants {
            for (size_name, size) in SIZES {
                Text::new(format!("{name} {size_name}").to_lowercase()).shade(Shade::Muted).show(ui);
                for &(state, enabled, loading) in states {
                    Button::new(label.clone())
                        .variant(variant)
                        .size(size)
                        .enabled(enabled)
                        .loading(loading)
                        .id_salt((name, size_name, state))
                        .show(ui);
                }
            }
        }
    };
    let full = Grid::new(ALL).measure_in(ui, Proposal::Ideal, |ui, _| rows(ui, &STATES));
    let (columns, states) = if full.x <= ui.available_width() { (ALL, &STATES[..]) } else { (&ALL[..3], &STATES[..2]) };
    Grid::new(columns).show(ui, |ui, _| rows(ui, states)).into()
}

#[story(Button<'static>, covers = [Default], group = "Playground", title = "Playground")]
fn button(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let variants = [("Primary", Variant::PRIMARY), ("Secondary", Variant::SECONDARY), ("Ghost", Variant::GHOST), ("Danger", Variant::DANGER)];
    let variant = *data.choice("variant", Variant::PRIMARY, &variants);
    let size = *data.choice("size", ControlSize::Md, &SIZES);
    let icon = *data.choice("icon", Some("save"), &[("None", None), ("Save", Some("save")), ("chevron_right", Some("chevron_right"))]);
    let disabled = *data.value("disabled", false);
    let loading = *data.value("loading", false);
    let label = data.value("label", String::from("Save the document as a copy")).clone();
    let mut button = Button::new(label).variant(variant).size(size).enabled(!disabled).loading(loading);
    if let Some(icon) = icon {
        button = button.icon(IconId::new(icon));
    }
    let shown = button.show(ui);
    shown.into()
}

#[story(Button<'static>, covers = [], disabled_text = "Nothing to save", group = "Playground", title = "With reason")]
fn button_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Button::new("Save").tooltip(data.tooltip("Save the document")).show(ui).into()
}

#[story(Button<'static>, covers = [], drive = Drive::Never, group = "Playground", title = "Loading")]
fn button_loading(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Button::new("Saving").icon(IconId::new("save")).loading(true).show(ui).into()
}

#[story(IconButton<'static>, covers = [Default])]
fn icon_button(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    IconButton::new(IconId::new("settings")).label("Settings").show(ui).into()
}

/// A tool strip: the first tool selected, as the gallery card shows it.
#[story(IconButton<'static>, covers = [], drive = Drive::Never)]
fn icon_button_selected(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let shown = Row::new().show(ui, |ui, _| {
        for (i, (icon, label)) in
            [("undo", "Undo"), ("redo", "Redo"), ("copy", "Copy"), ("trash", "Delete"), ("settings", "Settings")].into_iter().enumerate()
        {
            IconButton::new(IconId::new(icon)).label(label).selected(i == 0).id_salt(icon).show(ui);
        }
    });
    shown.into()
}

/// One toggle off and one on with an icon, as the gallery card shows them.
#[story(ToggleButton<'static>, covers = [], drive = Drive::Never)]
fn toggle_button_on(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = Row::new().show(ui, |ui, _| {
        ToggleButton::new("Snap to grid", Bind::bound(data.value("snap", false))).id_salt("snap").show(ui);
        ToggleButton::new("Guides", Bind::bound(data.value("on", true))).icon(IconId::new("eye")).id_salt("guides").show(ui);
    });
    shown.into()
}

#[story(ToggleButton<'static>, covers = [Default])]
fn toggle_button(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &SIZES);
    let disabled = *data.value("disabled", false);
    let label = data.value("label", String::from("Snap to grid")).clone();
    ToggleButton::new(label, Bind::bound(data.value("on", false))).icon(IconId::new("eye")).size(size).enabled(!disabled).show(ui).into()
}

#[story(ToggleButton<'static>, covers = [Mixed], drive = Drive::Never)]
fn toggle_button_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ToggleButton::new("Snap to grid", Bind::bound_mixed(data.value("mixed", Mixed::Mixed))).show(ui).into()
}

#[story(ToggleButton<'static>, covers = [Invalid], drive = Drive::Never)]
fn toggle_button_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_on = |on: &bool| if *on { Validity::Refuse(Message::new("Snapping is not available here")) } else { Validity::Ok };
    ToggleButton::new("Snap to grid", Bind::bound(data.value("invalid", true)).check(refuse_on)).show(ui).into()
}

fn presets() -> [Entry<u8>; 4] {
    [
        Entry::new(0, "Black and white").icon(IconId::new("eye")).checked(true),
        Entry::new(1, "Colour"),
        Entry::separator(),
        Entry::new(2, "Grayscale").enabled(false),
    ]
}

#[story(SplitButton<'static, u8>, covers = [Default], drive = Drive::Activate)]
fn split_button(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let entries = presets();
    SplitButton::new("Trace", &entries).icon(IconId::new("image")).arrow_label("Tracing presets").show(ui).into()
}

fn open_menu(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
}

#[story(SplitButton<'static, u8>, covers = [Open], drive = Drive::Custom(open_menu))]
fn split_button_open(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let entries = presets();
    SplitButton::new("Trace", &entries).icon(IconId::new("image")).arrow_label("Tracing presets").show(ui).into()
}

#[story(Link, covers = [Default])]
fn link(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Link::new("Show all your shortcuts").url("https://example.com/shortcuts").show(ui).into()
}
