//! Stories of `Segmented`.

use super::Segmented;
use crate::contract::{Bind, Entry, Props as _, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{IconId, Mixed};
use craft_theme::ControlSize;
use egui::Ui;

/// The combine modes of the text stories, of which a story shows the first three or all five.
const MODES: [&str; 5] = ["New", "Add", "Subtract", "Intersect", "Exclude"];

/// The tools of the icon stories: icon and name.
const TOOLS: [(&str, &str); 5] = [("move", "Move"), ("hand", "Hand"), ("brush", "Brush"), ("eraser", "Eraser"), ("eyedropper", "Eyedropper")];

fn sizes() -> [Entry<u8>; 3] {
    [Entry::new(1, "Small"), Entry::new(2, "Medium"), Entry::new(3, "Large")]
}

fn modes(count: u8) -> Vec<Entry<u8>> {
    (0..count).zip(MODES).map(|(at, mode)| Entry::new(at, mode)).collect()
}

fn tools(count: u8) -> Vec<Entry<u8>> {
    (0..count).zip(TOOLS).map(|(at, (icon, name))| Entry::new(at, name).icon(IconId::new(icon))).collect()
}

/// The text row of the first `count` modes at `size`, starting on mode `first`, over its "Selected" readout.
fn text_row(ui: &mut Ui, data: &mut StoryData, count: u8, first: u8, size: ControlSize) -> StoryOutput {
    let entries = modes(count);
    let picked = data.value("mode", first);
    let shown: StoryOutput = Segmented::new(format!("Combine ({count})"), Bind::bound(&mut *picked), &entries).size(size).show(ui).into();
    shown.readout(format!("Selected: {}", MODES[usize::from(*picked)]))
}

/// The icon row of the first `count` tools, starting on tool `first`, over its "Selected" readout.
fn icon_row(ui: &mut Ui, data: &mut StoryData, count: u8, first: u8) -> StoryOutput {
    let entries = tools(count);
    let picked = data.value("tool", first);
    let shown: StoryOutput = Segmented::new(format!("Tool ({count})"), Bind::bound(&mut *picked), &entries).show(ui).into();
    shown.readout(format!("Selected: {}", TOOLS[usize::from(*picked)].1))
}

fn pick_add(fixture: &mut Fixture) {
    if let Some(add) = fixture.by_label("Add") {
        fixture.click(&add);
    }
}

#[story(Segmented<'static, u8>, covers = [Default], drive = Drive::Custom(pick_add), group = "Text", title = "Three segments")]
fn segmented(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    text_row(ui, data, 3, 0, ControlSize::Md)
}

#[story(Segmented<'static, u8>, covers = [], drive = Drive::Custom(pick_add), group = "Text", title = "Five segments")]
fn segmented_five(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    text_row(ui, data, 5, 2, ControlSize::Md)
}

#[story(Segmented<'static, u8>, covers = [], drive = Drive::Custom(pick_add), group = "Text", title = "Small")]
fn segmented_small(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    text_row(ui, data, 3, 0, ControlSize::Sm)
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

#[story(Segmented<'static, u8>, covers = [], drive = Drive::Never, group = "Icons", title = "Three icons")]
fn segmented_icons(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    icon_row(ui, data, 3, 0)
}

#[story(Segmented<'static, u8>, covers = [], drive = Drive::Never, group = "Icons", title = "Five icons")]
fn segmented_icons_five(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    icon_row(ui, data, 5, 2)
}
