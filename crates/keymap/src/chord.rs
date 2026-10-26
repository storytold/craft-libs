//! Keys, modifiers, chords and their text forms.
//!
//! Canonical text is `Cmd+Ctrl+Alt+Shift+Key`, written by [`Chord`]'s `Display` and read back by
//! [`Chord::parse`]. A double press is the chord twice, separated by a space: `A A`.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::layout::Layout;

pub mod foreign;

/// The longest chord or trigger text [`Chord::parse`] and [`Trigger::parse`] accept, in bytes.
pub const MAX_CHORD_TEXT: usize = 64;

/// The platform a keymap is used on. It decides display text and whether `ctrl` folds into `cmd`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    /// The platform this build runs on. Web builds count as Linux; with the `egui` feature,
    /// `egui::platform` gives the browser's operating system.
    pub const fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }

    pub const fn is_mac(self) -> bool {
        matches!(self, Platform::Mac)
    }
}

macro_rules! keys {
    ($($variant:ident => $name:literal),* $(,)?) => {
        /// A key on the keyboard, independent of any UI toolkit.
        ///
        /// Digits are the main row; numeric keypad keys map to the same keys.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[non_exhaustive]
        pub enum Key {
            $($variant),*
        }

        impl Key {
            /// Every key, in declaration order.
            pub const ALL: &'static [Key] = &[$(Key::$variant),*];

            /// The canonical name: `K`, `5`, `F5`, `PageUp`, `;`.
            pub const fn name(self) -> &'static str {
                match self {
                    $(Key::$variant => $name),*
                }
            }

            // The Rust variant name, which is also egui's `Key` variant name except for arrows.
            const fn variant_name(self) -> &'static str {
                match self {
                    $(Key::$variant => stringify!($variant)),*
                }
            }
        }
    };
}

keys! {
    A => "A", B => "B", C => "C", D => "D", E => "E", F => "F", G => "G", H => "H", I => "I",
    J => "J", K => "K", L => "L", M => "M", N => "N", O => "O", P => "P", Q => "Q", R => "R",
    S => "S", T => "T", U => "U", V => "V", W => "W", X => "X", Y => "Y", Z => "Z",
    Num0 => "0", Num1 => "1", Num2 => "2", Num3 => "3", Num4 => "4",
    Num5 => "5", Num6 => "6", Num7 => "7", Num8 => "8", Num9 => "9",
    F1 => "F1", F2 => "F2", F3 => "F3", F4 => "F4", F5 => "F5", F6 => "F6", F7 => "F7",
    F8 => "F8", F9 => "F9", F10 => "F10", F11 => "F11", F12 => "F12", F13 => "F13",
    F14 => "F14", F15 => "F15", F16 => "F16", F17 => "F17", F18 => "F18", F19 => "F19",
    F20 => "F20", F21 => "F21", F22 => "F22", F23 => "F23", F24 => "F24", F25 => "F25",
    F26 => "F26", F27 => "F27", F28 => "F28", F29 => "F29", F30 => "F30", F31 => "F31",
    F32 => "F32", F33 => "F33", F34 => "F34", F35 => "F35",
    Escape => "Escape", Tab => "Tab", Backspace => "Backspace", Enter => "Enter",
    Space => "Space", Insert => "Insert", Delete => "Delete", Home => "Home", End => "End",
    PageUp => "PageUp", PageDown => "PageDown",
    Up => "Up", Down => "Down", Left => "Left", Right => "Right",
    Minus => "-", Equals => "=", Plus => "+", OpenBracket => "[", CloseBracket => "]",
    Backslash => "\\", Semicolon => ";", Quote => "'", Backtick => "`",
    Comma => ",", Period => ".", Slash => "/",
    Colon => ":", Pipe => "|", Questionmark => "?", Exclamationmark => "!",
    OpenCurlyBracket => "{", CloseCurlyBracket => "}",
}

