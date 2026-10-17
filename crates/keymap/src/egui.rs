//! Conversions between the crate's keys and egui's, matching egui key presses to chords, and a
//! [`Dispatcher`] that turns a frame's key events into the commands they run.

use ::egui::{Event, InputState, Key as EguiKey, KeyboardShortcut, Modifiers};

use crate::chord::{Chord, Key, Mods, Platform};
use crate::command::CommandRef;
use crate::editor::DOUBLE_PRESS;
use crate::keymap::{Keymap, Lookup};
use crate::scope::Active;

// Both matches are generated from one list so that a key added to `Key` without an egui
// counterpart fails to compile.
macro_rules! egui_keys {
    ($($ours:ident = $theirs:ident),* ; $($same:ident),* $(,)?) => {
        impl Key {
            /// The key for an egui key. `None` for modifier keys, Copy, Cut, Paste, BrowserBack
            /// and IntlBackslash.
            pub fn from_egui(key: EguiKey) -> Option<Key> {
                match key {
                    $(EguiKey::$theirs => Some(Key::$ours),)*
                    $(EguiKey::$same => Some(Key::$same),)*
                    _ => None,
                }
            }

            pub fn to_egui(self) -> EguiKey {
                match self {
                    $(Key::$ours => EguiKey::$theirs,)*
                    $(Key::$same => EguiKey::$same,)*
                }
            }
        }
    };
}

egui_keys! {
    Up = ArrowUp, Down = ArrowDown, Left = ArrowLeft, Right = ArrowRight;
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20,
    F21, F22, F23, F24, F25, F26, F27, F28, F29, F30, F31, F32, F33, F34, F35,
    Escape, Tab, Backspace, Enter, Space, Insert, Delete, Home, End, PageUp, PageDown,
    Minus, Equals, Plus, OpenBracket, CloseBracket, Backslash, Semicolon, Quote, Backtick,
    Comma, Period, Slash, Colon, Pipe, Questionmark, Exclamationmark, OpenCurlyBracket, CloseCurlyBracket,
}

impl Mods {
    /// The modifiers egui reports, read for `platform`. On macOS cmd is ⌘ and ctrl the Control
    /// key; elsewhere Ctrl is cmd and ctrl stays false.
    pub fn from_egui(mods: &Modifiers, platform: Platform) -> Mods {
        if platform.is_mac() {
            // egui sets `command` together with `mac_cmd` on macOS; synthetic events may set
            // only one of them.
            Mods { cmd: mods.mac_cmd || mods.command, ctrl: mods.ctrl, alt: mods.alt, shift: mods.shift }
        } else {
            Mods { cmd: mods.ctrl || mods.command, ctrl: false, alt: mods.alt, shift: mods.shift }
        }
    }
}

impl Chord {
    /// The egui shortcut for this chord, for egui's own shortcut text and `consume_shortcut`.
    /// cmd becomes egui's `command` and ctrl its `ctrl`, which egui folds the same way off
    /// macOS.
    pub fn to_egui_shortcut(&self) -> KeyboardShortcut {
        let m = self.mods;
        KeyboardShortcut::new(Modifiers { alt: m.alt, ctrl: m.ctrl, shift: m.shift, mac_cmd: false, command: m.cmd }, self.key.to_egui())
    }
}

/// Base keys and the key egui reports for them with Shift held on a US layout.
const SHIFTED: &[(Key, Key)] = &[
    (Key::Semicolon, Key::Colon),
    (Key::Slash, Key::Questionmark),
    (Key::Backslash, Key::Pipe),
    (Key::OpenBracket, Key::OpenCurlyBracket),
    (Key::CloseBracket, Key::CloseCurlyBracket),
    (Key::Equals, Key::Plus),
    (Key::Num1, Key::Exclamationmark),
];

fn without_shift(m: Mods) -> Mods {
    Mods { shift: false, ..m }
}

/// Whether a key press egui reported triggers `binding` on `platform`.
///
/// Besides an exact match:
/// - Shift with a base key matches the shifted key egui reports (`Shift+;` matches `:`).
/// - A shifted symbol bound without Shift matches when Shift is held to type it (`?`).
/// - A binding on `=` matches `+` with or without Shift, and a binding on `+` matches
///   `Shift+=`, so zoom keys work with and without Shift.
pub fn matches(binding: &Chord, key: EguiKey, mods: &Modifiers, platform: Platform) -> bool {
    let Some(pressed) = Key::from_egui(key) else { return false };
    matches_press(binding, pressed, Mods::from_egui(mods, platform), platform)
}

