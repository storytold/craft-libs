//! The in-window bar's own rules over the model.

use craft_host::target::Os;
use craft_menu::{Item, Menu, MenuBar, MenuBarUi, Node};
use craft_ui::contract::Tracked;
use craft_ui::testing::{Fixture, trial};

/// A submenu the app disables stays greyed and never opens, though its rows could be chosen
/// (Open Recent with a placeholder row while the app keeps no recent files).
#[trial]
fn a_disabled_submenu_is_greyed_and_does_not_open() {
    let bar = Tracked::new(MenuBar::new(vec![Menu::new(
        "File",
        vec![
            Node::submenu("Open Recent", vec![Item::new("file.recent.a", "notes.txt").into()]).enabled(false),
            Node::submenu("Export", vec![Item::new("file.export.pdf", "PDF").into()]),
        ],
    )]));
    let mut fixture = Fixture::new(move |ui| {
        MenuBarUi::new(&bar).show(ui);
    });
    // The in-window bar's rules; a Mac's menus are in the system bar.
    fixture.os(Os::Linux);
    fixture.settle();
    let Some(file) = fixture.by_label("File") else { panic!("the bar shows File") };
    fixture.click(&file).settle();
    let Some(recent) = fixture.by_label("Open Recent") else { panic!("File shows Open Recent") };
    let Some(export) = fixture.by_label("Export") else { panic!("File shows Export") };
    assert!(fixture.tree().is_disabled_key(recent.node), "the disabled submenu is greyed");
    assert!(!fixture.tree().is_disabled_key(export.node), "its enabled sibling is not");
    fixture.click(&recent).settle();
    assert!(fixture.by_label("notes.txt").is_none(), "a click does not open it");
    fixture.click(&export).settle();
    assert!(fixture.by_label("PDF").is_some(), "the enabled one opens");
}
