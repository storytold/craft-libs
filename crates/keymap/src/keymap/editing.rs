//! The chords each platform reserves for text editing.

use super::*;

const CMD: Mods = Mods::CMD;
const CMD_SHIFT: Mods = Mods { shift: true, ..Mods::CMD };
const ALT: Mods = Mods::ALT;
const ALT_SHIFT: Mods = Mods { shift: true, ..Mods::ALT };
const CTRL: Mods = Mods::CTRL;

// The chords egui's TextEdit acts on that follow the platform's conventions. egui also takes
// Ctrl+H, Ctrl+K, Ctrl+U and Ctrl+W everywhere, but off macOS those are app commands.
const MAC_EDITING: &[Chord] = &[
    // Undo, redo, select all.
    Chord::new(CMD, Key::Z),
    Chord::new(CMD_SHIFT, Key::Z),
    Chord::new(CMD, Key::A),
    // The clipboard.
    Chord::new(CMD, Key::C),
    Chord::new(CMD, Key::X),
    Chord::new(CMD, Key::V),
    // Words.
    Chord::new(ALT, Key::Left),
    Chord::new(ALT, Key::Right),
    Chord::new(ALT_SHIFT, Key::Left),
    Chord::new(ALT_SHIFT, Key::Right),
    Chord::new(ALT, Key::Backspace),
    Chord::new(ALT, Key::Delete),
    // Line start and end.
    Chord::new(CMD, Key::Left),
    Chord::new(CMD, Key::Right),
    Chord::new(CMD_SHIFT, Key::Left),
    Chord::new(CMD_SHIFT, Key::Right),
    Chord::new(CMD, Key::Backspace),
    Chord::new(CMD, Key::Delete),
    // Text start and end.
    Chord::new(CMD, Key::Up),
    Chord::new(CMD, Key::Down),
    Chord::new(CMD_SHIFT, Key::Up),
    Chord::new(CMD_SHIFT, Key::Down),
    // The Control keys of Cocoa text fields: line start and end, character and line moves,
    // deletion.
    Chord::new(CTRL, Key::A),
    Chord::new(CTRL, Key::E),
    Chord::new(CTRL, Key::B),
    Chord::new(CTRL, Key::F),
    Chord::new(CTRL, Key::P),
    Chord::new(CTRL, Key::N),
    Chord::new(CTRL, Key::H),
    Chord::new(CTRL, Key::K),
    Chord::new(CTRL, Key::U),
    Chord::new(CTRL, Key::W),
];

const PC_EDITING: &[Chord] = &[
    // Undo, redo, select all.
    Chord::new(CMD, Key::Z),
    Chord::new(CMD, Key::Y),
    Chord::new(CMD_SHIFT, Key::Z),
    Chord::new(CMD, Key::A),
    // The clipboard.
    Chord::new(CMD, Key::C),
    Chord::new(CMD, Key::X),
    Chord::new(CMD, Key::V),
    Chord::new(CMD, Key::Insert),
    // Words.
    Chord::new(CMD, Key::Left),
    Chord::new(CMD, Key::Right),
    Chord::new(CMD_SHIFT, Key::Left),
    Chord::new(CMD_SHIFT, Key::Right),
    Chord::new(CMD, Key::Backspace),
    Chord::new(CMD, Key::Delete),
    // Text start and end.
    Chord::new(CMD, Key::Home),
    Chord::new(CMD, Key::End),
    Chord::new(CMD_SHIFT, Key::Home),
    Chord::new(CMD_SHIFT, Key::End),
    Chord::new(CMD, Key::Up),
    Chord::new(CMD, Key::Down),
    Chord::new(CMD_SHIFT, Key::Up),
    Chord::new(CMD_SHIFT, Key::Down),
];

/// The chords with modifiers that edit text on `platform`, as effective chords: undo, redo,
/// select all, the clipboard, and moving and deleting by word, line and text. While typing they go to the text
/// field under every [`TypingRule`]; only a binding in an active context takes one. Plain keys
/// (arrows, Home, End, Backspace, Delete, with or without Shift) are not listed, since every rule
/// holds them already.
pub fn text_editing_chords(platform: Platform) -> &'static [Chord] {
    if platform.is_mac() { MAC_EDITING } else { PC_EDITING }
}
