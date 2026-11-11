//! Integration tests of craft-keymap: chord.

use craft_keymap::{Chord, Key, Mods, Platform, Trigger};

fn chord(text: &str) -> Chord {
    Chord::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn mods(cmd: bool, ctrl: bool, alt: bool, shift: bool) -> Mods {
    Mods { cmd, ctrl, alt, shift }
}

#[test]
fn canonical_text_round_trips_for_every_key_and_modifier_set() {
    for &key in Key::ALL {
        for bits in 0..16u8 {
            let c = Chord::new(mods(bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0), key);
            let text = c.to_string();
            assert_eq!(Chord::parse(&text), Ok(c), "{text}");
            assert_eq!(Trigger::parse(&format!("{text} {text}")), Ok(Trigger::double(c)), "{text} double");
            let json = serde_json::to_string(&Trigger::single(c)).unwrap();
            assert_eq!(serde_json::from_str::<Trigger>(&json).unwrap(), Trigger::single(c));
            // macOS display text reads back as the same chord.
            assert_eq!(Chord::parse(&c.display(Platform::Mac)), Ok(c), "{}", c.display(Platform::Mac));
        }
    }
    assert_eq!(chord("Cmd+Ctrl+Alt+Shift+K").to_string(), "Cmd+Ctrl+Alt+Shift+K");
    assert_eq!(serde_json::to_string(&chord("shift+cmd+k")).unwrap(), "\"Cmd+Shift+K\"");
}
