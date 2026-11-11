//! Stories of the in-window bar over a small model.

use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

use crate::{Document, Item, LayoutOptions, Menu, MenuBar, MenuBarUi, MenuRole, Node, Target, layout};

fn model() -> MenuBar {
    let item = |id: &str, label: &str| Item::new(id, label);
    let bar = MenuBar {
        menus: vec![
            Menu::new(
                "File",
                vec![
                    item("file.new", "New").into(),
                    Node::submenu("Open Recent", vec![item("file.recent.a", "notes.txt").into()]),
                    Node::Separator,
                    item("file.revert", "Revert").enabled(false).into(),
                ],
            )
            .role(MenuRole::File),
            Menu::new(
                "View",
                vec![
                    item("view.rulers", "Rulers").check(true).into(),
                    Node::Separator,
                    item("view.light", "Light").radio(true).into(),
                    item("view.dark", "Dark").radio(false).into(),
                ],
            )
            .role(MenuRole::View),
            Menu::new("Window", Vec::new()).role(MenuRole::Window),
            Menu::new("Help", vec![item("help.about", "About").into()]).role(MenuRole::Help),
        ],
        documents: vec![Document::new("doc.0", "One"), Document::new("doc.1", "Two").active(true)],
    };
    layout(&bar, Target::Linux, &LayoutOptions::new("Sample")).bar
}

#[story(MenuBarUi<'static>, covers = [Default], drive = Drive::Never)]
fn menu_bar_ui(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    MenuBarUi::new(&model()).target(Target::Linux).show(ui);
    StoryOutput::default()
}

fn open_window(fixture: &mut Fixture) {
    if let Some(window) = fixture.by_label("Window") {
        fixture.click(&window);
    }
}

#[story(MenuBarUi<'static>, covers = [Open], drive = Drive::Custom(open_window))]
fn menu_bar_ui_with_documents(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    MenuBarUi::new(&model()).target(Target::Linux).show(ui);
    StoryOutput::default()
}

#[story(MenuBarUi<'static>, covers = [], drive = Drive::Never)]
fn menu_bar_ui_with_help_search(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    MenuBarUi::new(&model()).target(Target::Linux).help_search(true).show(ui);
    StoryOutput::default()
}
