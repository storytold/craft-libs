#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use egui::{Key, Modifiers};

use super::*;

fn row(path: &[&str], id: &str, label: &str, shortcut: Option<&str>) -> Row {
    Row {
        path: path.iter().map(|s| s.to_string()).collect(),
        id: id.into(),
        label: label.into(),
        shortcut: shortcut.map(Into::into),
        enabled: true,
        checked: None,
    }
}

fn sep(path: &[&str]) -> Row {
    row(path, "", "---", None)
}

/// A small app's menus, tagged. With `clashing`, app commands take ⌘H, ⌘M, ⌘W and ⌘,.
fn sample(clashing: bool, tag_quit: bool) -> MenuBar {
    let k = |own: &'static str| clashing.then_some(own);
    let rows = vec![
        row(&["File"], "file.new", "New", Some("Cmd+N")),
        sep(&["File"]),
        row(&["File"], "file.close", "Close", k("Cmd+W")),
        sep(&["File"]),
        row(&["File"], "file.exit", "Exit", None),
        row(&["Edit"], "edit.undo", "Undo", Some("Cmd+Z")),
        row(&["Edit"], "edit.redo", "Redo", Some("Cmd+Shift+Z")),
        sep(&["Edit"]),
        row(&["Edit", "Preferences"], "prefs.general", "General", None),
        row(&["Edit", "Preferences"], "prefs.keys", "Keys", None),
        row(&["Edit", "Language"], "lang.en", "English", None),
        row(&["View"], "view.extras", "Extras", k("Cmd+H")),
        row(&["View"], "view.hideLayers", "Hide Layers", k("Cmd+,")),
        row(&["View"], "view.curves", "Curves", k("Cmd+M")),
        row(&["View"], "view.zoomIn", "Zoom In", Some("Cmd++")),
        row(&["Window", "Theme"], "theme.dark", "Dark", None),
        row(&["Window"], "theme.next", "Next Theme", None),
        row(&["Window"], "panel.layers", "Layers", Some("F7")),
        row(&["Window"], "doc.0", "Untitled-1", None),
        row(&["Help"], "help.about", "About", None),
        row(&["Help"], "help.docs", "Docs", None),
    ];
    let mut bar = from_items(&["File", "Edit", "View", "Window", "Help"], &rows);
    bar.tag_item("help.about", ItemRole::About);
    if tag_quit {
        bar.tag_item("file.exit", ItemRole::Quit);
    }
    bar.tag_item("doc.0", ItemRole::Document);
    bar.tag_item("theme.next", ItemRole::Appearance);
    bar.tag_submenu("Edit", "Preferences", ItemRole::Settings);
    bar.tag_submenu("Edit", "Language", ItemRole::Language);
    bar.tag_submenu("Window", "Theme", ItemRole::Appearance);
    bar.tag_menu("File", MenuRole::File);
    bar.tag_menu("Window", MenuRole::Window);
    bar.tag_menu("Help", MenuRole::Help);
    bar
}

fn ids(nodes: &[Node]) -> Vec<String> {
    nodes
        .iter()
        .map(|n| match n {
            Node::Item(it) => it.id.clone(),
            Node::Separator => "---".into(),
            Node::Submenu { label, .. } => format!("[{label}]"),
            Node::Standard { kind, .. } => format!("<{kind:?}>"),
        })
        .collect()
}

fn titled<'a>(bar: &'a MenuBar, title: &str) -> &'a Menu {
    bar.menus.iter().find(|m| m.title == title).unwrap()
}

