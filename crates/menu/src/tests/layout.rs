//! The Windows, Linux and web layouts, platform filtering, and the keymap adapter.

use craft_keymap::{Bindable, Binding, Keymap, Platform, Policy, Scope, Scopes};

use super::*;

fn menu(bar: &MenuBar, role: MenuRole) -> &Menu {
    bar.menu(role).unwrap()
}

fn has(bar: &MenuBar, id: &str) -> bool {
    bar.find(&cmd(id)).is_some()
}

#[test]
fn windows_puts_exit_last_in_file_and_documents_in_window() {
    let l = layout(&sample(false, true), Target::Windows, &LayoutOptions::new("Sample"));
    assert!(l.clashes.is_empty());
    assert_eq!(ids(&menu(&l.bar, MenuRole::File).children), ["file.new", "---", "file.close", "---", "file.exit"]);
    assert_eq!(l.bar.find(&cmd("file.exit")).unwrap().label, "Exit");
    assert_eq!(ids(&menu(&l.bar, MenuRole::Edit).children), ["edit.undo", "edit.redo", "---", "[Language]", "---", "[Preferences]"]);
    assert_eq!(ids(&menu(&l.bar, MenuRole::Help).children), ["help.docs", "---", "help.about"]);
    assert_eq!(l.bar.find(&cmd("help.about")).unwrap().label, "About Sample");
    assert_eq!(ids(&menu(&l.bar, MenuRole::Window).children), ["[Theme]", "theme.next", "panel.layers", "---", "doc.0"]);
    assert_eq!(l.bar.find(&cmd("doc.0")).unwrap().kind, ItemKind::Radio(true), "the document in front is selected");
    assert!(!has(&l.bar, Builtin::Hide.id()) && !has(&l.bar, Builtin::Minimize.id()));
    assert_eq!(l.bar.menus.len(), 5);

    let mac = mac_layout(&sample(false, true), &LayoutOptions::new("Sample")).bar;
    assert_eq!(mac.find(&cmd("doc.0")).unwrap().kind, ItemKind::Radio(true));
}

/// Without the menus the moved items need, the layout adds them, with translated titles.
#[test]
fn linux_adds_quit_and_the_menus_it_needs() {
    let view = vec![
        Item::new("help.about", "About").role(ItemRole::About).into(),
        Item::new("app.settings", "Settings").role(ItemRole::Settings).into(),
        Item::new("view.zoom", "Zoom").into(),
    ];
    let mut bar = MenuBar { menus: vec![Menu::new("View", view).role(MenuRole::View)], documents: vec![Document::new("doc.0", "Untitled")] };
    bar.documents.push(Document { hidden: true, ..Document::new("doc.1", "Background") });
    let label = |s: &'static str| format!("«{s}»");
    let l = layout(&bar, Target::Linux, &LayoutOptions { label: &label, ..LayoutOptions::new("Sample") });
    let titles: Vec<&str> = l.bar.menus.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(titles, ["«File»", "«Edit»", "View", "«Window»", "«Help»"]);
    let quit = &menu(&l.bar, MenuRole::File).children;
    assert_eq!(ids(quit), [Builtin::Quit.id()]);
    assert_eq!(l.bar.find(&Builtin::Quit.command()).unwrap().label, "«Quit»");
    assert_eq!(l.bar.find(&cmd("app.settings")).unwrap().label, "«Preferences…»");
    assert_eq!(l.bar.find(&cmd("help.about")).unwrap().label, "«About Sample»");
    assert_eq!(ids(&menu(&l.bar, MenuRole::Window).children), ["doc.0"], "a hidden document isn't listed");
}

#[test]
fn the_web_has_no_quit_and_gives_the_browser_its_shortcuts() {
    let mut bar = sample(false, true);
    let file = bar.menus.iter_mut().find(|m| m.role == MenuRole::File).unwrap();
    file.children.push(Item::new("file.reopen", "Reopen").shortcut(chord("Cmd+Shift+T")).into());
    file.children.push(Item::new("file.tab", "Tab").shortcut(chord("Cmd+Alt+T")).into());
    let l = layout(&bar, Target::Web, &LayoutOptions::new("Sample"));
    assert!(!has(&l.bar, "file.exit") && !has(&l.bar, Builtin::Quit.id()));
    let clashes: Vec<(String, &str)> = l.clashes.iter().map(|c| (c.command.key(), c.system)).collect();
    assert_eq!(clashes, [("file.new".into(), "New Window"), ("file.reopen".into(), "Reopen Closed Tab")]);
    assert_eq!(l.bar.find(&cmd("file.new")).unwrap().shortcut, None);
    assert_eq!(l.bar.find(&cmd("file.tab")).unwrap().shortcut, Some(chord("Cmd+Alt+T")));
    assert_eq!(l.bar.find(&cmd("edit.undo")).unwrap().shortcut, Some(chord("Cmd+Z")));
}

