//! Stories of the bars.

use super::{Bar, BarHome, BarItem, BarItems, BarKind, BarTool, ItemDisplay};
use crate::choice::{Checkbox, Segmented};
use crate::contract::{Bind, Entry, Props as _, Sizing};
use crate::contract::{Show as _, ShowContainer as _};
use crate::display::{Divider, Shade, Text};
use crate::form::Field;
use crate::menu::{Menu, MenuItem};
use crate::pickers::Select;
use crate::runtime::commands::EditCommands;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use crate::text::NumberField;
use craft_behavior::IconId;
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

/// The brush tool's bar: Home and the tool, the brush cells, a rule, the sampling group and its hint, then commit
/// and cancel.
#[story(Bar<'static>, covers = [], drive = Drive::Never)]
fn bar_options(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let modes = [Entry::new(0u8, "Normal"), Entry::new(1, "Multiply"), Entry::new(2, "Screen")];
    let mut home = *data.value("home", false);
    let (mut size, mut opacity) = (*data.value("size", Some(30.0)), *data.value("opacity", Some(100.0)));
    let (mut mode, mut sample_all) = (*data.value("mode", 0u8), *data.value("sample all", false));
    let shown = Bar::new("Brush options", BarKind::Options).commit_cancel("Commit transform", "Cancel transform").show(ui, |ui, _| {
        BarHome::new("Home", Bind::bound(&mut home)).show(ui);
        BarTool::new(IconId::new("brush"), "Brush").show(ui);
        Field::new("Size").sizing(Sizing::Content).show(ui, |ui, _| NumberField::new("Size", Bind::bound(&mut size)).range(1.0..=5000.0).show(ui));
        Field::new("Opacity")
            .sizing(Sizing::Content)
            .show(ui, |ui, _| NumberField::new("Opacity", Bind::bound(&mut opacity)).range(1.0..=100.0).show(ui));
        Field::new("Mode").sizing(Sizing::Content).show(ui, |ui, _| Select::new("Mode", Bind::bound(&mut mode), &modes).show(ui));
        Divider::vertical().show(ui);
        Checkbox::new(Bind::bound(&mut sample_all)).label("Sample All Layers").show(ui);
        Text::new("Samples the visible image").shade(Shade::Faint).show(ui);
    });
    *data.value("home", false) = home;
    *data.value("size", Some(30.0)) = size;
    *data.value("opacity", Some(100.0)) = opacity;
    *data.value("mode", 0u8) = mode;
    *data.value("sample all", false) = sample_all;
    shown.into()
}

#[story(Bar<'static>, covers = [], drive = Drive::Never)]
fn bar_captioned(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let themes = [Entry::new(0u8, "ArtCraft"), Entry::new(1, "Paper")];
    let modes = [Entry::new(0u8, "Light"), Entry::new(1, "Dark"), Entry::new(2, "System")];
    Bar::new("Appearance", BarKind::Options)
        .trailing(|ui| {
            Field::new("Workspace").sizing(Sizing::Content).show(ui, |ui, _| Segmented::new("Workspace", Bind::given(0u8), &modes).show(ui));
        })
        .show(ui, |ui, _| {
            Field::new("Theme").sizing(Sizing::Content).show(ui, |ui, _| Select::new("Theme", Bind::given(0u8), &themes).show(ui));
            Field::new("Mode").sizing(Sizing::Content).show(ui, |ui, _| Segmented::new("Mode", Bind::given(1u8), &modes).show(ui));
        })
        .into()
}

#[story(Bar<'static>, covers = [], drive = Drive::Never)]
fn bar_status(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Bar::new("Status", BarKind::Status)
        .show(ui, |ui, _| {
            Text::new("Doc: 2.4M/8.1M").shade(Shade::Muted).show(ui);
            Divider::vertical().show(ui);
            Text::new("1920 x 1080 px").shade(Shade::Muted).show(ui);
            Divider::vertical().show(ui);
            Text::new("Ready").shade(Shade::Faint).show(ui);
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

#[story(BarHome<'static>, covers = [], drive = Drive::Never)]
fn bar_home(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut home = *data.value("home", false);
    let shown = Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
        BarHome::new("Home", Bind::bound(&mut home)).show(ui);
        Text::new("Tool options").show(ui);
    });
    *data.value("home", false) = home;
    shown.into()
}

/// The tool icon with its context menu: Reset Tool and Reset All Tools.
#[story(BarTool, covers = [], drive = Drive::Never)]
fn bar_tool(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Bar::new("Options", BarKind::Options)
        .show(ui, |ui, _| {
            let tool = BarTool::new(IconId::new("brush"), "Brush").show(ui);
            Menu::context("Brush", &tool.response).show(ui, |ui, _| {
                MenuItem::new("Reset Tool").show(ui);
                MenuItem::new("Reset All Tools").show(ui);
            });
            Text::new("Tool options").show(ui);
        })
        .into()
}