fn matches_press(binding: &Chord, pressed: Key, held: Mods, platform: Platform) -> bool {
    let want = binding.effective(platform);
    if pressed == want.key && held == want.mods {
        return true;
    }
    let same_but_shift = without_shift(held) == without_shift(want.mods);
    if !same_but_shift {
        return false;
    }
    let shifted_of = |base: Key| SHIFTED.iter().find(|(b, _)| *b == base).map(|&(_, s)| s);
    let is_shifted_symbol = SHIFTED.iter().any(|&(_, s)| s == want.key);
    match (want.key, pressed) {
        (Key::Equals, Key::Plus) => !want.mods.shift || held.shift,
        (Key::Plus, Key::Equals) => held.shift && !want.mods.shift,
        (w, p) if w == p => is_shifted_symbol && !want.mods.shift && held.shift,
        (w, p) => want.mods.shift && held.shift && shifted_of(w) == Some(p),
    }
}

/// The chords a press can stand for, most specific first: the press itself, then those that keep
/// Shift (`Shift+;` for `:`), then those without it (`=` for `+`). Each one [`matches`] the press.
fn candidates(pressed: Key, held: Mods, platform: Platform) -> Vec<Chord> {
    let base = SHIFTED.iter().find(|&&(_, s)| s == pressed).map(|&(b, _)| b);
    let swap = match pressed {
        Key::Plus => Some(Key::Equals),
        Key::Equals => Some(Key::Plus),
        _ => None,
    };
    let mut out = Vec::with_capacity(6);
    for mods in [held, without_shift(held)] {
        for key in [Some(pressed), base, swap].into_iter().flatten() {
            let chord = Chord::new(mods, key);
            if !out.contains(&chord) && matches_press(&chord, pressed, held, platform) {
                out.push(chord);
            }
        }
    }
    out
}

/// A command a key press runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// The command to run; for tools sharing the chord, the first of `cycle`.
    pub command: CommandRef,
    /// Tools of one group sharing the chord, in catalogue order (the app picks the next one);
    /// empty for a plain command.
    pub cycle: Vec<CommandRef>,
}