#[test]
fn the_app_menu_is_laid_out_the_mac_way() {
    let l = mac_layout(&sample(false, true), &LayoutOptions::new("Sample"));
    assert!(l.clashes.is_empty(), "{:?}", l.clashes);
    let app = &l.bar.menus[0];
    assert_eq!((app.title.as_str(), app.role), ("Sample", MenuRole::App));
    #[rustfmt::skip]
    let expected = [
        "help.about", "---", "[Settings]", "[Language]", "[Appearance]", "---",
        "<Services>", "---", HIDE, HIDE_OTHERS, "<ShowAll>", "---", "file.exit",
    ];
    assert_eq!(ids(&app.children), expected);
    let quit = l.bar.find("file.exit").unwrap();
    assert_eq!((quit.label.as_str(), quit.shortcut.as_deref()), ("Quit Sample", Some("Cmd+Q")), "the app's own Quit");
    assert_eq!(l.bar.find("help.about").unwrap().label, "About Sample");
    assert_eq!(l.bar.find(HIDE).unwrap().shortcut.as_deref(), Some("Cmd+H"));
    assert_eq!(l.bar.find(HIDE_OTHERS).unwrap().shortcut.as_deref(), Some("Cmd+Alt+H"));
    assert_eq!(l.bar.find("prefs.general").unwrap().shortcut.as_deref(), Some("Cmd+,"), "⌘, opens the first settings page");
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

/// Without a tagged Quit the layout adds craft-quit's; the app's translations name what the
/// layout writes, AppKit's items included.
#[test]
fn an_untagged_quit_is_craft_quits_and_labels_are_translated() {
    let mut bar = sample(false, false);
    bar.menus.retain(|m| m.role != MenuRole::Window);
    let german = |s: &'static str| match s {
        "Quit {app}" => "{app} beenden".to_string(),
        "Window" => "Fenster".into(),
        "Services" => "Dienste".into(),
        other => other.into(),
    };
    let l = mac_layout(&bar, &LayoutOptions { label: &german, ..LayoutOptions::new("Sample") });
    let quit = l.bar.find(craft_quit::QUIT_MENU_ID).unwrap();
    assert_eq!((quit.label.as_str(), quit.shortcut.as_deref(), quit.role), ("Sample beenden", Some("Cmd+Q"), Some(ItemRole::Quit)));
    assert!(ids(&titled(&l.bar, "File").children).contains(&"file.exit".to_string()), "File › Exit stays");
    let app = l.bar.menu(MenuRole::App).unwrap();
    assert!(app.children.contains(&Node::Standard { kind: Standard::Services, label: "Dienste".into() }));
    let titles: Vec<&str> = l.bar.menus.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(titles, ["Sample", "File", "Edit", "View", "Fenster", "Help"], "a Window menu before Help");
    assert_eq!(ids(&l.bar.menu(MenuRole::Window).unwrap().children), ["<Minimize>", "<Zoom>", "---", "<BringAllToFront>"]);
}

/// App commands keep ⌘H, ⌘M, ⌘W and ⌘,: the system items give way, and each clash is reported.
#[test]
fn clashes_are_resolved_in_the_apps_favour() {
    let l = mac_layout(&sample(true, true), &LayoutOptions::new("Sample"));
    let clash = |sc: &str| l.clashes.iter().find(|c| c.shortcut == sc).map(|c| c.command.as_str());
    assert_eq!(clash("Cmd+H"), Some("view.extras"));
    assert_eq!(clash("Cmd+M"), Some("view.curves"));
    assert_eq!(clash("Cmd+W"), Some("file.close"));
    assert_eq!(clash("Cmd+,"), Some("view.hideLayers"));
    assert_eq!(l.clashes.len(), 4);
    assert_eq!(l.bar.find(HIDE).unwrap().shortcut.as_deref(), Some("Ctrl+Cmd+H"));
    assert_eq!(l.bar.find(MINIMIZE).unwrap().shortcut.as_deref(), Some("Ctrl+Cmd+M"));
    assert_eq!(l.bar.find("prefs.general").unwrap().shortcut, None);
    assert_eq!(ids(&l.bar.menu(MenuRole::Window).unwrap().children)[..2], [MINIMIZE, "<Zoom>"]);
    assert!(!ids(&titled(&l.bar, "File").children).contains(&"<CloseWindow>".to_string()), "⌘W stays File › Close");

    // With ⌃⌘H taken too, Hide goes without a shortcut.
    let mut bar = sample(true, true);
    bar.menus[0].children.push(Item::new("file.other", "Other").shortcut("Ctrl+Cmd+H").into());
    let l = mac_layout(&bar, &LayoutOptions::new("Sample"));
    assert_eq!(l.bar.find(HIDE).unwrap().shortcut, None);
    assert_eq!(l.clashes.iter().find(|c| c.shortcut == "Cmd+H").map(|c| c.resolution), Some("Hide has no shortcut"));
}

