//! Flag-based compounding, as the hunspell(5) man page describes it and the `hunspell` 1.7.5 CLI
//! answers on synthetic dictionaries (black-box; every rule below has a test pinned to the CLI's
//! answer in the tests below).
//!
//! A compound is two or more parts, each at least `COMPOUNDMIN` characters, each a stem or an
//! affixed form of one whose flags (the stem's, and the continuation classes of its affixes)
//! allow its place: `COMPOUNDFLAG` anywhere, `COMPOUNDBEGIN` first, `COMPOUNDMIDDLE` inside,
//! `COMPOUNDEND` last. (`COMPOUNDFIRST` and `COMPOUNDLAST` are ignored, as the CLI does.)
//!
//! - A prefix may sit on any part but the last and a suffix only on the last, unless the affix
//!   carries `COMPOUNDPERMITFLAG`. Two suffixes in a row (continuation classes) only on the last.
//! - An affix carrying `ONLYINCOMPOUND` is only valid inside a compound: a prefix anywhere, a
//!   suffix anywhere but on the last part (the CLI's answers; the word alone is no word).
//! - `COMPOUNDFORBIDFLAG` on a stem or a suffix: that part may only be last; on a prefix: no part.
//! - A stem with `NEEDAFFIX` is a part only with an affix; `FORBIDDENWORD` stems are no part, and
//!   a forbidden word is never rebuilt from parts.
//! - `COMPOUNDWORDMAX`: most parts (a `COMPOUNDROOT` stem counts as two). With `COMPOUNDSYLLABLE`
//!   and `LANG hu`, more parts are allowed while the word has at most that many vowels; with
//!   `COMPOUNDSYLLABLE` and another language the CLI applies no limit, and neither does this.
//! - `CHECKCOMPOUNDDUP`: the last two parts may not be the same stem (`foofoo`; `foofoobar` is
//!   fine, as in the CLI).
//! - `CHECKCOMPOUNDCASE`: no capital on either side of a boundary, unless the other side is `-`.
//! - `CHECKCOMPOUNDTRIPLE`: no three equal letters across a boundary (`schiff|fahrt`).
//!   `SIMPLIFIEDTRIPLE`: a part ending in a double letter may share its last letter with the next
//!   part (`schiffahrt` = `schiff` + `fahrt`), but with `CHECKCOMPOUNDTRIPLE` not leave three
//!   equal letters (`busssen`).
//! - `CHECKCOMPOUNDPATTERN end[/flag] begin[/flag]`: no boundary where the part before ends with
//!   `end` and the part after begins with `begin` (and they carry the flags); `0` for `end` means
//!   a part without a suffix. The optional replacement field is refused.
//! - `CHECKCOMPOUNDREP`: no compound that a `REP` replacement turns into a word (not a compound),
//!   checked on every two neighbouring parts and on every run of parts that ends the word.
//! - A compound of three or more parts is rejected when one slip turns it into a word (not a
//!   compound): see [`AffixDict::near_word`]. The CLI's answers, with or without
//!   `CHECKCOMPOUNDREP`.
//! - `FORCEUCASE` on the last part's stem: a word with no capital is wrong.
//! - Case: `KEEPCASE` holds for the first part only; a capitalized stem may be a part, and a word
//!   with capitals inside may start with a capital (`FooQux`).
//! - `ONLYINCOMPOUND` homonyms beside others keep their own flags (compound parts only).
//! - `MAXCPDSUGS 0`: compounds are never suggested.
//!
//! Not read: `SYLLABLENUM` (Hungarian), which the man page doesn't describe; it is ignored.
//!
//! The work per word is bounded: words up to [`MAX_WORD_BYTES`](super::MAX_WORD_BYTES), at most
//! [`MAX_PARTS`] parts, each part's analyses computed once, and at most [`MAX_WORK`] steps (rules
//! tried and splits followed) per word; past that the word is no compound.

use super::{AffixDict, Excluded, FlagKey, FlagMode, Rule, has};

/// Most parts a compound may have, whatever `COMPOUNDWORDMAX` says.
pub(super) const MAX_PARTS: usize = 16;
/// Most steps one compound check may take (affix rules tried, parts looked up, splits followed).
pub(super) const MAX_WORK: usize = 1 << 16;
/// Most steps the compound search of one suggestion candidate may take.
pub(crate) const MAX_SUGGEST_WORK: usize = 1 << 13;
/// Most steps the compound searches of one suggestion call may take in all; the candidates
/// left are checked as simple words only, so a long garbled word's suggestions stay quick.
pub(crate) const MAX_SUGGEST_COMPOUND_WORK: usize = 1 << 17;
/// Most `CHECKCOMPOUNDPATTERN` lines (the Dutch dictionary has 52).
const MAX_PATTERNS: usize = 512;
/// Longest text on either side of a `CHECKCOMPOUNDPATTERN`, in bytes.
const MAX_PATTERN_BYTES: usize = 64;
/// Most vowels a `COMPOUNDSYLLABLE` line may list.
const MAX_VOWELS: usize = 64;
/// Most ways one piece of a word may be a part (distinct flags and places).
const MAX_INFOS: usize = 256;
/// Most `MAP` variants, `TRY` letters and `KEY` row letters the three-part check tries.
const MAX_MAP_VARIANTS: usize = 256;
const MAX_TRY: usize = 128;
const MAX_KEY_ROW: usize = 128;
/// The keyboard rows the CLI assumes when a dictionary has no `KEY`.
const DEFAULT_KEY: &str = "qwertyuiop|asdfghjkl|zxcvbnm";

/// The part may come first, in the middle, last.
const FIRST: u8 = 1;
const MIDDLE: u8 = 2;
const LAST: u8 = 4;

/// The single-flag compounding directives, in the order [`Raw::flags`] keeps them.
const FLAG_DIRECTIVES: [&str; 8] =
    ["COMPOUNDFLAG", "COMPOUNDBEGIN", "COMPOUNDMIDDLE", "COMPOUNDEND", "COMPOUNDPERMITFLAG", "COMPOUNDFORBIDFLAG", "COMPOUNDROOT", "FORCEUCASE"];
const FLAG: usize = 0;
const BEGIN: usize = 1;
const MIDDLE_FLAG: usize = 2;
const END: usize = 3;
const PERMIT: usize = 4;
const FORBID: usize = 5;
const ROOT: usize = 6;
const FORCE_UCASE: usize = 7;

/// The yes/no compounding directives, in the order [`Raw::switches`] keeps them.
const SWITCHES: [&str; 5] = ["CHECKCOMPOUNDDUP", "CHECKCOMPOUNDREP", "CHECKCOMPOUNDCASE", "CHECKCOMPOUNDTRIPLE", "SIMPLIFIEDTRIPLE"];
const DUP: usize = 0;
const REP: usize = 1;
const CASE: usize = 2;
const TRIPLE: usize = 3;
const SIMPLIFIED: usize = 4;

/// Compounding directives as read from the `.aff` or the compact format, before flags get ids.
#[derive(Clone, Debug, Default)]
pub(super) struct Raw {
    flags: [Option<FlagKey>; 8],
    switches: [bool; 5],
    word_max: Option<usize>,
    /// `COMPOUNDSYLLABLE`: most vowels, and the vowels.
    syllable: Option<(usize, String)>,
    /// `LANG hu…`: the Hungarian syllable rule applies.
    hungarian: bool,
    /// `CHECKCOMPOUNDPATTERN` lines, as written (`end[/flag] begin[/flag]`).
    patterns: Vec<(String, String)>,
    pattern_header: bool,
    /// `MAXCPDSUGS 0`.
    no_suggestions: bool,
    /// `NOSPLITSUGS`.
    no_split_suggestions: bool,
    /// `COMPOUNDMORESUFFIXES`: refused with flag-based compounding, which it would change;
    /// ignored without (Korean has it beside `COMPOUNDRULE`s only).
    more_suffixes: bool,
}

impl Raw {
    /// Read the compounding directive `dir` of the line `f` (its fields); `Ok(false)` when `dir`
    /// is none. `one_flag` reads a flag in the dictionary's `FLAG` mode.
    pub(super) fn read(&mut self, f: &[&str], one_flag: &mut impl FnMut(&str) -> Result<FlagKey, String>) -> Result<bool, String> {
        let Some(&dir) = f.first() else { return Ok(false) };
        let at = |k: usize| f.get(k).copied().unwrap_or("");
        if let Some(i) = FLAG_DIRECTIVES.iter().position(|&d| d == dir) {
            let k = one_flag(at(1)).map_err(|e| format!("{dir} needs one flag: {e}"))?;
            if let Some(slot) = self.flags.get_mut(i) {
                *slot = Some(k);
            }
            return Ok(true);
        }
        if let Some(i) = SWITCHES.iter().position(|&d| d == dir) {
            if let Some(s) = self.switches.get_mut(i) {
                *s = true;
            }
            return Ok(true);
        }
        match dir {
            // `COMPOUNDWORDMAX 2 y` (Hungarian): the number is the first field.
            "COMPOUNDWORDMAX" => {
                let n = at(1).parse::<usize>().map_err(|_| "COMPOUNDWORDMAX needs a number".to_string())?;
                self.word_max = Some(n.clamp(1, MAX_PARTS));
            }
            "COMPOUNDSYLLABLE" => {
                let n = at(1).parse::<usize>().map_err(|_| "COMPOUNDSYLLABLE needs a number".to_string())?;
                let vowels: String = at(2).chars().take(MAX_VOWELS).collect();
                self.syllable = Some((n.min(super::MAX_WORD_BYTES), vowels));
            }
            "LANG" => self.hungarian = at(1) == "hu" || at(1).starts_with("hu_") || at(1).starts_with("hu-"),
            "MAXCPDSUGS" => self.no_suggestions = at(1) == "0",
            "NOSPLITSUGS" => self.no_split_suggestions = true,
            "COMPOUNDMORESUFFIXES" => self.more_suffixes = true,
            "CHECKCOMPOUNDPATTERN" => self.read_pattern(f)?,
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// A `CHECKCOMPOUNDPATTERN` line: the count line, or `end[/flag] begin[/flag]` with an
    /// optional comment (`# …`). A replacement field is refused: no Language pack uses one.
    fn read_pattern(&mut self, f: &[&str]) -> Result<(), String> {
        let fields: Vec<&str> = f.iter().skip(1).copied().take_while(|x| !x.starts_with('#')).collect();
        if !self.pattern_header && fields.len() == 1 && fields.first().is_some_and(|x| x.parse::<usize>().is_ok()) {
            self.pattern_header = true;
            return Ok(());
        }
        let [end, begin] = fields.as_slice() else {
            return Err(format!("CHECKCOMPOUNDPATTERN `{}`: only `end[/flag] begin[/flag]` is supported", fields.join(" ")));
        };
        if end.len() > MAX_PATTERN_BYTES || begin.len() > MAX_PATTERN_BYTES || self.patterns.len() >= MAX_PATTERNS {
            return Err(format!("CHECKCOMPOUNDPATTERN: over {MAX_PATTERNS} patterns or {MAX_PATTERN_BYTES} bytes"));
        }
        self.patterns.push(((*end).to_string(), (*begin).to_string()));
        Ok(())
    }

    /// Whether any directive makes words compound by flags (the rest only refine that).
    fn compounds(&self) -> bool {
        [FLAG, BEGIN, MIDDLE_FLAG, END].iter().any(|&i| self.flags.get(i).is_some_and(Option::is_some))
    }

    /// Every flag the directives and patterns name, so that each gets an id.
    pub(super) fn keys(&self, mode: FlagMode) -> Result<Vec<FlagKey>, String> {
        let mut keys: Vec<FlagKey> = self.flags.iter().flatten().copied().collect();
        for (end, begin) in &self.patterns {
            for side in [end, begin] {
                if let Some((_, flag)) = side.split_once('/') {
                    keys.push(mode.parse_one(flag).map_err(|e| format!("CHECKCOMPOUNDPATTERN `{side}`: {e}"))?);
                }
            }
        }
        Ok(keys)
    }

    /// The compounding of a dictionary whose flags got ids by `id_of`; `None` when it doesn't
    /// compound by flags.
    pub(super) fn build(&self, mode: FlagMode, id_of: &impl Fn(FlagKey) -> Option<u16>) -> Result<Option<Box<Compounding>>, String> {
        if !self.compounds() {
            return Ok(None);
        }
        if self.more_suffixes {
            return Err("COMPOUNDMORESUFFIXES is not supported with flag-based compounding".into());
        }
        let id = |i: usize| self.flags.get(i).copied().flatten().and_then(id_of);
        let switch = |i: usize| self.switches.get(i).copied().unwrap_or(false);
        let mut c = Compounding {
            flag: id(FLAG),
            begin: id(BEGIN),
            middle: id(MIDDLE_FLAG),
            end: id(END),
            permit: id(PERMIT),
            forbid: id(FORBID),
            root: id(ROOT),
            force_ucase: id(FORCE_UCASE),
            dup: switch(DUP),
            rep: switch(REP),
            case: switch(CASE),
            triple: switch(TRIPLE),
            simplified: switch(SIMPLIFIED),
            no_suggestions: self.no_suggestions,
            two_words: !self.no_split_suggestions,
            raw: self.clone(),
            ..Compounding::default()
        };
        // `COMPOUNDSYLLABLE` turns `COMPOUNDWORDMAX` into the Hungarian syllable rule, or (other
        // languages) lifts it: the CLI's answers.
        match (&self.syllable, self.hungarian) {
            (None, _) => c.word_max = self.word_max,
            (Some((n, vowels)), true) => {
                c.word_max = self.word_max;
                c.syllables = Some((*n, vowels.chars().collect()));
            }
            (Some(_), false) => {}
        }
        if let Some(f) = c.force_ucase {
            c.marks.push(f);
        }
        for (end, begin) in &self.patterns {
            let side = |text: &str, c: &mut Compounding| -> Result<(String, Need), String> {
                let (chars, flag) = text.split_once('/').map_or((text, None), |(a, b)| (a, Some(b)));
                let need = match flag {
                    None => Need::Any,
                    Some(flag) => match id_of(mode.parse_one(flag)?) {
                        Some(f) => Need::Mark(c.mark(f)?),
                        None => Need::Never,
                    },
                };
                Ok((chars.to_string(), need))
            };
            let (end_chars, end_need) = side(end, &mut c)?;
            let (begin, begin_need) = side(begin, &mut c)?;
            let unmodified = end_chars == "0";
            let end = if unmodified { String::new() } else { end_chars };
            c.patterns.push(Pattern { end: end.into(), end_need, begin: begin.into(), begin_need, unmodified });
        }
        Ok(Some(Box::new(c)))
    }
}

/// What a side of a `CHECKCOMPOUNDPATTERN` asks of a part's flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Need {
    Any,
    /// The flag of [`Compounding::marks`] at this index.
    Mark(u8),
    /// A flag no stem or rule carries.
    Never,
}

impl Need {
    fn met(self, marks: u64) -> bool {
        match self {
            Need::Any => true,
            Need::Mark(i) => marks & (1 << i) != 0,
            Need::Never => false,
        }
    }
}

#[derive(Clone, Debug)]
struct Pattern {
    end: Box<str>,
    end_need: Need,
    begin: Box<str>,
    begin_need: Need,
    /// `0` for `end`: the part before is a stem without affixes.
    unmodified: bool,
}

/// The flag-based compounding of a dictionary (see the module docs).
#[derive(Clone, Debug, Default)]
pub(super) struct Compounding {
    flag: Option<u16>,
    begin: Option<u16>,
    middle: Option<u16>,
    end: Option<u16>,
    permit: Option<u16>,
    forbid: Option<u16>,
    root: Option<u16>,
    force_ucase: Option<u16>,
    word_max: Option<usize>,
    /// The Hungarian rule: past `word_max` parts, at most this many vowels in the word.
    syllables: Option<(usize, Box<[char]>)>,
    dup: bool,
    rep: bool,
    case: bool,
    triple: bool,
    simplified: bool,
    pub(super) no_suggestions: bool,
    /// A compound of three or more parts that splits into two words is wrong (unless
    /// `NOSPLITSUGS`: the CLI ties the two to each other).
    two_words: bool,
    patterns: Vec<Pattern>,
    /// Flags tracked per part (`FORCEUCASE`, the patterns' flags): bit `i` of [`Info::marks`].
    marks: Vec<u16>,
    /// The directives as read, written back by [`Compounding::write`].
    raw: Raw,
}

impl Compounding {
    /// The mark index of `flag`, adding it.
    fn mark(&mut self, flag: u16) -> Result<u8, String> {
        let i = match self.marks.iter().position(|&m| m == flag) {
            Some(i) => i,
            None => {
                self.marks.push(flag);
                self.marks.len() - 1
            }
        };
        u8::try_from(i).ok().filter(|&i| i < 64).ok_or_else(|| "CHECKCOMPOUNDPATTERN: more than 63 distinct flags".to_string())
    }

