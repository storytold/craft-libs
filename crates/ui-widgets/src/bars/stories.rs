//! Stories of the bars.

use super::{Bar, BarCell, BarItem, BarItems, BarKind, ItemDisplay};
use crate::choice::Segmented;
use crate::contract::{Bind, Entry};
use crate::contract::{Show as _, ShowContainer as _};
use crate::display::Text;
use crate::pickers::Select;
use crate::runtime::commands::EditCommands;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Key, Modifiers, Ui};

/// A width that holds the first items of `items` and not the rest.
const NARROW: f32 = 80.0;

fn items() -> Vec<BarItem> {
    vec![
        BarItem::new(EditCommands::Undo),
        BarItem::new(EditCommands::Redo),
        BarItem::new(EditCommands::Cut).section(1),
        BarItem::new(EditCommands::Copy).section(1),
        BarItem::new(EditCommands::Paste).section(1),
    ]
}

#[story(Bar<'static>, covers = [Default], drive = Drive::Never)]
fn bar(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Bar::new("Tools", BarKind::Tool)
        .show(ui, |ui, _| {
            BarItems::new(items()).show(ui);
        })
        .into()
}

#[story(Bar<'static>, covers = [], drive = Drive::Never)]
fn bar_options(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Bar::new("Brush options", BarKind::Options)
        .commit_cancel("Commit transform", "Cancel transform")
        .show(ui, |ui, _| {
            Text::new("Brush size").show(ui);
        })
        .into()
}

#[story(Bar<'static>, covers = [], drive = Drive::Never)]
fn bar_captioned(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let themes = [Entry::new(0u8, "ArtCraft"), Entry::new(1, "Paper")];
    let modes = [Entry::new(0u8, "Light"), Entry::new(1, "Dark"), Entry::new(2, "System")];
    Bar::new("Appearance", BarKind::Options)
        .trailing(|ui| {
            BarCell::new("Workspace").control(ui, |ui, label| {
                Segmented::new(label, Bind::given(0u8), &modes).show(ui);
            });
        })
        .show(ui, |ui, _| {
            BarCell::new("Theme").control(ui, |ui, label| {
                Select::new(label, Bind::given(0u8), &themes).show(ui);
            });
            BarCell::new("Mode").control(ui, |ui, label| {
                Segmented::new(label, Bind::given(1u8), &modes).show(ui);
            });
        })
        .into()
}

#[story(Bar<'static>, covers = [], drive = Drive::Never)]
fn bar_status(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Bar::new("Status", BarKind::Status)
        .show(ui, |ui, _| {
            Text::new("Ready").show(ui);
        })
        .into()
}

#[story(BarItems, covers = [Default], drive = Drive::Never)]
fn bar_items(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let labelled = items().into_iter().map(|item| item.display(ItemDisplay::IconText)).take(2);
    BarItems::new(labelled.chain(items().into_iter().skip(2)).collect()).show(ui).into()
}

fn open_overflow(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::End, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
}

#[story(BarItems, covers = [Open], drive = Drive::Custom(open_overflow))]
fn bar_items_overflow(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    crate::testing::memory::scoped(ui, |ui| {
        ui.set_max_width(NARROW);
        BarItems::new(items()).show(ui)
    })
    .into()
}