#[test]
fn rows_become_a_tidy_tree() {
    let rows = [
        sep(&["A"]),
        row(&["A"], "a", "a", None),
        sep(&["A"]),
        sep(&["A"]),
        sep(&["A", "Sub"]),
        row(&["A", "Sub"], "b", "b", None),
        sep(&["A", "Sub"]),
    ];
    let bar = from_items(&["A", "Empty"], &rows);
    assert_eq!(bar.menus.len(), 2);
    assert_eq!(bar.menus[0].children, [Item::new("a", "a").into(), Node::Separator, Node::submenu("Sub", vec![Item::new("b", "b").into()])]);
    assert!(bar.menus[1].children.is_empty());
    let nested = vec![Node::Separator, Node::submenu("S", vec![Node::Separator, Node::Separator]), Node::Separator, Node::Separator];
    assert_eq!(tidy(nested), [Node::submenu("S", vec![])]);
}

/// Labels, enabled and checked are updated in place; a shortcut or a check item that used to be
/// plain needs a rebuild. A document opening or closing rebuilds only the Window menu.
#[test]
fn keys_separate_rebuilds_from_updates_in_place() {
    let opts = LayoutOptions::new("Sample");
    let base = mac_layout(&sample(false, true), &opts).bar;
    let key = |bar: &MenuBar| (structure_key(bar), window_key(bar));
    let edit = |f: &dyn Fn(&mut Item)| {
        let mut bar = sample(false, true);
        for m in &mut bar.menus {
            for n in &mut m.children {
                if let Node::Item(it) = n
                    && it.id == "edit.undo"
                {
                    f(it);
                }
            }
        }
        mac_layout(&bar, &opts).bar
    };
    assert_eq!(key(&edit(&|it| it.label = "Undo Typing".into())), key(&base));
    assert_eq!(key(&edit(&|it| it.enabled = false)), key(&base));
    assert_ne!(structure_key(&edit(&|it| it.shortcut = Some("Ctrl+Z".into()))), structure_key(&base));
    assert_ne!(structure_key(&edit(&|it| it.checked = Some(false))), structure_key(&base));

    let mut two = sample(false, true);
    let window = two.menus.iter_mut().find(|m| m.role == MenuRole::Window).unwrap();
    window.children.push(Item::new("doc.1", "Untitled-2").checked(true).role(ItemRole::Document).into());
    let two = mac_layout(&two, &opts).bar;
    assert_eq!(structure_key(&two), structure_key(&base));
    assert_ne!(window_key(&two), window_key(&base));
    assert_eq!(ids(&two.menu(MenuRole::Window).unwrap().children).last().map(String::as_str), Some("doc.1"));
}

#[test]
fn chords_parse_and_become_egui_key_presses() {
    let c = Chord::parse("Shift+Cmd+Z").unwrap();
    assert_eq!(c.portable(), "Cmd+Shift+Z");
    assert_eq!(Chord::parse("Ctrl+Cmd+M").unwrap().portable(), "Ctrl+Cmd+M");
    assert_eq!(Chord::parse("Cmd++").unwrap().key, "+");
    assert_eq!(Chord::parse("Cmd+A+B"), None);
    assert_eq!(Chord::parse("Cmd"), None);
    assert!(Chord::parse("Cmd+B").unwrap().native_ok());
    assert!(Chord::parse("Ctrl+B").unwrap().native_ok());
    assert!(Chord::parse("F5").unwrap().native_ok());
    assert!(!Chord::parse("B").unwrap().native_ok());
    assert!(!Chord::parse("Alt+F").unwrap().native_ok());
    assert!(!Chord::parse("Shift+X").unwrap().native_ok());
    assert!(!Chord::parse("F25").unwrap().is_function_key());

    let Some(egui::Event::Key { key, modifiers, pressed: true, .. }) = c.key_event(true) else { panic!("a key press") };
    assert_eq!(key, Key::Z);
    assert!(modifiers.command && modifiers.mac_cmd && modifiers.shift && !modifiers.ctrl && !modifiers.alt);
    let ctrl = Chord::parse("Ctrl+Cmd+M").unwrap().key_event(true);
    assert!(matches!(ctrl, Some(egui::Event::Key { modifiers, .. }) if modifiers.ctrl && modifiers.command));
    assert_eq!(Chord::parse("Cmd+NoSuchKey").unwrap().key_event(true), None);

    // Every shortcut the layout writes parses back to itself and has a key press.
    for it in mac_layout(&sample(true, true), &LayoutOptions::new("Sample")).bar.items() {
        let Some(sc) = &it.shortcut else { continue };
        let c = Chord::parse(sc).unwrap();
        assert_eq!(&c.portable(), sc);
        assert!(c.key_event(true).is_some(), "{sc} on {}", it.id);
    }
}

