//! Which chord a key press means on the active keyboard layout, and the layout's own characters
//! for labels.
//!
//! One rule decides matching ([`Layout::candidates`]), recording ([`Layout::resolve`]) and labels
//! ([`Chord::display_for`]):
//! 1. **Letters A–Z:** the layout's Latin letter for the key, from [`KeyInput::latin`] when the
//!    platform reports the Latin layout's character: on Dvorak + Russian the `с` key is Dvorak's
//!    J, and the `й` key, `'` on Dvorak, runs no letter binding. Without a report, on a layout
//!    that is not Latin, the letter the toolkit reports as typed, else the key's position (the
//!    Russian `с` key is C, the Hebrew `/` key is Q); on a Latin layout (Dvorak, AZERTY), the
//!    letter typed.
//! 2. **Digits:** the digit typed, else the key's position (AZERTY's `&é"` row types no digit
//!    without Shift). A digit-row press that types punctuation also matches its position's digit,
//!    after the punctuation: AZERTY's `-` key runs Ctrl+- and, when that has no binding, Ctrl+6.
//! 3. **Punctuation:** the character typed, without Shift when the layout needs Shift for it
//!    (German `/` is ⇧7, so ⇧⌘7 is ⌘/; US `:` is ⇧;, so ⇧; is `:`). A Shift binding on a US
//!    base key also matches the same press (`Shift+;` matches `:`), and `=` and `+` stand for
//!    each other so zoom keys work with and without Shift. egui has no key for the characters the
//!    ISO key next to left Shift types (`<` and `>` on Turkish Q) and reports both as that key,
//!    `IntlBackslash`, so Shift stays in the chord: ⇧ on that key is `Shift+IntlBackslash`.
//! 4. **Whether a layout is Latin** is decided for the whole layout ([`Layout::is_latin`]), never
//!    per key: Dvorak's `'` key on the Q position stays `'`.
//! 5. **No position fallback for unnamed characters on moved keys:** egui reports a character
//!    that has no key (Dvorak's ⇧, types `<`) as the key's position, W. With Shift held on a
//!    Latin layout, such a press at a letter position that has been seen typing something other
//!    than its letter resolves to nothing, so Ctrl+Shift+, never runs Ctrl+Shift+W once a `,`
//!    has been typed. A change in the reported Latin status, or [`Layout::forget_letters`],
//!    clears what the positions typed.
//! 6. **Labels** show letters and digits by name, and a punctuation key as the character the
//!    layout types at its position when that character has no key of its own: Ctrl+[ is `Ctrl+Ü`
//!    on German and `Ctrl+Х` on Russian.
//! 7. **Symbol table and bracket fallback** ([`Layout::adjust`]): German Ö and # stand for `[` and
//!    `]`, and the keys at the US bracket positions are `[` and `]` on other layouts until a real
//!    bracket or a table symbol shows otherwise. Cmd chords on Period and Comma follow the key's
//!    position. No other position falls back.
//!
//! [`KeyInput`] carries what the toolkit reports for one press; toolkits that learn more from the
//! platform fill more of it. [`Layout`] holds what is known about the layout itself, reported by
//! the platform or learned from what plain typing produces ([`Layout::observe`]).

use std::collections::BTreeMap;

use crate::chord::{Chord, Key, Mods, Platform};

/// What the toolkit reports about one key press.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeyInput {
    /// The modifiers held, read for the platform as [`Mods`] describes.
    pub mods: Mods,
    /// The key for the character the layout typed with `mods` held, when there is one (egui's
    /// logical key).
    pub typed: Option<Key>,
    /// The key at the same position on a US QWERTY keyboard (winit's physical key, the web's
    /// `code`).
    pub physical: Option<Key>,
    /// The character this key types on the layout the platform takes shortcut letters from, when
    /// it reports one: the layout's ⌘ table on macOS, the first Latin layout group on Linux. A
    /// letter makes the press that letter; any other character means the key has no Latin letter,
    /// so the press runs no letter binding (Dvorak's `'` on the key Russian types `й` on is no
    /// Ctrl+Q).
    pub latin: Option<char>,
    /// Whether the layout needs Shift to type `typed`, when the platform reports it. Otherwise
    /// Shift counts as needed when `typed` differs from `physical`, and without `physical` when
    /// `typed` is a shifted symbol on a US layout (`:`, `?`, `+`). Only punctuation drops Shift.
    pub shift_needed: Option<bool>,
}

