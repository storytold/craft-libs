//! Stories of the menu bar, the edit menu and the panel menu.

use super::{MenuBar, MenuHelp, MenuTitle};
use crate::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use crate::display::Divider;
use crate::menu::{EditMenu, GroupItem, HelpEntry, Menu, MenuItem, PanelMenu, PanelRow};
use crate::runtime::commands::EditCommands;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_theme::ControlSize;
use egui::Ui;
use std::sync::LazyLock;

fn file(ui: &mut Ui) {
    MenuItem::new("&New\u{2026}").shortcut("Ctrl+N").show(ui);
    MenuItem::new("&Open\u{2026}").shortcut("Ctrl+O").show(ui);
    Menu::submenu_row(ui, "Open &Recent", |ui, _| {
        MenuItem::new("poster.png").show(ui);
        MenuItem::new("logo.svg").show(ui);
    });
    Divider::horizontal().show(ui);
    MenuItem::new("&Save").shortcut("Ctrl+S").show(ui);
    MenuItem::new("&Close").shortcut("Ctrl+W").show(ui);
}

fn view(ui: &mut Ui) {
    MenuItem::new("&Rulers").check(true).show(ui);
    MenuItem::new("&Grid").check(false).show(ui);
    Divider::horizontal().show(ui);
    MenuItem::new("&Fit on Screen").shortcut("Ctrl+0").show(ui);
}

fn image(ui: &mut Ui) {
    Menu::submenu_row(ui, "&Adjustments", |ui, _| {
        MenuItem::new("Levels\u{2026}").show(ui);
        MenuItem::new("Curves\u{2026}").show(ui);
        MenuItem::new("Hue/Saturation\u{2026}").show(ui);
    });
    MenuItem::new("Image &Size\u{2026}").show(ui);
    MenuItem::new("&Canvas Size\u{2026}").show(ui);
}

fn window(ui: &mut Ui) {
    MenuItem::new("&Workspace").show(ui);
    MenuItem::new("&Layers").show(ui);
}

static HELP: LazyLock<Vec<HelpEntry>> = LazyLock::new(|| {
    vec![HelpEntry::new(EditCommands::Undo, "Undo").with_path(["Edit"]), HelpEntry::new(EditCommands::SelectAll, "Select All").with_path(["Edit"])]
});

fn titles() -> Vec<MenuTitle<'static>> {
    vec![
        MenuTitle::new("&File", file),
        MenuTitle::new("&Edit", |ui| {
            EditMenu.show(ui);
        }),
        MenuTitle::new("&View", view),
        MenuTitle::new("&Image", image),
        MenuTitle::new("&Window", window),
        MenuTitle::new("&Help", |ui| {
            MenuItem::new("&Keyboard Shortcuts").show(ui);
            MenuItem::new("&About").show(ui);
        }),
    ]
}

#[story(MenuBar<'static>, covers = [Default], drive = Drive::Never)]
fn menu_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &[("Sm", ControlSize::Sm), ("Md", ControlSize::Md)]);
    let search = *data.value("search", true);
    let bar = MenuBar::new("Main menu", titles()).size(size);
    let bar = if search { bar.help(MenuHelp::new(5, HELP.as_slice())) } else { bar };
    bar.show(ui).into()
}

fn open_file(fixture: &mut Fixture) {
    if let Some(file) = fixture.by_label("File") {
        fixture.click(&file);
    }
}

#[story(MenuBar<'static>, covers = [Open], drive = Drive::Custom(open_file))]
fn menu_bar_open(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    MenuBar::new("Main menu", titles()).show(ui).into()
}

#[story(MenuBar<'static>, covers = [], drive = Drive::Never)]
fn menu_bar_with_help_search(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    MenuBar::new("Main menu", titles()).help(MenuHelp::new(5, HELP.as_slice())).show(ui).into()
}

#[story(EditMenu, covers = [Default], drive = Drive::Never)]
fn edit_menu(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    StoryOutput::from(Menu::new("Edit").open(Bind::bound(data.value("open", true))).show(ui, |ui, _| EditMenu.show(ui)).output)
}

#[story(PanelMenu, covers = [Default], drive = Drive::Never)]
fn panel_menu(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let panel = || {
        let host = vec![PanelRow::Command(EditCommands::Undo.into()), PanelRow::Separator];
        let group = vec![
            PanelRow::Item(Entry::new(GroupItem::CollapseToIcons, "Collapse to Icons")),
            PanelRow::Item(Entry::new(GroupItem::AutoCollapse, "Auto-Collapse Iconic Panels")),
        ];
        PanelMenu::new(host, group)
    };
    StoryOutput::from(Menu::new("Panel menu").open(Bind::bound(data.value("open", true))).show(ui, |ui, _| panel().show(ui)).output)
}
