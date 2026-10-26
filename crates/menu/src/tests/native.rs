//! The driver's side of the native menu: the gate, the command hook, display-only shortcuts and
//! shortcut recording.

use std::sync::{Arc, Mutex};

use super::*;
use crate::driver::chosen_event;

fn modal() -> Gate {
    Gate { modal: true, ..Gate::default() }
}

fn recording() -> Gate {
    Gate { recording_shortcut: true, ..Gate::default() }
}

type Shared<T> = Rc<RefCell<Vec<T>>>;

/// A driver over a fake backend, with what it synced, its pending events and its dispatch.
fn driver() -> (NativeMenu, Shared<MenuBar>, Shared<Event>, Dispatch) {
    let fake = Fake::default();
    let (synced, events, dispatch) = (fake.synced.clone(), fake.events.clone(), fake.dispatch.clone());
    let menu = NativeMenu::new(Box::new(fake));
    let dispatch = dispatch.borrow().clone().expect("NativeMenu::new attaches the dispatch");
    (menu, synced, events, dispatch)
}

fn take_clicks(menu: &mut NativeMenu) -> Vec<CommandRef> {
    let ctx = egui::Context::default();
    let mut clicks = Vec::new();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| clicks = menu.take_clicks(ui.ctx()));
    out.textures_delta.clear();
    clicks
}

/// The gate is checked when the app takes the clicks, not only when the menu was open: a click
/// that arrived before a modal dialog opened doesn't run behind it. Built-ins still do.
#[test]
fn a_click_waiting_when_a_dialog_opens_is_dropped() {
    let (mut menu, _, events, _) = driver();
    events.borrow_mut().extend([Event::Click(cmd("file.save")), Event::Click(Builtin::HideOthers.command())]);
    menu.raw_input(&mut egui::RawInput::default());
    menu.set_gate(modal());
    assert_eq!(take_clicks(&mut menu), [Builtin::HideOthers.command()]);

    events.borrow_mut().push(Event::Click(cmd("file.save")));
    menu.raw_input(&mut egui::RawInput::default());
    menu.set_gate(Gate::default());
    assert_eq!(take_clicks(&mut menu), [cmd("file.save")]);
}

/// While a dialog is open the app's items are greyed and the built-ins aren't. Opening and
/// closing it re-syncs the last menus in place, without laying them out again.
#[test]
fn a_modal_gate_greys_the_apps_items() {
    let (mut menu, synced, _, _) = driver();
    let opts = LayoutOptions::new("Sample");
    let builds = Cell::new(0);
    let build = || {
        builds.set(builds.get() + 1);
        mac_layout(&sample(false, true), &opts)
    };
    menu.sync_with(1, build);
    menu.set_gate(modal());
    menu.sync_with(1, build);
    menu.sync_with(1, build);
    menu.set_gate(Gate::default());
    menu.sync_with(1, build);
    assert_eq!((builds.get(), synced.borrow().len()), (1, 3), "one layout; synced open, greyed, open");

    let synced = synced.borrow();
    let enabled = |bar: &MenuBar, c: &CommandRef| bar.find(c).unwrap().enabled;
    let greyed = &synced[1];
    assert!(!enabled(greyed, &cmd("file.new")) && !enabled(greyed, &cmd("help.about")));
    // AppKit sends a disabled item's key equivalent nowhere: ⌘Z must reach the dialog's fields.
    assert!(enabled(greyed, &cmd("edit.undo")), "the text-editing keys stay");
    assert!(enabled(greyed, &Builtin::Hide.command()) && enabled(greyed, &Builtin::HideOthers.command()));
    assert!(enabled(&synced[2], &cmd("file.new")));
    assert_eq!(structure_key(greyed), structure_key(&synced[0]), "greying is an update in place");

    assert!(Gate::default().grey(&synced[0]).as_ref() == &synced[0]);
    assert!(typing(true).grey(&synced[0]).as_ref() == &synced[0], "text editing greys nothing");
}

