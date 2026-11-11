//! Flag syntax (`FLAG` modes), exclusion sets for probes, and step counters for the simple-word checks.

use super::*;

/// U+FE0F VARIATION SELECTOR-16 asks for the emoji form of the character before it. A dictionary
/// that uses an emoji as a flag may write it after the flag (RLA-ES Spanish: `PFX ☎️`, `banco/S☎️`),
/// but it is not a flag of its own: it is dropped wherever flags are read.
pub(super) const EMOJI_SELECTOR: char = '\u{fe0f}';

/// A flag as written in the source, before it gets a dense id: a character (one-character mode),
/// two characters (`FLAG long`, the first in the high 32 bits) or a number (`FLAG num`).
pub(super) type FlagKey = u64;

/// How flags are written: the `.aff`'s `FLAG` line, also kept in the compact format. Every flag
/// field (`.dic` entries, `PFX`/`SFX` flags and continuation classes, `FORBIDDENWORD`…) is read
/// and written here, and nowhere else.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlagMode {
    /// One character per flag (no `FLAG` line, `FLAG UTF-8`, `FLAG DEFAULT`).
    #[default]
    Char,
    /// Two characters per flag (`FLAG long`): `Aa`, `B1`. Read as characters of the decoded
    /// text, where Hunspell reads two bytes: the same for the ASCII flags real dictionaries use,
    /// different only for a non-ASCII flag in a UTF-8 file.
    Long,
    /// Decimal numbers from 0 to 65,535 separated by commas (`FLAG num`): `12,104`. As in
    /// Hunspell, each number is the item's leading digits: Nepali writes `0/17X` for flag 17.
    Num,
}

impl FlagMode {
    /// The mode a `FLAG` line names, if this engine reads it.
    pub(super) fn from_name(name: &str) -> Option<FlagMode> {
        match name {
            "UTF-8" | "DEFAULT" | "utf-8" | "default" | "UTF8" | "" => Some(FlagMode::Char),
            "long" | "LONG" => Some(FlagMode::Long),
            "num" | "NUM" => Some(FlagMode::Num),
            _ => None,
        }
    }

    /// The `FLAG` value the compact format writes (one-character mode writes no line).
    pub(super) fn name(self) -> Option<&'static str> {
        match self {
            FlagMode::Char => None,
            FlagMode::Long => Some("long"),
            FlagMode::Num => Some("num"),
        }
    }

    /// The flags of a field (`.dic` flags, continuation classes), in source order, at most
    /// [`MAX_FLAGS_PER_ENTRY`]. An empty field has none.
    pub(super) fn parse(self, field: &str) -> Result<Vec<FlagKey>, String> {
        self.parse_at_most(field, MAX_FLAGS_PER_ENTRY, false)
    }

    /// The flags of a field, at most `cap` of them. `lenient` (a `.dic` entry's flags) reads
    /// malformed items as Hunspell does, keeping the word (checked black-box with the `hunspell`
    /// CLI): under `FLAG num` an item that is no number from 0 to 65,535 (empty, `x2`, `-7`,
    /// `70000`) is flag 0, and under `FLAG long` an odd last character is dropped. Otherwise
    /// (continuation classes, which Hunspell drops the rule for, and the compact format) they are
    /// an error.
    pub(super) fn parse_at_most(self, field: &str, cap: usize, lenient: bool) -> Result<Vec<FlagKey>, String> {
        let mut out = Vec::new();
        let mut push = |k: FlagKey| {
            if out.len() >= cap {
                return Err(format!("more than {cap} flags in `{}`", field.chars().take(40).collect::<String>()));
            }
            out.push(k);
            Ok(())
        };
        match self {
            FlagMode::Char => {
                for c in field.chars().filter(|&c| c != EMOJI_SELECTOR) {
                    push(FlagKey::from(c))?;
                }
            }
            FlagMode::Long => {
                let mut chars = field.chars();
                while let Some(a) = chars.next() {
                    let Some(b) = chars.next() else {
                        if lenient {
                            break;
                        }
                        return Err(format!("long flags `{field}` have an odd number of characters"));
                    };
                    push((FlagKey::from(a) << 32) | FlagKey::from(b))?;
                }
            }
            FlagMode::Num if field.is_empty() => {}
            FlagMode::Num => {
                for item in field.split(',') {
                    // The leading digits (no sign), 0 to 65,535: at most five digits. Hunspell
                    // reads flag 0 too, and the Turkish dictionary uses it (`SFX 0 N 1`).
                    // Leading zeros don't count (`000001` is 1, as in Hunspell).
                    let digits = item.bytes().take_while(u8::is_ascii_digit).count();
                    let d = item.get(..digits).unwrap_or_default().trim_start_matches('0');
                    let n = match d.len() {
                        _ if digits == 0 => None,
                        0 => Some(0),
                        1..=5 => d.parse::<u32>().ok().filter(|&n| n <= 0xffff),
                        _ => None,
                    };
                    match n {
                        Some(n) => push(FlagKey::from(n))?,
                        None if lenient => push(0)?,
                        None => return Err(format!("`{}` is not a flag number (0 to 65535)", item.chars().take(20).collect::<String>())),
                    }
                }
            }
        }
        Ok(out)
    }

    /// The one flag of a field (`PFX`/`SFX` flags, `FORBIDDENWORD`, `NOSUGGEST`…).
    pub(super) fn parse_one(self, field: &str) -> Result<FlagKey, String> {
        match self.parse(field)?.as_slice() {
            [k] => Ok(*k),
            _ => Err(format!("`{field}` is not one flag")),
        }
    }

    /// Append the flags `keys` to `out` as the source writes them.
    pub(super) fn write(self, keys: impl Iterator<Item = FlagKey>, out: &mut String) {
        let ch = |k: FlagKey| u32::try_from(k).ok().and_then(char::from_u32);
        for (i, k) in keys.enumerate() {
            match self {
                FlagMode::Char => out.extend(ch(k)),
                FlagMode::Long => {
                    out.extend(ch(k >> 32));
                    out.extend(ch(k & 0xffff_ffff));
                }
                FlagMode::Num => {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&k.to_string());
                }
            }
        }
    }
}

