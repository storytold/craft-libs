//! The bar from the keyboard: Alt and F10 focus it, Alt+letter opens a menu, letters run rows,
//! mnemonics are assigned per level (with "(F)" for letterless labels) and underlined only when
//! the keyboard drives the bar.

use egui::{Event, Key, Modifiers, PointerButton, pos2};

use super::*;
use crate::bar::keys::{Keys, assign, is_command_key, key_char, localized};
use crate::bar::{Labeled, is_focused, leave_for_shortcut};

impl Bar {
    fn keys(&self) -> Keys {
        Keys::current(&self.ctx, Id::unique("craft-menu-bar").with("keys"))
    }

    fn focus(&self) -> Option<usize> {
        self.keys().focus
    }

    fn alt(&mut self, pressed: bool) {
        let modifiers = if pressed { Modifiers::ALT } else { Modifiers::NONE };
        self.events.push(Event::Key { key: Key::AltLeft, physical_key: None, pressed, repeat: false, modifiers });
        self.run(2);
    }

    /// A tap on Alt: down, then up with nothing else pressed between.
    fn alt_tap(&mut self) {
        self.alt(true);
        self.alt(false);
        self.run(1);
    }

    /// `key` pressed and released with Alt held.
    fn alt_key(&mut self, key: Key) {
        for pressed in [true, false] {
            self.events.push(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::ALT });
        }
        self.run(3);
    }

    /// A letter typed: its key and the text it types.
    fn letter(&mut self, c: char) {
        let key = Key::from_name(&c.to_ascii_uppercase().to_string()).unwrap();
        self.events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        self.events.push(Event::Text(c.to_ascii_lowercase().to_string()));
        self.events.push(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
        self.run(3);
    }

    fn mnemonic(&self, level: usize, command: &str) -> Option<char> {
        self.nav().rows.get(level)?.iter().find(|r| r.command.as_ref().is_some_and(|c| c.id == command))?.mnemonic
    }
}

fn chars(s: &str) -> Vec<Option<char>> {
    s.chars().map(Some).collect()
}

#[test]
fn mnemonics_are_unique_word_starts_first_and_take_preferred_letters() {
    let labels = ["File", "Edit", "Image", "Layer", "Type", "Select", "Filter", "View", "Window", "Help"];
    assert_eq!(assign(&labels, &[]), chars("FEILTSRVWH"), "first letters, then the next free one");
    let preferred = chars("FEILYSTVWH");
    assert_eq!(assign(&labels, &preferred), chars("FEILYSTVWH"), "the app's letters where the labels have them");
    // Items: first letters first, then the next free letter of the word starts, then any.
    assert_eq!(assign(&["New…", "Open…", "Open Recent", "Close", "Close All", "Save", "Save As…"], &[]), chars("NORCASV"));
    // A preferred letter the label lacks (a translation) falls back; a taken one too.
    assert_eq!(assign(&["Datei", "Bearbeiten"], &[Some('F'), Some('E')]), [Some('D'), Some('E')]);
    assert_eq!(assign(&["Aa", "Ab", "Ba"], &[Some('A'), Some('A'), Some('A')]), [Some('A'), Some('B'), None]);
    // Only ASCII letters and digits are keys: no mnemonic without one.
    assert_eq!(assign(&["ファイル", "Zoom 100%", "✔ Rulers"], &[]), [None, Some('Z'), Some('R')]);
}

#[test]
fn key_chars_are_plain_letters_and_digits_and_command_keys_leave_the_bar() {
    assert_eq!(key_char(Key::A), Some('A'));
    assert_eq!(key_char(Key::Num7), Some('7'));
    assert_eq!(key_char(Key::F10), None);
    assert_eq!(key_char(Key::ArrowDown), None);
    assert_eq!(key_char(Key::Space), None);
    assert!(is_command_key(Key::D, Modifiers::COMMAND));
    assert!(is_command_key(Key::F7, Modifiers::NONE));
    assert!(!is_command_key(Key::F10, Modifiers::NONE), "F10 toggles the bar");
    assert!(!is_command_key(Key::ControlLeft, Modifiers::CTRL), "Ctrl going down is no shortcut yet");
    assert!(!is_command_key(Key::E, Modifiers::NONE), "a letter is a mnemonic");
    assert!(!is_command_key(Key::E, Modifiers::ALT), "Alt+letter opens a menu");
    assert!(!is_command_key(Key::Enter, Modifiers::NONE));
}