/// The hook sees app commands clicked while the gate allows them, as they arrive; key presses and
/// built-ins don't reach it. `take_clicks` still returns the commands.
#[test]
fn on_command_sees_allowed_clicks_only() {
    let (mut menu, _, events, dispatch) = driver();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    menu.on_command(Box::new(move |c| {
        log.lock().unwrap().push(c.clone());
        false
    }));

    let save = chord("Cmd+S");
    let mouse = |c: &CommandRef| dispatch.chosen(c, Some(&save), false, None);
    events.borrow_mut().push(mouse(&cmd("file.save")));
    assert_eq!(dispatch.chosen(&cmd("file.save"), Some(&save), false, held([true, false, false, false])), Event::Key(save));
    mouse(&Builtin::Quit.command());
    menu.set_gate(modal());
    mouse(&cmd("file.export"));
    menu.set_gate(Gate::default());
    assert_eq!(*seen.lock().unwrap(), [cmd("file.save")]);

    menu.raw_input(&mut egui::RawInput::default());
    assert_eq!(take_clicks(&mut menu), [cmd("file.save")], "a click the hook didn't run");

    menu.on_command(Box::new(|c| c.id == "file.save"));
    events.borrow_mut().extend([mouse(&cmd("file.save")), mouse(&cmd("file.export"))]);
    assert_eq!(events.borrow()[0], Event::Handled(cmd("file.save")));
    menu.raw_input(&mut egui::RawInput::default());
    assert_eq!(take_clicks(&mut menu), [cmd("file.export")], "the hook ran file.save");
}

/// While a text field has the keys, clicking Edit › Copy or Undo acts on the field: the item's
/// shortcut is tapped (press and release, since a click has no key-up), not the app's command run.
#[test]
fn a_click_on_a_text_editing_item_while_typing_taps_its_shortcut() {
    let (copy, undo, save) = (chord("Cmd+C"), chord("Cmd+Z"), chord("Cmd+S"));
    assert_eq!(chosen_event(&cmd("edit.copy"), Some(&copy), false, None, typing(true)), Event::Tap(copy));
    assert_eq!(chosen_event(&cmd("edit.undo"), Some(&undo), false, None, typing(true)), Event::Tap(undo));
    assert_eq!(chosen_event(&cmd("file.save"), Some(&save), false, None, typing(true)), Event::Click(cmd("file.save")));
    assert_eq!(chosen_event(&cmd("edit.undo"), Some(&undo), false, None, typing(false)), Event::Click(cmd("edit.undo")));

    let (mut menu, _, events, _) = driver();
    menu.set_gate(typing(true));
    events.borrow_mut().extend([Event::Tap(undo), Event::Tap(copy)]);
    let mut raw = egui::RawInput::default();
    menu.raw_input(&mut raw);
    assert_eq!(raw.events, [key_event(&undo, true), key_event(&undo, false), egui::Event::Copy]);
    assert!(take_clicks(&mut menu).is_empty());
}

/// `sync` lays nothing out itself: it compares each frame's layout with the last, so a change
/// without input still reaches the menus, and an unchanged one costs no sync.
#[test]
fn sync_compares_each_frames_layout() {
    let (mut menu, synced, _, _) = driver();
    let opts = LayoutOptions::new("Sample");
    let mut bar = sample(false, true);
    menu.sync(mac_layout(&bar, &opts));
    menu.sync(mac_layout(&bar, &opts));
    assert_eq!(synced.borrow().len(), 1, "unchanged: no sync");
    bar.for_each_item_mut(|it| {
        if it.command.id == "file.new" {
            it.enabled = false;
        }
    });
    menu.sync(mac_layout(&bar, &opts));
    assert_eq!(synced.borrow().len(), 2);
    assert!(!synced.borrow()[1].find(&cmd("file.new")).unwrap().enabled);
}

/// An item that was a check item stays one when the app's state stops saying checked
/// (`checked: None`): unchecked, with the same structure, so the menus aren't rebuilt.
#[test]
fn a_check_item_keeps_its_kind() {
    let (mut menu, synced, _, _) = driver();
    let opts = LayoutOptions::new("Sample");
    let with = |checked: Option<bool>| {
        let mut bar = sample(false, true);
        apply_state(&mut bar, |c| ItemState { checked: checked.filter(|_| c.id == "file.new"), ..ItemState::default() });
        mac_layout(&bar, &opts)
    };
    menu.sync(with(Some(true)));
    menu.sync(with(None));
    let synced = synced.borrow();
    assert_eq!(synced[1].find(&cmd("file.new")).unwrap().kind, ItemKind::Check(false));
    assert_eq!(structure_key(&synced[1]), structure_key(&synced[0]));
}