#[derive(Default)]
struct Fake {
    synced: Rc<RefCell<Vec<MenuBar>>>,
    events: Rc<RefCell<Vec<Event>>>,
    tracking: Rc<Cell<bool>>,
    cancels: Rc<Cell<usize>>,
    /// Keeps reporting a menu open after `cancel_tracking`.
    stuck: bool,
    text_editing: Rc<Cell<bool>>,
}

impl Backend for Fake {
    fn sync(&mut self, bar: &MenuBar) {
        self.synced.borrow_mut().push(bar.clone());
    }
    fn drain(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events.borrow_mut())
    }
    fn tracking(&self) -> bool {
        self.tracking.get()
    }
    fn cancel_tracking(&mut self) {
        self.cancels.set(self.cancels.get() + 1);
        if !self.stuck {
            self.tracking.set(false);
        }
    }
    fn set_text_editing(&mut self, editing: bool) {
        self.text_editing.set(editing);
    }
}

/// AppKit takes a key equivalent's key-down but winit still delivers the key-up: the menu adds
/// only the press, before a key-up already queued, so the shortcut runs once. (⌘V as a key, as
/// an app taking the raw clipboard keys gets it.)
#[test]
fn key_equivalents_become_one_key_press() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut menu = NativeMenu::new(Box::new(Fake { events: events.clone(), ..Default::default() })).clipboard_keys(ClipboardKeys::Raw);
    let paste = Chord::parse("Cmd+V").unwrap();
    let release = egui::Event::Key { key: Key::V, physical_key: Some(Key::V), pressed: false, repeat: false, modifiers: Modifiers::COMMAND };
    let press = paste.key_event(true).unwrap();

    events.borrow_mut().push(Event::Key(paste.clone()));
    let mut raw = egui::RawInput { events: vec![egui::Event::ModifiersChanged(Modifiers::COMMAND), release.clone()], ..Default::default() };
    menu.raw_input(&mut raw);
    assert_eq!(raw.events, [egui::Event::ModifiersChanged(Modifiers::COMMAND), press.clone(), release]);

    events.borrow_mut().push(Event::Key(paste));
    let mut raw = egui::RawInput::default();
    menu.raw_input(&mut raw);
    assert_eq!(raw.events, [press], "no made-up release: the real one comes in a later frame");
}

/// Minimize and craft-quit's Quit run in the crate; other clicks go to the app. The menus are
/// laid out again only when the state changed or an event arrived.
#[test]
fn clicks_and_syncs() {
    let synced = Rc::new(RefCell::new(Vec::new()));
    let events =
        Rc::new(RefCell::new(vec![Event::Click(MINIMIZE.into()), Event::Click("edit.undo".into()), Event::Click(craft_quit::QUIT_MENU_ID.into())]));
    let mut menu = NativeMenu::new(Box::new(Fake { synced: synced.clone(), events: events.clone(), ..Default::default() }));
    let mut raw = egui::RawInput::default();
    menu.raw_input(&mut raw);
    assert!(raw.events.is_empty());

    let ctx = egui::Context::default();
    let mut clicks = Vec::new();
    let mut out = ctx.run_ui(raw, |ui| clicks = menu.take_clicks(ui.ctx()));
    out.textures_delta.clear(); // not painted
    assert_eq!(clicks, ["edit.undo"]);
    let commands = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|o| o.commands).unwrap_or_default();
    assert!(commands.contains(&egui::ViewportCommand::Minimized(true)));
    assert!(commands.contains(&egui::ViewportCommand::Close), "Quit asks the window to close");

    let opts = LayoutOptions::new("Sample");
    let builds = Cell::new(0);
    let build = || {
        builds.set(builds.get() + 1);
        mac_layout(&sample(true, true), &opts)
    };
    assert_eq!(menu.sync_with(1, build).len(), 4, "the clashes, once");
    assert!(menu.sync_with(1, build).is_empty());
    assert_eq!(builds.get(), 1, "same state: nothing built");
    assert!(menu.sync_with(2, build).is_empty(), "same clashes: not reported again");
    events.borrow_mut().push(Event::Click("edit.redo".into()));
    menu.raw_input(&mut egui::RawInput::default());
    menu.sync_with(2, build);
    assert_eq!((builds.get(), synced.borrow().len()), (3, 3), "an event re-syncs");
}

