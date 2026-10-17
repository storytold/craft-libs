//! Key text as other applications write it in their shortcut files and menus.
//!
//! Windows sets name the command modifier `Ctrl`; Mac sets name it `Cmd` and use `Ctrl` for the
//! Control key. [`read_foreign`] reads either into a [`Trigger`], tolerating separators
//! (`+`, `-`, space), spelled-out key names (`Page Down`, `Up Arrow`) and numeric keypad keys.

use serde::{Deserialize, Serialize};

use super::{Chord, Key, MAX_CHORD_TEXT, Mods, Trigger};

/// Which modifier naming a shortcut set uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Flavor {
    /// `Ctrl` is the command modifier.
    Windows,
    /// `Cmd` is the command modifier and `Ctrl` the Control key.
    Mac,
}

impl Flavor {
    /// Mac when any text names cmd, command, opt or option, or uses ⌘ or ⌥; Windows otherwise.
    pub fn detect<I, S>(texts: I) -> Flavor
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mac = texts.into_iter().any(|t| {
            let t = t.as_ref();
            t.contains(['⌘', '⌥']) || {
                let lower = t.to_ascii_lowercase();
                lower.contains("cmd") || lower.contains("command") || lower.contains("opt")
            }
        });
        if mac { Flavor::Mac } else { Flavor::Windows }
    }
}

/// Something [`read_foreign`] changed while reading that an import report may mention.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ReadNote {
    /// A numeric keypad key was read as the main key of the same name.
    Numpad,
}

/// Why foreign key text could not be read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ForeignError {
    Empty,
    /// Longer than [`MAX_CHORD_TEXT`].
    TooLong,
    UnknownKey(String),
    /// A modifier with no counterpart here, such as the Windows key.
    UnsupportedModifier(String),
}

impl std::fmt::Display for ForeignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ForeignError::Empty => f.write_str("empty shortcut"),
            ForeignError::TooLong => write!(f, "shortcut longer than {MAX_CHORD_TEXT} bytes"),
            ForeignError::UnknownKey(k) => write!(f, "unknown key `{k}`"),
            ForeignError::UnsupportedModifier(m) => write!(f, "unsupported modifier `{m}`"),
        }
    }
}

impl std::error::Error for ForeignError {}

#[derive(Clone, Copy)]
enum Word {
    Command,
    Control,
    Alt,
    Shift,
    Unsupported,
}

const GLYPHS: &[(char, Word)] = &[('⌘', Word::Command), ('⌃', Word::Control), ('⌥', Word::Alt), ('⇧', Word::Shift)];

// Longer words first where one is a prefix of another.
const WORDS: &[(&str, Word)] = &[
    ("command", Word::Command),
    ("cmd", Word::Command),
    ("control", Word::Control),
    ("ctrl", Word::Control),
    ("option", Word::Alt),
    ("opt", Word::Alt),
    ("alt", Word::Alt),
    ("shift", Word::Shift),
    ("windows", Word::Unsupported),
    ("win", Word::Unsupported),
    ("meta", Word::Unsupported),
    ("super", Word::Unsupported),
];

/// Key names some applications use that [`Key::parse`] does not read. Compared after removing
/// whitespace and lower-casing.
const FOREIGN_KEYS: &[(&str, Key)] = &[
    ("uparrow", Key::Up),
    ("downarrow", Key::Down),
    ("leftarrow", Key::Left),
    ("rightarrow", Key::Right),
    ("pageup", Key::PageUp),
    ("pagedown", Key::PageDown),
    ("⇞", Key::PageUp),
    ("⇟", Key::PageDown),
    ("↖", Key::Home),
    ("↘", Key::End),
    ("⌤", Key::Enter),
    ("forwarddelete", Key::Delete),
    ("help", Key::Insert),
];

/// Reads one shortcut as another application writes it: `Shift+Ctrl+S`, `Ctrl+Num 5`,
/// `Page Down`, `⇧⌘S`, `Opt-Left`. `flavor` decides what `Ctrl` means. The notes say what was
/// approximated.
pub fn read_foreign(text: &str, flavor: Flavor) -> Result<(Trigger, Vec<ReadNote>), ForeignError> {
    if text.len() > MAX_CHORD_TEXT {
        return Err(ForeignError::TooLong);
    }
    let mut rest = text.trim();
    if rest.is_empty() {
        return Err(ForeignError::Empty);
    }
    let mut mods = Mods::NONE;
    // Each pass consumes at least one byte.
    'strip: loop {
        for &(glyph, word) in GLYPHS {
            if let Some(after) = rest.strip_prefix(glyph) {
                apply(&mut mods, word, flavor, rest.get(..glyph.len_utf8()).unwrap_or_default())?;
                let after = after.trim_start();
                rest = match after.strip_prefix('+') {
                    Some(r) if !r.trim().is_empty() => r.trim_start(),
                    _ => after,
                };
                continue 'strip;
            }
        }
        for &(name, word) in WORDS {
            let (Some(head), Some(after)) = (rest.get(..name.len()), rest.get(name.len()..)) else { continue };
            // A modifier word needs a separator and a key after it: `Alt` alone is a key name.
            let mut chars = after.chars();
            if head.eq_ignore_ascii_case(name) && matches!(chars.next(), Some('+' | '-' | ' ')) && !chars.as_str().trim().is_empty() {
                apply(&mut mods, word, flavor, head)?;
                rest = chars.as_str().trim_start();
                continue 'strip;
            }
        }
        break;
    }
    let (key, numpad) = foreign_key(rest).ok_or_else(|| ForeignError::UnknownKey(rest.trim().to_owned()))?;
    let notes = if numpad { vec![ReadNote::Numpad] } else { Vec::new() };
    Ok((Trigger::single(Chord::new(mods, key)), notes))
}

fn apply(mods: &mut Mods, word: Word, flavor: Flavor, written: &str) -> Result<(), ForeignError> {
    match word {
        Word::Command => mods.cmd = true,
        Word::Control if flavor == Flavor::Mac => mods.ctrl = true,
        Word::Control => mods.cmd = true,
        Word::Alt => mods.alt = true,
        Word::Shift => mods.shift = true,
        Word::Unsupported => return Err(ForeignError::UnsupportedModifier(written.to_owned())),
    }
    Ok(())
}

/// The key a foreign key name stands for, and whether it was on the numeric keypad.
fn foreign_key(name: &str) -> Option<(Key, bool)> {
    let squashed: String = name.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    if squashed.is_empty() {
        return None;
    }
    // egui reports keypad keys as the main keys, so `Num 5` binds `5`.
    let keypad = squashed.strip_prefix("numpad").or_else(|| squashed.strip_prefix("num")).filter(|r| !r.is_empty() && *r != "lock");
    let (n, numpad) = match keypad {
        Some(r) => (r, true),
        None => (squashed.as_str(), false),
    };
    let key = FOREIGN_KEYS.iter().find(|(alias, _)| *alias == n).map(|&(_, k)| k).or_else(|| Key::parse(n))?;
    Some((key, numpad))
}