    /// The directives as compact-format lines (flags written by `write_flag`).
    pub(super) fn write(&self, out: &mut String, write_flag: &impl Fn(FlagKey, &mut String)) {
        let raw = &self.raw;
        if raw.hungarian {
            out.push_str("LANG hu\n");
        }
        for (name, key) in FLAG_DIRECTIVES.iter().zip(&raw.flags) {
            if let Some(k) = key {
                out.push_str(name);
                out.push(' ');
                write_flag(*k, out);
                out.push('\n');
            }
        }
        for (name, on) in SWITCHES.iter().zip(&raw.switches) {
            if *on {
                out.push_str(name);
                out.push('\n');
            }
        }
        if let Some(n) = raw.word_max {
            out.push_str(&format!("COMPOUNDWORDMAX {n}\n"));
        }
        if let Some((n, vowels)) = &raw.syllable {
            out.push_str(&format!("COMPOUNDSYLLABLE {n} {vowels}\n"));
        }
        if raw.no_suggestions {
            out.push_str("MAXCPDSUGS 0\n");
        }
        if raw.no_split_suggestions {
            out.push_str("NOSPLITSUGS\n");
        }
        for (end, begin) in &raw.patterns {
            out.push_str(&format!("CHECKCOMPOUNDPATTERN {end} {begin}\n"));
        }
    }
}

/// One way a piece of the word can be a compound part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Info {
    /// Where it may sit: [`FIRST`], [`MIDDLE`], [`LAST`].
    pos: u8,
    /// Which of [`Compounding::marks`] its flags hold.
    marks: u64,
    /// No suffix (`CHECKCOMPOUNDPATTERN`'s `0`).
    unmodified: bool,
    /// The stem carries `FORCEUCASE`.
    force: bool,
    /// The stem's index (`CHECKCOMPOUNDDUP` compares stems).
    stem: u32,
    /// How many words it counts as (`COMPOUNDROOT`: two).
    words: u8,
    /// Syllables of its suffix the Hungarian rule leaves out (as the last part): a suffix with no
    /// continuation classes doesn't count.
    tail: u8,
}

/// A stem found for a part, and the affixes that make the part from it.
#[derive(Clone, Copy)]
struct Analysis<'a> {
    stem: u32,
    set: &'a [u16],
    pfx: Option<&'a Rule>,
    sfx: Option<&'a Rule>,
    /// The inner suffix of two in a row (`sfx` is the outer one).
    inner: Option<&'a Rule>,
}

/// Most pieces a [`Pieces`] cache keeps.
const MAX_CACHED_PIECES: usize = 1 << 14;

/// The infos of pieces, kept across the compound checks of one suggestion call: its candidates
/// differ from the word by a letter or two, so most of their pieces are the same. Indexed by
/// whether the piece starts the word (KEEPCASE holds there).
#[derive(Default)]
pub(crate) struct Pieces([crate::patterns::FxMap<Box<str>, Box<[Info]>>; 2], usize);

/// One compound check: the word, its parts' analyses (each piece looked up once), the parts
/// chosen so far and the work done.
struct Search<'a> {
    c: &'a Compounding,
    w: &'a str,
    /// Byte offset of each char, and `w.len()`.
    cuts: Vec<usize>,
    chars: Vec<char>,
    excluded: Excluded,
    /// The first part's (KEEPCASE holds there).
    first_excluded: Excluded,
    capitalized: bool,
    /// Per piece `(start, end)` in chars: its infos as a range of `infos`.
    memo: Vec<Option<(u32, u32)>>,
    infos: Vec<Info>,
    /// The parts so far: start and end char, and the info.
    path: Vec<(usize, usize, Info)>,
    work: usize,
    /// [`AffixDict::near_word`], once answered.
    near: Option<bool>,
    /// Pieces analysed by earlier checks with the same exclusions.
    cache: Option<&'a mut Pieces>,
}

impl AffixDict {
    /// Is `w` a compound by the flag-based rules (see the module docs)? `capitalized`: the word
    /// as the user wrote it starts with a capital (`FORCEUCASE`).
    pub(super) fn flag_compound(&self, w: &str, excluded: Excluded, capitalized: bool) -> bool {
        // A suggestion candidate's search gets MAX_SUGGEST_WORK steps.
        let mut budget = if excluded.suggesting() { MAX_SUGGEST_WORK } else { MAX_WORK };
        self.flag_compound_spending(w, excluded, capitalized, &mut budget, None)
    }

    /// [`AffixDict::flag_compound`] in at most `budget` steps (and [`MAX_WORK`]), taking the
    /// steps it spends off `budget`. `cache`: pieces analysed before, with the same `excluded`.
    pub(super) fn flag_compound_spending(
        &self,
        w: &str,
        excluded: Excluded,
        capitalized: bool,
        budget: &mut usize,
        cache: Option<&mut Pieces>,
    ) -> bool {
        let Some(c) = self.cpd.as_deref() else { return false };
        if excluded.no_compounds() || w.is_empty() || w.len() > super::MAX_WORD_BYTES || *budget == 0 {
            return false;
        }
        if self.is_forbidden(w) {
            return false;
        }
        // KEEPCASE holds for the first part only (from `baz/K`, `Foobaz` and `FOOBAZ` are right
        // in the CLI, `Bazfoo` and `BAZFOO` wrong).
        let first_excluded = excluded;
        let excluded = excluded.without_keep();
        let cuts: Vec<usize> = w.char_indices().map(|(i, _)| i).chain(std::iter::once(w.len())).collect();
        let chars: Vec<char> = w.chars().collect();
        let n = cuts.len();
        let mut s = Search {
            c,
            w,
            cuts,
            chars,
            excluded,
            first_excluded,
            capitalized,
            memo: vec![None; n * n],
            infos: Vec::new(),
            path: Vec::with_capacity(MAX_PARTS),
            work: MAX_WORK - (*budget).min(MAX_WORK),
            near: None,
            cache,
        };
        let start = s.work;
        let found = self.parts_from(&mut s, 0);
        *budget = budget.saturating_sub(s.work.saturating_sub(start).max(1));
        found
    }

    /// Where a slip in `w` (no compound) likely is, in chars: from the furthest point its start
    /// reaches as compound parts to the nearest its end does, if anything reaches. A candidate
    /// changed there is checked as a compound first. `capitalized`: the same with a capital
    /// first letter counts too.
    pub(crate) fn compound_gap(&self, w: &str, capitalized: bool, budget: &mut usize, pieces: &mut Pieces) -> Option<(usize, usize)> {
        let c = self.cpd.as_deref()?;
        if w.is_empty() || w.len() > super::MAX_WORD_BYTES || *budget == 0 {
            return None;
        }
        let excluded = self.excluded(true);
        let n = w.chars().count();
        let min = self.compound_min.max(1);
        let mut reach = vec![false; n + 1];
        let mut back = vec![false; n + 1];
        let cap = capitalized.then(|| super::capitalize(w));
        for word in std::iter::once(w).chain(cap.as_deref()) {
            let mut s = Search {
                c,
                w: word,
                cuts: word.char_indices().map(|(i, _)| i).chain(std::iter::once(word.len())).collect(),
                chars: word.chars().collect(),
                excluded: excluded.without_keep(),
                first_excluded: excluded,
                capitalized,
                memo: vec![None; (n + 1) * (n + 1)],
                infos: Vec::new(),
                path: Vec::new(),
                work: MAX_WORK - (*budget).min(MAX_WORK),
                near: None,
                cache: Some(&mut *pieces),
            };
            let start = s.work;
            let fits = |s: &mut Search, a: usize, b: usize, place: u8| {
                let (x, y) = self.infos_of(s, a, b);
                s.infos.get(x as usize..y as usize).is_some_and(|infos| infos.iter().any(|i| i.pos & place != 0))
            };
            for a in 0..n {
                if a > 0 && !reach.get(a).copied().unwrap_or(false) {
                    continue;
                }
                for b in a.saturating_add(min)..n {
                    if s.work <= MAX_WORK
                        && fits(&mut s, a, b, if a == 0 { FIRST } else { MIDDLE })
                        && let Some(r) = reach.get_mut(b)
                    {
                        *r = true;
                    }
                }
            }
            for b in (1..=n).rev() {
                if b < n && !back.get(b).copied().unwrap_or(false) {
                    continue;
                }
                for a in 1..=b.saturating_sub(min) {
                    if s.work <= MAX_WORK
                        && fits(&mut s, a, b, if b == n { LAST } else { MIDDLE })
                        && let Some(r) = back.get_mut(a)
                    {
                        *r = true;
                    }
                }
            }
            *budget = budget.saturating_sub(s.work.saturating_sub(start));
        }
        let from = reach.iter().rposition(|&r| r).unwrap_or(0);
        let to = back.iter().position(|&r| r).unwrap_or(n);
        Some((from.min(to), from.max(to)))
    }

    /// Can the rest of the word, from char `start`, be split into parts after `s.path`?
    fn parts_from(&self, s: &mut Search, start: usize) -> bool {
        let total = s.chars.len();
        let count = s.path.len();
        let min = self.compound_min.max(1);
        for end in start.saturating_add(min)..=total {
            s.work += 1;
            if s.work > MAX_WORK {
                return false;
            }
            let last = end == total;
            if !last && count + 1 >= MAX_PARTS {
                break;
            }
            let place = if last {
                LAST
            } else if count == 0 {
                FIRST
            } else {
                MIDDLE
            };
            let (a, b) = self.infos_of(s, start, end);
            for k in a..b {
                // Each way of the piece costs a step, and a step per pattern at the boundary.
                s.work += 1 + s.c.patterns.len();
                if s.work > MAX_WORK {
                    return false;
                }
                let Some(info) = s.infos.get(k as usize).copied() else { continue };
                if info.pos & place == 0 || !self.boundary_ok(s, start, &info) {
                    continue;
                }
                s.path.push((start, end, info));
                let ok = if last { self.whole_ok(s) } else { self.words_ok(s, false) && self.next_parts(s, end) };
                s.path.pop();
                if ok {
                    return true;
                }
            }
        }
        false
    }

    /// After a part ending at char `end`: the next part starts there, or (`SIMPLIFIEDTRIPLE`)
    /// one char earlier when the part ends in a double letter.
    fn next_parts(&self, s: &mut Search, end: usize) -> bool {
        if self.parts_from(s, end) {
            return true;
        }
        let double = end >= 2 && s.chars.get(end - 1).is_some_and(|c| s.chars.get(end - 2) == Some(c));
        s.c.simplified && double && self.parts_from(s, end - 1)
    }