/// Only shortcuts with ⌘ or ⌃ are key equivalents. A tool key (`B`, `⇧B`, `⌥B`, `F5`) is
/// display-only: choosing its item with ↩ in the open menu is a click.
#[test]
fn only_command_or_control_shortcuts_are_key_equivalents() {
    use crate::driver::native_ok;
    for s in ["Cmd+B", "Ctrl+B", "Ctrl+F5", "Alt+Cmd+F12", "Shift+Ctrl+Tab"] {
        assert!(native_ok(&chord(s)), "{s}");
    }
    for s in ["B", "Shift+B", "Alt+B", "Shift+Alt+B", "F5", "Shift+F5", "Escape"] {
        assert!(!native_ok(&chord(s)), "{s}");
        let event = chosen_event(&cmd("tool"), Some(&chord(s)), false, held([false; 4]), Gate::default());
        assert_eq!(event, Event::Click(cmd("tool")), "{s}");
    }
}

/// While a shortcut is recorded every key equivalent reaches egui as the key that was pressed,
/// also those that are clicks otherwise (⌘Q on the app's Quit, Minimize, while typing) and the
/// clipboard keys.
#[test]
fn recording_a_shortcut_passes_key_equivalents_through() {
    let cmd_held = held([true, false, false, false]);
    let quit = chord("Cmd+Q");
    assert_eq!(chosen_event(&cmd("file.exit"), Some(&quit), true, cmd_held, recording()), Event::Key(quit));
    let min = chord("Ctrl+Cmd+M");
    assert_eq!(chosen_event(&Builtin::Minimize.command(), Some(&min), false, held([true, true, false, false]), recording()), Event::Key(min));
    let hide = chord("Cmd+H");
    assert_eq!(chosen_event(&Builtin::Hide.command(), Some(&hide), true, cmd_held, recording()), Event::Key(hide));
    let typing_and_recording = Gate { text_editing: true, recording_shortcut: true, ..Gate::default() };
    assert_eq!(chosen_event(&cmd("file.save"), Some(&chord("Cmd+S")), false, cmd_held, typing_and_recording), Event::Key(chord("Cmd+S")));
    assert_eq!(chosen_event(&cmd("file.exit"), Some(&quit), true, None, recording()), Event::Click(cmd("file.exit")), "a mouse click");

    let (mut menu, _, events, _) = driver();
    menu.set_gate(recording());
    events.borrow_mut().extend([Event::Key(chord("Cmd+C")), Event::Key(chord("Cmd+V"))]);
    let mut raw = egui::RawInput::default();
    menu.raw_input(&mut raw);
    assert_eq!(raw.events, [key_event(&chord("Cmd+C"), true), key_event(&chord("Cmd+V"), true)], "no Copy or Paste events");
}

/// Dynamic lists (recent files, workspaces) change the tree: adding or removing one rebuilds,
/// relabelling one updates in place.
#[test]
fn dynamic_lists_rebuild_when_they_grow_or_shrink() {
    let recents = |files: &[&str]| {
        let items = files
            .iter()
            .enumerate()
            .map(|(i, f)| Item::new(CommandRef::parse_key(&format!(r#"file.open_recent {{"index":{i}}}"#)).unwrap(), *f).into())
            .collect();
        let file = Menu::new("File", vec![Node::submenu("Open Recent", items)]).role(MenuRole::File);
        MenuBar { menus: vec![file], documents: Vec::new() }
    };
    let two = structure_key(&recents(&["a.pdf", "b.pdf"]));
    assert_ne!(structure_key(&recents(&["a.pdf", "b.pdf", "c.pdf"])), two);
    assert_ne!(structure_key(&recents(&["a.pdf"])), two);
    assert_eq!(structure_key(&recents(&["b.pdf", "a.pdf"])), two, "same commands, new labels");
}
