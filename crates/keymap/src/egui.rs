//! Conversions between the crate's keys and egui's, and matching egui key presses to chords.
//! [`platform`] gives the platform to build a keymap for, the browser's operating system included
//! on the web.

use ::egui::{Context, Event, Key as EguiKey, KeyboardShortcut, Modifiers};

use crate::chord::{Chord, Key, Mods, Platform};
use crate::layout::{KeyInput, Layout};

/// The platform to build a keymap for in this context. Unlike [`Platform::current`], a web build
/// gets the browser's operating system, which eframe reads from the user agent, so a Mac browser
/// shows and matches Cmd shortcuts. eframe sets it before the first frame; an app that builds its
/// keymap earlier rebuilds it when this differs.
pub fn platform(ctx: &Context) -> Platform {
    Platform::of_egui(ctx.os())
}

// Both matches are generated from one list so that a key added to `Key` without an egui
// counterpart fails to compile.
macro_rules! egui_keys {
    ($($ours:ident = $theirs:ident),* ; $($same:ident),* $(,)?) => {
        impl Key {
            /// The key for an egui key. `None` for modifier keys, Copy, Cut, Paste and
            /// BrowserBack.
            pub fn from_egui(key: EguiKey) -> Option<Key> {
                match key {
                    $(EguiKey::$theirs => Some(Key::$ours),)*
                    $(EguiKey::$same => Some(Key::$same),)*
                    _ => None,
                }
            }

            /// The egui key for this key.
            pub fn to_egui(self) -> EguiKey {
                match self {
                    $(Key::$ours => EguiKey::$theirs,)*
                    $(Key::$same => EguiKey::$same,)*
                }
            }
        }
    };
}

egui_keys! {
    Up = ArrowUp, Down = ArrowDown, Left = ArrowLeft, Right = ArrowRight;
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20,
    F21, F22, F23, F24, F25, F26, F27, F28, F29, F30, F31, F32, F33, F34, F35,
    Escape, Tab, Menu, Backspace, Enter, Space, Insert, Delete, Home, End, PageUp, PageDown,
    Minus, Equals, Plus, OpenBracket, CloseBracket, Backslash, Semicolon, Quote, Backtick,
    Comma, Period, Slash, IntlBackslash, Colon, Pipe, Questionmark, Exclamationmark, OpenCurlyBracket, CloseCurlyBracket,
}

impl Mods {
    /// The modifiers egui reports, read for `platform`. On macOS cmd is ⌘ and ctrl the Control
    /// key; elsewhere Ctrl is cmd and ctrl stays false.
    pub fn from_egui(mods: &Modifiers, platform: Platform) -> Mods {
        if platform.is_mac() {
            // egui sets `command` together with `mac_cmd` on macOS; synthetic events may set
            // only one of them.
            Mods { cmd: mods.mac_cmd || mods.command, ctrl: mods.ctrl, alt: mods.alt, shift: mods.shift }
        } else {
            Mods { cmd: mods.ctrl || mods.command, ctrl: false, alt: mods.alt, shift: mods.shift }
        }
    }
}

impl Chord {
    /// The egui shortcut for this chord, for egui's own shortcut text and `consume_shortcut`.
    /// cmd becomes egui's `command` and ctrl its `ctrl`, which egui folds the same way off
    /// macOS.
    pub fn to_egui_shortcut(&self) -> KeyboardShortcut {
        let m = self.mods;
        KeyboardShortcut::new(Modifiers { alt: m.alt, ctrl: m.ctrl, shift: m.shift, mac_cmd: false, command: m.cmd }, self.key.to_egui())
    }
}

impl KeyInput {
    /// What an egui key press reports: the logical key as typed and the physical key. On a layout
    /// that is not Latin, the suite's egui-winit reports the Latin layout's letter as the logical
    /// key (Linux, Windows, and macOS with ⌘ held), and crates.io egui the physical key, so
    /// [`Layout::resolve`] takes a letter there before the position. The Latin layout's character
    /// comes from the frame's input. egui reports no Shift
    /// level, so [`Layout`] infers it. `None` for events other than key presses and releases.
    pub fn from_egui(event: &Event, platform: Platform) -> Option<KeyInput> {
        let Event::Key { key, physical_key, modifiers, .. } = event else { return None };
        let mut input = KeyInput::new(Mods::from_egui(modifiers, platform));
        input.typed = Key::from_egui(*key);
        input.physical = physical_key.and_then(Key::from_egui);
        Some(input)
    }
}

/// Whether a key press egui reported triggers `binding` on `platform`, on a layout about which
/// nothing is known, for a press whose physical key is unknown. [`Layout::matches`] takes the
/// whole press and the layout.
///
/// Besides an exact match:
/// - Shift with a base key matches the shifted key egui reports (`Shift+;` matches `:`).
/// - A shifted symbol bound without Shift matches when Shift is held to type it (`?`).
/// - A binding on `=` matches `+` with or without Shift, and a binding on `+` matches
///   `Shift+=`, so zoom keys work with and without Shift.
pub fn matches(binding: &Chord, key: EguiKey, mods: &Modifiers, platform: Platform) -> bool {
    let Some(pressed) = Key::from_egui(key) else { return false };
    Layout::new().matches(binding, &KeyInput::new(Mods::from_egui(mods, platform)).with_typed(pressed), platform)
}