/// Key aliases beyond the canonical and egui variant names. Matched without regard to ASCII case.
const KEY_ALIASES: &[(&str, Key)] = &[
    ("esc", Key::Escape),
    ("⎋", Key::Escape),
    ("return", Key::Enter),
    ("⏎", Key::Enter),
    ("↩", Key::Enter),
    ("del", Key::Delete),
    ("⌦", Key::Delete),
    ("⌫", Key::Backspace),
    ("⇥", Key::Tab),
    ("pgup", Key::PageUp),
    ("pgdn", Key::PageDown),
    ("page up", Key::PageUp),
    ("page down", Key::PageDown),
    ("arrowup", Key::Up),
    ("arrowdown", Key::Down),
    ("arrowleft", Key::Left),
    ("arrowright", Key::Right),
    ("↑", Key::Up),
    ("↓", Key::Down),
    ("←", Key::Left),
    ("→", Key::Right),
    ("spacebar", Key::Space),
    ("ins", Key::Insert),
    ("equal", Key::Equals),
    ("grave", Key::Backtick),
    ("backquote", Key::Backtick),
    ("apostrophe", Key::Quote),
    // The minus sign menus and docs print (U+2212), not the hyphen-minus the key types.
    ("−", Key::Minus),
];

impl Key {
    /// Reads a key name: canonical (`K`, `PageUp`, `;`), egui variant names (`ArrowUp`, `Num5`,
    /// `Semicolon`) and the aliases `esc`, `return`, `del`, `pgup`, `pgdn`, `spacebar`, `ins`,
    /// the Mac key glyphs and the minus sign `−`. Case does not matter.
    pub fn parse(name: &str) -> Option<Key> {
        Key::ALL
            .iter()
            .copied()
            .find(|k| k.name().eq_ignore_ascii_case(name) || k.variant_name().eq_ignore_ascii_case(name))
            .or_else(|| KEY_ALIASES.iter().find(|(alias, _)| alias.eq_ignore_ascii_case(name)).map(|&(_, k)| k))
    }

    /// `A` to `Z`.
    pub const fn is_letter(self) -> bool {
        matches!(
            self,
            Key::A
                | Key::B
                | Key::C
                | Key::D
                | Key::E
                | Key::F
                | Key::G
                | Key::H
                | Key::I
                | Key::J
                | Key::K
                | Key::L
                | Key::M
                | Key::N
                | Key::O
                | Key::P
                | Key::Q
                | Key::R
                | Key::S
                | Key::T
                | Key::U
                | Key::V
                | Key::W
                | Key::X
                | Key::Y
                | Key::Z
        )
    }

    /// `0` to `9`.
    pub const fn is_digit(self) -> bool {
        matches!(self, Key::Num0 | Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8 | Key::Num9)
    }

    /// Punctuation and symbol keys: `-`, `=`, `+`, brackets, `\`, `;`, `'`, `` ` ``, `,`, `.`,
    /// `/` and the shifted symbols `:`, `|`, `?`, `!`, `{`, `}`.
    pub const fn is_symbol(self) -> bool {
        matches!(
            self,
            Key::Minus
                | Key::Equals
                | Key::Plus
                | Key::OpenBracket
                | Key::CloseBracket
                | Key::Backslash
                | Key::Semicolon
                | Key::Quote
                | Key::Backtick
                | Key::Comma
                | Key::Period
                | Key::Slash
                | Key::Colon
                | Key::Pipe
                | Key::Questionmark
                | Key::Exclamationmark
                | Key::OpenCurlyBracket
                | Key::CloseCurlyBracket
        )
    }

    /// The letter, digit or symbol key for a typed character, ignoring case.
    pub fn from_char(c: char) -> Option<Key> {
        let mut buf = [0; 4];
        let text: &str = c.encode_utf8(&mut buf);
        Key::ALL.iter().copied().find(|k| (k.is_letter() || k.is_digit() || k.is_symbol()) && k.name().eq_ignore_ascii_case(text))
    }

    /// `F1` to `F35`.
    pub const fn is_function_key(self) -> bool {
        matches!(
            self,
            Key::F1
                | Key::F2
                | Key::F3
                | Key::F4
                | Key::F5
                | Key::F6
                | Key::F7
                | Key::F8
                | Key::F9
                | Key::F10
                | Key::F11
                | Key::F12
                | Key::F13
                | Key::F14
                | Key::F15
                | Key::F16
                | Key::F17
                | Key::F18
                | Key::F19
                | Key::F20
                | Key::F21
                | Key::F22
                | Key::F23
                | Key::F24
                | Key::F25
                | Key::F26
                | Key::F27
                | Key::F28
                | Key::F29
                | Key::F30
                | Key::F31
                | Key::F32
                | Key::F33
                | Key::F34
                | Key::F35
        )
    }