/// A label with no Latin letter (Chinese, Japanese…) shows its English label's letter in
/// parentheses, before a trailing ellipsis, and takes it as its mnemonic; the letters stay unique.
#[test]
fn labels_without_latin_letters_show_their_english_key() {
    let en = |v: &[&str]| v.iter().map(|s| Some((*s).to_owned())).collect::<Vec<_>>();
    let (shown, keys) = localized(&["文件", "编辑", "Image", "图层"], &en(&["File", "Edit", "Image", "Layer"]), &[]);
    assert_eq!(shown, ["文件(F)", "编辑(E)", "Image", "图层(L)"]);
    assert_eq!(keys, chars("FEIL"));
    // Rows: the key goes before the ellipsis; a clash takes the next letter of the shown label.
    let (shown, keys) = localized(&["新建…", "打开...", "关闭"], &en(&["New…", "Open...", "Close"]), &[]);
    assert_eq!(shown, ["新建(N)…", "打开(O)...", "关闭(C)"]);
    assert_eq!(keys, chars("NOC"));
    let (shown, keys) = localized(&["关闭", "复制"], &en(&["Close", "Copy"]), &[]);
    assert_eq!(shown, ["关闭(C)", "复制(O)"]);
    assert_eq!(keys, chars("CO"));
    // A shown "(C)" keeps its key even when a label with letters of its own comes first.
    let (shown, keys) = localized(&["Copy", "关闭"], &en(&["Open", "Close"]), &[]);
    assert_eq!(shown, ["Copy", "关闭(C)"]);
    assert_eq!(keys, chars("OC"));
    // Without an English label, no suffix and no key.
    let (shown, keys) = localized(&["文件", "Edit"], &[None, None], &[]);
    assert_eq!(shown, ["文件", "Edit"]);
    assert_eq!(keys, [None, Some('E')]);
    // An English UI is unchanged.
    assert_eq!(localized(&["File", "Edit"], &en(&["File", "Edit"]), &[]).0, ["File", "Edit"]);
}

#[test]
fn alt_focuses_the_bar_arrows_move_and_down_opens_the_lit_menu() {
    let mut b = wide();
    b.alt_tap();
    assert_eq!(b.focus(), Some(0), "Alt puts the keyboard on File");
    assert!(is_focused(&b.ctx) && !is_open(&b.ctx), "the shortcuts wait for the bar; no menu opens yet");
    b.key(Key::ArrowRight);
    assert_eq!(b.focus(), Some(1), "→ moves to Edit");
    b.key(Key::ArrowLeft);
    b.key(Key::ArrowLeft);
    assert_eq!(b.focus(), Some(3), "← wraps round to Help");
    b.key(Key::ArrowRight);
    b.key(Key::ArrowRight);
    b.key(Key::ArrowDown);
    assert!(is_open(&b.ctx) && b.shown("Undo"), "↓ opens the lit menu, Edit");
    assert_eq!(b.focus(), None, "the focus moves into the menu");
    assert_eq!(b.nav().highlighted(0), Some(0), "its first row is lit, as on Windows");
    assert!(b.nav().underline, "a menu opened from the keyboard shows its mnemonics");
    b.key(Key::Escape);
    assert!(!is_open(&b.ctx) && b.focus().is_none(), "Esc leaves the menu and the bar");
    assert!(!b.nav().underline && !b.keys().show_underlines(&b.ctx), "and the underlines go");
}

#[test]
fn a_second_alt_esc_or_a_click_takes_the_keyboard_off_the_bar_and_f10_focuses_it() {
    let mut b = wide();
    b.alt_tap();
    b.alt_tap();
    assert_eq!(b.focus(), None, "a second tap leaves");
    b.key(Key::F10);
    assert_eq!(b.focus(), Some(0), "F10 focuses the bar too");
    b.key(Key::Escape);
    assert!(b.focus().is_none() && !is_focused(&b.ctx));
    b.key(Key::F10);
    b.click_at(pos2(600.0, 300.0));
    assert_eq!(b.focus(), None, "a click anywhere leaves");
}

