//! Integration tests of craft-keymap: layout.

use craft_keymap::{Chord, Key, KeyInput, Layout, Mods, Platform};

fn chord(text: &str) -> Chord {
    Chord::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

/// A press as egui reports it at the US position `physical`: the modifiers held and the logical
/// key, which is the physical key when the character typed has no key (`ф`, `é`, `ข`).
fn at(reported: &str, physical: Key) -> KeyInput {
    let c = chord(reported);
    KeyInput::new(c.mods).with_typed(c.key).with_physical(physical)
}

fn texts(chords: &[Chord]) -> Vec<String> {
    chords.iter().map(Chord::to_string).collect()
}

#[test]
fn reported_cases_match_only_the_chord_the_press_means() {
    let p = Platform::Windows;
    let arabic = Layout::new().with_latin(false);
    let ctrl_shift_d = at("Cmd+Shift+]", Key::D);
    assert!(arabic.matches(&chord("Ctrl+Shift+D"), &ctrl_shift_d, p));
    assert!(!arabic.matches(&chord("Ctrl+Shift+]"), &ctrl_shift_d, p), "Arabic Ctrl+Shift+D must not run Ctrl+Shift+]");

    let dvorak_russian_q = at("Cmd+Q", Key::Q).with_latin('\'');
    assert!(!Layout::new().with_latin(false).matches(&chord("Ctrl+Q"), &dvorak_russian_q, p), "Dvorak + Russian й must not quit");

    let qwerty_cmd = at("Cmd+K", Key::V).with_latin('v');
    assert!(Layout::new().matches(&chord("Cmd+V"), &qwerty_cmd, Platform::Mac));
    assert!(!Layout::new().matches(&chord("Cmd+K"), &qwerty_cmd, Platform::Mac), "⌘V must not open the command palette");

    let german = Layout::new();
    let slash = at("Cmd+Shift+/", Key::Num7);
    assert_eq!(texts(&german.candidates(&slash)), ["Cmd+/", "Cmd+Shift+7"]);
    assert!(german.matches(&chord("⌘/"), &slash, Platform::Mac));
    // A US-written ⌘? binding is reached with German's ⇧ß, not with ⇧7.
    assert!(!german.matches(&chord("Cmd+Shift+/"), &slash, p));
    assert_eq!(texts(&german.candidates(&at("Cmd+Shift+?", Key::Minus))), ["Cmd+?", "Cmd+Shift+/"]);
    // German Ctrl+; is Ctrl+Shift+, and must not run a Ctrl+Shift+, (font size) binding.
    let semicolon = at("Cmd+Shift+;", Key::Comma);
    assert!(german.matches(&chord("Ctrl+;"), &semicolon, p));
    assert!(!german.matches(&chord("Ctrl+Shift+,"), &semicolon, p));

    // AZERTY's 6 key types -: Ctrl+- first, then Ctrl+6.
    assert_eq!(texts(&Layout::new().candidates(&at("Cmd+-", Key::Num6))), ["Cmd+-", "Cmd+6"]);
    // Zoom keys: + also runs =, and Shift+= runs +.
    assert_eq!(texts(&Layout::new().candidates(&at("Cmd+Shift++", Key::Equals))), ["Cmd++", "Cmd+Shift+=", "Cmd+="]);
    assert_eq!(texts(&Layout::new().candidates(&KeyInput::new(Mods::SHIFT).with_typed(Key::Equals))), ["Shift+=", "+"]);
}
