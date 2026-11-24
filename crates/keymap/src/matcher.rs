//! Matching one key press against a keymap: the chord candidates of the layout, the double-press pair and the
//! Alt variant. It reads no toolkit's events; craft-ui's key stage runs it.

use crate::DOUBLE_PRESS;
use crate::chord::{Chord, Key, Mods, PressKind, Trigger};
use crate::command::CommandRef;
use crate::keymap::{DeleteKeys, Keymap, Lookup};
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
    /// A matcher with no previous press.
    pub fn new() -> Matcher {
        Matcher::default()
    }

    /// The command `press` runs, with `layout` reading the key.
    pub fn press(&mut self, layout: &Layout, keymap: &Keymap, press: &Press<'_>) -> Option<Hit> {
        self.press_where(layout, keymap, press, &|_| true)
    }

    /// [`press`](Self::press) among the commands `accept` takes ([`Keymap::lookup_where`]): a binding of a refused
    /// command (one disabled where the press lands) is passed over, and the press matches the next binding, in an outer
    /// scope or of its next chord candidate, as if the refused one were not bound.
    pub fn press_where(&mut self, layout: &Layout, keymap: &Keymap, press: &Press<'_>, accept: &dyn Fn(&CommandRef) -> bool) -> Option<Hit> {
        let Press { input, repeat, now, active, outer } = *press;
        let chords = Chords::of(layout, keymap, input)?;
        let m = Match { keymap, active, outer, accept };
        if repeat {
            return chords.single(&m);
        }
        let first = chords.first();
        let again = self
            .last
            .as_ref()
            .is_some_and(|l| l.chord == first && !l.ran_double && l.panel.as_deref() == active.panel && now >= l.at && now - l.at < DOUBLE_PRESS);
        let double = if again { chords.find(&m, true) } else { None };
        self.last = Some(LastPress { chord: first, at: now, ran_double: double.is_some(), panel: active.panel.map(str::to_owned) });
        double.or_else(|| chords.single(&m))
    }

    /// The command a press of `input` runs on its own, as [`press`](Self::press) matches a press that completes no
    /// double press: for a key that is held down rather than pressed now (a key automation holds). Keeps no state.
    pub fn single(layout: &Layout, keymap: &Keymap, input: &KeyInput, active: &Active<'_>, outer: &[&str]) -> Option<Hit> {
        Chords::of(layout, keymap, input)?.single(&Match { keymap, active, outer, accept: &|_| true })
    }

    /// The command a hold of `input` runs: for a key that stays down past the hold time. Keeps no state.
    // PLAN: `Keymap::hold_of` ranks without the outer panels, so `outer` has no effect yet.
    pub fn hold(layout: &Layout, keymap: &Keymap, input: &KeyInput, active: &Active<'_>, _outer: &[&str]) -> Option<Hit> {
        Chords::of(layout, keymap, input)?.hold(keymap, active)
    }
}

/// What one press is matched against: the keymap, what has the keyboard, the panels around it, and the commands the
/// press may run.
#[derive(Clone, Copy)]
struct Match<'a> {
    keymap: &'a Keymap,
    active: &'a Active<'a>,
    outer: &'a [&'a str],
    accept: &'a dyn Fn(&CommandRef) -> bool,
}

/// The chords one press can run, most specific first: the chord with the Shift the layout dropped kept (rule 3's
/// exception), the layout's candidates, then, where the keymap takes them as one key ([`DeleteKeys::OneKey`]), Delete
/// and Backspace standing for each other.
struct Chords(Vec<Chord>, Chord);

impl Chords {
    /// `None` when the press names no key.
    fn of(layout: &Layout, keymap: &Keymap, input: &KeyInput) -> Option<Chords> {
        let mut chords = layout.candidates(input);
        let meant = chords.first().copied()?;
        // Rule 3's exception (`layout` docs): where the layout dropped the Shift the person held for a symbol that
        // needs it, the chord with Shift kept is tried first, so its binding wins over the dropped chord's. The press
        // still means the dropped chord (a double press, `Hit::exact`).
        if input.mods.shift && !meant.mods.shift && meant.key.is_symbol() {
            let kept = Chord::new(input.mods, meant.key);
            chords.retain(|chord| *chord != kept);
            chords.insert(0, kept);
        }
        if keymap.delete_keys() == DeleteKeys::OneKey {
            let twins: Vec<Chord> = chords
                .iter()
                .filter_map(|c| match c.key {
                    Key::Delete => Some(Chord::new(c.mods, Key::Backspace)),
                    Key::Backspace => Some(Chord::new(c.mods, Key::Delete)),
                    _ => None,
                })
                .collect();
            chords.extend(twins);
        }
        Some(Chords(chords, meant))
    }

    /// The chord the press means ([`Layout::resolve`]).
    fn first(&self) -> Chord {
        self.1
    }

    /// The first chord with a binding, pressed twice when `double`.
    fn find(&self, m: &Match<'_>, double: bool) -> Option<Hit> {
        let first = self.first();
        self.0.iter().find_map(|&chord| {
            Hit::from_lookup(
                m.keymap.lookup_where(&chord, double, m.active, m.outer, m.accept),
                Trigger { chord, press: if double { PressKind::Double } else { PressKind::Single } },
                chord == first,
            )
        })
    }

    /// The first chord a hold is bound to.
    fn hold(&self, keymap: &Keymap, active: &Active<'_>) -> Option<Hit> {
        let first = self.first();
        self.0.iter().find_map(|&chord| {
            let command = keymap.hold_of(&chord, active)?;
            Some(Hit { command, cycle: Vec::new(), trigger: Trigger::hold(chord), exact: chord == first, alt_variant: false })
        })
    }

    /// The single press's binding, else its Alt variant.
    fn single(&self, m: &Match<'_>) -> Option<Hit> {
        self.find(m, false).or_else(|| self.alt_variant(m))
    }

    /// Only when no binding owns the Alt chord itself, so a bound Alt chord runs its own command.
    fn alt_variant(&self, m: &Match<'_>) -> Option<Hit> {
        let Match { keymap, active, outer, accept } = *m;
        let (first, platform) = (self.first(), keymap.platform());
        self.0.iter().filter(|c| c.mods.alt).find_map(|&chord| {
            if active.typing && !keymap.typing_rule().fires(&chord, platform) {
                return None;
            }
            let plain = Chord::new(Mods { alt: false, ..chord.mods }, chord.key);
            match keymap.lookup_where(&plain, false, &Active { typing: false, ..*active }, outer, accept) {
                Lookup::Run(command) if keymap.bindable(&command).is_some_and(|b| b.has_alt_variant) => Some(Hit {
                    command,
                    cycle: Vec::new(),
                    trigger: Trigger { chord, press: PressKind::Single },
                    exact: chord == first,
                    alt_variant: true,
                }),
                _ => None,
            }
        })
    }
}
