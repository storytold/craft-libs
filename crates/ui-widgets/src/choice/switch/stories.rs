//! Stories of `Switch`.

use super::Switch;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _, Validity};
use crate::display::Divider;
use crate::layout_components::Stack;
use crate::runtime::env::Message;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use egui::Ui;

/// A label too long for one line of the card: it wraps, the track on its first line.
const LONG_LABEL: &str = "Show the transform controls on the selected layer whenever the move tool is active";

/// The settings list's rows: value name, label and starting state. The last row is disabled.
const SETTINGS: [(&str, &str, bool); 4] = [
    ("autosave", "Autosave every five minutes", true),
    ("rulers", "Show rulers", false),
    ("snap", "Snap to the pixel grid", true),
    ("gpu", "Use the graphics card", true),
];

#[story(Switch<'static>, covers = [Default], group = "States", title = "Off")]
fn switch(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Switch::new(Bind::bound(data.value("on", false))).label("Show rulers").show(ui).into()
}

#[story(Switch<'static>, covers = [], group = "States", title = "On")]
fn switch_on(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Switch::new(Bind::bound(data.value("on", true))).label("Show rulers").show(ui).into()
}

#[story(Switch<'static>, covers = [], group = "States", title = "Long label")]
fn switch_long_label(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Switch::new(Bind::bound(data.value("on", true))).label(LONG_LABEL).show(ui).into()
}

#[story(Switch<'static>, covers = [Mixed], drive = Drive::Never)]
fn switch_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Switch::new(Bind::bound_mixed(data.value("mixed", Mixed::Mixed))).label("Show rulers").show(ui).into()
}

#[story(Switch<'static>, covers = [Invalid], drive = Drive::Never)]
fn switch_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_on = |on: &bool| if *on { Validity::Refuse(Message::new("This cannot be on")) } else { Validity::Ok };
    Switch::new(Bind::bound(data.value("invalid", true)).check(refuse_on)).label("Lock aspect").show(ui).into()
}

#[story(Switch<'static>, covers = [], disabled_text = "Turn on the grid first")]
fn switch_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tooltip = data.tooltip("Snap to the grid");
    Switch::new(Bind::bound(data.value("on", false))).label("Snap").tooltip(tooltip).show(ui).into()
}

#[story(Switch<'static>, covers = [], drive = Drive::Never, group = "States", title = "Disabled off")]
fn switch_disabled_off(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Switch::new(Bind::bound(data.value("off", false))).label("Disabled (off)").enabled(false).show(ui).into()
}

#[story(Switch<'static>, covers = [], drive = Drive::Never, group = "States", title = "Disabled on")]
fn switch_disabled_on(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Switch::new(Bind::bound(data.value("on", true))).label("Disabled (on)").enabled(false).show(ui).into()
}

/// A settings list: one switch per row between dividers, the last one disabled.
#[story(Switch<'static>, covers = [], drive = Drive::Never, group = "Settings", title = "Settings")]
fn switch_settings(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Stack::new()
        .show(ui, |ui, _| {
            for (at, (name, label, on)) in SETTINGS.into_iter().enumerate() {
                if at > 0 {
                    Divider::horizontal().show(ui);
                }
                Switch::new(Bind::bound(data.value(name, on))).label(label).enabled(at + 1 < SETTINGS.len()).show(ui);
            }
        })
        .into()
}