impl KeyInput {
    /// A press with `mods` and nothing else known about the key.
    pub const fn new(mods: Mods) -> KeyInput {
        KeyInput { mods, typed: None, physical: None, latin: None, shift_needed: None }
    }

    /// Sets [`typed`](Self::typed).
    pub const fn with_typed(mut self, key: Key) -> KeyInput {
        self.typed = Some(key);
        self
    }

    /// Sets [`physical`](Self::physical).
    pub const fn with_physical(mut self, key: Key) -> KeyInput {
        self.physical = Some(key);
        self
    }

    /// Sets [`latin`](Self::latin).
    pub const fn with_latin(mut self, c: char) -> KeyInput {
        self.latin = Some(c);
        self
    }

    /// Sets [`shift_needed`](Self::shift_needed).
    pub const fn with_shift_needed(mut self, needed: bool) -> KeyInput {
        self.shift_needed = Some(needed);
        self
    }

    fn needs_shift(&self, typed: Key) -> bool {
        // A shifted symbol is no key position (egui's numeric keypad Plus is the one physical
        // key among them), so it says nothing about the Shift level.
        self.shift_needed.unwrap_or(match self.physical {
            Some(physical) if us_base(physical).is_none() => physical != typed,
            _ => us_base(typed).is_some(),
        })
    }
}

/// The active keyboard layout, as far as the platform reports it or typing has shown it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    reported_latin: Option<bool>,
    seen_latin: Option<bool>,
    /// The character typed without modifiers at each punctuation position.
    chars: BTreeMap<Key, char>,
    /// The character, lower case, typed without Cmd, Ctrl or Alt at each letter position.
    letters: BTreeMap<Key, char>,
    /// See [`Layout::with_position_fallback`].
    pub(crate) position_fallback: bool,
    /// A real bracket was typed ([`Layout::adjust`]): the bracket fallback would only steal keys.
    brackets: bool,
    /// A [`SYMBOL_TABLE`] layout's own symbol was seen: its keys are the brush keys, not the
    /// bracket fallback's.
    table_layout: bool,
}

impl Default for Layout {
    fn default() -> Layout {
        Layout {
            reported_latin: None,
            seen_latin: None,
            chars: BTreeMap::new(),
            letters: BTreeMap::new(),
            position_fallback: true,
            brackets: false,
            table_layout: false,
        }
    }
}

/// US base keys and the key egui reports for them with Shift held.
const SHIFTED: &[(Key, Key)] = &[
    (Key::Semicolon, Key::Colon),
    (Key::Slash, Key::Questionmark),
    (Key::Backslash, Key::Pipe),
    (Key::OpenBracket, Key::OpenCurlyBracket),
    (Key::CloseBracket, Key::CloseCurlyBracket),
    (Key::Equals, Key::Plus),
    (Key::Num1, Key::Exclamationmark),
];

/// One keyboard layout's symbol keys, for [`Layout::adjust`].
#[derive(Clone, Copy, Debug)]
pub struct SymbolLayout {
    /// `(text typed, Shift held, key the press stands for)`.
    pub keys: &'static [(&'static str, bool, Key)],
}

/// Rows valid on every layout: on layouts with a key of its own for `+` (German, Italian,
/// Spanish...) Shift+`+` types `*` and Shift+`-` types `_`, which egui has no keys for, so they
/// arrive as the physical `]` and `/`. They become `+` and `-`, as Shift+`=` arrives as `+` on a
/// US layout. A US Shift+`]` still arrives as `}` and Shift+8 (`*` too) as a digit.
pub const COMMON_SYMBOLS: SymbolLayout = SymbolLayout { keys: &[("*", true, Key::Plus), ("_", true, Key::Minus)] };

