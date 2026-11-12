//! Stories of `Checkbox`.

use super::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use craft_theme::ControlSize;
use egui::Ui;

/// A label too long for one line of the card: it wraps, the box on its first line.
const LONG_LABEL: &str = "Show the transform controls on the selected layer whenever the move tool is active";

#[story(Checkbox<'static>, covers = [Default], group = "States", title = "Unchecked")]
fn checkbox(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &[("Sm", ControlSize::Sm), ("Md", ControlSize::Md)]);
    let label = data.value("label", String::from("Snap to grid")).clone();
    Checkbox::new(Bind::bound(data.value("checked", false))).label(label).size(size).show(ui).into()
}

#[story(Checkbox<'static>, covers = [], group = "States", title = "Checked")]
fn checkbox_checked(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Checkbox::new(Bind::bound(data.value("checked", true))).label("Snap to grid").show(ui).into()
}

#[story(Checkbox<'static>, covers = [Mixed], drive = Drive::Never, group = "States", title = "Mixed")]
fn checkbox_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Checkbox::new(Bind::bound_mixed(data.value("mixed", Mixed::Mixed))).label("Snap to grid").show(ui).into()
}

#[story(Checkbox<'static>, covers = [Invalid], drive = Drive::Never)]
fn checkbox_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_on = |on: &bool| if *on { Validity::Refuse(Message::new("This cannot be on")) } else { Validity::Ok };
    Checkbox::new(Bind::bound(data.value("invalid", true)).check(refuse_on)).label("Lock aspect").show(ui).into()
}

#[story(Checkbox<'static>, covers = [], disabled_text = "Turn on the grid first")]
fn checkbox_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tooltip = data.tooltip("Snap to the grid");
    Checkbox::new(Bind::bound(data.value("on", false))).label("Snap").tooltip(tooltip).show(ui).into()
}

#[story(Checkbox<'static>, covers = [], drive = Drive::Never, group = "States", title = "Disabled unchecked")]
fn checkbox_disabled_unchecked(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Checkbox::new(Bind::bound(data.value("off", false))).label("Disabled (unchecked)").enabled(false).show(ui).into()
}

#[story(Checkbox<'static>, covers = [], drive = Drive::Never, group = "States", title = "Disabled checked")]
fn checkbox_disabled_checked(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Checkbox::new(Bind::bound(data.value("on", true))).label("Disabled (checked)").enabled(false).show(ui).into()
}

#[story(Checkbox<'static>, covers = [], drive = Drive::Never, group = "States", title = "Disabled mixed")]
fn checkbox_disabled_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Checkbox::new(Bind::bound_mixed(data.value("mixed", Mixed::Mixed))).label("Disabled (mixed)").enabled(false).show(ui).into()
}

#[story(Checkbox<'static>, covers = [], group = "States", title = "Long label")]
fn checkbox_long_label(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Checkbox::new(Bind::bound(data.value("checked", true))).label(LONG_LABEL).show(ui).into()
}
