//! Chords the operating system takes before the app sees them.
//!
//! A keymap reserves these only when the app passes them to
//! [`Keymap::with_reserved`](crate::Keymap::with_reserved).

use crate::chord::{Chord, Platform};

// from photocraft crates/engine/src/prefs.rs `reserved_shortcut`
const MAC: &[&str] = &["Cmd+Tab", "Cmd+Shift+Tab", "Cmd+Alt+Escape", "Cmd+`"];
const WINDOWS: &[&str] = &["Alt+Tab", "Alt+Shift+Tab", "Alt+F4", "Cmd+Alt+Delete", "Cmd+Escape", "Cmd+Shift+Escape"];
// Linux and the BSDs: window switching and closing, and the virtual consoles.
const LINUX: &[&str] = &[
    "Alt+Tab",
    "Alt+Shift+Tab",
    "Alt+F4",
    "Cmd+Alt+Delete",
    "Cmd+Alt+F1",
    "Cmd+Alt+F2",
    "Cmd+Alt+F3",
    "Cmd+Alt+F4",
    "Cmd+Alt+F5",
    "Cmd+Alt+F6",
    "Cmd+Alt+F7",
    "Cmd+Alt+F8",
    "Cmd+Alt+F9",
    "Cmd+Alt+F10",
    "Cmd+Alt+F11",
    "Cmd+Alt+F12",
];

/// The window switching, closing and session chords of `platform`, for
/// [`Keymap::with_reserved`](crate::Keymap::with_reserved).
pub fn system(platform: Platform) -> Vec<Chord> {
    let list = match platform {
        Platform::Mac => MAC,
        Platform::Windows => WINDOWS,
        Platform::Linux => LINUX,
    };
    list.iter().filter_map(|t| Chord::parse(t).ok()).collect()
}
