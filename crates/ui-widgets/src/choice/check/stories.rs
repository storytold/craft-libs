//! Stories of `Checkbox`.

use super::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use craft_theme::ControlSize;
use egui::Ui;

#[story(Checkbox<'static>, covers = [Default])]
fn checkbox(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &[("Sm", ControlSize::Sm), ("Md", ControlSize::Md)]);
    let label = data.value("label", String::from("Show grid")).clone();
    Checkbox::new(Bind::bound(data.value("checked", false))).label(label).size(size).show(ui).into()
}

#[story(Checkbox<'static>, covers = [Mixed], drive = Drive::Never)]
fn checkbox_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Checkbox::new(Bind::bound_mixed(data.value("mixed", Mixed::Mixed))).label("Show grid").show(ui).into()
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

#[story(Checkbox<'static>, covers = [], drive = Drive::Never)]
fn checkbox_disabled_unchecked(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Checkbox::new(Bind::bound(data.value("off", false))).label("Disabled (unchecked)").enabled(false).show(ui).into()
}

#[story(Checkbox<'static>, covers = [], drive = Drive::Never)]
fn checkbox_disabled_checked(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Checkbox::new(Bind::bound(data.value("on", true))).label("Disabled (checked)").enabled(false).show(ui).into()
}