fn press() -> egui::Event {
    egui::Event::PointerButton { pos: egui::pos2(300.0, 200.0), button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }
}

#[test]
fn outside_press_dismisses_without_starting_a_canvas_gesture() {
    use crate::driver::dismiss_input;
    let moved = egui::Event::PointerMoved(egui::pos2(300.0, 200.0));
    let mut raw = egui::RawInput { events: vec![moved.clone(), press()], ..Default::default() };
    assert_eq!(dismiss_input(&mut raw, || true), Some(true));
    assert_eq!(raw.events, [moved]);
    assert_eq!(dismiss_input(&mut raw, || true), None);
    raw.events.push(press());
    assert_eq!(dismiss_input(&mut raw, || false), Some(false));
    assert_eq!(raw.events.len(), 2, "ordinary canvas presses still reach egui");
}

#[test]
fn escape_dismisses_without_reaching_the_active_tool() {
    let escape = |pressed| egui::Event::Key { key: Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    let mut raw = egui::RawInput { events: vec![escape(true), escape(false)], ..Default::default() };
    assert_eq!(crate::driver::dismiss_input(&mut raw, || true), Some(true));
    assert!(raw.events.is_empty());
}

#[test]
fn moving_the_pointer_does_not_dismiss_a_tracking_menu() {
    let mut raw = egui::RawInput { events: vec![egui::Event::PointerMoved(egui::pos2(300.0, 200.0))], ..Default::default() };
    assert_eq!(crate::driver::dismiss_input(&mut raw, || true), None);
    assert_eq!(raw.events.len(), 1);
}

/// `raw_input` closes an open menu on a click and keeps the click from egui; with no menu open
/// the click reaches egui (the menu is still asked to close, in case one is on screen).
#[test]
fn a_click_closes_an_open_menu() {
    let tracking = Rc::new(Cell::new(true));
    let cancels = Rc::new(Cell::new(0));
    let mut menu = NativeMenu::new(Box::new(Fake { tracking: tracking.clone(), cancels: cancels.clone(), ..Default::default() }));
    let mut raw = egui::RawInput { events: vec![press()], ..Default::default() };
    menu.raw_input(&mut raw);
    assert!(raw.events.is_empty());
    assert_eq!((cancels.get(), tracking.get()), (1, false));

    let mut raw = egui::RawInput { events: vec![press()], ..Default::default() };
    menu.raw_input(&mut raw);
    assert_eq!(raw.events, [press()]);
    assert_eq!(cancels.get(), 2);

    menu.raw_input(&mut egui::RawInput { events: vec![egui::Event::PointerMoved(egui::pos2(1.0, 1.0))], ..Default::default() });
    assert_eq!(cancels.get(), 2, "a pointer move closes nothing");
}

/// A backend that keeps reporting a menu open costs one window click, not all of them; once it
/// reports the menu closed, the next open menu's closing click is kept from egui again.
#[test]
fn a_stuck_open_menu_keeps_only_one_click() {
    let tracking = Rc::new(Cell::new(true));
    let mut menu = NativeMenu::new(Box::new(Fake { tracking: tracking.clone(), stuck: true, ..Default::default() }));
    let mut raw = egui::RawInput { events: vec![press()], ..Default::default() };
    menu.raw_input(&mut raw);
    assert!(raw.events.is_empty(), "the click that closes the menu");
    for _ in 0..3 {
        let mut raw = egui::RawInput { events: vec![press()], ..Default::default() };
        menu.raw_input(&mut raw);
        assert_eq!(raw.events, [press()], "later clicks reach egui");
    }
    tracking.set(false);
    menu.raw_input(&mut egui::RawInput::default());
    tracking.set(true);
    let mut raw = egui::RawInput { events: vec![press()], ..Default::default() };
    menu.raw_input(&mut raw);
    assert!(raw.events.is_empty());
}

fn key(key: Key, pressed: bool, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers }
}