    /// The analyses of the piece from char `a` to char `b`, computed once: a range of `s.infos`.
    fn infos_of(&self, s: &mut Search, a: usize, b: usize) -> (u32, u32) {
        let n = s.cuts.len();
        let slot = a.saturating_mul(n).saturating_add(b);
        if let Some(Some(r)) = s.memo.get(slot) {
            return *r;
        }
        let from = u32::try_from(s.infos.len()).unwrap_or(u32::MAX);
        let cached = s.cuts.get(a).zip(s.cuts.get(b)).and_then(|(&x, &y)| s.w.get(x..y)).and_then(|piece| {
            let map = s.cache.as_deref()?.0.get(usize::from(a == 0))?;
            map.get(piece).map(|v| v.to_vec())
        });
        if let Some(v) = cached {
            s.work += 1;
            s.infos.extend(v);
        } else if let (Some(&x), Some(&y)) = (s.cuts.get(a), s.cuts.get(b))
            && let Some(piece) = s.w.get(x..y)
        {
            // Deduplicated by hashing, and at most MAX_INFOS per piece (each one is tried at
            // every split, against every pattern).
            let mut seen: std::collections::HashSet<Info, std::hash::BuildHasherDefault<crate::patterns::FxHasher>> = Default::default();
            let mut found: Vec<Info> = Vec::new();
            let mut work = s.work;
            let excluded = if a == 0 { s.first_excluded } else { s.excluded };
            self.analyses(piece, excluded, &mut work, &mut |an: Analysis| {
                if found.len() < MAX_INFOS
                    && let Some(info) = self.info(s.c, &an)
                    && seen.insert(info)
                {
                    found.push(info);
                }
            });
            s.work = work;
            // Only a piece analysed in full is kept (one cut short by the budget isn't).
            if work <= MAX_WORK
                && let Some(c) = s.cache.as_deref_mut()
                && c.1 < MAX_CACHED_PIECES
                && let Some(map) = c.0.get_mut(usize::from(a == 0))
            {
                map.insert(piece.into(), found.clone().into());
                c.1 += 1;
            }
            s.infos.extend(found);
        }
        let to = u32::try_from(s.infos.len()).unwrap_or(u32::MAX);
        if let Some(m) = s.memo.get_mut(slot) {
            *m = Some((from, to));
        }
        (from, to)
    }

    /// Where the analysis `an` lets its piece sit in a compound, if anywhere.
    fn info(&self, c: &Compounding, an: &Analysis) -> Option<Info> {
        let sets = [Some(an.set), an.pfx.map(|r| self.set(r.cont)), an.sfx.map(|r| self.set(r.cont)), an.inner.map(|r| self.set(r.cont))];
        let carries = |f: Option<u16>| f.is_some_and(|f| sets.iter().flatten().any(|s| has(s, f)));
        // COMPOUNDFORBIDFLAG on a prefix: no part anywhere (`bar|defoo` is wrong in the CLI).
        if an.pfx.is_some_and(|r| self.cont_holds(r, c.forbid)) {
            return None;
        }
        let mut pos = 0;
        if carries(c.flag) {
            pos = FIRST | MIDDLE | LAST;
        }
        for (flag, p) in [(c.begin, FIRST), (c.middle, MIDDLE), (c.end, LAST)] {
            if carries(flag) {
                pos |= p;
            }
        }
        // A prefix without COMPOUNDPERMITFLAG: any part but the last (`foo|unfoo|bar` is right
        // in the CLI, `foo|unfoo` wrong).
        if an.pfx.is_some_and(|r| !self.cont_holds(r, c.permit)) {
            pos &= !LAST;
        }
        if let Some(r) = an.sfx {
            if !self.cont_holds(r, c.permit) {
                pos &= LAST;
            }
            if self.cont_holds(r, self.only_in_compound) {
                pos &= !LAST;
            }
        }
        if an.inner.is_some() {
            pos &= LAST;
        }
        if carries(c.forbid) {
            pos &= LAST;
        }
        if pos == 0 {
            return None;
        }
        let mut marks = 0u64;
        for (i, &m) in c.marks.iter().enumerate().take(64) {
            if carries(Some(m)) {
                marks |= 1 << i;
            }
        }
        // COMPOUNDROOT and FORCEUCASE count on the stem only (an affix's continuation class
        // holding them is ignored);
        // a pattern's `0` means no suffix (a prefix doesn't count): the CLI's answers.
        let words = if c.root.is_some_and(|f| has(an.set, f)) { 2 } else { 1 };
        let force = c.force_ucase.is_some_and(|f| has(an.set, f));
        // The Hungarian syllable rule leaves out a suffix with no continuation classes (the
        // CLI's answers: `bab|bab|baata` from `ata` is right, from `ata/Z` wrong).
        let tail = match (&c.syllables, an.sfx) {
            (Some((_, vowels)), Some(r)) if self.set(r.cont).is_empty() => r.add.chars().filter(|ch| vowels.contains(ch)).count(),
            _ => 0,
        };
        let tail = u8::try_from(tail).unwrap_or(u8::MAX);
        Some(Info { pos, marks, unmodified: an.sfx.is_none(), force, stem: an.stem, words, tail })
    }

    /// Every way `p` is a stem, or a stem with a prefix, one or two suffixes, or a prefix and a
    /// suffix, as a compound part may be (`ONLYINCOMPOUND` affixes and stems included).
    fn analyses<'a>(&'a self, p: &str, excluded: Excluded, work: &mut usize, emit: &mut impl FnMut(Analysis<'a>)) {
        for (stem, set) in self.part_stems(p, excluded) {
            if !self.need_affix.is_some_and(|n| has(set, n)) {
                emit(Analysis { stem, set, pfx: None, sfx: None, inner: None });
            }
        }
        self.suffix_analyses(p, None, excluded, work, emit);
        let mut root = String::with_capacity(p.len() + 8);
        let ends = std::iter::once(0).chain(p.char_indices().map(|(i, c)| i + c.len_utf8()));
        for b in ends.take(self.max_pfx_chars + 1) {
            let (Some(add), Some(rest)) = (p.get(..b), p.get(b..)) else { continue };
            if rest.is_empty() && !self.full_strip {
                continue;
            }
            let Some(ids) = self.pfx_index.get(add) else { continue };
            for r in ids.iter().filter_map(|&i| self.prefixes.get(i as usize)) {
                *work += 1;
                if *work > MAX_WORK {
                    return;
                }
                root.clear();
                root.push_str(&r.strip);
                root.push_str(rest);
                if !r.cond_matches(&root, false) {
                    continue;
                }
                let (circ, needs) = (self.cont_holds(r, self.circumfix), self.cont_holds(r, self.need_affix));
                if !circ && !needs {
                    for (stem, set) in self.part_stems(&root, excluded) {
                        if has(set, r.flag) {
                            emit(Analysis { stem, set, pfx: Some(r), sfx: None, inner: None });
                        }
                    }
                }
                if r.cross {
                    self.suffix_analyses(&root, Some(r), excluded, work, emit);
                }
            }
        }
    }

