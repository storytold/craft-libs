//! `ModifierKeysPanel`: on-screen toggles for the modifier keys, for a touch screen without a keyboard.

use crate::buttons::ToggleButton;
use crate::contract::{Bind, Pattern, Props as _, Show as _, ShowContainer as _};
use crate::layout_components::Row;
use crate::runtime::env::{Env, Label};
use craft_behavior::{EditEvent, Mixed};
use craft_keymap::{Mods, Platform};
use egui::{Response, Ui};
use std::sync::Arc;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The panel's buttons in their fixed order, each with the one modifier it latches: Shift, the command key (⌘ on
/// macOS, Ctrl elsewhere) and Alt (⌥ on macOS).
const KEYS: [(&str, Mods); 3] = [("shift", Mods::SHIFT), ("command", Mods::CMD), ("alt", Mods::ALT)];

/// A panel of latching modifier-key toggles: a press on one holds that modifier for the next click or key.
///
/// - Three toggle buttons, Shift, the command key and Alt, over one binding of the latched modifiers.
/// - Each face is the modifier as chord text writes it on the OS (`⇧ ⌘ ⌥` on macOS, `Shift Ctrl Alt` elsewhere), and
///   its accessible name the modifier's spoken name (`Shift`, `Cmd`, `Option` on macOS).
/// - The app hands the binding's value to the runtime as the frame's latched modifiers (`FrameInput::latched`): every
///   press and drag then reads them as held. Key chords read the keyboard's own modifiers.
/// - The panel adds no words: its title is the dock panel's.
#[must_use = "show the panel with `show`"]
#[derive(Pattern)]
pub struct ModifierKeysPanel<'a> {
    latched: Bind<'a, Mods>,
}

impl<'a> ModifierKeysPanel<'a> {
    /// A panel over the latched modifiers in `latched`.
    pub fn new(latched: Bind<'a, Mods>) -> Self {
        ModifierKeysPanel { latched }
    }

    /// Shows the panel.
    pub fn show(self, ui: &mut Ui) -> Response {
        let ModifierKeysPanel { mut latched } = self;
        let os = Env::of(ui).view().shell().os;
        Row::new()
            .show(ui, |ui, row| {
                let held = latched.reborrow().read(row).and_then(|held| held.same().copied()).unwrap_or_default();
                // Off macOS a held Ctrl is the command key: the buttons show and edit the modifiers as the OS reads them.
                let before = held.effective(os);
                let mut now = before;
                for (salt, key) in KEYS {
                    let mut on = holds(now, key);
                    ToggleButton::new(face(key, os), Bind::bound(&mut on)).spoken(Label::from(spoken(key, os))).id_salt(salt).show(ui);
                    now = set(now, key, on);
                }
                if now != before {
                    let toggled = KEYS.iter().find(|(_, key)| holds(now, *key) != holds(before, *key)).map_or(Mods::NONE, |(_, key)| *key);
                    let name: Arc<str> = Arc::from(spoken(toggled, os).as_str());
                    let _ = latched.reborrow().write(row, &EditEvent::set(Mixed::Same(held), now, name, None));
                }
            })
            .output
    }
}

/// The face of the button for the one-modifier `key`: its symbol on macOS, its name elsewhere.
fn face(key: Mods, os: Platform) -> String {
    key.parts(os).concat()
}

/// The name a screen reader says for the one-modifier `key`.
fn spoken(key: Mods, os: Platform) -> String {
    key.spoken(os).join(" ")
}

/// Whether `mods` holds the one modifier of `key`.
fn holds(mods: Mods, key: Mods) -> bool {
    (key.shift && mods.shift) || (key.cmd && mods.cmd) || (key.alt && mods.alt)
}

/// `mods` with the one modifier of `key` held or released.
fn set(mods: Mods, key: Mods, on: bool) -> Mods {
    Mods {
        shift: if key.shift { on } else { mods.shift },
        cmd: if key.cmd { on } else { mods.cmd },
        alt: if key.alt { on } else { mods.alt },
        ctrl: mods.ctrl,
    }
}