/// AppKit takes ⌘C, ⌘X and ⌘V for the Edit menu, so egui-winit never makes its Copy, Cut and
/// Paste events: the driver makes them, unless the app takes the raw keys.
#[test]
fn clipboard_key_equivalents_reach_text_fields() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut menu = NativeMenu::new(Box::new(Fake { events: events.clone(), ..Default::default() }));
    let chord = |s: &str| Event::Key(Chord::parse(s).unwrap());
    events.borrow_mut().extend([chord("Cmd+C"), chord("Cmd+X"), chord("Cmd+Shift+V")]);
    let release = key(Key::C, false, Modifiers::COMMAND);
    let mut raw = egui::RawInput { events: vec![release.clone()], ..Default::default() };
    menu.raw_input(&mut raw);
    let shift_v = Chord::parse("Cmd+Shift+V").unwrap().key_event(true).unwrap();
    assert_eq!(raw.events, [egui::Event::Copy, release, egui::Event::Cut, shift_v], "⌘⇧V is an app shortcut");

    events.borrow_mut().push(chord("Cmd+V"));
    let mut raw = egui::RawInput::default();
    menu.raw_input(&mut raw);
    assert!(raw.events.is_empty(), "the paste comes from eframe, with the clipboard's text");
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(raw, |ui| {
        menu.take_clicks(ui.ctx());
    });
    out.textures_delta.clear();
    let commands = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|o| o.commands).unwrap_or_default();
    assert!(commands.contains(&egui::ViewportCommand::RequestPaste));

    let mut menu = NativeMenu::new(Box::new(Fake { events: events.clone(), ..Default::default() })).clipboard_keys(ClipboardKeys::Raw);
    events.borrow_mut().extend([chord("Cmd+C"), chord("Cmd+V")]);
    let mut raw = egui::RawInput::default();
    menu.raw_input(&mut raw);
    let press = |s: &str| Chord::parse(s).unwrap().key_event(true).unwrap();
    assert_eq!(raw.events, [press("Cmd+C"), press("Cmd+V")]);
}

/// The layout marks the shortcuts it gives (the app's key handling doesn't bind them), and a key
/// equivalent on such an item comes back as a click.
#[test]
fn shortcuts_the_layout_assigns_come_back_as_clicks() {
    use crate::driver::chosen_event;
    let l = mac_layout(&sample(false, true), &LayoutOptions::new("Sample"));
    let assigned = |id: &str| l.bar.find(id).unwrap().layout_shortcut;
    assert!(assigned("file.exit") && assigned("prefs.general") && assigned(HIDE));
    assert!(!assigned("edit.undo") && !assigned("prefs.keys"));
    let mut bar = sample(false, true);
    bar.menus[0].children.push(Item::new("file.quit", "Quit").shortcut("Cmd+Q").role(ItemRole::Quit).into());
    bar.tag_item("file.exit", ItemRole::About);
    assert!(!mac_layout(&bar, &LayoutOptions::new("Sample")).bar.find("file.quit").unwrap().layout_shortcut, "the app's own ⌘Q");

    let cmd_q = Chord::parse("Cmd+Q").unwrap();
    let held = Some([true, false, false, false]);
    assert_eq!(chosen_event("file.exit", Some(&cmd_q), true, held, false), Event::Click("file.exit".into()));
    assert_eq!(chosen_event("file.quit", Some(&cmd_q), false, held, false), Event::Key(cmd_q.clone()));
    // ⇧ held on a ⌘R item: the key press that was made.
    let shift_r = Chord::parse("Cmd+Shift+R").unwrap();
    assert_eq!(
        chosen_event("view.reload", Some(&Chord::parse("Cmd+R").unwrap()), false, Some([true, false, false, true]), false),
        Event::Key(shift_r)
    );
    assert_eq!(chosen_event("file.quit", Some(&cmd_q), false, None, false), Event::Click("file.quit".into()), "a mouse click");
    assert_eq!(
        chosen_event(MINIMIZE, Some(&Chord::parse("Ctrl+Cmd+M").unwrap()), false, Some([true, true, false, false]), false),
        Event::Click(MINIMIZE.into())
    );
    // ↩ inside the open menu: on a ⌘ item without ⌘ held, and on a bare tool key.
    let enter = Some([false; 4]);
    assert_eq!(chosen_event("file.quit", Some(&cmd_q), false, enter, false), Event::Click("file.quit".into()));
    assert_eq!(chosen_event("tool.brush", Some(&Chord::parse("B").unwrap()), false, enter, false), Event::Click("tool.brush".into()));
}