    /// The key as macOS menus show it: arrows and editing keys as glyphs.
    pub(crate) fn mac_symbol(self) -> &'static str {
        match self {
            Key::Up => "↑",
            Key::Down => "↓",
            Key::Left => "←",
            Key::Right => "→",
            Key::Enter => "⏎",
            Key::Backspace => "⌫",
            Key::Delete => "⌦",
            Key::Escape => "⎋",
            Key::Tab => "⇥",
            k => k.text_name(),
        }
    }

    /// The key as Windows and Linux show it.
    pub(crate) fn text_name(self) -> &'static str {
        match self {
            Key::PageUp => "Page Up",
            Key::PageDown => "Page Down",
            k => k.name(),
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl Serialize for Key {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Key, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        if text.len() > MAX_CHORD_TEXT {
            return Err(serde::de::Error::custom(ParseError::TooLong));
        }
        Key::parse(&text).ok_or_else(|| serde::de::Error::custom(ParseError::UnknownKey(text.into_owned())))
    }
}

/// Modifier keys held with a chord's key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Mods {
    /// The command modifier: ⌘ on macOS, Ctrl on Windows and Linux.
    pub cmd: bool,
    /// The Control key on macOS. Off macOS it folds into `cmd`.
    pub ctrl: bool,
    /// Alt, Option on macOS.
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    pub const NONE: Mods = Mods { cmd: false, ctrl: false, alt: false, shift: false };
    pub const CMD: Mods = Mods { cmd: true, ctrl: false, alt: false, shift: false };
    pub const CTRL: Mods = Mods { cmd: false, ctrl: true, alt: false, shift: false };
    pub const ALT: Mods = Mods { cmd: false, ctrl: false, alt: true, shift: false };
    pub const SHIFT: Mods = Mods { cmd: false, ctrl: false, alt: false, shift: true };

    pub const fn is_none(self) -> bool {
        !(self.cmd || self.ctrl || self.alt || self.shift)
    }

    /// The modifiers as `platform` sees them: off macOS `ctrl` becomes `cmd`.
    pub const fn effective(self, platform: Platform) -> Mods {
        if platform.is_mac() || !self.ctrl { self } else { Mods { cmd: true, ctrl: false, ..self } }
    }
}

/// A key with modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    pub mods: Mods,
    pub key: Key,
}

/// Why chord or trigger text was refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ParseError {
    Empty,
    /// Longer than [`MAX_CHORD_TEXT`].
    TooLong,
    /// Modifiers with no key, or a trailing `+`.
    MissingKey,
    UnknownKey(String),
    UnknownModifier(String),
    DuplicateModifier(String),
    /// Two different chords: key sequences are not supported.
    Sequence,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => f.write_str("empty shortcut"),
            ParseError::TooLong => write!(f, "shortcut longer than {MAX_CHORD_TEXT} bytes"),
            ParseError::MissingKey => f.write_str("shortcut has no key"),
            ParseError::UnknownKey(k) => write!(f, "unknown key `{k}`"),
            ParseError::UnknownModifier(m) => write!(f, "unknown modifier `{m}`"),
            ParseError::DuplicateModifier(m) => write!(f, "modifier `{m}` given twice"),
            ParseError::Sequence => f.write_str("key sequences are not supported"),
        }
    }
}

impl std::error::Error for ParseError {}

#[derive(Clone, Copy)]
enum Modifier {
    Cmd,
    Ctrl,
    Alt,
    Shift,
}

fn modifier_named(name: &str) -> Option<Modifier> {
    const NAMES: &[(&str, Modifier)] = &[
        ("cmd", Modifier::Cmd),
        ("command", Modifier::Cmd),
        ("meta", Modifier::Cmd),
        ("super", Modifier::Cmd),
        ("mod", Modifier::Cmd),
        ("⌘", Modifier::Cmd),
        ("ctrl", Modifier::Ctrl),
        ("control", Modifier::Ctrl),
        ("ctl", Modifier::Ctrl),
        ("⌃", Modifier::Ctrl),
        ("alt", Modifier::Alt),
        ("opt", Modifier::Alt),
        ("option", Modifier::Alt),
        ("⌥", Modifier::Alt),
        ("shift", Modifier::Shift),
        ("⇧", Modifier::Shift),
    ];
    NAMES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|&(_, m)| m)
}

/// A modifier glyph at the start of `text`, and the text after it.
fn strip_glyph(text: &str) -> Option<(Modifier, &str)> {
    let mut chars = text.chars();
    let m = match chars.next()? {
        '⌘' => Modifier::Cmd,
        '⌃' => Modifier::Ctrl,
        '⌥' => Modifier::Alt,
        '⇧' => Modifier::Shift,
        _ => return None,
    };
    Some((m, chars.as_str()))
}

