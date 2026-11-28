//! Recording through the dialog: the recorder takes every key, the state decides what a chord means.

use craft_keymap::{Binding, CommandRef, Platform, Policy};
use craft_menu::shortcuts::Layout;
use craft_ui::testing::trial;
use egui::{Key, Modifiers};

use super::fixture::{self, Rig};

/// At a compact opening size, replacing the last long shortcut through the detail pane preserves its scope
/// and keeps the other bindings.
#[trial]
fn the_detail_pane_can_replace_a_shortcut_that_does_not_fit_inline() {
    let command = CommandRef::from("file.save");
    let mut km = fixture::keymap(Platform::Windows);
    let original = ["Ctrl+Alt+Shift+PageUp", "Ctrl+Alt+Shift+PageDown", "Text: Ctrl+Alt+Shift+End"];
    let bindings: Vec<_> = original.into_iter().map(|text| Binding::parse(text).unwrap()).collect();
    km.set_bindings(&command, bindings, Policy::Keep).unwrap();
    let mut options = fixture::options();
    options.size = Some(egui::vec2(640.0, 560.0));
    let mut rig = Rig::new(km, options, None);
    rig.shared.borrow_mut().state.select(&command);
    rig.pass();

    let replace = rig.find("Replace Ctrl+Alt+Shift+End (Text) for Save");
    assert!(replace.rect.width() >= 16.0, "the replace control fits: {:?}", replace.rect);
    rig.fixture.click(&replace);
    rig.pass();
    rig.press(Modifiers::NONE, Key::F11);
    let expected = [original[0], original[1], "Text: F11"].map(|text| Binding::parse(text).unwrap().to_string());
    assert_eq!(rig.bindings("file.save"), expected);
    rig.click("Undo");
    assert_eq!(rig.bindings("file.save"), original.map(|text| Binding::parse(text).unwrap().to_string()));
}

#[trial]
fn going_read_only_cancels_a_pending_detail_replacement() {
    let command = CommandRef::from("file.save");
    let mut rig = Rig::tuned(Platform::Windows, |options| options.double_press = true);
    let original = rig.bindings("file.save");
    rig.shared.borrow_mut().state.select(&command);
    rig.pass();
    rig.click("Replace Ctrl+S for Save");
    rig.press(Modifiers::NONE, Key::F11);
    {
        let mut shared = rig.shared.borrow_mut();
        assert_eq!(shared.state.snapshot(&shared.km).unwrap().recording, Some(command));
        shared.options.editable = false;
    }

    rig.fixture.advance(1.0);
    rig.pass();
    assert_eq!(rig.bindings("file.save"), original);
    let shared = rig.shared.borrow();
    assert_eq!(shared.state.snapshot(&shared.km).unwrap().recording, None);
}

#[trial]
fn going_read_only_dismisses_a_pending_detail_conflict() {
    let command = CommandRef::from("file.save");
    let mut rig = Rig::mac();
    let original = rig.bindings("file.save");
    rig.shared.borrow_mut().state.select(&command);
    rig.pass();
    rig.click("Replace ⌘S for Save");
    rig.press(Modifiers::COMMAND, Key::E);
    {
        let mut shared = rig.shared.borrow_mut();
        assert!(shared.state.snapshot(&shared.km).unwrap().conflict.is_some());
        shared.options.editable = false;
    }

    rig.pass();
    assert_eq!(rig.bindings("file.save"), original);
    let shared = rig.shared.borrow();
    assert!(shared.state.snapshot(&shared.km).unwrap().conflict.is_none());
}

#[trial]
fn clicking_search_leaves_detail_recording() {
    let mut rig = Rig::mac();
    let original = rig.bindings("file.save");
    rig.shared.borrow_mut().state.select(&CommandRef::from("file.save"));
    rig.pass();
    rig.click("Replace ⌘S for Save");
    rig.click("Search:");
    rig.fixture.type_key(Key::F, "f");
    rig.pass();

    assert_eq!(rig.bindings("file.save"), original);
    let shared = rig.shared.borrow();
    let snapshot = shared.state.snapshot(&shared.km).unwrap();
    assert_eq!(snapshot.query, "f");
    assert_eq!(snapshot.recording, None);
}

#[trial]
fn clicking_search_cancels_the_pending_detail_timeout() {
    let mut rig = Rig::tuned(Platform::Windows, |options| options.double_press = true);
    let original = rig.bindings("file.save");
    rig.shared.borrow_mut().state.select(&CommandRef::from("file.save"));
    rig.pass();
    rig.click("Replace Ctrl+S for Save");
    rig.press(Modifiers::NONE, Key::F11);

    let search = rig.find("Search:");
    let pos = search.center();
    rig.fixture.advance(1.0).frame(vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
    ]);
    rig.fixture.release(&search);
    rig.pass();
    assert_eq!(rig.bindings("file.save"), original);
    let shared = rig.shared.borrow();
    assert_eq!(shared.state.snapshot(&shared.km).unwrap().recording, None);
}