/// Alt+wheel zooms in many apps, and Alt that went to the desktop (Alt+Tab) is no tap either.
#[test]
fn alt_with_a_wheel_scroll_a_lost_focus_or_a_drag_is_no_tap() {
    let mut b = wide();
    b.alt(true);
    b.events.push(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, -1.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: Modifiers::ALT,
    });
    b.run(2);
    b.alt(false);
    assert_eq!(b.focus(), None, "Alt+scroll");
    b.alt(true);
    b.events.push(Event::WindowFocused(false));
    b.events.push(Event::WindowFocused(true));
    b.run(1);
    b.alt(false);
    assert_eq!(b.focus(), None, "Alt+Tab");
    // Alt pressed during a drag is a modifier for the drag, with the button let go before or after.
    b.move_to(pos2(600.0, 300.0));
    for button_up_first in [true, false] {
        b.button(true);
        b.alt(true);
        if button_up_first {
            b.button(false);
            b.alt(false);
        } else {
            b.alt(false);
            b.button(false);
        }
        assert_eq!(b.focus(), None, "a drag, button up first: {button_up_first}");
    }
    // The whole gesture within one frame: press, Alt down, Alt up, release.
    let (at, alt, none) = (b.pointer, Modifiers::ALT, Modifiers::NONE);
    b.events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: none });
    b.events.push(Event::Key { key: Key::AltLeft, physical_key: None, pressed: true, repeat: false, modifiers: alt });
    b.events.push(Event::Key { key: Key::AltLeft, physical_key: None, pressed: false, repeat: false, modifiers: none });
    b.events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: none });
    b.run(2);
    assert_eq!(b.focus(), None, "one frame");
    b.alt_tap();
    assert_eq!(b.focus(), Some(0), "a lone tap still focuses the bar");
}

#[test]
fn alt_with_a_letter_opens_that_menu_and_enter_or_a_letter_opens_one_from_the_bar() {
    let mut b = wide();
    b.alt_key(Key::E);
    assert!(is_open(&b.ctx) && b.shown("Undo"), "Alt+E opens Edit");
    b.alt_key(Key::F);
    assert!(b.shown("Open…") && !b.shown("Undo"), "Alt+F switches to File while a menu is open");
    b.key(Key::Escape);
    b.key(Key::F10);
    b.key(Key::Enter);
    assert!(b.shown("Open…"), "Enter opens the lit File menu");
    b.key(Key::Escape);
    b.key(Key::F10);
    b.letter('v');
    assert!(is_open(&b.ctx) && b.shown("Grid"), "a letter on the focused bar opens that menu (V = View)");
    assert!(b.take().is_empty());
}

#[test]
fn a_letter_in_an_open_menu_runs_the_row_with_that_mnemonic_or_opens_its_submenu() {
    let mut b = wide();
    b.alt_key(Key::V);
    assert_eq!(b.mnemonic(0, "view.grid"), Some('G'));
    let letters: Vec<char> = b.nav().rows[0].iter().filter_map(|r| r.mnemonic).collect();
    let mut unique = letters.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(letters.len(), unique.len(), "mnemonics are unique within a level: {letters:?}");
    b.letter('g');
    assert_eq!(b.take(), vec![CommandRef::new("view.grid")], "G ran Grid");
    assert!(!is_open(&b.ctx), "and closed the menu");
    // File: New N, Open… O, Open Recent R; a submenu's letter opens it, with its first row lit.
    b.alt_key(Key::F);
    b.letter('r');
    let nav = b.nav();
    assert_eq!((nav.level, nav.highlighted(1)), (1, Some(0)), "R opened Open Recent on its first row");
    assert!(b.shown("a.txt"));
    b.letter('b');
    assert_eq!(b.take(), vec![CommandRef::new("file.recent.b")], "B ran b.txt from the submenu");
    // A disabled row's letter does nothing.
    b.alt_key(Key::F);
    assert_eq!(b.mnemonic(0, "file.locked"), Some('L'));
    b.letter('l');
    assert!(b.take().is_empty() && is_open(&b.ctx), "Locked is disabled");
}

#[test]
fn mnemonics_are_underlined_only_with_alt_or_a_keyboard_opened_menu() {
    let mut b = wide();
    b.click("File");
    assert!(is_open(&b.ctx));
    assert!(!b.nav().underline, "a menu opened with the mouse shows no underlines");
    let underlined = |b: &Bar| {
        let ctx = b.ctx.clone();
        // The rows' mnemonics are in AccessKit either way; the drawn text has the underline.
        b.node("New").and_then(|n| n.access_key().map(str::to_owned)).is_some()
            && Nav::current(&ctx, Id::unique("craft-menu-bar").with("nav")).underline
    };
    assert!(!underlined(&b));
    b.alt(true);
    assert!(underlined(&b), "while Alt is held they show");
    b.alt(false);
    assert!(!is_open(&b.ctx), "an Alt tap closes an open menu, as on Windows");
    b.alt_key(Key::F);
    assert!(is_open(&b.ctx) && underlined(&b), "a menu opened from the keyboard shows them");
    assert_eq!(b.node("New").and_then(|n| n.access_key()), Some("N"), "AccessKit has the mnemonic");
    assert_eq!(b.title("File").and_then(|n| n.access_key()), Some("F"));
}