fn add_modifier(mods: &mut Mods, m: Modifier, written: &str) -> Result<(), ParseError> {
    let slot = match m {
        Modifier::Cmd => &mut mods.cmd,
        Modifier::Ctrl => &mut mods.ctrl,
        Modifier::Alt => &mut mods.alt,
        Modifier::Shift => &mut mods.shift,
    };
    if *slot {
        return Err(ParseError::DuplicateModifier(written.to_owned()));
    }
    *slot = true;
    Ok(())
}

impl Chord {
    pub const fn new(mods: Mods, key: Key) -> Chord {
        Chord { mods, key }
    }

    /// The key with no modifiers.
    pub const fn plain(key: Key) -> Chord {
        Chord { mods: Mods::NONE, key }
    }

    /// Reads canonical text (`Cmd+Shift+Z`) and the common aliases: modifier words in any order
    /// and case (`cmd command meta super mod`, `ctrl control ctl`, `alt opt option`, `shift`),
    /// Mac glyphs, glued (`⇧⌘Z`) or with `+`, and the key names [`Key::parse`] reads. `Cmd++` is
    /// Cmd with the Plus key.
    pub fn parse(text: &str) -> Result<Chord, ParseError> {
        if text.len() > MAX_CHORD_TEXT {
            return Err(ParseError::TooLong);
        }
        let mut rest = text.trim();
        if rest.is_empty() {
            return Err(ParseError::Empty);
        }
        let mut mods = Mods::NONE;
        // Each pass consumes at least one byte, so the loop ends within MAX_CHORD_TEXT passes.
        loop {
            if let Some((m, after)) = strip_glyph(rest) {
                add_modifier(&mut mods, m, rest.get(..rest.len() - after.len()).unwrap_or_default())?;
                // `⌘+K` and `⌘K` are the same; in `⌘+` the `+` is the key.
                rest = match after.strip_prefix('+') {
                    Some(r) if !r.is_empty() => r,
                    _ => after,
                };
                if rest.trim().is_empty() {
                    return Err(ParseError::MissingKey);
                }
                continue;
            }
            // A `+` in first position is the Plus key, never a separator.
            let first = rest.chars().next().map_or(0, char::len_utf8);
            let separator = rest.get(first..).and_then(|r| r.find('+')).map(|i| i + first);
            match separator {
                Some(i) => {
                    let token = rest.get(..i).unwrap_or_default().trim();
                    let after = rest.get(i + 1..).unwrap_or_default();
                    if after.trim().is_empty() {
                        return Err(ParseError::MissingKey);
                    }
                    let m = modifier_named(token).ok_or_else(|| ParseError::UnknownModifier(token.to_owned()))?;
                    add_modifier(&mut mods, m, token)?;
                    rest = after;
                }
                None => {
                    let token = rest.trim();
                    if modifier_named(token).is_some() {
                        return Err(ParseError::MissingKey);
                    }
                    let key = Key::parse(token).ok_or_else(|| ParseError::UnknownKey(token.to_owned()))?;
                    return Ok(Chord { mods, key });
                }
            }
        }
    }

    /// Canonical text, the same as `to_string()`: `Cmd+Ctrl+Alt+Shift+Key`.
    pub fn portable(&self) -> String {
        self.to_string()
    }

    /// The chord as `platform` sees it: off macOS `ctrl` folds into `cmd`. Lookups and conflict
    /// checks compare effective chords.
    pub const fn effective(self, platform: Platform) -> Chord {
        Chord { mods: self.mods.effective(platform), key: self.key }
    }

    /// Text for menus and settings: `⌃⌥⇧⌘K` on macOS, `Ctrl+Alt+Shift+K` elsewhere.
    pub fn display(&self, platform: Platform) -> String {
        self.display_key(platform, None)
    }

    /// [`display`](Self::display) for the keys of `layout`: a punctuation key shows the character
    /// the layout types at its position when that character has no key of its own (`Ctrl+Ü` for
    /// Ctrl+[ on German). Letters and digits show as in `display`.
    pub fn display_for(&self, platform: Platform, layout: &Layout) -> String {
        self.display_key(platform, layout.key_label(self.key).as_deref())
    }

