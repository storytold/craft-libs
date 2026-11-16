//! Android's key codes and meta state in egui's terms. Pure tables, so they are tested on any host.

use egui::{Key, Modifiers};

/// `KeyEvent.META_SHIFT_ON`.
const META_SHIFT_ON: i32 = 0x1;
/// `KeyEvent.META_ALT_ON`.
const META_ALT_ON: i32 = 0x2;
/// `KeyEvent.META_CTRL_ON`.
const META_CTRL_ON: i32 = 0x1000;
/// `KeyEvent.META_META_ON` (the Windows / Search / Command key).
const META_META_ON: i32 = 0x10000;

/// The modifiers a `KeyEvent.getMetaState()` holds. Control is the command key, as on every
/// non-Apple desktop egui runs on; the Meta key counts too, as keyboards made for Chromebooks and
/// iPads put it where Command is.
pub(crate) fn modifiers(meta: i32) -> Modifiers {
    let ctrl = meta & META_CTRL_ON != 0;
    let command = ctrl || meta & META_META_ON != 0;
    Modifiers { alt: meta & META_ALT_ON != 0, ctrl, shift: meta & META_SHIFT_ON != 0, mac_cmd: false, command }
}

/// The egui key for an Android `KeyEvent` key code; `None` for keys egui has no name for (the
/// system's own: Back, volume, media), which the shell leaves to Android.
pub(crate) fn key(code: i32) -> Option<Key> {
    const LETTERS: [Key; 26] = [
        Key::A,
        Key::B,
        Key::C,
        Key::D,
        Key::E,
        Key::F,
        Key::G,
        Key::H,
        Key::I,
        Key::J,
        Key::K,
        Key::L,
        Key::M,
        Key::N,
        Key::O,
        Key::P,
        Key::Q,
        Key::R,
        Key::S,
        Key::T,
        Key::U,
        Key::V,
        Key::W,
        Key::X,
        Key::Y,
        Key::Z,
    ];
    const DIGITS: [Key; 10] = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
    const FUNCTION: [Key; 12] = [Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6, Key::F7, Key::F8, Key::F9, Key::F10, Key::F11, Key::F12];
    let from = |table: &[Key], first: i32| usize::try_from(code - first).ok().and_then(|i| table.get(i).copied());
    Some(match code {
        // KEYCODE_0..KEYCODE_9, KEYCODE_NUMPAD_0..KEYCODE_NUMPAD_9.
        7..=16 => from(&DIGITS, 7)?,
        144..=153 => from(&DIGITS, 144)?,
        // KEYCODE_A..KEYCODE_Z.
        29..=54 => from(&LETTERS, 29)?,
        // KEYCODE_F1..KEYCODE_F12.
        131..=142 => from(&FUNCTION, 131)?,
        19 => Key::ArrowUp,
        20 => Key::ArrowDown,
        21 => Key::ArrowLeft,
        22 => Key::ArrowRight,
        55 => Key::Comma,
        56 | 158 => Key::Period,
        61 => Key::Tab,
        62 => Key::Space,
        66 | 160 => Key::Enter,
        67 => Key::Backspace,
        68 => Key::Backtick,
        69 | 156 => Key::Minus,
        70 | 161 => Key::Equals,
        71 => Key::OpenBracket,
        72 => Key::CloseBracket,
        73 => Key::Backslash,
        74 => Key::Semicolon,
        75 => Key::Quote,
        76 | 154 => Key::Slash,
        81 | 157 => Key::Plus,
        92 => Key::PageUp,
        93 => Key::PageDown,
        111 => Key::Escape,
        112 => Key::Delete,
        122 => Key::Home,
        123 => Key::End,
        124 => Key::Insert,
        277 => Key::Cut,
        278 => Key::Copy,
        279 => Key::Paste,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_digits_and_function_keys_line_up() {
        assert_eq!(key(29), Some(Key::A));
        assert_eq!(key(54), Some(Key::Z));
        assert_eq!(key(7), Some(Key::Num0));
        assert_eq!(key(16), Some(Key::Num9));
        assert_eq!(key(153), Some(Key::Num9), "the keypad's digits are digits");
        assert_eq!(key(131), Some(Key::F1));
        assert_eq!(key(142), Some(Key::F12));
    }

    #[test]
    fn editing_keys_have_their_egui_names() {
        assert_eq!(key(67).map(Key::name), Some("Backspace"), "KEYCODE_DEL is Backspace");
        assert_eq!(key(112).map(Key::name), Some("Delete"), "KEYCODE_FORWARD_DEL is Delete");
        assert_eq!(key(66).map(Key::name), Some("Enter"));
        assert_eq!(key(160).map(Key::name), Some("Enter"));
    }

    /// Back, volume and the like stay Android's.
    #[test]
    fn system_keys_are_left_to_android() {
        for code in [0, 3, 4, 24, 25, 82, 300] {
            assert_eq!(key(code), None, "{code}");
        }
    }

    /// The name crosses to the host as text: every key must come back as itself.
    #[test]
    fn every_key_survives_its_name() {
        for code in 0..400 {
            if let Some(k) = key(code) {
                assert_eq!(Key::from_name(k.name()), Some(k), "{code}");
            }
        }
    }

    #[test]
    fn control_and_meta_are_command() {
        assert_eq!(modifiers(0), Modifiers::NONE);
        let ctrl = modifiers(META_CTRL_ON);
        assert!(ctrl.ctrl && ctrl.command && !ctrl.mac_cmd);
        let meta = modifiers(META_META_ON | META_SHIFT_ON);
        assert!(meta.command && meta.shift && !meta.ctrl);
        assert!(modifiers(META_ALT_ON).alt);
    }
}