impl Hit {
    fn from_lookup(lookup: Lookup) -> Option<Hit> {
        match lookup {
            Lookup::None => None,
            Lookup::Run(command) => Some(Hit { command, cycle: Vec::new() }),
            Lookup::Cycle(cycle) => Some(Hit { command: cycle.first()?.clone(), cycle }),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct LastPress {
    chord: Chord,
    at: f64,
    ran_double: bool,
}

/// Runs a keymap against egui's input, one call per frame before the UI reads its keys.
///
/// It reads key presses (repeats only when [`with_repeat`](Self::with_repeat) is on) and egui's
/// Copy, Cut and Paste events, which egui-winit sends in place of the Cmd+C, Cmd+X and Cmd+V
/// presses. A press that runs a command is removed from the events, together with the text it
/// typed when it has no command modifier, so no widget sees it too. Other events stay.
///
/// While [`Active::typing`], the text field keeps, without lookup:
/// - clipboard events, and the clipboard key presses the web build sends with them (Cmd+C, Cmd+X,
///   Cmd+V, Shift+Delete, Shift+Insert, Ctrl+Insert);
/// - a press followed by a Text event before the next press, since it typed that text (the web
///   build's text agent can send these);
/// - off macOS, a printable key (letter, digit, punctuation, Space) pressed with Ctrl and Alt.
///   Windows and some Linux setups report AltGr as Ctrl+Alt, so AltGr+Q (`@` on a German layout)
///   arrives as Ctrl+Alt+Q, and egui-winit sends no Text event while Ctrl is held, so the press
///   cannot be told from a command chord. Ctrl+Alt with other keys (F-keys, arrows) still runs.
#[derive(Clone, Debug, Default)]
pub struct Dispatcher {
    last: Option<LastPress>,
    repeat: bool,
}

impl Dispatcher {
    pub fn new() -> Dispatcher {
        Dispatcher::default()
    }

    /// Whether held keys run their command again on each repeat (off by default), so a held
    /// arrow can nudge. Repeats never count toward a double press.
    pub fn with_repeat(mut self, on: bool) -> Dispatcher {
        self.repeat = on;
        self
    }

    /// The commands this frame's key presses run with `active` focus, in press order.
    ///
    /// A chord pressed again within [`DOUBLE_PRESS`] runs its double binding when it has one;
    /// the first press has already run the single binding. The press after a double starts over.
    pub fn dispatch(&mut self, input: &mut InputState, keymap: &Keymap, active: &Active) -> Vec<Hit> {
        let platform = keymap.platform();
        let now = input.time;
        let frame_mods = Mods::from_egui(&input.modifiers, platform);
        let mut hits = Vec::new();
        let mut kept = Vec::with_capacity(input.events.len());
        // The Text event egui sends after a key press belongs to that press.
        let mut drop_text = false;
        // The web build sends both the Cmd+C key press and the Copy event it causes.
        let mut clipboard_key = None;
        let events = std::mem::take(&mut input.events);
        let typed = if active.typing { typed_text(&events) } else { Vec::new() };
        for (i, event) in events.into_iter().enumerate() {
            match &event {
                Event::Key { key, pressed: true, repeat, modifiers, .. } if !*repeat || self.repeat => {
                    drop_text = false;
                    let held = Mods::from_egui(modifiers, platform);
                    if let Some(key) = Key::from_egui(*key) {
                        if active.typing && (typed.get(i) == Some(&true) || is_clipboard_press(key, held) || may_be_alt_gr(key, held, platform)) {
                            kept.push(event);
                            continue;
                        }
                        if held.cmd && matches!(key, Key::C | Key::X | Key::V) {
                            clipboard_key = Some(key);
                        }
                        if let Some(hit) = self.press(key, held, *repeat, now, keymap, active) {
                            hits.push(hit);
                            drop_text = !(held.cmd || held.ctrl);
                            continue;
                        }
                    }
                }
                Event::Text(_) if drop_text => {
                    drop_text = false;
                    continue;
                }
                // A focused text field copies and pastes on these events itself.
                Event::Copy | Event::Cut | Event::Paste(_) if !active.typing => {
                    let (key, held) = clipboard_chord(&event, frame_mods);
                    if clipboard_key == Some(key) {
                        clipboard_key = None;
                    } else if let Some(hit) = self.press(key, held, false, now, keymap, active) {
                        hits.push(hit);
                        continue;
                    }
                }
                _ => {}
            }
            kept.push(event);
        }
        input.events = kept;
        hits
    }

    fn press(&mut self, key: Key, held: Mods, repeat: bool, now: f64, keymap: &Keymap, active: &Active) -> Option<Hit> {
        let chords = candidates(key, held, keymap.platform());
        let first = *chords.first()?;
        let find = |double: bool| chords.iter().find_map(|c| Hit::from_lookup(keymap.lookup(c, double, active)));
        if repeat {
            return find(false);
        }
        let again = self.last.is_some_and(|l| l.chord == first && !l.ran_double && now > l.at && now - l.at < DOUBLE_PRESS);
        let double = if again { find(true) } else { None };
        self.last = Some(LastPress { chord: first, at: now, ran_double: double.is_some() });
        double.or_else(|| find(false))
    }
}

/// For each event, whether it is a key press followed by a Text event before the next press.
fn typed_text(events: &[Event]) -> Vec<bool> {
    let mut out = vec![false; events.len()];
    let mut text_ahead = false;
    for (event, typed) in events.iter().zip(out.iter_mut()).rev() {
        match event {
            Event::Text(t) if !t.is_empty() => text_ahead = true,
            Event::Key { pressed: true, .. } => {
                *typed = text_ahead;
                text_ahead = false;
            }
            _ => {}
        }
    }
    out
}

/// Whether a press could be AltGr typing a character: Ctrl+Alt off macOS on a key that types.
fn may_be_alt_gr(key: Key, held: Mods, platform: Platform) -> bool {
    !platform.is_mac() && held.cmd && held.alt && types_text(key)
}

/// Letters, digits, punctuation and Space.
fn types_text(key: Key) -> bool {
    !key.is_function_key()
        && !matches!(
            key,
            Key::Escape
                | Key::Tab
                | Key::Backspace
                | Key::Enter
                | Key::Insert
                | Key::Delete
                | Key::Home
                | Key::End
                | Key::PageUp
                | Key::PageDown
                | Key::Up
                | Key::Down
                | Key::Left
                | Key::Right
        )
}

/// Whether a press is one a backend turns into a Copy, Cut or Paste event.
fn is_clipboard_press(key: Key, held: Mods) -> bool {
    match key {
        Key::C | Key::X | Key::V => held.cmd,
        Key::Delete => held.shift,
        Key::Insert => held.shift || held.cmd || held.ctrl,
        _ => false,
    }
}

/// The press a clipboard event stands for, with the modifiers held. Off macOS egui-winit also
/// sends Cut for Shift+Delete and Paste for Shift+Insert.
fn clipboard_chord(event: &Event, held: Mods) -> (Key, Mods) {
    let with_cmd = Mods { cmd: true, ..held };
    let shift_only = held.shift && !held.cmd;
    match event {
        Event::Cut if shift_only => (Key::Delete, held),
        Event::Cut => (Key::X, with_cmd),
        Event::Paste(_) if shift_only => (Key::Insert, held),
        Event::Paste(_) => (Key::V, with_cmd),
        _ => (Key::C, with_cmd),
    }
}
