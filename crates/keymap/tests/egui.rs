//! Integration tests of craft-keymap: the egui conversions.

use ::egui::Modifiers;
use craft_keymap::egui::matches;
use craft_keymap::{Chord, Mods, Platform};

fn chord(text: &str) -> Chord {
    Chord::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

#[test]
fn modifiers_convert_to_egui_and_back_on_every_platform() {
    let all = (0..16u8).map(|b| Mods { cmd: b & 1 != 0, ctrl: b & 2 != 0, alt: b & 4 != 0, shift: b & 8 != 0 });
    for mods in all {
        assert_eq!(Mods::from_egui(&mods.to_egui(Platform::Mac), Platform::Mac), mods, "{mods:?} on Mac");
        for platform in [Platform::Windows, Platform::Linux] {
            let folded = Mods { cmd: mods.cmd || mods.ctrl, ctrl: false, ..mods };
            assert_eq!(Mods::from_egui(&mods.to_egui(platform), platform), folded, "{mods:?} on {platform:?}");
        }
    }

    // What egui-winit reports for the Command key on macOS and the Ctrl key elsewhere.
    assert_eq!(Mods::CMD.to_egui(Platform::Mac), Modifiers { mac_cmd: true, command: true, ..Modifiers::NONE });
    assert_eq!(Mods::CTRL.to_egui(Platform::Mac), Modifiers { ctrl: true, ..Modifiers::NONE });
    let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    assert_eq!(Mods::CMD.to_egui(Platform::Windows), ctrl);
    assert_eq!(Mods::CTRL.to_egui(Platform::Linux), ctrl);
    let alt_shift = Mods { alt: true, shift: true, ..Mods::NONE };
    assert_eq!(alt_shift.to_egui(Platform::Windows), Modifiers { alt: true, shift: true, ..Modifiers::NONE });

    // A press built from a chord matches that chord on its platform.
    let presses = [("Cmd+Shift+Z", Platform::Mac), ("Ctrl+Alt+K", Platform::Mac), ("Cmd+S", Platform::Windows), ("Ctrl+S", Platform::Linux)];
    for (text, platform) in presses {
        let c = chord(text);
        assert!(matches(&c, c.key.to_egui(), &c.mods.to_egui(platform), platform), "{text} on {platform:?}");
    }
}
