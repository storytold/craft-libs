//! Shortcuts as the menus write them, and the egui key presses they stand for.

use egui::{Key, Modifiers};

/// A shortcut as written in an [`crate::Item`] (`Cmd+Shift+Z`). `Cmd` is ⌘ and `Ctrl` the Control
/// key, never folded together: ⌃⌘M and ⌘M are different shortcuts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The key as written: `Z`, `1`, `'`, `F5`, `Delete`, `Enter`.
    pub key: String,
}

impl Chord {
    pub fn parse(s: &str) -> Option<Chord> {
        let mut c = Chord { cmd: false, ctrl: false, alt: false, shift: false, key: String::new() };
        for part in s.split('+') {
            match part {
                "Cmd" => c.cmd = true,
                "Ctrl" => c.ctrl = true,
                "Alt" => c.alt = true,
                "Shift" => c.shift = true,
                // `Cmd++` splits into an empty last part.
                "" => c.key = "+".into(),
                k if c.key.is_empty() => c.key = k.into(),
                _ => return None,
            }
        }
        (!c.key.is_empty()).then_some(c)
    }

    /// The written form, modifiers in a fixed order: `Ctrl+Cmd+Alt+Shift+Key`.
    pub fn portable(&self) -> String {
        let mut parts = Vec::new();
        for (on, name) in [(self.ctrl, "Ctrl"), (self.cmd, "Cmd"), (self.alt, "Alt"), (self.shift, "Shift")] {
            if on {
                parts.push(name);
            }
        }
        parts.push(&self.key);
        parts.join("+")
    }

    pub fn is_function_key(&self) -> bool {
        self.key.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=24).contains(&n))
    }

    /// May the native menu show this as a key equivalent? Only with ⌘ or ⌃, or a function key.
    /// Bare letters and ⌥/⇧-only keys are tool keys: as key equivalents AppKit would take a `b`
    /// typed into a text field.
    pub fn native_ok(&self) -> bool {
        self.cmd || self.ctrl || self.is_function_key()
    }

    /// The key press egui would have received for this chord.
    pub fn key_event(&self, pressed: bool) -> Option<egui::Event> {
        let key = match self.key.as_str() {
            "+" => Key::Plus,
            "'" => Key::Quote,
            ";" => Key::Semicolon,
            "=" => Key::Equals,
            "-" => Key::Minus,
            "[" => Key::OpenBracket,
            "]" => Key::CloseBracket,
            "\\" => Key::Backslash,
            "/" => Key::Slash,
            "," => Key::Comma,
            "." => Key::Period,
            k => Key::from_name(k)?,
        };
        let modifiers = Modifiers { alt: self.alt, ctrl: self.ctrl, shift: self.shift, mac_cmd: self.cmd, command: self.cmd };
        Some(egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers })
    }
}
