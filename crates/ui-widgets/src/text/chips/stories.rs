//! Stories of `ChipField`.

use super::ChipField;
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use craft_theme::ControlSize;
use egui::Ui;

fn keywords() -> Vec<String> {
    ["kite", "red", "beach"].map(String::from).to_vec()
}

#[story(ChipField<'static>, covers = [Default], drive = Drive::Type("sky"))]
fn chip_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &[("Sm", ControlSize::Sm), ("Md", ControlSize::Md)]);
    let disabled = *data.value("disabled", false);
    ChipField::new("Keywords", Bind::bound(data.value("keywords", keywords())))
        .placeholder("Add a keyword")
        .size(size)
        .enabled(!disabled)
        .show(ui)
        .into()
}

#[story(ChipField<'static>, covers = [LongText], drive = Drive::Never)]
fn chip_field_long(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let many = (1..=14).map(|n| format!("A keyword number {n} that runs long")).collect();
    ChipField::new("Keywords", Bind::bound(data.value("long", many))).show(ui).into()
}

#[story(ChipField<'static>, covers = [Mixed], drive = Drive::Never)]
fn chip_field_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ChipField::new("Keywords", Bind::bound_mixed(data.value("mixed", Mixed::<Vec<String>>::Mixed))).show(ui).into()
}

#[story(ChipField<'static>, covers = [Invalid], drive = Drive::Never)]
fn chip_field_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_none = |chips: &Vec<String>| if chips.is_empty() { Validity::Refuse(Message::new("Add at least one keyword")) } else { Validity::Ok };
    ChipField::new("Keywords", Bind::bound(data.value("empty", Vec::new())).check(refuse_none)).show(ui).into()
}