    /// The analyses of `p` that end in a suffix: on a stem (after the prefix `pfx`, if any), or
    /// (without a prefix) on a form of another suffix.
    fn suffix_analyses<'a>(&'a self, p: &str, pfx: Option<&'a Rule>, excluded: Excluded, work: &mut usize, emit: &mut impl FnMut(Analysis<'a>)) {
        let mut root = String::with_capacity(p.len() + 8);
        let cuts = std::iter::once(p.len()).chain(p.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0 || self.full_strip));
        for b in cuts.take(self.max_sfx_chars + 1) {
            let (Some(base), Some(add)) = (p.get(..b), p.get(b..)) else { continue };
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups {
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                let rules = || g.rules.iter().filter_map(|&i| self.suffixes.get(i as usize));
                for (stem, set) in self.part_stems(&root, excluded) {
                    for r in rules() {
                        *work += 1;
                        if *work > MAX_WORK {
                            return;
                        }
                        if self.suffix_fits(r, set, pfx) && r.cond_matches(&root, true) {
                            emit(Analysis { stem, set, pfx, sfx: Some(r), inner: None });
                        }
                    }
                }
                // Two suffixes in a row: the outer ones that another suffix's continuation
                // classes can license, looked for together under one inner scan.
                if pfx.is_none() && !g.outer_rules.is_empty() {
                    let outers: Vec<&Rule> = g
                        .outer_rules
                        .iter()
                        .filter_map(|&i| self.suffixes.get(i as usize))
                        .filter(|r| !self.cont_holds(r, self.only_in_compound) && r.cond_matches(&root, true))
                        .collect();
                    *work += outers.len();
                    if !outers.is_empty() {
                        self.inner_analyses(&root, &outers, excluded, work, emit);
                    }
                }
            }
        }
    }

    /// Does the suffix `r` apply to a stem with the flags `set`, after the prefix `pfx` (if any)?
    /// Circumfix alike; a suffix that needs another affix only after a prefix that doesn't.
    fn suffix_fits(&self, r: &Rule, set: &[u16], pfx: Option<&Rule>) -> bool {
        if !has(set, r.flag) {
            return false;
        }
        let circ = self.cont_holds(r, self.circumfix);
        let needs = self.cont_holds(r, self.need_affix);
        match pfx {
            None => !circ && !needs,
            Some(p) => {
                r.cross
                    && (has(set, p.flag) || self.cont_holds(r, Some(p.flag)))
                    && circ == self.cont_holds(p, self.circumfix)
                    && !(needs && self.cont_holds(p, self.need_affix))
            }
        }
    }

    /// `form` (what an outer suffix of `outers` was taken off) as a stem with an inner suffix
    /// whose continuation classes hold that outer suffix's flag.
    fn inner_analyses<'a>(&'a self, form: &str, outers: &[&'a Rule], excluded: Excluded, work: &mut usize, emit: &mut impl FnMut(Analysis<'a>)) {
        let mut root = String::with_capacity(form.len() + 8);
        let cuts = std::iter::once(form.len()).chain(form.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0 || self.full_strip));
        for b in cuts.take(self.max_sfx_chars + 1) {
            let (Some(base), Some(add)) = (form.get(..b), form.get(b..)) else { continue };
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups.iter().filter(|g| !g.cont_rules.is_empty()) {
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                for (stem, set) in self.part_stems(&root, excluded) {
                    for r in g.cont_rules.iter().filter_map(|&i| self.suffixes.get(i as usize)) {
                        *work += 1;
                        if *work > MAX_WORK {
                            return;
                        }
                        if !has(set, r.flag)
                            || self.cont_holds(r, self.circumfix)
                            || self.cont_holds(r, self.only_in_compound)
                            || !r.cond_matches(&root, true)
                        {
                            continue;
                        }
                        for &outer in outers.iter().filter(|o| self.cont_holds(r, Some(o.flag))) {
                            *work += 1;
                            emit(Analysis { stem, set, pfx: None, sfx: Some(outer), inner: Some(r) });
                        }
                    }
                }
            }
        }
    }

    /// The stem `w` as a compound part: its index and flags, unless it is forbidden or excluded.
    /// A stem with `ONLYINCOMPOUND` homonyms beside others is two: each group's flags apply on
    /// their own (`foo/XOS` and `foo/T`: `foos` is a part, `foot` is not, as in the CLI).
    ///
    /// Homonyms kept apart ([`AffixDict::homonyms`]) are each a part of their own: a suffix and
    /// the compound flag must come from one (Dutch `poeder` the noun compounds, `poederden` the
    /// verb form doesn't: `schuurpoederden` is wrong in the CLI).
    fn part_stems(&self, w: &str, excluded: Excluded) -> impl Iterator<Item = (u32, &[u16])> {
        let found = self.stem_lookup(w);
        let apart = found.and_then(|(stem, _)| self.homonyms.get(&stem).map(|h| (stem, h)));
        let main = found.filter(|_| apart.is_none());
        let each = apart.into_iter().flat_map(move |(stem, h)| h.iter().map(move |&id| (stem, self.set(id))));
        let only = found.and_then(|(stem, _)| self.cpd_sets.get(&stem).map(|&c| (stem, self.set(c))));
        main.into_iter().chain(each).chain(only).filter(move |(_, set)| !excluded.hits(set) && !self.forbidden.is_some_and(|f| has(set, f)))
    }

    /// May a part with `info` start at char `start`, after the last part of `s.path`
    /// (`CHECKCOMPOUNDCASE`, `CHECKCOMPOUNDTRIPLE`, `CHECKCOMPOUNDPATTERN`)?
    fn boundary_ok(&self, s: &Search, start: usize, info: &Info) -> bool {
        let Some(&(ps, pe, prev)) = s.path.last() else { return true };
        let c = s.c;
        let (Some(&a), Some(&b)) = (s.chars.get(pe.saturating_sub(1)), s.chars.get(start)) else { return true };
        if c.case && (a.is_uppercase() || b.is_uppercase()) && a != '-' && b != '-' {
            return false;
        }
        if c.triple && start == pe && a == b {
            let before = pe >= 2 && s.chars.get(pe - 2) == Some(&a);
            let after = s.chars.get(start + 1) == Some(&b);
            if before || after {
                return false;
            }
        }
        // A SIMPLIFIEDTRIPLE join (the parts share a letter) still may not leave three equal
        // letters in the word (`buss|ssen` as `busssen`, `boo|oob` as `booob`).
        if c.triple && start + 1 == pe && s.chars.get(pe) == Some(&a) {
            return false;
        }
        if c.patterns.is_empty() {
            return true;
        }
        let text = |x: usize, y: usize| s.cuts.get(x).zip(s.cuts.get(y)).and_then(|(&x, &y)| s.w.get(x..y)).unwrap_or_default();
        let (before, after) = (text(ps, pe), text(start, s.chars.len()));
        !c.patterns.iter().any(|p| {
            before.ends_with(&*p.end)
                && after.starts_with(&*p.begin)
                && (!p.unmodified || prev.unmodified)
                && p.end_need.met(prev.marks)
                && p.begin_need.met(info.marks)
        })
    }

    /// Are the parts of `s.path` few enough (`COMPOUNDWORDMAX`, `COMPOUNDROOT`)? `whole`: the
    /// word is complete (the Hungarian syllable rule counts its vowels).
    fn words_ok(&self, s: &Search, whole: bool) -> bool {
        let Some(max) = s.c.word_max else { return true };
        let words: usize = s.path.iter().map(|p| usize::from(p.2.words)).sum();
        if words <= max {
            return true;
        }
        match &s.c.syllables {
            None => false,
            Some(_) if !whole => true,
            Some((most, vowels)) => {
                let tail = s.path.last().map_or(0, |p| usize::from(p.2.tail));
                s.chars.iter().filter(|c| vowels.contains(c)).count().saturating_sub(tail) <= *most
            }
        }
    }

    /// The checks on a complete split: two parts at least, `COMPOUNDWORDMAX`, `CHECKCOMPOUNDDUP`,
    /// `FORCEUCASE`, the three-part rule and `CHECKCOMPOUNDREP`.
    fn whole_ok(&self, s: &mut Search) -> bool {
        let n = s.path.len();
        if n < 2 || !self.words_ok(s, true) {
            return false;
        }
        let (Some(&(_, _, last)), Some(&(_, _, prev))) = (s.path.last(), s.path.get(n - 2)) else { return false };
        if s.c.dup && last.stem == prev.stem {
            return false;
        }
        // FORCEUCASE: a word with no capital at all is wrong (`fooQuxbar` is right in the CLI).
        if last.force && !s.capitalized && !s.chars.iter().any(|c| c.is_uppercase()) {
            return false;
        }
        let span = |s: &Search, i: usize, j: usize| -> String {
            let (Some(&(a, ..)), Some(&(_, b, _))) = (s.path.get(i), s.path.get(j)) else { return String::new() };
            s.cuts.get(a).zip(s.cuts.get(b)).and_then(|(&x, &y)| s.w.get(x..y)).unwrap_or_default().to_string()
        };
        if n >= 3 && self.near_word(s) {
            return false;
        }
        if s.c.rep && !self.rep.is_empty() {
            let pairs = (0..n - 1).map(|i| (i, i + 1));
            let tails = (0..n - 1).map(|i| (i, n - 1));
            let spans: Vec<String> = pairs.chain(tails).map(|(i, j)| span(s, i, j)).collect();
            if spans.iter().any(|sp| self.rep_fault(sp, s)) {
                return false;
            }
        }
        true
    }

    /// The check on a compound of three or more parts: is the word one slip away from a word
    /// (not a compound)? Then it is taken for a misspelling of that word. The slips are the
    /// CLI's (black-box, see the tests): a letter too many, a pair typed twice, two letters
    /// swapped or one moved (up to 4 apart), a keyboard neighbour (`KEY`), a `TRY` letter, a
    /// `REP` or `MAP` change, the word capitalized, and (without `NOSPLITSUGS`) two words. It
    /// depends on the word only, so it is answered once per check.
    fn near_word(&self, s: &mut Search) -> bool {
        if let Some(near) = s.near {
            return near;
        }
        let excluded = s.excluded;
        let mut work = s.work;
        // A word listed as a stem that can't stand alone (ONLYINCOMPOUND, NEEDAFFIX) counts here
        // only with a suffix, not with a prefix (`ver` + `kehr` beside `verkehr/zo`:
        // `Fahrradverkehr` is right in the CLI).
        let mut is_word = |cand: &[char]| {
            work += 1;
            let cand: String = cand.iter().collect();
            // Past its budget a candidate counts as a word: the compound is then rejected, as
            // it is past MAX_WORK, so a suggestion never offers one the check would refuse.
            work > MAX_WORK || self.simple_word_budgeted(&cand, excluded, self.stem_set(&cand).is_none()).unwrap_or(true)
        };
        let near = self.near_miss(&s.chars, s.c.two_words, &mut is_word);
        s.work = work;
        s.near = Some(near);
        near
    }

    fn near_miss(&self, w: &[char], two_words: bool, is_word: &mut impl FnMut(&[char]) -> bool) -> bool {
        let n = w.len();
        let mut cand: Vec<char> = Vec::with_capacity(n + 8);
        let mut first = w.iter().take(1).flat_map(|c| c.to_uppercase()).collect::<Vec<char>>();
        first.extend(w.iter().skip(1));
        if is_word(&first) || is_word(&w.iter().flat_map(|c| c.to_uppercase()).collect::<Vec<char>>()) {
            return true;
        }
        for i in 0..n {
            // A letter too many, and a pair typed twice (`abab` for `ab`).
            cand.clear();
            cand.extend(w.iter().take(i).chain(w.iter().skip(i + 1)));
            if is_word(&cand) {
                return true;
            }
            if i + 3 < n && w.get(i) == w.get(i + 2) && w.get(i + 1) == w.get(i + 3) {
                cand.clear();
                cand.extend(w.iter().take(i).chain(w.iter().skip(i + 2)));
                if is_word(&cand) {
                    return true;
                }
            }
            for d in 1..=4 {
                if i + d >= n {
                    break;
                }
                // Swapped with a letter up to 4 further on.
                cand.clear();
                cand.extend_from_slice(w);
                cand.swap(i, i + d);
                if cand != w && is_word(&cand) {
                    return true;
                }
                // Moved 2 to 4 places on or back (one place is a swap).
                if d >= 2 {
                    cand.clear();
                    cand.extend_from_slice(w);
                    for turn in [1, 2] {
                        if let Some(slice) = cand.get_mut(i..=i + d) {
                            // Left by one: `w[i]` moves on; then right by two: `w[i + d]` back.
                            if turn == 1 { slice.rotate_left(1) } else { slice.rotate_right(2) }
                        }
                        if is_word(&cand) {
                            return true;
                        }
                    }
                }
            }
        }
        // One letter for another: a keyboard neighbour, a TRY letter.
        let key = if self.key.is_empty() { DEFAULT_KEY } else { self.key.as_str() };
        let neighbours = |c: char| {
            key.split('|').flat_map(move |row| {
                let row: Vec<char> = row.chars().take(MAX_KEY_ROW).collect();
                let at: Vec<usize> = row.iter().enumerate().filter(|(_, k)| **k == c).map(|(i, _)| i).collect();
                at.into_iter()
                    .flat_map(move |i| [i.checked_sub(1).and_then(|j| row.get(j)).copied(), row.get(i + 1).copied()])
                    .flatten()
                    .collect::<Vec<char>>()
            })
        };
        let tries: Vec<char> = self.try_chars.chars().take(MAX_TRY).collect();
        for i in 0..n {
            let Some(&c) = w.get(i) else { continue };
            for r in neighbours(c).chain(tries.iter().copied()) {
                if r == c {
                    continue;
                }
                cand.clear();
                cand.extend_from_slice(w);
                if let Some(x) = cand.get_mut(i) {
                    *x = r;
                }
                if is_word(&cand) {
                    return true;
                }
            }
        }
        // `REP` (here `^` and `$` tie to the word's start and end) and `MAP` changes.
        let text: String = w.iter().collect();
        for (from, to) in &self.rep {
            let (start, from) = from.strip_prefix('^').map_or((false, from.as_str()), |f| (true, f));
            let (end, from) = from.strip_suffix('$').map_or((false, from), |f| (true, f));
            if from.is_empty() || to.contains(' ') {
                continue;
            }
            for (i, _) in text.match_indices(from) {
                if (start && i != 0) || (end && i + from.len() != text.len()) {
                    continue;
                }
                let changed: Vec<char> = text
                    .get(..i)
                    .unwrap_or_default()
                    .chars()
                    .chain(to.chars())
                    .chain(text.get(i + from.len()..).unwrap_or_default().chars())
                    .collect();
                if is_word(&changed) {
                    return true;
                }
            }
        }
        let mut budget = MAX_MAP_VARIANTS;
        if !self.map.is_empty() && self.map_variant(w, 0, &mut w.to_vec(), &mut budget, is_word) {
            return true;
        }
        two_words
            && (1..n).any(|i| {
                let (head, tail) = w.split_at(i);
                is_word(head) && is_word(tail)
            })
    }

    /// Does changing letters of `cand` from `i` on to others of their `MAP` group make a word?
    fn map_variant(&self, w: &[char], i: usize, cand: &mut Vec<char>, budget: &mut usize, is_word: &mut impl FnMut(&[char]) -> bool) -> bool {
        let Some(&c) = w.get(i) else { return cand.as_slice() != w && is_word(cand) };
        if self.map_variant(w, i + 1, cand, budget, is_word) {
            return true;
        }
        for group in self.map.iter().filter(|g| g.contains(&c)) {
            for &r in group.iter().filter(|&&r| r != c) {
                if *budget == 0 {
                    return false;
                }
                *budget -= 1;
                if let Some(x) = cand.get_mut(i) {
                    *x = r;
                }
                let hit = self.map_variant(w, i + 1, cand, budget, is_word);
                if let Some(x) = cand.get_mut(i) {
                    *x = c;
                }
                if hit {
                    return true;
                }
            }
        }
        false
    }

    /// Does a `REP` replacement turn `text` into a word (not a compound)?
    fn rep_fault(&self, text: &str, s: &mut Search) -> bool {
        let mut cand = String::with_capacity(text.len() + 8);
        for (from, to) in &self.rep {
            // Replacements tied to a word's start or end (`^`, `$`) are for suggestions: the CLI
            // never applies them here.
            if from.is_empty() || from.starts_with('^') || from.ends_with('$') {
                continue;
            }
            for (i, _) in text.match_indices(from.as_str()) {
                s.work += 1;
                if s.work > MAX_WORK {
                    return true;
                }
                cand.clear();
                cand.push_str(text.get(..i).unwrap_or_default());
                cand.push_str(to);
                cand.push_str(text.get(i + from.len()..).unwrap_or_default());
                if self.simple_word(&cand, s.excluded) {
                    return true;
                }
            }
        }
        false
    }
}

