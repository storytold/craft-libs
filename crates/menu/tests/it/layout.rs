//! The Mac layout, its shortcut clashes, and the keys that decide rebuild against update in place.

use craft_menu::{Builtin, Document, Item, ItemKind, LayoutOptions, MenuBar, MenuRole, Node, mac_layout, structure_key, window_key};
use craft_ui::testing::trial;

use crate::sample::{chord, cmd, ids, sample, titled};

#[trial]
fn the_app_menu_is_laid_out_the_mac_way() {
    let l = mac_layout(&sample(false, true), &LayoutOptions::new("Sample"));
    assert!(l.clashes.is_empty(), "{:?}", l.clashes);
    let app = &l.bar.menus[0];
    assert_eq!((app.title.as_str(), app.role), ("Sample", MenuRole::App));
    #[rustfmt::skip]
    let expected = [
        "help.about", "---", "[Settings]", "[Language]", "[Appearance]", "---",
        "<Services>", "---", Builtin::Hide.id(), Builtin::HideOthers.id(), "<ShowAll>", "---", "file.exit",
    ];
    assert_eq!(ids(&app.children), expected);
    let quit = l.bar.find(&cmd("file.exit")).unwrap();
    assert_eq!((quit.label.as_str(), quit.shortcut), ("Quit Sample", Some(chord("Cmd+Q"))), "the app's own Quit");
    assert_eq!(l.bar.find(&cmd("help.about")).unwrap().label, "About Sample");
    assert_eq!(l.bar.find(&Builtin::Hide.command()).unwrap().shortcut, Some(chord("Cmd+H")));
    assert_eq!(l.bar.find(&Builtin::HideOthers.command()).unwrap().shortcut, Some(chord("Cmd+Alt+H")));
    assert_eq!(l.bar.find(&cmd("prefs.general")).unwrap().shortcut, Some(chord("Cmd+,")), "⌘, opens the first settings page");
    let Some(Node::Submenu { children, .. }) = app.children.iter().find(|n| matches!(n, Node::Submenu { label, .. } if label == "Appearance")) else {
        panic!("an Appearance submenu");
    };
    assert_eq!(ids(children), ["theme.dark", "---", "theme.next"]);

    // The moved items leave their menus, and emptied separators go with them.
    let titles: Vec<&str> = l.bar.menus.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(titles, ["Sample", "File", "Edit", "View", "Window", "Help"]);
    assert_eq!(ids(&titled(&l.bar, "File").children), ["file.new", "---", "file.close", "---", "<CloseWindow>"]);
    assert_eq!(ids(&titled(&l.bar, "Edit").children), ["edit.undo", "edit.redo"]);
    assert_eq!(ids(&titled(&l.bar, "Help").children), ["help.docs"]);
    assert_eq!(
        ids(&l.bar.menu(MenuRole::Window).unwrap().children),
        ["<Minimize>", "<Zoom>", "---", "panel.layers", "---", "<BringAllToFront>", "---", "doc.0"],
        "the open documents follow Bring All to Front"
    );
}

/// App commands keep ⌘H, ⌘M, ⌘W and ⌘,: the system items give way, and each clash is reported.
#[trial]
fn clashes_are_resolved_in_the_apps_favour() {
    let l = mac_layout(&sample(true, true), &LayoutOptions::new("Sample"));
    let clash = |sc: &str| l.clashes.iter().find(|c| c.shortcut == chord(sc)).map(|c| c.command.id.as_str());
    assert_eq!(clash("Cmd+H"), Some("view.extras"));
    assert_eq!(clash("Cmd+M"), Some("view.curves"));
    assert_eq!(clash("Cmd+W"), Some("file.close"));
    assert_eq!(clash("Cmd+,"), Some("view.hideLayers"));
    assert_eq!(l.clashes.len(), 4);
    assert_eq!(l.bar.find(&Builtin::Hide.command()).unwrap().shortcut, Some(chord("Ctrl+Cmd+H")));
    assert_eq!(l.bar.find(&Builtin::Minimize.command()).unwrap().shortcut, Some(chord("Ctrl+Cmd+M")));
    assert_eq!(l.bar.find(&cmd("prefs.general")).unwrap().shortcut, None);
    assert_eq!(ids(&l.bar.menu(MenuRole::Window).unwrap().children)[..2], [Builtin::Minimize.id(), "<Zoom>"]);
    assert!(!ids(&titled(&l.bar, "File").children).contains(&"<CloseWindow>".to_string()), "⌘W stays File › Close");

    // With ⌃⌘H taken too, Hide goes without a shortcut.
    let mut bar = sample(true, true);
    bar.menus[0].children.push(Item::new("file.other", "Other").shortcut(chord("Ctrl+Cmd+H")).into());
    let l = mac_layout(&bar, &LayoutOptions::new("Sample"));
    assert_eq!(l.bar.find(&Builtin::Hide.command()).unwrap().shortcut, None);
    assert_eq!(l.clashes.iter().find(|c| c.shortcut == chord("Cmd+H")).map(|c| c.resolution), Some("Hide has no shortcut"));
}

/// Labels, enabled and checked are updated in place; a shortcut or a check item that used to be
/// plain needs a rebuild. A document opening or closing rebuilds only the Window menu.
#[trial]
fn keys_separate_rebuilds_from_updates_in_place() {
    let opts = LayoutOptions::new("Sample");
    let base = mac_layout(&sample(false, true), &opts).bar;
    let key = |bar: &MenuBar| (structure_key(bar), window_key(bar));
    let edit = |f: &dyn Fn(&mut Item)| {
        let mut bar = sample(false, true);
        for m in &mut bar.menus {
            for n in &mut m.children {
                if let Node::Item(it) = n
                    && it.command.id == "edit.undo"
                {
                    f(it);
                }
            }
        }
        mac_layout(&bar, &opts).bar
    };
    assert_eq!(key(&edit(&|it| it.label = "Undo Typing".into())), key(&base));
    assert_eq!(key(&edit(&|it| it.enabled = false)), key(&base));
    assert_ne!(structure_key(&edit(&|it| it.shortcut = Some(chord("Ctrl+Z")))), structure_key(&base));
    assert_ne!(structure_key(&edit(&|it| it.kind = ItemKind::Check(false))), structure_key(&base));

    let mut two = sample(false, true);
    two.documents.push(Document::new("doc.1", "Untitled-2"));
    let two = mac_layout(&two, &opts).bar;
    assert_eq!(structure_key(&two), structure_key(&base));
    assert_ne!(window_key(&two), window_key(&base));
    assert_eq!(ids(&two.menu(MenuRole::Window).unwrap().children).last().map(String::as_str), Some("doc.1"));
}
