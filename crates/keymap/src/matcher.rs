//! Matching one key press against a keymap: the chord candidates of the layout, the double-press pair and the
//! Alt variant. It reads no toolkit's events; craft-ui's key stage runs it.

use crate::DOUBLE_PRESS;
use crate::chord::{Chord, Key, Mods, Trigger};
use crate::command::CommandRef;
use crate::keymap::{Keymap, Lookup};
use crate::layout::{KeyInput, Layout};
use crate::scope::Active;

/// A command a key press runs.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Hit {
    /// The command to run; for tools sharing the chord, the first of `cycle`.
    pub command: CommandRef,
    /// Tools of one group sharing the chord, in catalog order (the app picks the next one);
    /// empty for a plain command.
    pub cycle: Vec<CommandRef>,
    /// The chord whose binding matched, with `double` set when the double binding ran.
    pub trigger: Trigger,
    /// Whether `trigger` is the chord the press means ([`Layout::resolve`]), the one the shortcut
    /// recorder records. `false` when the press matched through a stand-in: a Shift binding on the
    /// US base key (`Shift+;` for `:`), `+` and `=` standing for each other, or a digit-row
    /// position's digit.
    pub exact: bool,
    /// The press is a bound chord of `command` with Alt added, and `command` opted in
    /// ([`CommandDef::has_alt_variant`](crate::CommandDef::has_alt_variant)). `trigger` is the chord
    /// pressed, Alt included.
    pub alt_variant: bool,
}

impl Hit {
    pub(crate) fn from_lookup(lookup: Lookup, trigger: Trigger, exact: bool) -> Option<Hit> {
        let (command, cycle) = match lookup {
            Lookup::None => return None,
            Lookup::Run(command) => (command, Vec::new()),
            Lookup::Cycle(cycle) => (cycle.first()?.clone(), cycle),
        };
        Some(Hit { command, cycle, trigger, exact, alt_variant: false })
    }
}

#[derive(Clone, Debug)]
struct LastPress {
    chord: Chord,
    at: f64,
    ran_double: bool,
    /// The focused panel: a press in another panel never completes the pair.
    panel: Option<String>,
}

/// One key press to match.
#[derive(Clone, Copy, Debug)]
pub struct Press<'a> {
    /// What the toolkit reports of the key.
    pub input: &'a KeyInput,
    /// A repeat of a held key: it runs a single binding and never counts toward a double press.
    pub repeat: bool,
    /// The time of the press in seconds.
    pub now: f64,
    /// What has the keyboard.
    pub active: &'a Active<'a>,
    /// The panels around the focused one, innermost first ([`Keymap::lookup_nested`]).
    pub outer: &'a [&'a str],
}

/// The state that matching presses carries between them: the previous press, for double presses.
#[derive(Clone, Debug, Default)]
pub struct Matcher {
    last: Option<LastPress>,
}

impl Matcher {
    /// A matcher with Delete and Backspace standing for each other.
    pub fn new() -> Matcher {
        Matcher::default()
    }

    /// The command `press` runs, with `layout` reading the key.
    pub fn press(&mut self, layout: &Layout, keymap: &Keymap, press: &Press<'_>) -> Option<Hit> {
        let Press { input, repeat, now, active, outer } = *press;
        let chords = Chords::of(layout, input)?;
        if repeat {
            return chords.single(keymap, active, outer);
        }
        let first = chords.first();
        let again = self
            .last
            .as_ref()
            .is_some_and(|l| l.chord == first && !l.ran_double && l.panel.as_deref() == active.panel && now >= l.at && now - l.at < DOUBLE_PRESS);
        let double = if again { chords.find(keymap, true, active, outer) } else { None };
        self.last = Some(LastPress { chord: first, at: now, ran_double: double.is_some(), panel: active.panel.map(str::to_owned) });
        double.or_else(|| chords.single(keymap, active, outer))
    }

    /// The command a press of `input` runs on its own, as [`press`](Self::press) matches a press that completes no
    /// double press: for a key that is held down rather than pressed now (a key automation holds). Keeps no state.
    pub fn single(layout: &Layout, keymap: &Keymap, input: &KeyInput, active: &Active<'_>, outer: &[&str]) -> Option<Hit> {
        Chords::of(layout, input)?.single(keymap, active, outer)
    }
}

/// The chords one press can run, most specific first: the layout's candidates, then Delete and Backspace standing
/// for each other.
struct Chords(Vec<Chord>);

impl Chords {
    /// `None` when the press names no key.
    fn of(layout: &Layout, input: &KeyInput) -> Option<Chords> {
        let mut chords = layout.candidates(input);
        chords.first()?;
        let twins: Vec<Chord> = chords
            .iter()
            .filter_map(|c| match c.key {
                Key::Delete => Some(Chord::new(c.mods, Key::Backspace)),
                Key::Backspace => Some(Chord::new(c.mods, Key::Delete)),
                _ => None,
            })
            .collect();
        chords.extend(twins);
        Some(Chords(chords))
    }

    /// The chord the press means ([`Layout::resolve`]).
    fn first(&self) -> Chord {
        self.0[0]
    }

    /// The first chord with a binding, pressed twice when `double`.
    fn find(&self, keymap: &Keymap, double: bool, active: &Active<'_>, outer: &[&str]) -> Option<Hit> {
        let first = self.first();
        self.0
            .iter()
            .find_map(|&chord| Hit::from_lookup(keymap.lookup_nested(&chord, double, active, outer), Trigger { chord, double }, chord == first))
    }

    /// The single press's binding, else its Alt variant.
    fn single(&self, keymap: &Keymap, active: &Active<'_>, outer: &[&str]) -> Option<Hit> {
        self.find(keymap, false, active, outer).or_else(|| self.alt_variant(keymap, active, outer))
    }

    /// Only when no binding owns the Alt chord itself, so a bound Alt chord runs its own command.
    fn alt_variant(&self, keymap: &Keymap, active: &Active<'_>, outer: &[&str]) -> Option<Hit> {
        let (first, platform) = (self.first(), keymap.platform());
        self.0.iter().filter(|c| c.mods.alt).find_map(|&chord| {
            if active.typing && !keymap.typing_rule().fires(&chord, platform) {
                return None;
            }
            let plain = Chord::new(Mods { alt: false, ..chord.mods }, chord.key);
            match keymap.lookup_nested(&plain, false, &Active { typing: false, ..*active }, outer) {
                Lookup::Run(command) if keymap.bindable(&command).is_some_and(|b| b.has_alt_variant) => {
                    Some(Hit { command, cycle: Vec::new(), trigger: Trigger { chord, double: false }, exact: chord == first, alt_variant: true })
                }
                _ => None,
            }
        })
    }
}