/// Stem flags that rule a lookup out: `FORBIDDENWORD` always, and per lookup `NOSUGGEST` and
/// `SUBSTANDARD` (suggestions), `KEEPCASE` (case variants of a word) or `NEEDAFFIX` (a stem on
/// its own).
/// The second field: no flag-based compounds; the third: a suggestion candidate, whose compound
/// search gets a small budget ([`compound::MAX_SUGGEST_WORK`]); the fourth: `KEEPCASE`, kept
/// apart since it holds for a compound's first part only (and may share its id with another).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Excluded([Option<u16>; 5], bool, pub(super) bool, Option<u16>);

impl Excluded {
    /// Does the flag set `set` carry an excluded flag?
    pub(super) fn hits(self, set: &[u16]) -> bool {
        self.0.iter().chain(std::iter::once(&self.3)).flatten().any(|f| has(set, *f))
    }

    /// These exclusions, and no flag-based compounds (`MAXCPDSUGS 0` for suggestions).
    pub(super) fn without_compounds(mut self) -> Excluded {
        self.1 = true;
        self
    }

    pub(super) fn no_compounds(self) -> bool {
        self.1
    }

    pub(super) fn suggesting(self) -> bool {
        self.2
    }

    /// These exclusions and `KEEPCASE` (`flag`): a case variant of the word is being checked.
    pub(super) fn keeping(mut self, flag: Option<u16>) -> Excluded {
        self.3 = flag;
        self
    }

    /// These exclusions without `KEEPCASE` (a compound part after the first).
    pub(super) fn without_keep(mut self) -> Excluded {
        self.3 = None;
        self
    }

    /// These exclusions and `flag` too.
    pub(super) fn and(mut self, flag: Option<u16>) -> Excluded {
        if flag.is_some()
            && let Some(slot) = self.0.iter_mut().find(|s| s.is_none())
        {
            *slot = flag;
        }
        self
    }
}

/// A sorted flag set to test other sets against: a bitmap over all flag ids when it is large,
/// so a test costs one probe per flag of the other set.
pub(super) enum FlagProbe<'a> {
    Few(&'a [u16]),
    Many(Vec<u64>),
}

impl<'a> FlagProbe<'a> {
    pub(super) fn new(flags: &'a [u16]) -> FlagProbe<'a> {
        if flags.len() <= 16 {
            return FlagProbe::Few(flags);
        }
        let mut bits = vec![0u64; (usize::from(flags.last().copied().unwrap_or(0)) >> 6) + 1];
        for &f in flags {
            if let Some(w) = bits.get_mut(usize::from(f) >> 6) {
                *w |= 1 << (f & 63);
            }
        }
        FlagProbe::Many(bits)
    }

    /// Does the sorted set `set` share a flag with the probe's?
    pub(super) fn meets(&self, set: &[u16]) -> bool {
        match self {
            FlagProbe::Few(flags) => intersects(set, flags),
            FlagProbe::Many(bits) => set.iter().any(|&f| bits.get(usize::from(f) >> 6).is_some_and(|w| w & (1 << (f & 63)) != 0)),
        }
    }
}

// ponytail: a thread-local rather than a parameter through every check path; the guard below
// restores it, so nested or concurrent calls on other threads don't share it.
thread_local! {
    /// Steps one suggestion call may still take in its simple-word checks, while it runs
    /// ([`SuggestSteps`]); `None` outside one.
    pub(super) static SUGGEST_STEPS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// While alive, the simple-word checks on this thread share [`MAX_SUGGEST_STEPS`] steps
/// (one suggestion call: thousands of candidates can't each spend a full check's budget).
pub(crate) struct SuggestSteps(Option<usize>);

impl SuggestSteps {
    pub(crate) fn start() -> SuggestSteps {
        SuggestSteps(SUGGEST_STEPS.with(|s| s.replace(Some(MAX_SUGGEST_STEPS))))
    }
}

// Dropping restores the outer budget as it was: the steps a nested call took are not charged to
// it. Harmless while suggestion calls don't nest; a nested call would have to charge the outer one.
impl Drop for SuggestSteps {
    fn drop(&mut self) {
        SUGGEST_STEPS.with(|s| s.set(self.0));
    }
}

/// Steps a word's check may still take ([`MAX_SIMPLE_WORK`] at first).
pub(super) struct Steps(pub(super) std::cell::Cell<usize>);

impl Steps {
    /// Pay for looking through a flag set of `len` flags: about [`FLAGS_PER_STEP`] flags a step.
    pub(super) fn scan(&self, len: usize) -> bool {
        self.spend(len / FLAGS_PER_STEP + 1)
    }

    /// Pay for a rule's condition check (a step per character class).
    pub(super) fn cond(&self, r: &Rule) -> bool {
        self.spend(r.cond.len())
    }

    /// Take `n` steps; false when they ran out.
    pub(super) fn spend(&self, n: usize) -> bool {
        let left = self.0.get();
        self.0.set(left.saturating_sub(n.max(1)));
        left > 0
    }
}
