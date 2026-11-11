//! Stories of `Segmented`.

use super::Segmented;
use crate::contract::{Bind, Entry, Props as _, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{IconId, Mixed};
use egui::Ui;

fn sizes() -> [Entry<u8>; 3] {
    [Entry::new(1, "Small"), Entry::new(2, "Medium"), Entry::new(3, "Large")]
}

fn pick_medium(fixture: &mut Fixture) {
    if let Some(medium) = fixture.by_label("Medium") {
        fixture.click(&medium);
    }
}

#[story(Segmented<'static, u8>, covers = [Default], drive = Drive::Custom(pick_medium))]
fn segmented(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Segmented::new("Size", Bind::bound(data.value("size", 1u8)), &sizes()).show(ui).into()
}

#[story(Segmented<'static, u8>, covers = [Mixed], drive = Drive::Never)]
fn segmented_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Segmented::new("Size", Bind::bound_mixed(data.value("mixed", Mixed::Mixed)), &sizes()).show(ui).into()
}

#[story(Segmented<'static, u8>, covers = [Invalid], drive = Drive::Never)]
fn segmented_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_large = |size: &u8| if *size == 3 { Validity::Refuse(Message::new("Too large")) } else { Validity::Ok };
    Segmented::new("Size", Bind::bound(data.value("large", 3u8)).check(refuse_large), &sizes()).show(ui).into()
}

#[story(Segmented<'static, u8>, covers = [], disabled_text = "Pick a layer first")]
fn segmented_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tooltip = data.tooltip("The size of the brush");
    Segmented::new("Size", Bind::bound(data.value("size", 1u8)), &sizes()).tooltip(tooltip).show(ui).into()
}

#[story(Segmented<'static, u8>, covers = [], drive = Drive::Never)]
fn segmented_icons(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let views =
        [Entry::new(1u8, "List").icon(IconId::new("list")), Entry::new(2, "Grid").icon(IconId::new("grid")), Entry::new(3, "Detail").enabled(false)];
    Segmented::new("View", Bind::bound(data.value("view", 1u8)), &views).show(ui).into()
}
