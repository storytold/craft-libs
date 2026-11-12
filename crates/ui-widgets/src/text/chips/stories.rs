//! Stories of `ChipField`.

use super::ChipField;
use crate::contract::Role;
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
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
    let clear_all = *data.value("clear all", true);
    let field = ChipField::new("Keywords", Bind::bound(data.value("keywords", keywords())))
        .placeholder("Add a keyword")
        .partial(|chip: &str| chip == "red")
        .size(size)
        .enabled(!disabled);
    if clear_all { field.clear_all() } else { field }.show(ui).into()
}

#[story(ChipField<'static>, covers = [LongText], drive = Drive::Never)]
fn chip_field_long(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let many = (1..=14).map(|n| format!("A keyword number {n} that runs long")).collect();
    let max_lines = *data.choice("max lines", Some(2usize), &[("2", Some(2usize)), ("All", None)]);
    let field = ChipField::new("Keywords", Bind::bound(data.value("long", many)));
    match max_lines {
        Some(lines) => field.max_lines(lines),
        None => field,
    }
    .show(ui)
    .into()
}

/// Opens the menu of the chips past the line limit.
fn open_more(fixture: &mut Fixture) {
    let tree = fixture.tree();
    let more = fixture
        .by_role(Role::BUTTON)
        .into_iter()
        .find(|node| crate::testing::access::name(tree, node.node).is_some_and(|name| name.starts_with('+')));
    if let Some(more) = more {
        fixture.click(&more);
    }
}

#[story(ChipField<'static>, covers = [Open], drive = Drive::Custom(open_more))]
fn chip_field_more(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let many = ["kite", "red", "beach", "sky", "sand", "dune", "wave", "tide", "shell", "gull"].map(String::from).to_vec();
    ChipField::new("Keywords", Bind::bound(data.value("more", many))).max_lines(1).clear_all().show(ui).into()
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