/// Layouts with application-specific bracket keys. German: Ö and # are `[` and `]`, and Shift+Ö
/// and Shift+# the same keys with Shift; `ö` and `#` arrive as the physical `;` and `\`, and
/// Shift+# types `'`. Other layouts work through the bracket fallback.
pub const SYMBOL_TABLE: &[SymbolLayout] = &[SymbolLayout {
    keys: &[("ö", false, Key::OpenBracket), ("Ö", true, Key::OpenBracket), ("#", false, Key::CloseBracket), ("'", true, Key::CloseBracket)],
}];

fn lookup_symbol(layout: &SymbolLayout, text: &str, shift: bool) -> Option<Key> {
    layout.keys.iter().find(|(t, s, _)| *t == text && *s == shift).map(|r| r.2)
}

fn us_base(shifted: Key) -> Option<Key> {
    SHIFTED.iter().find(|&&(_, s)| s == shifted).map(|&(b, _)| b)
}

const fn without_shift(m: Mods) -> Mods {
    Mods { shift: false, ..m }
}

/// Letters of the Latin script: ASCII, Latin-1, Latin Extended-A and -B, Latin Extended
/// Additional.
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && matches!(c, 'A'..='Z' | 'a'..='z' | '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}')
}

impl Layout {
    /// Nothing known yet: the layout counts as Latin until the platform or typing says otherwise.
    pub fn new() -> Layout {
        Layout::default()
    }

    /// Whether the layout is Latin, as the platform reports it.
    pub fn with_latin(mut self, latin: bool) -> Layout {
        self.reported_latin = Some(latin);
        self
    }

    /// Whether the layout is Latin, as the platform reports it; `None` when it cannot tell, so
    /// that typing decides. Call again when the user switches layouts. A report that differs
    /// from the last one means another layout, so what letter positions typed is forgotten
    /// ([`forget_letters`](Self::forget_letters)).
    pub fn set_latin(&mut self, latin: Option<bool>) {
        if latin != self.reported_latin {
            self.forget_letters();
        }
        self.reported_latin = latin;
    }

    /// Forgets what each letter position has been seen typing, for an app that learns of a
    /// layout switch the platform does not report: until typing shows the new layout, Shift
    /// presses at letter positions resolve as their letters again (rule 5).
    pub fn forget_letters(&mut self) {
        self.letters.clear();
    }

    /// Whether letters are read as typed (rule 1): what the platform reported, else what a letter
    /// key typed last, else true.
    pub fn is_latin(&self) -> bool {
        self.reported_latin.or(self.seen_latin).unwrap_or(true)
    }

    /// The character the layout types without modifiers at the punctuation position `physical`.
    pub fn with_char(mut self, physical: Key, typed: char) -> Layout {
        self.set_char(physical, typed);
        self
    }

    /// See [`with_char`](Self::with_char). Positions other than punctuation are ignored.
    pub fn set_char(&mut self, physical: Key, typed: char) {
        if physical.is_symbol() {
            self.chars.insert(physical, typed);
        }
    }

    /// The character known to be typed without modifiers at the punctuation position `physical`.
    pub fn char_at(&self, physical: Key) -> Option<char> {
        self.chars.get(&physical).copied()
    }