/// Items and submenus show only on their platforms; a submenu left empty goes, one holding only
/// the app's disabled placeholder stays.
#[test]
fn layouts_keep_only_what_the_target_shows() {
    let file = vec![
        Item::new("file.new", "New").into(),
        Item::new("file.tabs", "Show Tab Bar").platforms(Platforms::MAC).into(),
        Node::submenu("Export", vec![Item::new("file.export_native", "Native").platforms(Platforms::MAC).into()]),
        Node::submenu("Open Recent", vec![Item::new("file.no_recent", "No Recent Files").enabled(false).into()]),
        Node::submenu("Plugins", vec![Item::new("plugin.a", "A").into()]).platforms(Platforms::ALL.without(Platforms::WEB)),
    ];
    let bar = MenuBar { menus: vec![Menu::new("File", file).role(MenuRole::File)], documents: Vec::new() };
    let file_on = |target| {
        let l = layout(&bar, target, &LayoutOptions::new("Sample"));
        let file = menu(&l.bar, MenuRole::File).children.iter().take_while(|n| **n != Node::Separator).cloned().collect::<Vec<_>>();
        ids(&file)
    };
    assert_eq!(file_on(Target::Mac), ["file.new", "file.tabs", "[Export]", "[Open Recent]", "[Plugins]"]);
    assert_eq!(file_on(Target::Linux), ["file.new", "[Open Recent]", "[Plugins]"]);
    assert_eq!(file_on(Target::Web), ["file.new", "[Open Recent]"]);
}

fn keymap() -> Keymap {
    let app = Scope::app();
    let layers = Scope::new("Layers").unwrap();
    let catalog = vec![
        Bindable::new("file.save", "Save", app.clone()).with_default(Binding::new(chord("Cmd+S"))),
        Bindable::new("view.rulers", "Rulers", app.clone()).toggle().with_default(Binding::new(chord("Cmd+R"))),
        Bindable::new("view.grid", "Grid", app.clone()).toggle(),
        Bindable::new("layer.new", "New Layer", layers.clone())
            .with_default(Binding::in_scope(chord("Cmd+Shift+N"), app.clone()))
            .with_default(Binding::new(chord("Cmd+L"))),
        Bindable::new("view.mode", "Mode", app).with_default(Binding::new(chord("M"))),
    ];
    Keymap::new(catalog, Scopes::new().with_panel("Layers"), Platform::Linux)
}

#[test]
fn the_keymap_gives_shortcuts_and_check_items() {
    let mut keys = keymap();
    keys.set_bindings(&cmd("file.save"), vec![Binding::new(chord("Cmd+Alt+S"))], Policy::Steal).unwrap();
    let view = vec![
        Item::new("file.save", "Save").shortcut(chord("Cmd+S")).into(),
        Item::new("view.rulers", "Rulers").into(),
        Item::new("view.grid", "Grid").check(true).into(),
        Item::new("layer.new", "New Layer").into(),
        Item::new("view.mode", "Mode").radio(true).into(),
        Item::new("view.other", "Other").shortcut(chord("F9")).into(),
    ];
    let mut bar = MenuBar { menus: vec![Menu::new("View", view)], documents: Vec::new() };
    apply_keymap(&mut bar, &keys);
    let item = |id: &str| bar.find(&cmd(id)).unwrap().clone();
    assert_eq!(item("file.save").shortcut, Some(chord("Cmd+Alt+S")), "the user's override");
    assert_eq!((item("view.rulers").shortcut, item("view.rulers").kind), (Some(chord("Cmd+R")), ItemKind::Check(false)));
    assert_eq!((item("view.grid").shortcut, item("view.grid").kind), (None, ItemKind::Check(true)), "a toggle keeps its state");
    assert_eq!(item("layer.new").shortcut, Some(chord("Cmd+L")), "the home scope's binding first");
    assert_eq!(item("view.mode").kind, ItemKind::Radio(true));
    assert_eq!(item("view.other").shortcut, Some(chord("F9")), "a command the keymap doesn't know keeps its shortcut");
}

#[test]
fn the_app_sets_enabled_and_checked_each_frame() {
    let mut bar = mac_layout(&sample(false, true), &LayoutOptions::new("Sample")).bar;
    let mut asked = Vec::new();
    apply_state(&mut bar, |c| {
        asked.push(c.key());
        match c.id.as_str() {
            "edit.undo" => ItemState { enabled: false, checked: None },
            "panel.layers" => ItemState { checked: Some(true), ..ItemState::default() },
            _ => ItemState::default(),
        }
    });
    assert!(!bar.find(&cmd("edit.undo")).unwrap().enabled);
    assert_eq!(bar.find(&cmd("panel.layers")).unwrap().kind, ItemKind::Check(true));
    assert!(asked.contains(&"file.new".to_string()));
    assert!(!asked.iter().any(|k| k == Builtin::Hide.id() || k == "doc.0"), "built-ins and documents are left alone: {asked:?}");
}
