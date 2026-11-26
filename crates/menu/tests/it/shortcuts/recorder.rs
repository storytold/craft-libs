//! Recording through the dialog: the recorder takes every key, the state decides what a chord means.

use craft_keymap::Platform;
use craft_ui::testing::trial;
use egui::{Key, Modifiers};

use super::fixture::Rig;

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