    /// Learns from a press at `physical` with `mods` held that typed `text`: a letter typed at a
    /// letter position tells whether the layout is Latin, and a character typed without
    /// modifiers at a punctuation position is that position's label. Presses with Cmd, Ctrl or
    /// Alt, and text longer than one character, teach nothing.
    pub fn observe(&mut self, physical: Key, mods: Mods, text: &str) {
        let mut chars = text.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else { return };
        if mods.cmd || mods.ctrl || mods.alt {
            return;
        }
        if physical.is_letter() {
            // ª, µ and º are Latin-1 symbols on Latin layouts, no sign of another script.
            if c.is_alphabetic() && !matches!(c, 'ª' | 'µ' | 'º') {
                self.seen_latin = Some(is_latin_letter(c));
            }
            if !c.is_control() {
                self.letters.insert(physical, c.to_lowercase().next().unwrap_or(c));
            }
        } else if !mods.shift && !c.is_control() {
            self.set_char(physical, c);
        }
    }

    /// The chord a press means, as the shortcut recorder records it and lookups try first.
    /// `None` when the press names no key.
    pub fn resolve(&self, input: &KeyInput) -> Option<Chord> {
        let held = input.mods;
        let latin = input.latin.map(|c| Key::from_char(c).filter(|k| k.is_letter()));
        if let Some(Some(letter)) = latin {
            return Some(Chord::new(held, letter));
        }
        // A letter typed on a layout that is not Latin is the toolkit's letter for shortcuts, or
        // egui standing in the position for a letter it has no key for.
        if latin.is_none()
            && !self.is_latin()
            && let Some(letter) = input.typed.filter(|k| k.is_letter()).or(input.physical).filter(|k| k.is_letter())
        {
            return Some(Chord::new(held, letter));
        }
        if held.shift
            && let Some(position) = input.physical.filter(|k| k.is_letter())
            && input.typed.is_none_or(|typed| typed == position)
            && !self.types_own_letter(position)
        {
            return None;
        }
        let mut key = input.typed.or(input.physical)?;
        // The Latin layout has no letter here, so a letter egui reports is its position: the
        // press is the Latin layout's key, or nothing.
        if let (Some(c), true) = (input.latin, key.is_letter()) {
            key = Key::from_char(c)?;
        }
        let drop_shift = held.shift && key.is_symbol() && input.needs_shift(key);
        Some(Chord::new(if drop_shift { without_shift(held) } else { held }, key))
    }

    /// Rule 5: whether a press reported as the letter at its own position can be that letter:
    /// no only when the position has been seen typing something else, so the press likely typed
    /// a character egui has no key for.
    fn types_own_letter(&self, position: Key) -> bool {
        self.letters.get(&position).is_none_or(|&c| Key::from_char(c) == Some(position))
    }

    /// The chords a press can run, most specific first: [`resolve`](Self::resolve)'s chord, the
    /// same press written as Shift with a US base key (`Shift+;` for `:`), `=` for `+` and `+`
    /// for Shift+`=`, and on the digit row the position's digit.
    pub fn candidates(&self, input: &KeyInput) -> Vec<Chord> {
        let Some(first) = self.resolve(input) else { return Vec::new() };
        let mut out = Vec::with_capacity(4);
        let mut push = |chord: Chord| {
            if !out.contains(&chord) {
                out.push(chord);
            }
        };
        push(first);
        if input.mods.shift
            && !first.mods.shift
            && let Some(base) = us_base(first.key)
        {
            push(Chord::new(input.mods, base));
        }
        match first.key {
            Key::Plus => push(Chord::new(first.mods, Key::Equals)),
            Key::Equals if first.mods.shift => push(Chord::new(without_shift(first.mods), Key::Plus)),
            _ => {}
        }
        if let Some(digit) = input.physical.filter(|k| k.is_digit())
            && first.key.is_symbol()
        {
            push(Chord::new(input.mods, digit));
        }
        // Shift that typed the character (German ⇧ß is `?`) says nothing about the US key. Only
        // the bracket positions fall back, and not once brackets or a table layout were seen.
        if self.bracket_fallback()
            && first.mods.shift == input.mods.shift
            && let Some(position @ (Key::OpenBracket | Key::CloseBracket)) = input.physical
        {
            push(Chord::new(input.mods, position));
        }
        out
    }

