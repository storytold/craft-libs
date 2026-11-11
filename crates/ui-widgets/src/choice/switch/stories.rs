//! Stories of `Switch`.

use super::Switch;
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use egui::Ui;

#[story(Switch<'static>, covers = [Default])]
fn switch(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Switch::new(Bind::bound(data.value("on", false))).label("Show rulers").show(ui).into()
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

#[story(Switch<'static>, covers = [], drive = Drive::Never)]
fn switch_disabled_off(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Switch::new(Bind::bound(data.value("off", false))).label("Disabled (off)").enabled(false).show(ui).into()
}

#[story(Switch<'static>, covers = [], drive = Drive::Never)]
fn switch_disabled_on(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Switch::new(Bind::bound(data.value("on", true))).label("Disabled (on)").enabled(false).show(ui).into()
}