#[test]
fn the_app_can_prefer_letters_and_name_english_labels_for_letterless_ones() {
    let mut b = wide();
    b.preferred = vec![("View", 'W'), ("file.open", 'P')];
    b.english = Some(|l: Labeled<'_>| match l {
        Labeled::Title("帮助") => Some("Help".to_owned()),
        Labeled::Command(c) if c.id == "file.new" => Some("New".to_owned()),
        _ => None,
    });
    b.bar.menus[3].title = "帮助".to_owned();
    b.bar.menus[0].children[0] = item("file.new", "新建…").into();
    b.run(2);
    assert!(b.title("帮助(H)").is_some(), "a letterless title shows its English key");
    b.alt_key(Key::W);
    assert!(b.shown("Grid"), "View took the app's W");
    b.alt_key(Key::H);
    assert!(b.shown("About"), "Alt+H opens 帮助(H)");
    b.alt_key(Key::F);
    assert!(b.shown("新建(N)…"), "the key goes before the ellipsis");
    assert_eq!(b.mnemonic(0, "file.open"), Some('P'));
    b.letter('n');
    assert_eq!(b.take(), vec![CommandRef::new("file.new")]);
}

#[test]
fn a_dialog_or_a_text_field_keeps_its_keys() {
    let mut b = wide();
    b.gate = Gate { modal: true, ..Gate::default() };
    b.alt_tap();
    assert_eq!(b.focus(), None, "a dialog keeps Enter and Esc: Alt doesn't take the keyboard");
    b.alt_key(Key::F);
    assert!(!is_open(&b.ctx), "nor does Alt+F");
    b.click("File");
    b.letter('n');
    assert!(b.take().is_empty() && is_open(&b.ctx), "a gate-forbidden item's letter doesn't run it");
    b.key(Key::Escape);

    b.gate = Gate { text_editing: true, ..Gate::default() };
    b.alt_tap();
    assert_eq!(b.focus(), Some(0), "a tap on Alt still focuses the bar from a text field");
    b.letter('e');
    assert!(!is_open(&b.ctx) && b.focus().is_none(), "a letter goes on to the field and leaves the bar");
    b.alt_key(Key::E);
    assert!(b.shown("Undo"), "Alt+E opens Edit");
    b.letter('u');
    assert_eq!(b.take(), vec![CommandRef::new("edit.undo")], "with a menu open the letters are its");
}

#[test]
fn a_command_shortcut_takes_the_keyboard_off_the_bar() {
    let mut b = wide();
    b.alt_tap();
    assert!(is_focused(&b.ctx));
    b.events.push(Event::Key { key: Key::S, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
    let mut left = false;
    b.time += b.dt;
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, b.size)),
        time: Some(b.time),
        events: std::mem::take(&mut b.events),
        ..Default::default()
    };
    let bar = &b.bar;
    let mut out = b.ctx.run_ui(raw, |ui| {
        left = leave_for_shortcut(ui.ctx());
        MenuBarUi::new(bar).target(Target::Linux).show(ui);
    });
    out.textures_delta.clear();
    assert!(left && !is_focused(&b.ctx), "Ctrl+S leaves the bar for the app's shortcut");
    b.run(2);
    assert_eq!(b.focus(), None);
    assert!(!is_open(&b.ctx));
}

#[test]
fn a_mac_target_has_no_mnemonics() {
    let mut b = wide();
    b.target = Target::Mac;
    b.alt_tap();
    b.key(Key::F10);
    assert!(b.focus().is_none() && !is_focused(&b.ctx), "Alt and F10 leave the bar alone");
    b.alt_key(Key::F);
    assert!(!is_open(&b.ctx), "Option+F is a typing key");
    b.click("File");
    b.alt(true);
    assert!(!b.nav().underline, "Option shows no underlines");
    assert!(b.nav().rows[0].iter().all(|r| r.mnemonic.is_none()), "the rows have no mnemonics");
    assert_eq!(b.node("New").and_then(|n| n.access_key()), None);
    b.alt(false);
    b.letter('n');
    assert!(b.take().is_empty(), "letters run nothing");
}