    /// Whether the keys at the US bracket positions stand for the brackets when they type
    /// something else: no real bracket was typed and no [`SYMBOL_TABLE`] layout was seen.
    fn bracket_fallback(&self) -> bool {
        self.position_fallback && !self.brackets && !self.table_layout
    }

    /// Gives a press the key the layout's symbol table and the bracket fallback assign, before
    /// it is matched. `text` is what the press typed (empty when unknown). Learns from the press:
    /// a typed bracket or a [`SYMBOL_TABLE`] symbol switches the bracket fallback off.
    ///
    /// - Cmd chords on the Period and Comma keys follow the key's position, as in Photoshop:
    ///   Shift+Cmd+. is Shift+Cmd+> on a US layout, but types `:` on Nordic and German ones.
    /// - [`COMMON_SYMBOLS`] and [`SYMBOL_TABLE`] rows apply to a press whose typed key egui took
    ///   from the physical key (it had no name for the symbol) or from that very text, and never
    ///   to a digit. A table row also sets the physical key, so Shift stays in the chord
    ///   (Shift+# is Shift+`]`).
    /// - Otherwise the keys at the US `[` and `]` positions are `[` and `]` while the fallback is
    ///   on (see [`with_position_fallback`](Self::with_position_fallback)).
    ///
    /// Presses with Ctrl or Alt held, and Cmd presses other than those on Period and Comma, are
    /// returned unchanged.
    pub fn adjust(&mut self, input: KeyInput, text: &str) -> KeyInput {
        let mut input = input;
        if input.mods.cmd
            && let Some(p @ (Key::Period | Key::Comma)) = input.physical
        {
            input.typed = Some(p);
            return input;
        }
        if input.mods.cmd || input.mods.ctrl || input.mods.alt {
            return input;
        }
        if matches!(text, "[" | "]" | "{" | "}") {
            self.brackets = true;
            return input;
        }
        let mut chars = text.chars();
        let typed_key = match (chars.next(), chars.next()) {
            (Some(c), None) => Key::from_char(c),
            _ => None,
        };
        let from_physical = input.typed.is_some() && (input.typed == input.physical || typed_key == input.typed);
        if !text.is_empty() && from_physical && input.physical.is_some() && !input.typed.is_some_and(Key::is_digit) {
            let shift = input.mods.shift;
            if let Some(k) = lookup_symbol(&COMMON_SYMBOLS, text, shift) {
                input.typed = Some(k);
                return input;
            }
            if let Some(k) = SYMBOL_TABLE.iter().find_map(|l| lookup_symbol(l, text, shift)) {
                self.table_layout = true;
                input.typed = Some(k);
                input.physical = Some(k);
                return input;
            }
        }
        if self.bracket_fallback()
            && let Some(p @ (Key::OpenBracket | Key::CloseBracket)) = input.physical
        {
            input.typed = Some(p);
        }
        input
    }

    /// Whether a press runs `binding` on `platform`: one of its [`candidates`](Self::candidates)
    /// is the binding.
    pub fn matches(&self, binding: &Chord, input: &KeyInput, platform: Platform) -> bool {
        let want = binding.effective(platform);
        self.candidates(input).iter().any(|c| c.effective(platform) == want)
    }

    /// Rule 6: the layout's character for the punctuation key `key`, when pressing that position
    /// means `key` and the layout types `key`'s own character nowhere else that is known.
    pub(crate) fn key_label(&self, key: Key) -> Option<String> {
        let typed = self.char_at(key)?;
        if Key::from_char(typed).is_some() {
            return None;
        }
        // `IntlBackslash` types no character of its own, so only a one-character name is checked.
        let mut name = key.name().chars();
        if let (Some(own), None) = (name.next(), name.next())
            && self.chars.values().any(|&c| c == own)
        {
            return None;
        }
        let mut upper = typed.to_uppercase();
        Some(match (upper.next(), upper.next()) {
            (Some(u), None) => u.to_string(),
            _ => typed.to_string(),
        })
    }
}