/// The compounding directives a compact-format line may hold, read like the `.aff`'s.
pub(super) fn is_directive(dir: &str) -> bool {
    FLAG_DIRECTIVES.contains(&dir)
        || SWITCHES.contains(&dir)
        || matches!(dir, "COMPOUNDWORDMAX" | "COMPOUNDSYLLABLE" | "LANG" | "MAXCPDSUGS" | "NOSPLITSUGS" | "CHECKCOMPOUNDPATTERN")
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::affix::AffixDict;

    /// A synthetic dictionary and the `hunspell` 1.7.5 CLI's answers on it (black-box: `hunspell
    /// -a`), for every word of one to three `pieces` and the `extra` words. `listed` are the
    /// right words when `right`, else the wrong ones; every other word is the opposite.
    struct Pinned {
        name: &'static str,
        aff: &'static str,
        dic: &'static str,
        pieces: &'static str,
        extra: &'static str,
        right: bool,
        listed: &'static str,
    }

    fn load(aff: &str, dic: &str) -> AffixDict {
        let entries: Vec<&str> = dic.split_whitespace().collect();
        let dic = format!("{}\n{}\n", entries.len(), entries.join("\n"));
        AffixDict::from_hunspell(&format!("SET UTF-8\n{aff}\n"), &dic, &[]).unwrap()
    }

    fn candidates(p: &Pinned) -> Vec<String> {
        let pieces: Vec<&str> = p.pieces.split_whitespace().collect();
        let mut out: Vec<String> = pieces.iter().map(|a| (*a).to_string()).collect();
        for a in &pieces {
            for b in &pieces {
                out.push(format!("{a}{b}"));
            }
        }
        for a in &pieces {
            for b in &pieces {
                for c in &pieces {
                    out.push(format!("{a}{b}{c}"));
                }
            }
        }
        out.extend(p.extra.split_whitespace().map(str::to_string));
        out
    }

    /// One case per directive (or pair that only means something together), each word's answer
    /// pinned to the CLI's, before and after the compact format.
    #[test]
    fn flag_compounds_answer_as_the_cli_does() {
        let mut failed = Vec::new();
        for p in PINNED {
            let d = load(p.aff, p.dic);
            let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
            let listed: HashSet<&str> = p.listed.split_whitespace().collect();
            for w in candidates(p) {
                let want = listed.contains(w.as_str()) == p.right;
                if d.check(&w) != want || back.check(&w) != want {
                    failed.push(format!("{}: {w} should be {}", p.name, if want { "right" } else { "wrong" }));
                }
            }
        }
        assert!(failed.is_empty(), "{failed:#?}");
    }

    /// Real dictionaries (the Language packs' `.aff`/`.dic`): words the CLI answers right and
    /// wrong on the same files (black-box; mostly random compounds of the dictionary's forms).
    /// Run when a craft-i18n checkout is in `CRAFT_I18N_DIR` and its harness has fetched the pack
    /// files (`CRAFT_I18N_CACHE`, default `$CRAFT_I18N_DIR/target/language-data-cache`, files named
    /// by their sha256); each language whose files are missing is skipped.
    #[test]
    fn pack_dictionaries_compound_as_the_cli_does() {
        let Some(dir) = std::env::var_os("CRAFT_I18N_DIR").map(std::path::PathBuf::from) else { return };
        let Ok(manifest) = std::fs::read_to_string(dir.join("language-data/manifest.toml")) else { return };
        let cache = std::env::var_os("CRAFT_I18N_CACHE").map_or_else(|| dir.join("target/language-data-cache"), std::path::PathBuf::from);
        let manifest: toml::Table = manifest.parse().unwrap();
        let files = manifest.get("file").and_then(toml::Value::as_array).cloned().unwrap_or_default();
        let read = |locale: &str, kind: &str| {
            let f = files.iter().find(|f| {
                f.get("locale").and_then(toml::Value::as_str) == Some(locale) && f.get("kind").and_then(toml::Value::as_str) == Some(kind)
            })?;
            std::fs::read(cache.join(f.get("sha256")?.as_str()?)).ok()
        };
        for (locale, right, wrong) in REAL {
            let (Some(aff), Some(dic)) = (read(locale, "spelling-aff"), read(locale, "spelling-dic")) else {
                eprintln!("{locale}: pack files not fetched, skipped");
                continue;
            };
            let (aff, dic) = crate::hunspell::decode(&aff, &dic).unwrap();
            let d = AffixDict::from_hunspell(&aff, &dic, &[]).unwrap_or_else(|e| panic!("{locale}: {e}"));
            for w in right.split_whitespace() {
                assert!(d.check(w), "{locale}: {w} is right");
            }
            for w in wrong.split_whitespace() {
                assert!(!d.check(w), "{locale}: {w} is wrong");
            }
            // A long garbled word (wrong words run together): its suggestions stay quick.
            let garbled: String = wrong.chars().filter(|c| !c.is_whitespace()).take(38).collect();
            let t = std::time::Instant::now();
            let _ = crate::affix_suggest::suggest_in(&d, &garbled, 8, &crate::affix_suggest::Tuning::DEFAULT);
            let took = t.elapsed();
            eprintln!("{locale}: suggestions for {garbled} in {took:?}");
            assert!(took.as_secs() < 5, "{locale}: {took:?}");
            // A one-letter slip of a long compound gets the compound back.
            for (typo, meant) in TYPOS.iter().filter(|t| t.0 == *locale).flat_map(|t| t.1.iter()) {
                let got = d.suggest(typo, 8);
                assert!(got.iter().any(|g| g == meant), "{locale}: {typo} → {got:?}, not {meant}");
            }
        }
    }

    /// (locale, [(typo, the compound meant)]): each one's suggestions hold it.
    const TYPOS: &[(&str, &[(&str, &str)])] = &[
        (
            "de",
            &[
                ("Donaudampfschff", "Donaudampfschiff"),
                ("Arbetszimmer", "Arbeitszimmer"),
                ("Kinerspielplatz", "Kinderspielplatz"),
                ("Bundesverfassugsgericht", "Bundesverfassungsgericht"),
            ],
        ),
        (
            "nl",
            &[
                ("ziekenhusopname", "ziekenhuisopname"),
                ("voetbawledstrijd", "voetbalwedstrijd"),
                ("arbeidsongeschiktheidsverzeekring", "arbeidsongeschiktheidsverzekering"),
                ("tandartsafsparak", "tandartsafspraak"),
                ("belastingaagifte", "belastingaangifte"),
            ],
        ),
    ];

    /// (locale, right words, wrong words), the CLI's answers.
    const REAL: &[(&str, &str, &str)] = &[
        (
            "da",
            "sugammadexgårdfacadens kraftudtagssmutterbuttetheden pebermakrellensudvalgsarbejde Helsinkifirbladen",
            "arbejdslagernebegivende visdommensrustet farfarentraveport egyptologiskopladesvoldtoges",
        ),
        (
            "de",
            "Südwestbeifallsturm Machtfeudalideologie Schleifhärtering Gesprächskontur schule Verfilmungen Aufwärmungen",
            "chilisoßeswitttenaus Magmabewegungaminosäurebasis ausgezählterzehntagefieber heerfolgetemperatechnik haus buch verfilmungs verfilmungen aufwärmungen",
        ),
        (
            "de-at",
            "Verträglichkeitsnackenleder Sichelmangrovenwäldern Wehrpflichtbeiruter Kühlerblockigelnothilfe",
            "Jungenschulemakromere Dagösnestküken vogtlandbahnelektronenarmem qualitativmedwedew",
        ),
        (
            "de-ch",
            "Königsstahlextensionsfähigkeit Barlachpregelinsel Euroasiendysprosiums Hexendiabasgesteine",
            "eingeglittenswohlbeleibtesten plakatflächewürdigens durchzuwandernliebediener zurückerinnernskolonnadenform",
        ),
        (
            "eo",
            "elipsoidolumŝirmila resonancosonarkivojn bazanumeralosukersciuraj vivspertoellistiĝkapablo",
            "nekapneatsgvidrolosmitozon Alaholaŭpove kapbalanadajsaŭtomatiĝadajn filamentecukrematoriosilviajn",
        ),
        (
            "et",
            "jahvakemgisesski talõššidekäunuvaikski kõhnunuimatessegikräunujaisse nekroosiupsusina",
            "spinningtoismele kollektivismessüngimainagi elektrolüüsispilgutelebki fosforisesverääni",
        ),
        ("fa", "", "بنمایاندمنتراوی زرشکتبهترانملجاجتکارترینید پارامترمبرقصاندیم"),
        ("hr", "truležne", "vaeašteka razuvjeriliuzdanajpsujućemu pilencadiparieovom časomicevoziti"),
        (
            "hu",
            "eurofelsálazásiabbjáéin",
            "síposékéibahetvenegyszerezni BMsorbánocskáérakönigesedéknek antihősekorgalvántelepeitekig Grazcákompendiumnyijaimmal",
        ),
        ("mn", "Канейдианхалифат Балжойрд", "өехийлчхөгсудлагчдархуутайгүй зээнцрүүдийгээбачиг цуглавруудынхаатөвшинт"),
        ("nb", "", "fellesomkostningasweeperrollen kremfløytenalkoholdebatter Elsepålestikka translatøreksamenerkeldgufsene"),
        (
            "nl",
            "pikjesboekbinderspersen samengingengeloofsverdeeldheid sleepliftenerfgoed stortputtenbehadeling",
            "onbeleefdheidhortensiastraat verwindjuilee Lotharengersvolontair gechromeerdsrietpluimen",
        ),
        ("nn", "", "orstuvastavantgardistar påskemåndagarkjempekenguruar piggtrådmusikkarreknemaskinar stimreinselikniårekeiper"),
        (
            "sv",
            "målvärdenservername sunniglädjepirrets kedjelängdendöbattangerna basfärgmetropoliterna Norges Sveriges norge",
            "ogenomskådligsbåtsmanstorps tännkarheiskanens stängningstaggensbehängas råseleedemonlägeouppvärmt norges sveriges afrikas",
        ),
        (
            "sv-fi",
            "Ausåsciseleringar uppsatsämnesinsatsens pimpelfiskelagstiftningsarbete orderstocksglutenet",
            "smitartäbyeors triialiseringarkronkoloni databasenslågstadiumcardiganen springformsknegarensronneby",
        ),
    ];

    /// craft-libs#497 review: hundreds of prefixes and suffixes that each add a part to the same
    /// one-letter stem, with distinct pattern flags, make tens of thousands of ways for one
    /// piece. They are capped and counted, so a word still answers at once.
    #[test]
    fn many_ways_for_one_piece_stay_cheap() {
        let marks: Vec<char> = (0..62).filter_map(|i| char::from_u32(0x3041 + i)).collect();
        let mut aff = String::from("SET UTF-8\nFLAG UTF-8\nCOMPOUNDFLAG X\nCOMPOUNDMIN 1\nCOMPOUNDPERMITFLAG P\nCOMPOUNDWORDMAX 99\n");
        let mut pats: Vec<String> = marks.iter().map(|m| format!("zz/{m} zz/{m}")).collect();
        pats.push("b a".into());
        aff.push_str(&format!("CHECKCOMPOUNDPATTERN {}\n", pats.len()));
        for p in &pats {
            aff.push_str(&format!("CHECKCOMPOUNDPATTERN {p}\n"));
        }
        let mut flags = String::new();
        for (kind, base, shift) in [("PFX", 0x4e00, 0), ("SFX", 0x5e00, 31)] {
            for i in 0..250u32 {
                let Some(f) = char::from_u32(base + i) else { continue };
                let m: String = (0..31).filter(|b| (i >> b) & 1 == 1).filter_map(|b| marks.get(shift + b)).collect();
                aff.push_str(&format!("{kind} {f} Y 1\n{kind} {f} 0 a/XP{m} .\n"));
                flags.push(f);
            }
        }
        let d = AffixDict::from_hunspell(&aff, &format!("2\na/X{flags}\nb/X\n"), &[]).unwrap();
        let t = std::time::Instant::now();
        for w in ["baaa", "bbbbbbbbbbbbbbbbbbbbaaa", "ab"] {
            let _ = d.check(w);
        }
        assert!(t.elapsed().as_millis() < 2000, "{:?}", t.elapsed());
    }

    /// `COMPOUNDRULE` as the Swedish (`)k`) and Mongolian (`[a0,a1]`, `%?`, `.`) dictionaries
    /// write it, read as the CLI answers on the same rules.
    #[test]
    fn compound_rules_with_odd_marks_answer_as_the_cli_does() {
        let d = load("COMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE )k", "a/) b/k");
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        for d in [&d, &back] {
            assert!(d.check("ab") && !d.check("abb") && !d.check("aab"));
        }
        let d = load("FLAG long\nCOMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE (aa)*[a0,a1](bb)", "x/aa y/bb");
        assert!(d.check("xy") && d.check("xxy") && !d.check("xyy") && !d.check("yx"));
        for rule in ["(nn)*%?", "(nn)*.(nn)*%?", "(aa)*%?(bb)"] {
            let d = load(&format!("FLAG long\nCOMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE {rule}"), "x/nn y/nn z/aa w/bb");
            let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
            for d in [&d, &back] {
                assert!(!d.check("xy") && !d.check("xxy") && !d.check("zw") && d.check("x"), "{rule}");
            }
        }
    }

    /// The worst dictionary for the search: every letter and pair of letters a part anywhere,
    /// every check on, hundreds of `REP`s, a long `TRY`, `MAP` groups and chained suffixes that
    /// permit compounding. Each word still answers within the work bound.
    #[test]
    fn hostile_compounding_stays_cheap() {
        let letters: Vec<char> = ('a'..='z').collect();
        let mut aff = String::from(
            "COMPOUNDFLAG X\nCOMPOUNDMIN 1\nCOMPOUNDWORDMAX 99\nCOMPOUNDPERMITFLAG P\nONLYINCOMPOUND O\nCHECKCOMPOUNDDUP\nCHECKCOMPOUNDREP\n\
             CHECKCOMPOUNDCASE\nCHECKCOMPOUNDTRIPLE\nSIMPLIFIEDTRIPLE\nTRY abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ\n\
             MAP 3\nMAP aeiou\nMAP bcdfg\nMAP hjklm\nCHECKCOMPOUNDPATTERN 2\nCHECKCOMPOUNDPATTERN a/X b/X\nCHECKCOMPOUNDPATTERN 0/P c\n",
        );
        aff.push_str(&format!("REP {}\n", letters.len() * letters.len()));
        for a in &letters {
            for b in &letters {
                aff.push_str(&format!("REP {a} {b}{a}\n"));
            }
        }
        for (i, a) in letters.iter().enumerate() {
            aff.push_str(&format!("SFX {a} Y 1\nSFX {a} 0 {a}/XPO{} .\n", letters.get(i + 1).copied().unwrap_or('a')));
        }
        let flags: String = letters.iter().collect();
        let mut dic: Vec<String> = letters.iter().map(|a| format!("{a}/X{flags}")).collect();
        for a in &letters {
            for b in &letters {
                dic.push(format!("{a}{b}/X{flags}"));
            }
        }
        let d = AffixDict::from_hunspell(&format!("SET UTF-8\n{aff}"), &format!("{}\n{}\n", dic.len(), dic.join("\n")), &[]).unwrap();
        let mut rng = 7u64;
        let random: String = (0..120)
            .map(|_| {
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                letters.get((rng >> 33) as usize % 26).copied().unwrap_or('a')
            })
            .collect();
        let mut slowest = std::time::Duration::ZERO;
        let mut words: Vec<String> = vec!["a".repeat(120), "ab".repeat(60), random.clone()];
        for n in [8, 16, 24, 32, 40, 48, 64] {
            words.extend([random.chars().take(n).collect(), "a".repeat(n), "ab".repeat(n / 2), "aab".repeat(n / 3)]);
        }
        for w in &words {
            let t = std::time::Instant::now();
            let _ = d.check(w);
            slowest = slowest.max(t.elapsed());
        }
        let t = std::time::Instant::now();
        let _ = crate::affix_suggest::suggest_in(&d, &"qwertyuiopasdfghjklzxcvbnm".repeat(2)[..38], 8, &crate::affix_suggest::Tuning::DEFAULT);
        let suggest = t.elapsed();
        eprintln!("hostile compounding: slowest word {slowest:?}, suggestions {suggest:?}");
        assert!(suggest.as_secs() < 10, "{suggest:?}");
        assert!(slowest.as_secs() < 5, "{slowest:?}");
    }

    const PINNED: &[Pinned] = &[
        Pinned {
            name: "flag",
            aff: "COMPOUNDFLAG X",
            dic: "foo/X bar/X ba/X baz",
            pieces: "foo bar ba baz",
            extra: "",
            right: true,
            listed: "foo bar ba baz foofoo foobar barfoo barbar foofoofoo foofoobar foobarfoo foobarbar barfoofoo barfoobar \
                barbarfoo barbarbar",
        },
        Pinned { name: "min", aff: "COMPOUNDFLAG X\nCOMPOUNDMIN 1", dic: "f/X bar/X", pieces: "f bar", extra: "", right: false, listed: "" },
        Pinned {
            name: "affixes",
            aff: "COMPOUNDFLAG X\nCOMPOUNDPERMITFLAG Z\nSFX S Y 1\nSFX S 0 s .\nPFX P Y 1\nPFX P 0 un .\nSFX T Y 1\nSFX T 0 t/Z .\nPFX Q Y 1\nPFX Q 0 re/Z .",
            dic: "foo/XSPTQ bar/XSPTQ baz/S",
            pieces: "foo foos unfoo foot refoo bar",
            extra: "barunfoo unfoobar barfoot footbar refoobar barrefoo barfoos foosbar baz",
            right: false,
            listed: "foounfoo foosfoo foosfoos foosunfoo foosfoot foosrefoo foosbar unfoounfoo footunfoo refoounfoo barunfoo \
                foofoounfoo foofoosfoo foofoosfoos foofoosunfoo foofoosfoot foofoosrefoo foofoosbar foounfoounfoo foofootunfoo \
                foorefoounfoo foobarunfoo foosfoofoo foosfoofoos foosfoounfoo foosfoofoot foosfoorefoo foosfoobar foosfoosfoo \
                foosfoosfoos foosfoosunfoo foosfoosfoot foosfoosrefoo foosfoosbar foosunfoofoo foosunfoofoos foosunfoounfoo \
                foosunfoofoot foosunfoorefoo foosunfoobar foosfootfoo foosfootfoos foosfootunfoo foosfootfoot foosfootrefoo \
                foosfootbar foosrefoofoo foosrefoofoos foosrefoounfoo foosrefoofoot foosrefoorefoo foosrefoobar foosbarfoo \
                foosbarfoos foosbarunfoo foosbarfoot foosbarrefoo foosbarbar unfoofoounfoo unfoofoosfoo unfoofoosfoos \
                unfoofoosunfoo unfoofoosfoot unfoofoosrefoo unfoofoosbar unfoounfoounfoo unfoofootunfoo unfoorefoounfoo \
                unfoobarunfoo footfoounfoo footfoosfoo footfoosfoos footfoosunfoo footfoosfoot footfoosrefoo footfoosbar \
                footunfoounfoo footfootunfoo footrefoounfoo footbarunfoo refoofoounfoo refoofoosfoo refoofoosfoos \
                refoofoosunfoo refoofoosfoot refoofoosrefoo refoofoosbar refoounfoounfoo refoofootunfoo refoorefoounfoo \
                refoobarunfoo barfoounfoo barfoosfoo barfoosfoos barfoosunfoo barfoosfoot barfoosrefoo barfoosbar \
                barunfoounfoo barfootunfoo barrefoounfoo barbarunfoo",
        },
        Pinned {
            name: "affix_flags",
            aff: "COMPOUNDFLAG X\nSFX A Y 1\nSFX A 0 s/X .\nPFX B Y 1\nPFX B 0 un/X .\nSFX C Y 1\nSFX C 0 en .",
            dic: "arbeit/AC comp/XC haus/B",
            pieces: "arbeit arbeits comp compen haus unhaus",
            extra: "",
            right: true,
            listed: "arbeit arbeits comp compen haus unhaus comparbeits compcomp compcompen unhausarbeits unhauscomp unhauscompen \
                compcomparbeits compcompcomp compcompcompen compunhausarbeits compunhauscomp compunhauscompen \
                unhauscomparbeits unhauscompcomp unhauscompcompen unhausunhausarbeits unhausunhauscomp unhausunhauscompen",
        },
        Pinned {
            name: "positions",
            aff: "COMPOUNDFLAG X\nCOMPOUNDBEGIN B\nCOMPOUNDMIDDLE M\nCOMPOUNDEND E",
            dic: "beg/B mid/M end/E any/X all/BME",
            pieces: "beg mid end any all",
            extra: "",
            right: true,
            listed: "beg mid end any all begend begany begall anyend anyany anyall allend allany allall begmidend begmidany \
                begmidall beganyend beganyany beganyall begallend begallany begallall anymidend anymidany anymidall anyanyend \
                anyanyany anyanyall anyallend anyallany anyallall allmidend allmidany allmidall allanyend allanyany allanyall \
                allallend allallany allallall",
        },
        Pinned {
            name: "first_last",
            aff: "COMPOUNDFIRST B\nCOMPOUNDLAST E",
            dic: "beg/B end/E",
            pieces: "beg end",
            extra: "",
            right: true,
            listed: "beg end",
        },
        Pinned {
            name: "onlyincompound",
            aff: "COMPOUNDFLAG X\nONLYINCOMPOUND O\nSFX A Y 1\nSFX A 0 er/B .\nSFX B Y 1\nSFX B 0 s .\nSFX F Y 1\nSFX F 0 s/O .",
            dic: "foo/XAF bar/XAF zap/XO qux/A",
            pieces: "foo foos fooers bar zap",
            extra: "zap foos fooer fooers quxer quxers",
            right: true,
            listed: "foo fooers bar foofoo foofooers foobar foozap barfoo barfooers barbar barzap zapfoo zapfooers zapbar zapzap \
                foofoofoo foofoofooers foofoobar foofoozap foobarfoo foobarfooers foobarbar foobarzap foozapfoo foozapfooers \
                foozapbar foozapzap barfoofoo barfoofooers barfoobar barfoozap barbarfoo barbarfooers barbarbar barbarzap \
                barzapfoo barzapfooers barzapbar barzapzap zapfoofoo zapfoofooers zapfoobar zapfoozap zapbarfoo zapbarfooers \
                zapbarbar zapbarzap zapzapfoo zapzapfooers zapzapbar zapzapzap fooer quxer quxers",
        },
        Pinned {
            name: "onlyincompound_permit",
            aff: "COMPOUNDFLAG X\nONLYINCOMPOUND O\nCOMPOUNDPERMITFLAG P\nSFX F Y 1\nSFX F 0 s/OP .\nSFX G Y 1\nSFX G 0 s/O .\nPFX H Y 1\nPFX H 0 un/O .\nPFX I Y 1\nPFX I 0 re/OP .",
            dic: "foo/XFHI bar/XFHI baz/XG",
            pieces: "foo foos unfoo refoo bazs",
            extra: "foos unfoo refoo bazs baz",
            right: true,
            listed: "foo foofoo foorefoo foosfoo foosrefoo unfoofoo unfoorefoo refoofoo refoorefoo foofoofoo foofoorefoo foofoosfoo \
                foofoosrefoo foounfoofoo foounfoorefoo foorefoofoo foorefoorefoo foosfoofoo foosfoorefoo foosfoosfoo \
                foosfoosrefoo foosunfoofoo foosunfoorefoo foosrefoofoo foosrefoorefoo unfoofoofoo unfoofoorefoo unfoofoosfoo \
                unfoofoosrefoo unfoounfoofoo unfoounfoorefoo unfoorefoofoo unfoorefoorefoo refoofoofoo refoofoorefoo \
                refoofoosfoo refoofoosrefoo refoounfoofoo refoounfoorefoo refoorefoofoo refoorefoorefoo baz",
        },
        Pinned {
            name: "forbid_needaffix",
            aff: "COMPOUNDFLAG X\nCOMPOUNDFORBIDFLAG Q\nCOMPOUNDPERMITFLAG P\nNEEDAFFIX N\nSFX S Y 1\nSFX S 0 s .\nSFX T Y 1\nSFX T 0 t/Q .\nSFX U Y 1\nSFX U 0 u/QP .\nSFX V Y 1\nSFX V 0 v/P .",
            dic: "foo/XSTUV bar/XSTUV qoo/XQS nee/XNSV noo/NSV",
            pieces: "foo foot foou qoo nee nees neev",
            extra: "nee noo noos",
            right: true,
            listed: "foo foot foou qoo nees neev foofoo foofoot foofoou fooqoo foonees fooneev neevfoo neevfoot neevfoou neevqoo \
                neevnees neevneev foofoofoo foofoofoot foofoofoou foofooqoo foofoonees foofooneev fooneevfoo fooneevfoot \
                fooneevfoou fooneevqoo fooneevnees fooneevneev neevfoofoo neevfoofoot neevfoofoou neevfooqoo neevfoonees \
                neevfooneev neevneevfoo neevneevfoot neevneevfoou neevneevqoo neevneevnees neevneevneev noos",
        },
        Pinned {
            name: "dup",
            aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDDUP\nSFX S Y 1\nSFX S 0 s .",
            dic: "foo/XS bar/XS foos/X",
            pieces: "foo foos bar",
            extra: "",
            right: false,
            listed: "foofoo barbar foofoofoo foobarbar foosfoofoo foosbarbar barfoofoo barbarbar",
        },
        Pinned {
            name: "case",
            aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDCASE",
            dic: "foo/X Bar/X baz/X QUX/X",
            pieces: "foo Bar baz QUX",
            extra: "",
            right: true,
            listed: "foo Bar baz QUX foofoo foobaz Barfoo Barbaz bazfoo bazbaz foofoofoo foofoobaz foobazfoo foobazbaz Barfoofoo \
                Barfoobaz Barbazfoo Barbazbaz bazfoofoo bazfoobaz bazbazfoo bazbazbaz",
        },
        Pinned {
            name: "triple",
            aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDTRIPLE",
            dic: "foo/X bar/X schiff/X fahrt/X ooze/X baa/X",
            pieces: "foo schiff fahrt ooze baa",
            extra: "",
            right: false,
            listed: "fooooze schifffoo schifffahrt foofooooze fooschifffoo fooschifffahrt foooozefoo foooozeschiff foooozefahrt \
                foooozeooze foooozebaa schifffoofoo schifffooschiff schifffoofahrt schifffooooze schifffoobaa schiffschifffoo \
                schiffschifffahrt schifffahrtfoo schifffahrtschiff schifffahrtfahrt schifffahrtooze schifffahrtbaa \
                fahrtfooooze fahrtschifffoo fahrtschifffahrt oozefooooze oozeschifffoo oozeschifffahrt baafooooze baaschifffoo \
                baaschifffahrt",
        },
        Pinned {
            name: "simplified_triple",
            aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDTRIPLE\nSIMPLIFIEDTRIPLE",
            dic: "foo/X bar/X schiff/X fahrt/X ooze/X baa/X",
            pieces: "foo schiff fahrt ooze baa",
            extra: "schiffahrt",
            right: false,
            listed: "fooooze schifffoo schifffahrt foofooooze fooschifffoo fooschifffahrt foooozefoo foooozeschiff foooozefahrt \
                foooozeooze foooozebaa schifffoofoo schifffooschiff schifffoofahrt schifffooooze schifffoobaa schiffschifffoo \
                schiffschifffahrt schifffahrtfoo schifffahrtschiff schifffahrtfahrt schifffahrtooze schifffahrtbaa \
                fahrtfooooze fahrtschifffoo fahrtschifffahrt oozefooooze oozeschifffoo oozeschifffahrt baafooooze baaschifffoo \
                baaschifffahrt",
        },
        Pinned {
            name: "simplified_only",
            aff: "COMPOUNDFLAG X\nSIMPLIFIEDTRIPLE",
            dic: "foo/X schiff/X fahrt/X ooze/X",
            pieces: "foo schiff fahrt ooze",
            extra: "schiffahrt fooze",
            right: false,
            listed: "fooze",
        },
        Pinned {
            name: "rep",
            aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDREP\nREP 4\nREP oo o\nREP rb r\nREP ^ba x\nREP az$ x\nSFX S Y 1\nSFX S 0 s .",
            dic: "foo/XS bar/XS baz/X fobar/S barb/X baraz/S fobaz bx",
            pieces: "foo foos bar bars baz barb",
            extra: "",
            right: true,
            listed: "foo foos bar bars baz barb foofoo foofoos foobarb barfoo barfoos barbar barbars barbarb bazfoo bazfoos bazbar \
                bazbars bazbaz bazbarb barbfoo barbfoos barbbar barbbars barbbaz barbbarb foofoofoo foofoofoos foofoobarb \
                foobarbfoo foobarbfoos foobarbbar foobarbbars foobarbbaz foobarbbarb barfoofoo barfoofoos barfoobarb barbarfoo \
                barbarfoos barbarbar barbarbars barbarbarb barbarbfoo barbarbfoos barbarbbar barbarbbars barbarbbaz \
                barbarbbarb bazfoofoo bazfoofoos bazfoobarb bazbarfoo bazbarfoos bazbarbar bazbarbars bazbarbarb bazbazfoo \
                bazbazfoos bazbazbar bazbazbars bazbazbaz bazbazbarb bazbarbfoo bazbarbfoos bazbarbbar bazbarbbars bazbarbbaz \
                bazbarbbarb barbfoofoo barbfoofoos barbfoobarb barbbarfoo barbbarfoos barbbarbar barbbarbars barbbarbarb \
                barbbazfoo barbbazfoos barbbazbar barbbazbars barbbazbaz barbbazbarb barbbarbfoo barbbarbfoos barbbarbbar \
                barbbarbbars barbbarbbaz barbbarbbarb",
        },
        Pinned {
            name: "rep_spans",
            aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDREP\nREP 1\nREP oo o",
            dic: "foo/X bar/X baz/X qux/X fobarbaz quxfobar",
            pieces: "foo bar baz qux",
            extra: "",
            right: false,
            listed: "foobarbaz quxfoobar",
        },
        Pinned {
            name: "three_parts",
            aff: "COMPOUNDFLAG X",
            dic: "foo/X bar/X baz/X qux/X foobar fooba barbazqux",
            pieces: "foo bar baz qux",
            extra: "fooxbarbaz",
            right: false,
            listed: "foofoobar foobarfoo foobarbar foobarbaz foobarqux barfoobar bazfoobar bazbarqux quxfoobar fooxbarbaz",
        },
        Pinned {
            name: "three_parts_nosplitsugs",
            aff: "COMPOUNDFLAG X\nNOSPLITSUGS",
            dic: "foo/X bar/X baz/X qux/X foobar fooba",
            pieces: "foo bar baz qux",
            extra: "",
            right: false,
            listed: "",
        },
        Pinned {
            name: "pattern",
            aff: "COMPOUNDFLAG X\nCOMPOUNDPERMITFLAG P\nCHECKCOMPOUNDPATTERN 3\nCHECKCOMPOUNDPATTERN oo ba # comment\nCHECKCOMPOUNDPATTERN /F /G\nCHECKCOMPOUNDPATTERN er /H\nSFX S Y 1\nSFX S 0 s/P .\nSFX T Y 1\nSFX T 0 t/PF .\nSFX E Y 1\nSFX E 0 er/P .",
            dic: "foo/XST bar/XSG baz/XS qux/XF zed/XH hey/XSE heyer/XS",
            pieces: "foo foot bar baz qux zed heyer hey",
            extra: "",
            right: false,
            listed: "foobar foobaz footbar quxbar heyerzed foofoobar foofoobaz foofootbar foobarfoo foobarfoot foobarbar foobarbaz \
                foobarqux foobarzed foobarheyer foobarhey foobazfoo foobazfoot foobazbar foobazbaz foobazqux foobazzed \
                foobazheyer foobazhey fooquxbar fooheyerzed footfoobar footfoobaz footfootbar footbarfoo footbarfoot \
                footbarbar footbarbaz footbarqux footbarzed footbarheyer footbarhey footquxbar footheyerzed barfoobar \
                barfoobaz barfootbar barquxbar barheyerzed bazfoobar bazfoobaz bazfootbar bazquxbar bazheyerzed quxfoobar \
                quxfoobaz quxfootbar quxbarfoo quxbarfoot quxbarbar quxbarbaz quxbarqux quxbarzed quxbarheyer quxbarhey \
                quxquxbar quxheyerzed zedfoobar zedfoobaz zedfootbar zedquxbar zedheyerzed heyerfoobar heyerfoobaz \
                heyerfootbar heyerquxbar heyerzedfoo heyerzedfoot heyerzedbar heyerzedbaz heyerzedqux heyerzedzed \
                heyerzedheyer heyerzedhey heyerheyerzed heyfoobar heyfoobaz heyfootbar heyquxbar heyheyerzed",
        },
        Pinned {
            name: "pattern_unmodified_forceucase",
            aff: "COMPOUNDFLAG X\nFORCEUCASE U\nCHECKCOMPOUNDPATTERN 1\nCHECKCOMPOUNDPATTERN 0/A b\nCOMPOUNDPERMITFLAG P\nSFX S Y 1\nSFX S 0 s .\nSFX R Y 1\nSFX R 0 r/P .",
            dic: "foo/XSR straat/XUS bar/X zoo/XAR big/X",
            pieces: "foo straat bar zoo zoor big",
            extra: "Foostraat FOOSTRAAT Foostraats",
            right: false,
            listed: "foostraat straatstraat barstraat zoostraat zoobar zoobig zoorstraat bigstraat foofoostraat foostraatstraat \
                foobarstraat foozoostraat foozoobar foozoobig foozoorstraat foobigstraat straatfoostraat straatstraatstraat \
                straatbarstraat straatzoostraat straatzoobar straatzoobig straatzoorstraat straatbigstraat barfoostraat \
                barstraatstraat barbarstraat barzoostraat barzoobar barzoobig barzoorstraat barbigstraat zoofoostraat \
                zoostraatstraat zoobarfoo zoobarstraat zoobarbar zoobarzoo zoobarzoor zoobarbig zoozoostraat zoozoobar \
                zoozoobig zoozoorstraat zoobigfoo zoobigstraat zoobigbar zoobigzoo zoobigzoor zoobigbig zoorfoostraat \
                zoorstraatstraat zoorbarstraat zoorzoostraat zoorzoobar zoorzoobig zoorzoorstraat zoorbigstraat bigfoostraat \
                bigstraatstraat bigbarstraat bigzoostraat bigzoobar bigzoobig bigzoorstraat bigbigstraat",
        },
        Pinned {
            name: "wordmax_root",
            aff: "COMPOUNDFLAG X\nCOMPOUNDWORDMAX 2\nCOMPOUNDROOT R",
            dic: "foo/X bar/X foobaz/XR",
            pieces: "foo bar foobaz",
            extra: "",
            right: true,
            listed: "foo bar foobaz foofoo foobar barfoo barbar",
        },
        Pinned {
            name: "wordmax",
            aff: "COMPOUNDFLAG X\nCOMPOUNDWORDMAX 3\nCOMPOUNDMIN 1",
            dic: "fo/X ba/X",
            pieces: "fo ba",
            extra: "fobafoba fobafobafo",
            right: false,
            listed: "fobafoba fobafobafo",
        },
        Pinned {
            name: "syllable",
            aff: "COMPOUNDFLAG X\nCOMPOUNDWORDMAX 2\nCOMPOUNDSYLLABLE 6 aeiou\nCOMPOUNDMIN 1",
            dic: "ba/X be/X baaa/X",
            pieces: "ba be baaa",
            extra: "babababa",
            right: false,
            listed: "",
        },
        Pinned {
            name: "syllable_hu",
            aff: "LANG hu_HU\nCOMPOUNDFLAG X\nCOMPOUNDWORDMAX 2 y\nCOMPOUNDSYLLABLE 6 aeiou\nCOMPOUNDMIN 1",
            dic: "ba/X be/X baaa/X",
            pieces: "ba be baaa",
            extra: "babababa",
            right: false,
            listed: "babaaabaaa bebaaabaaa baaababaaa baaabebaaa baaabaaaba baaabaaabe baaabaaabaaa",
        },
        Pinned {
            name: "near_slips",
            aff: "COMPOUNDFLAG X\nCOMPOUNDMIN 4\nTRY x\nREP 1\nREP ^ab yz\nMAP 1\nMAP eë",
            dic: "abcd/X efgh/X ijkl/X mnop/X abcdefghikjl efghabcdjkli ijklabcdefgg xnopabcdefgh yzcdmnopijkl ëfghmnopabcd Ijklmnopabcd",
            pieces: "abcd efgh ijkl mnop",
            extra: "",
            right: false,
            listed: "abcdefghijkl abcdmnopijkl efghabcdijkl efghmnopabcd ijklabcdefgh ijklmnopabcd mnopabcdefgh",
        },
        Pinned {
            name: "onlyincompound_homonyms",
            aff: "COMPOUNDFLAG X\nONLYINCOMPOUND O\nSFX S Y 1\nSFX S 0 s .\nSFX T Y 1\nSFX T 0 t .",
            dic: "foo/XOS foo/T bar/X baz/XO baz/XT",
            pieces: "foo foos foot bar baz bazt bazs",
            extra: "",
            right: true,
            listed: "foo foot bar baz bazt foofoo foofoos foobar foobaz foobazt barfoo barfoos barbar barbaz barbazt bazfoo bazfoos \
                bazbar bazbaz bazbazt foofoofoo foofoofoos foofoobar foofoobaz foofoobazt foobarfoo foobarfoos foobarbar \
                foobarbaz foobarbazt foobazfoo foobazfoos foobazbar foobazbaz foobazbazt barfoofoo barfoofoos barfoobar \
                barfoobaz barfoobazt barbarfoo barbarfoos barbarbar barbarbaz barbarbazt barbazfoo barbazfoos barbazbar \
                barbazbaz barbazbazt bazfoofoo bazfoofoos bazfoobar bazfoobaz bazfoobazt bazbarfoo bazbarfoos bazbarbar \
                bazbarbaz bazbarbazt bazbazfoo bazbazfoos bazbazbar bazbazbaz bazbazbazt",
        },
        Pinned {
            name: "onlyincompound_homonyms_alone",
            aff: "ONLYINCOMPOUND O\nNEEDAFFIX N\nSFX S Y 1\nSFX S 0 s .\nPFX U Y 1\nPFX U 0 un .",
            dic: "bar/OS bar/U qux/O qux/S zed/OS",
            pieces: "",
            extra: "bar bars unbar qux quxs unqux zed zeds",
            right: true,
            listed: "bar unbar qux quxs",
        },
        Pinned {
            name: "simplified_triple_checked",
            aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDTRIPLE\nSIMPLIFIEDTRIPLE",
            dic: "boo/X oob/X ob/X xaa/X aay/X ay/X schiff/X fahrt/X ffahrt/X buss/X ssen/X fjell/X llama/X",
            pieces: "",
            extra: "booob boob boooob boobob xaaay xaay xaaaay schiffahrt schifffahrt schiffffahrt booobboo boobboo busssen bussen fjelllama fjellama",
            right: true,
            listed: "schiffahrt",
        },
        Pinned {
            name: "simplified_triple_min",
            aff: "COMPOUNDFLAG X\nCOMPOUNDMIN 3\nCHECKCOMPOUNDTRIPLE\nSIMPLIFIEDTRIPLE",
            dic: "foo/X ooa/X oo/X bb/X bbb/X abb/X bba/X",
            pieces: "foo ooa oo abb bba bbb",
            extra: "fooa abba abbba abbbba",
            right: true,
            listed: "foo ooa oo abb bba bbb foofoo fooabb foobba foobbb ooafoo ooaooa ooaabb ooabba ooabbb abbfoo abbooa abbabb \
                bbafoo bbaooa bbaabb bbabba bbabbb bbbfoo bbbooa bbbabb foofoofoo foofooabb foofoobba foofoobbb fooabbfoo \
                fooabbooa fooabbabb foobbafoo foobbaooa foobbaabb foobbabba foobbabbb foobbbfoo foobbbooa foobbbabb ooafoofoo \
                ooafooabb ooafoobba ooafoobbb ooaooafoo ooaooaooa ooaooaabb ooaooabba ooaooabbb ooaabbfoo ooaabbooa ooaabbabb \
                ooabbafoo ooabbaooa ooabbaabb ooabbabba ooabbabbb ooabbbfoo ooabbbooa ooabbbabb abbfoofoo abbfooabb abbfoobba \
                abbfoobbb abbooafoo abbooaooa abbooaabb abbooabba abbooabbb abbabbfoo abbabbooa abbabbabb bbafoofoo bbafooabb \
                bbafoobba bbafoobbb bbaooafoo bbaooaooa bbaooaabb bbaooabba bbaooabbb bbaabbfoo bbaabbooa bbaabbabb bbabbafoo \
                bbabbaooa bbabbaabb bbabbabba bbabbabbb bbabbbfoo bbabbbooa bbabbbabb bbbfoofoo bbbfooabb bbbfoobba bbbfoobbb \
                bbbooafoo bbbooaooa bbbooaabb bbbooabba bbbooabbb bbbabbfoo bbbabbooa bbbabbabb",
        },
        Pinned {
            name: "keepcase_parts",
            aff: "COMPOUNDFLAG X\nKEEPCASE K",
            dic: "foo/X baz/XK bar/X",
            pieces: "foo baz bar Foo Baz BAZ FOO",
            extra: "",
            right: true,
            listed: "foo baz bar Foo FOO foofoo foobaz foobar bazfoo bazbaz bazbar barfoo barbaz barbar Foofoo Foobaz Foobar FOOBAZ \
                FOOFOO foofoofoo foofoobaz foofoobar foobazfoo foobazbaz foobazbar foobarfoo foobarbaz foobarbar bazfoofoo \
                bazfoobaz bazfoobar bazbazfoo bazbazbaz bazbazbar bazbarfoo bazbarbaz bazbarbar barfoofoo barfoobaz barfoobar \
                barbazfoo barbazbaz barbazbar barbarfoo barbarbaz barbarbar Foofoofoo Foofoobaz Foofoobar Foobazfoo Foobazbaz \
                Foobazbar Foobarfoo Foobarbaz Foobarbar FOOBAZBAZ FOOBAZFOO FOOFOOBAZ FOOFOOFOO",
        },
        Pinned {
            name: "capitalized_stem_parts",
            aff: "COMPOUNDFLAG X",
            dic: "foo/X Qux/X bar/X",
            pieces: "foo Qux qux bar Foo QUX",
            extra: "",
            right: true,
            listed: "foo Qux bar Foo QUX foofoo fooQux foobar Quxfoo QuxQux Quxbar barfoo barQux barbar Foofoo FooQux Foobar \
                foofoofoo foofooQux foofoobar fooQuxfoo fooQuxQux fooQuxbar foobarfoo foobarQux foobarbar Quxfoofoo QuxfooQux \
                Quxfoobar QuxQuxfoo QuxQuxQux QuxQuxbar Quxbarfoo QuxbarQux Quxbarbar barfoofoo barfooQux barfoobar barQuxfoo \
                barQuxQux barQuxbar barbarfoo barbarQux barbarbar Foofoofoo FoofooQux Foofoobar FooQuxfoo FooQuxQux FooQuxbar \
                Foobarfoo FoobarQux Foobarbar",
        },
        Pinned {
            name: "forceucase_mixed",
            aff: "COMPOUNDFLAG X\nFORCEUCASE U\nKEEPCASE K",
            dic: "foo/X bar/XU baz/XK Qux/X",
            pieces: "foo bar baz Foo Bar Qux",
            extra: "FOOBAR FooBar fooBar FOOBAZ Foobaz QuxBar Quxbar QUXBAR",
            right: true,
            listed: "foo bar baz Foo Bar Qux foofoo foobaz fooQux barfoo barbaz barQux bazfoo bazbaz bazQux Foofoo Foobar Foobaz \
                FooQux Barfoo Barbar Barbaz BarQux Quxfoo Quxbar Quxbaz QuxQux foofoofoo foofoobaz foofooQux foobarfoo \
                foobarbaz foobarQux foobazfoo foobazbaz foobazQux fooQuxfoo fooQuxbar fooQuxbaz fooQuxQux barfoofoo barfoobaz \
                barfooQux barbarfoo barbarbaz barbarQux barbazfoo barbazbaz barbazQux barQuxfoo barQuxbar barQuxbaz barQuxQux \
                bazfoofoo bazfoobaz bazfooQux bazbarfoo bazbarbaz bazbarQux bazbazfoo bazbazbaz bazbazQux bazQuxfoo bazQuxbar \
                bazQuxbaz bazQuxQux Foofoofoo Foofoobar Foofoobaz FoofooQux Foobarfoo Foobarbar Foobarbaz FoobarQux Foobazfoo \
                Foobazbar Foobazbaz FoobazQux FooQuxfoo FooQuxbar FooQuxbaz FooQuxQux Barfoofoo Barfoobar Barfoobaz BarfooQux \
                Barbarfoo Barbarbar Barbarbaz BarbarQux Barbazfoo Barbazbar Barbazbaz BarbazQux BarQuxfoo BarQuxbar BarQuxbaz \
                BarQuxQux Quxfoofoo Quxfoobar Quxfoobaz QuxfooQux Quxbarfoo Quxbarbar Quxbarbaz QuxbarQux Quxbazfoo Quxbazbar \
                Quxbazbaz QuxbazQux QuxQuxfoo QuxQuxbar QuxQuxbaz QuxQuxQux FOOBAR FOOBAZ QUXBAR",
        },
        Pinned {
            name: "forceucase_suffix",
            aff: "COMPOUNDFLAG X\nFORCEUCASE U\nSFX S Y 1\nSFX S 0 s/U .",
            dic: "foo/X baz/XS",
            pieces: "foo baz bazs",
            extra: "Foobazs",
            right: false,
            listed: "bazsfoo bazsbaz bazsbazs foobazsfoo foobazsbaz foobazsbazs bazbazsfoo bazbazsbaz bazbazsbazs bazsfoofoo \
                bazsfoobaz bazsfoobazs bazsbazfoo bazsbazbaz bazsbazbazs bazsbazsfoo bazsbazsbaz bazsbazsbazs",
        },
        Pinned {
            name: "pattern_unsuffixed",
            aff: "COMPOUNDFLAG X\nCOMPOUNDPERMITFLAG P\nSFX S Y 1\nSFX S 0 s/P .\nPFX U Y 1\nPFX U 0 un/P .\nCHECKCOMPOUNDPATTERN 2\nCHECKCOMPOUNDPATTERN 0 b\nCHECKCOMPOUNDPATTERN 0 un",
            dic: "foo/XSU bar/XSU oof/X",
            pieces: "foo foos unfoo bar bars unbar oof",
            extra: "",
            right: true,
            listed: "foo foos unfoo bar bars unbar oof foofoo foofoos foooof foosfoo foosfoos foosunfoo foosbar foosbars foosunbar \
                foosoof unfoofoo unfoofoos unfoooof barfoo barfoos baroof barsfoo barsfoos barsunfoo barsbar barsbars \
                barsunbar barsoof unbarfoo unbarfoos unbaroof ooffoo ooffoos oofoof foofoofoo foofoofoos foofoooof foofoosfoo \
                foofoosfoos foofoosunfoo foofoosbar foofoosbars foofoosunbar foofoosoof fooooffoo fooooffoos foooofoof \
                foosfoofoo foosfoofoos foosfoooof foosfoosfoo foosfoosfoos foosfoosunfoo foosfoosbar foosfoosbars \
                foosfoosunbar foosfoosoof foosunfoofoo foosunfoofoos foosunfoooof foosbarfoo foosbarfoos foosbaroof \
                foosbarsfoo foosbarsfoos foosbarsunfoo foosbarsbar foosbarsbars foosbarsunbar foosbarsoof foosunbarfoo \
                foosunbarfoos foosunbaroof foosooffoo foosooffoos foosoofoof unfoofoofoo unfoofoofoos unfoofoooof unfoofoosfoo \
                unfoofoosfoos unfoofoosunfoo unfoofoosbar unfoofoosbars unfoofoosunbar unfoofoosoof unfooooffoo unfooooffoos \
                unfoooofoof barfoofoo barfoofoos barfoooof barfoosfoo barfoosfoos barfoosunfoo barfoosbar barfoosbars \
                barfoosunbar barfoosoof barooffoo barooffoos baroofoof barsfoofoo barsfoofoos barsfoooof barsfoosfoo \
                barsfoosfoos barsfoosunfoo barsfoosbar barsfoosbars barsfoosunbar barsfoosoof barsunfoofoo barsunfoofoos \
                barsunfoooof barsbarfoo barsbarfoos barsbaroof barsbarsfoo barsbarsfoos barsbarsunfoo barsbarsbar barsbarsbars \
                barsbarsunbar barsbarsoof barsunbarfoo barsunbarfoos barsunbaroof barsooffoo barsooffoos barsoofoof \
                unbarfoofoo unbarfoofoos unbarfoooof unbarfoosfoo unbarfoosfoos unbarfoosunfoo unbarfoosbar unbarfoosbars \
                unbarfoosunbar unbarfoosoof unbarooffoo unbarooffoos unbaroofoof ooffoofoo ooffoofoos ooffoooof ooffoosfoo \
                ooffoosfoos ooffoosunfoo ooffoosbar ooffoosbars ooffoosunbar ooffoosoof oofooffoo oofooffoos oofoofoof",
        },
        Pinned {
            name: "forbidflag_prefix",
            aff: "COMPOUNDFLAG X\nCOMPOUNDPERMITFLAG P\nCOMPOUNDFORBIDFLAG Q\nPFX C Y 1\nPFX C 0 de/PQ .",
            dic: "foo/XC bar/X",
            pieces: "foo defoo bar",
            extra: "",
            right: true,
            listed: "foo defoo bar foofoo foobar barfoo barbar foofoofoo foofoobar foobarfoo foobarbar barfoofoo barfoobar \
                barbarfoo barbarbar",
        },
        Pinned {
            name: "forbidden",
            aff: "COMPOUNDFLAG X\nFORBIDDENWORD F\nSFX S Y 1\nSFX S 0 en .",
            dic: "foo/X brok/XS broken/F bar/XF foobar/F",
            pieces: "foo brok broken bar",
            extra: "foobar broken",
            right: true,
            listed: "foo brok foofoo foobrok foobroken brokfoo brokbrok brokbroken foofoofoo foofoobrok foofoobroken foobrokfoo \
                foobrokbrok foobrokbroken brokfoofoo brokfoobrok brokfoobroken brokbrokfoo brokbrokbrok brokbrokbroken",
        },
        Pinned {
            name: "mixed_keep",
            aff: "COMPOUNDFLAG X\nKEEPCASE K",
            dic: "foo/X baz/XK Qux/X bar/X",
            pieces: "foo baz Qux Baz Foo BAZ QUX",
            extra: "BazQux BazfooQux FooQux FooBaz fooBaz FOOQux BAZQux",
            right: true,
            listed: "foo baz Qux Foo QUX foofoo foobaz fooQux bazfoo bazbaz bazQux Quxfoo Quxbaz QuxQux Foofoo Foobaz FooQux QUXBAZ \
                foofoofoo foofoobaz foofooQux foobazfoo foobazbaz foobazQux fooQuxfoo fooQuxbaz fooQuxQux bazfoofoo bazfoobaz \
                bazfooQux bazbazfoo bazbazbaz bazbazQux bazQuxfoo bazQuxbaz bazQuxQux Quxfoofoo Quxfoobaz QuxfooQux Quxbazfoo \
                Quxbazbaz QuxbazQux QuxQuxfoo QuxQuxbaz QuxQuxQux Foofoofoo Foofoobaz FoofooQux Foobazfoo Foobazbaz FoobazQux \
                FooQuxfoo FooQuxbaz FooQuxQux QUXBAZBAZ",
        },
        Pinned {
            name: "mixed_case_chk",
            aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDCASE",
            dic: "foo/X Qux/X bar/X",
            pieces: "foo Qux Foo bar Bar",
            extra: "",
            right: true,
            listed: "foo Qux Foo bar Bar foofoo foobar Quxfoo Quxbar Foofoo Foobar barfoo barbar Barfoo Barbar foofoofoo foofoobar \
                foobarfoo foobarbar Quxfoofoo Quxfoobar Quxbarfoo Quxbarbar Foofoofoo Foofoobar Foobarfoo Foobarbar barfoofoo \
                barfoobar barbarfoo barbarbar Barfoofoo Barfoobar Barbarfoo Barbarbar",
        },
        Pinned {
            name: "mixed_forbid",
            aff: "COMPOUNDFLAG X\nFORBIDDENWORD F",
            dic: "foo/X Qux/X fooQux/F bar/X",
            pieces: "foo Qux Foo bar",
            extra: "FooQux fooQux FOOQUX",
            right: false,
            listed: "fooQux fooFoo QuxFoo FooQux FooFoo barFoo foofooFoo fooQuxFoo fooFoofoo fooFooQux fooFooFoo fooFoobar \
                foobarFoo QuxfooFoo QuxQuxFoo QuxFoofoo QuxFooQux QuxFooFoo QuxFoobar QuxbarFoo FoofooFoo FooQuxFoo FooFoofoo \
                FooFooQux FooFooFoo FooFoobar FoobarFoo barfooFoo barQuxFoo barFoofoo barFooQux barFooFoo barFoobar barbarFoo \
                FOOQUX",
        },
        Pinned {
            name: "mixed_onlyc",
            aff: "COMPOUNDFLAG X\nONLYINCOMPOUND O",
            dic: "foo/XO Qux/X bar/X",
            pieces: "foo Qux Foo bar Bar",
            extra: "",
            right: true,
            listed: "Qux bar Bar foofoo fooQux foobar Quxfoo QuxQux Quxbar Foofoo FooQux Foobar barfoo barQux barbar Barfoo BarQux \
                Barbar foofoofoo foofooQux foofoobar fooQuxfoo fooQuxQux fooQuxbar foobarfoo foobarQux foobarbar Quxfoofoo \
                QuxfooQux Quxfoobar QuxQuxfoo QuxQuxQux QuxQuxbar Quxbarfoo QuxbarQux Quxbarbar Foofoofoo FoofooQux Foofoobar \
                FooQuxfoo FooQuxQux FooQuxbar Foobarfoo FoobarQux Foobarbar barfoofoo barfooQux barfoobar barQuxfoo barQuxQux \
                barQuxbar barbarfoo barbarQux barbarbar Barfoofoo BarfooQux Barfoobar BarQuxfoo BarQuxQux BarQuxbar Barbarfoo \
                BarbarQux Barbarbar",
        },
        Pinned {
            name: "mixed_force",
            aff: "COMPOUNDFLAG X\nFORCEUCASE U",
            dic: "foo/X Qux/XU bar/XU",
            pieces: "foo Qux Foo bar Bar",
            extra: "",
            right: true,
            listed: "foo Qux Foo bar Bar foofoo fooQux Quxfoo QuxQux Quxbar Foofoo FooQux Foobar barfoo barQux Barfoo BarQux Barbar \
                foofoofoo foofooQux fooQuxfoo fooQuxQux fooQuxbar foobarfoo foobarQux Quxfoofoo QuxfooQux Quxfoobar QuxQuxfoo \
                QuxQuxQux QuxQuxbar Quxbarfoo QuxbarQux Quxbarbar Foofoofoo FoofooQux Foofoobar FooQuxfoo FooQuxQux FooQuxbar \
                Foobarfoo FoobarQux Foobarbar barfoofoo barfooQux barQuxfoo barQuxQux barQuxbar barbarfoo barbarQux Barfoofoo \
                BarfooQux Barfoobar BarQuxfoo BarQuxQux BarQuxbar Barbarfoo BarbarQux Barbarbar",
        },
        Pinned {
            name: "keep_nosug",
            aff: "COMPOUNDFLAG X\nKEEPCASE K\nNOSUGGEST K",
            dic: "foo/X baz/XK",
            pieces: "foo baz Foo Baz",
            extra: "",
            right: true,
            listed: "foo baz Foo foofoo foobaz bazfoo bazbaz Foofoo Foobaz foofoofoo foofoobaz foobazfoo foobazbaz bazfoofoo \
                bazfoobaz bazbazfoo bazbazbaz Foofoofoo Foofoobaz Foobazfoo Foobazbaz",
        },
        Pinned {
            name: "root_affix",
            aff: "COMPOUNDFLAG Y\nCOMPOUNDWORDMAX 2\nCOMPOUNDROOT R\nCOMPOUNDPERMITFLAG P\nSFX A Y 1\nSFX A 0 s/RP .\nPFX B Y 1\nPFX B 0 un/RP .",
            dic: "foo/YAB bar/YAB baz/YR",
            pieces: "foo foos unfoo bar bars baz",
            extra: "",
            right: true,
            listed: "foo foos unfoo bar bars baz foofoo foofoos foounfoo foobar foobars foosfoo foosfoos foosunfoo foosbar foosbars \
                unfoofoo unfoofoos unfoounfoo unfoobar unfoobars barfoo barfoos barunfoo barbar barbars barsfoo barsfoos \
                barsunfoo barsbar barsbars",
        },
        Pinned {
            name: "syllable_hu_suffix",
            aff: "LANG hu_HU\nCOMPOUNDMIN 1\nCOMPOUNDFLAG Y\nCOMPOUNDWORDMAX 2\nCOMPOUNDSYLLABLE 4 aeiou\nSFX C Y 1\nSFX C 0 ata .\nSFX D Y 1\nSFX D 0 ete/Z .\nSFX E Y 1\nSFX E 0 t/F .\nSFX F Y 1\nSFX F 0 ete .",
            dic: "ba/Y bab/YCDE",
            pieces: "ba bab babata babete babtete",
            extra: "bababababata babababababete bababababtete",
            right: true,
            listed: "ba bab babata babete babtete baba babab bababata bababete bababtete babba babbab babbabata babbabete \
                babbabtete bababa bababab babababata babababtete bababba bababbab bababbabata bababbabtete babbaba babbabab \
                babbababata babbababtete babbabba babbabbab babbabbabata babbabbabtete bababababata bababababtete",
        },
        Pinned {
            name: "near_prefix_shadow",
            aff: "COMPOUNDBEGIN x\nCOMPOUNDMIDDLE y\nCOMPOUNDEND z\nNEEDAFFIX h\nONLYINCOMPOUND o\nSFX S Y 1\nSFX S 0 ad .\nPFX P Y 1\nPFX P 0 fa .\nSFX T Y 1\nSFX T 0 hr .\nPFX V Y 1\nPFX V 0 ve .",
            dic: "fahr/x rad/y verkehr/zo hrrad/P fahrrad/h rke/VT fahrr/S",
            pieces: "fahr rad verkehr",
            extra: "fahrradverkehr fahrradverkehrr fahrradverkehrad",
            right: true,
            listed: "fahr rad verkehr fahrrad fahrverkehr fahrradverkehr",
        },
    ];
}