#[trial]
fn a_focused_detail_recording_commits_when_the_timeout_expires() {
    let mut rig = Rig::tuned(Platform::Windows, |options| options.double_press = true);
    rig.shared.borrow_mut().state.select(&CommandRef::from("file.save"));
    rig.pass();
    let original = rig.bindings("file.save");
    rig.click("Replace Ctrl+S for Save");
    rig.press(Modifiers::NONE, Key::F11);
    assert_eq!(rig.bindings("file.save"), original);

    rig.fixture.advance(1.0);
    rig.pass();
    assert_eq!(rig.bindings("file.save"), ["F11"]);
}

#[trial]
fn escape_cancels_detail_recording_on_the_timeout_frame() {
    let mut rig = Rig::tuned(Platform::Windows, |options| options.double_press = true);
    rig.shared.borrow_mut().state.select(&CommandRef::from("file.save"));
    rig.pass();
    let original = rig.bindings("file.save");
    rig.click("Replace Ctrl+S for Save");
    rig.press(Modifiers::NONE, Key::F11);

    rig.fixture.advance(1.0);
    rig.press(Modifiers::NONE, Key::Escape);
    assert!(rig.is_open());
    assert_eq!(rig.bindings("file.save"), original);
}

#[trial]
fn an_accessibility_action_can_start_an_unfocused_detail_recorder() {
    use egui::accesskit::{Action, ActionRequest, NodeId, TreeId};

    let mut rig = Rig::mac();
    rig.shared.borrow_mut().state.select(&CommandRef::from("file.save"));
    rig.pass();
    rig.click("Search:");
    let replace = rig.find("Replace ⌘S for Save");
    rig.fixture.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: NodeId(replace.node.raw()),
        data: None,
    })]);
    rig.press(Modifiers::NONE, Key::F11);
    assert_eq!(rig.bindings("file.save"), ["F11"]);
}

#[trial]
fn clicking_search_as_recording_opens_leaves_focus_in_search() {
    use egui::accesskit::{Action, ActionRequest, NodeId, TreeId};

    let mut rig = Rig::mac();
    let original = rig.bindings("file.save");
    rig.shared.borrow_mut().state.select(&CommandRef::from("file.save"));
    rig.pass();
    let replace = rig.find("Replace ⌘S for Save");
    let search = rig.find("Search:");
    rig.fixture.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: NodeId(replace.node.raw()),
        data: None,
    })]);
    let pos = search.center();
    rig.fixture.frame(vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
    ]);
    rig.fixture.release(&search).type_key(Key::F, "f");
    rig.pass();
    let shared = rig.shared.borrow();
    let snapshot = shared.state.snapshot(&shared.km).unwrap();
    assert_eq!(snapshot.query, "f");
    assert_eq!(snapshot.recording, None);
    assert_eq!(snapshot.keys, original);
}

#[trial]
fn changing_layout_cancels_a_pending_detail_replacement() {
    let mut rig = Rig::tuned(Platform::Windows, |options| options.double_press = true);
    let original = rig.bindings("file.save");
    rig.shared.borrow_mut().state.select(&CommandRef::from("file.save"));
    rig.pass();
    rig.click("Replace Ctrl+S for Save");
    rig.press(Modifiers::NONE, Key::F11);
    rig.shared.borrow_mut().options.layout = Layout::CommandLists;

    rig.fixture.advance(1.0);
    rig.pass();
    assert_eq!(rig.bindings("file.save"), original);
    let shared = rig.shared.borrow();
    assert_eq!(shared.state.snapshot(&shared.km).unwrap().recording, None);
}

/// While recording, Tab, an arrow and Shift+Escape are chords, not focus moves or a close.
#[trial]
fn recording_takes_tab_arrows_and_shift_escape() {
    let mut rig = Rig::tuned(Platform::Mac, |_| {});
    let mut current = "Save: ⌘S".to_owned();
    for (key, modifiers, expected, shown) in [
        (Key::Tab, Modifiers::NONE, "Tab", "⇥"),
        (Key::ArrowDown, Modifiers::NONE, "Down", "↓"),
        (Key::Escape, Modifiers::SHIFT, "Shift+Escape", "⇧⎋"),
    ] {
        rig.click(&current);
        rig.press(modifiers, key);
        assert_eq!(rig.bindings("file.save"), [expected]);
        current = format!("Save: {shown}");
        assert!(rig.has(&current));
    }
    assert!(rig.is_open(), "Shift+Escape did not close the dialog");
}
