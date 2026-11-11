//! Reading the compounding directives of an `.aff` or compact-format line, and writing them back.

use super::*;

/// Most `CHECKCOMPOUNDPATTERN` lines (the Dutch dictionary has 52).
const MAX_PATTERNS: usize = 512;

/// Longest text on either side of a `CHECKCOMPOUNDPATTERN`, in bytes.
const MAX_PATTERN_BYTES: usize = 64;

/// Most vowels a `COMPOUNDSYLLABLE` line may list.
const MAX_VOWELS: usize = 64;

/// The single-flag compounding directives, in the order [`Raw::flags`] keeps them.
const FLAG_DIRECTIVES: [&str; 8] =
    ["COMPOUNDFLAG", "COMPOUNDBEGIN", "COMPOUNDMIDDLE", "COMPOUNDEND", "COMPOUNDPERMITFLAG", "COMPOUNDFORBIDFLAG", "COMPOUNDROOT", "FORCEUCASE"];

pub(super) const FLAG: usize = 0;

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

pub(super) const REP: usize = 1;

const CASE: usize = 2;

const TRIPLE: usize = 3;

const SIMPLIFIED: usize = 4;

/// Compounding directives as read from the `.aff` or the compact format, before flags get ids.
#[derive(Clone, Debug, Default)]
pub(in crate::affix) struct Raw {
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
    pub(in crate::affix) fn read(&mut self, f: &[&str], one_flag: &mut impl FnMut(&str) -> Result<FlagKey, String>) -> Result<bool, String> {
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
                self.syllable = Some((n.min(super::super::MAX_WORD_BYTES), vowels));
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
    pub(in crate::affix) fn keys(&self, mode: FlagMode) -> Result<Vec<FlagKey>, String> {
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
    pub(in crate::affix) fn build(&self, mode: FlagMode, id_of: &impl Fn(FlagKey) -> Option<u16>) -> Result<Option<Box<Compounding>>, String> {
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
pub(super) enum Need {
    Any,
    /// The flag of [`Compounding::marks`] at this index.
    Mark(u8),
    /// A flag no stem or rule carries.
    Never,
}

impl Need {
    pub(super) fn met(self, marks: u64) -> bool {
        match self {
            Need::Any => true,
            Need::Mark(i) => marks & (1 << i) != 0,
            Need::Never => false,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Pattern {
    pub(super) end: Box<str>,
    pub(super) end_need: Need,
    pub(super) begin: Box<str>,
    pub(super) begin_need: Need,
    /// `0` for `end`: the part before is a stem without affixes.
    pub(super) unmodified: bool,
}

/// The flag-based compounding of a dictionary (see the module docs).
#[derive(Clone, Debug, Default)]
pub(in crate::affix) struct Compounding {
    pub(super) flag: Option<u16>,
    pub(super) begin: Option<u16>,
    pub(super) middle: Option<u16>,
    pub(super) end: Option<u16>,
    pub(super) permit: Option<u16>,
    pub(super) forbid: Option<u16>,
    pub(super) root: Option<u16>,
    pub(super) force_ucase: Option<u16>,
    pub(super) word_max: Option<usize>,
    /// The Hungarian rule: past `word_max` parts, at most this many vowels in the word.
    pub(super) syllables: Option<(usize, Box<[char]>)>,
    pub(super) dup: bool,
    pub(super) rep: bool,
    pub(super) case: bool,
    pub(super) triple: bool,
    pub(super) simplified: bool,
    pub(in crate::affix) no_suggestions: bool,
    /// A compound of three or more parts that splits into two words is wrong (unless
    /// `NOSPLITSUGS`: the CLI ties the two to each other).
    pub(super) two_words: bool,
    pub(super) patterns: Vec<Pattern>,
    /// Flags tracked per part (`FORCEUCASE`, the patterns' flags): bit `i` of [`Info::marks`].
    pub(super) marks: Vec<u16>,
    /// The directives as read, written back by [`Compounding::write`].
    pub(super) raw: Raw,
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
    pub(in crate::affix) fn write(&self, out: &mut String, write_flag: &impl Fn(FlagKey, &mut String)) {
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

/// The compounding directives a compact-format line may hold, read like the `.aff`'s.
pub(in crate::affix) fn is_directive(dir: &str) -> bool {
    FLAG_DIRECTIVES.contains(&dir)
        || SWITCHES.contains(&dir)
        || matches!(dir, "COMPOUNDWORDMAX" | "COMPOUNDSYLLABLE" | "LANG" | "MAXCPDSUGS" | "NOSPLITSUGS" | "CHECKCOMPOUNDPATTERN")
}