    fn display_key(&self, platform: Platform, key_text: Option<&str>) -> String {
        let m = self.mods.effective(platform);
        let mut s = String::new();
        if platform.is_mac() {
            for (on, glyph) in [(m.ctrl, "⌃"), (m.alt, "⌥"), (m.shift, "⇧"), (m.cmd, "⌘")] {
                if on {
                    s.push_str(glyph);
                }
            }
            s.push_str(key_text.unwrap_or(self.key.mac_symbol()));
        } else {
            for (on, name) in [(m.cmd, "Ctrl+"), (m.alt, "Alt+"), (m.shift, "Shift+")] {
                if on {
                    s.push_str(name);
                }
            }
            s.push_str(key_text.unwrap_or(self.key.text_name()));
        }
        s
    }

    pub const fn is_function_key(&self) -> bool {
        self.key.is_function_key()
    }

    /// No cmd, ctrl or alt. Shift alone still counts as plain: such chords are tool keys that
    /// typing in a text field would trigger.
    pub const fn is_plain(&self) -> bool {
        !(self.mods.cmd || self.mods.ctrl || self.mods.alt)
    }

    /// Holds the command modifier: cmd, or ctrl (Control on macOS, folded into cmd elsewhere).
    pub const fn has_command(&self) -> bool {
        self.mods.cmd || self.mods.ctrl
    }
}

impl From<Key> for Chord {
    fn from(key: Key) -> Chord {
        Chord::plain(key)
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [(self.mods.cmd, "Cmd+"), (self.mods.ctrl, "Ctrl+"), (self.mods.alt, "Alt+"), (self.mods.shift, "Shift+")] {
            if on {
                f.write_str(name)?;
            }
        }
        f.write_str(self.key.name())
    }
}

impl std::str::FromStr for Chord {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Chord, ParseError> {
        Chord::parse(s)
    }
}

impl Serialize for Chord {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Chord {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Chord, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        Chord::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// What starts a command: a chord, pressed once or twice in a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Trigger {
    pub chord: Chord,
    /// The chord pressed twice.
    pub double: bool,
}

impl Trigger {
    pub const fn single(chord: Chord) -> Trigger {
        Trigger { chord, double: false }
    }

    pub const fn double(chord: Chord) -> Trigger {
        Trigger { chord, double: true }
    }

    /// Reads a chord as [`Chord::parse`] does, or the same chord twice separated by whitespace
    /// (`A A`) as a double press. Two different chords are [`ParseError::Sequence`].
    pub fn parse(text: &str) -> Result<Trigger, ParseError> {
        if text.len() > MAX_CHORD_TEXT {
            return Err(ParseError::TooLong);
        }
        let single = match Chord::parse(text) {
            Ok(chord) => return Ok(Trigger::single(chord)),
            Err(e) => e,
        };
        let mut parts = text.split_whitespace();
        match (parts.next(), parts.next(), parts.next()) {
            (Some(first), Some(second), None) => {
                let (first, second) = (Chord::parse(first)?, Chord::parse(second)?);
                if first == second { Ok(Trigger::double(first)) } else { Err(ParseError::Sequence) }
            }
            _ => Err(single),
        }
    }

    /// Canonical text, the same as `to_string()`.
    pub fn portable(&self) -> String {
        self.to_string()
    }

    pub const fn effective(self, platform: Platform) -> Trigger {
        Trigger { chord: self.chord.effective(platform), double: self.double }
    }

    /// [`Chord::display`], twice with a space between for a double press.
    pub fn display(&self, platform: Platform) -> String {
        let one = self.chord.display(platform);
        if self.double { format!("{one} {one}") } else { one }
    }

    /// [`Chord::display_for`], twice with a space between for a double press.
    pub fn display_for(&self, platform: Platform, layout: &Layout) -> String {
        let one = self.chord.display_for(platform, layout);
        if self.double { format!("{one} {one}") } else { one }
    }
}

impl From<Chord> for Trigger {
    fn from(chord: Chord) -> Trigger {
        Trigger::single(chord)
    }
}

impl fmt::Display for Trigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.double { write!(f, "{0} {0}", self.chord) } else { write!(f, "{}", self.chord) }
    }
}

impl std::str::FromStr for Trigger {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Trigger, ParseError> {
        Trigger::parse(s)
    }
}

impl Serialize for Trigger {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Trigger {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Trigger, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        Trigger::parse(&text).map_err(serde::de::Error::custom)
    }
}