/// While a text field has the keys, a key equivalent runs its command as a click (⌘S saves while
/// typing), except the editing keys the field handles itself.
#[test]
fn key_equivalents_run_commands_while_text_editing() {
    use crate::driver::chosen_event;
    let editing = Rc::new(Cell::new(false));
    let mut menu = NativeMenu::new(Box::new(Fake { text_editing: editing.clone(), ..Default::default() }));
    menu.set_text_editing(true);
    assert!(editing.get(), "the backend has the flag for chosen_event");

    let chosen = |shortcut: &str, held: [bool; 4], text_editing: bool| {
        chosen_event("cmd", Some(&Chord::parse(shortcut).unwrap()), false, Some(held), text_editing)
    };
    let cmd = [true, false, false, false];
    let shift_cmd = [true, false, false, true];
    for (shortcut, held) in
        [("Cmd+S", cmd), ("Cmd+N", cmd), ("Cmd+Shift+S", shift_cmd), ("Ctrl+Cmd+F", [true, true, false, false]), ("F5", [false; 4])]
    {
        assert_eq!(chosen(shortcut, held, true), Event::Click("cmd".into()), "{shortcut} while typing");
        assert!(matches!(chosen(shortcut, held, false), Event::Key(_)), "{shortcut} with no text field");
    }
    for (shortcut, held) in [("Cmd+C", cmd), ("Cmd+X", cmd), ("Cmd+V", cmd), ("Cmd+A", cmd), ("Cmd+Z", cmd), ("Cmd+Shift+Z", shift_cmd)] {
        assert_eq!(chosen(shortcut, held, true), Event::Key(Chord::parse(shortcut).unwrap()), "{shortcut} stays with the field");
    }
    assert_eq!(chosen("Cmd+A", shift_cmd, true), Event::Click("cmd".into()), "⇧⌘A is not select all");
}

/// Hide Others is the crate's own item: an app command on ⌥⌘H, in a menu or bound elsewhere,
/// keeps the key, and shortcuts bound outside the menus clash like menu ones.
#[test]
fn hide_others_and_extra_shortcuts_give_way_to_the_app() {
    let extra = [("Cmd+Alt+H".to_string(), "canvas.hideGuides".to_string()), ("Shift+Cmd+Q".to_string(), "canvas.other".to_string())];
    let l = mac_layout(&sample(false, true), &LayoutOptions::new("Sample").extra_shortcuts(&extra));
    assert_eq!(l.bar.find(HIDE_OTHERS).unwrap().shortcut, None);
    assert_eq!(
        l.clashes,
        [Clash { shortcut: "Cmd+Alt+H", system: "Hide Others", command: "canvas.hideGuides".into(), resolution: "Hide Others has no shortcut" }]
    );
    let extra = [("Cmd+H".to_string(), "canvas.hand".to_string())];
    let l = mac_layout(&sample(false, true), &LayoutOptions::new("Sample").extra_shortcuts(&extra));
    assert_eq!(l.bar.find(HIDE).unwrap().shortcut.as_deref(), Some("Ctrl+Cmd+H"));
}

/// An app menu in the app's own tree merges into the layout's: no second app menu.
#[test]
fn an_apps_own_app_menu_is_merged() {
    let mut bar = sample(false, true);
    let own = vec![
        Item::new("app.discord", "Join Our Discord").into(),
        Node::submenu("Theme", vec![Item::new("theme.light", "Light").into()]).role(ItemRole::Appearance),
    ];
    bar.menus.insert(0, Menu::new("Sample", own).role(MenuRole::App));
    bar.tag_item("theme.next", ItemRole::Document);
    bar.menus.iter_mut().find(|m| m.title == "Window").unwrap().children.retain(|n| !matches!(n, Node::Submenu { .. }));
    let l = mac_layout(&bar, &LayoutOptions::new("Sample"));
    assert_eq!(l.bar.menus.iter().filter(|m| m.role == MenuRole::App).count(), 1);
    let app = l.bar.menu(MenuRole::App).unwrap();
    assert_eq!(ids(&app.children)[..7], ["help.about", "---", "[Settings]", "[Language]", "[Appearance]", "---", "app.discord"]);
}
