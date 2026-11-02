//! Hunspell-style affix dictionaries — the subset the SJP.PL Polish dictionary (`pl_PL`) uses.
//!
//! A word is correct when it is a stem, a stem with one suffix, a stem with one prefix, or a stem
//! with a prefix and a suffix that both allow cross products; each rule applies only to stems
//! carrying its flag and ending (suffixes) or starting (prefixes) as its condition says. Highly
//! inflected languages store millions of forms this way in a few hundred thousand stems: the
//! Polish dictionary's ~350k stems and ~7,500 rules accept ~4.5 million forms.
//!
//! The rules and stems come from a Hunspell `.aff`/`.dic` pair ([`AffixDict::from_hunspell`]),
//! read at runtime from the dictionaries installed on the system ([`crate::hunspell`]). They can
//! also be stored in WordCraft's compact format ([`AffixDict::to_bytes`]): the rules as text
//! lines and the stems front-coded, raw-deflated together (the Polish dictionary is under 1 MB).
//!
//! Supported: one-character flags (any Unicode character — `FLAG UTF-8`), two-character flags
//! (`FLAG long`) and numeric flags (`FLAG num`: comma-separated numbers from 0 to 65,535), up to
//! 65,536 distinct flags in all ([`FlagMode`]), flag aliases (`AF`: a number standing for a set of
//! flags), `PFX`/`SFX` with strip, append and condition (`.`, letters, `[abc]`, `[^abc]`), cross
//! products, continuation classes on suffixes (at most two suffixes in a row, as in Hunspell),
//! `FORBIDDENWORD`, `NOSUGGEST`, `KEEPCASE` (the stem is a word only in its own letter case),
//! `NEEDAFFIX`, `CIRCUMFIX`, `FULLSTRIP`, `CHECKSHARPS` (German `SS` for `ß` in capitals), input conversion (`ICONV`: ligatures such as `ﬁ` read as
//! `fi`), output conversion of suggestions (`OCONV`) and ignored characters (`IGNORE`), and the
//! suggestion tables `TRY`, `KEY`, `MAP` and `REP`.
//! Compounds by `COMPOUNDRULE` (patterns of flags with `*` and `?`, as SCOWL's English
//! dictionaries use for ordinals: `21st`, `112th`; with long or numeric flags each flag is written
//! in parentheses, `(aa)*(bb)`), with `ONLYINCOMPOUND` and `COMPOUNDMIN`; the parts of such a
//! compound are stems, without affixes, as in Hunspell.
//! Morphological data (`AM` aliases, the fields after a stem) is skipped. Rejected: flag-based
//! compounding (`COMPOUNDFLAG`, `COMPOUNDBEGIN`…) and the other directives that change which
//! words are correct.
//!
//! Each distinct flag gets a dense `u16` id; a stem's flags and a rule's continuation classes are
//! a sorted set of ids, interned once in a flat table, so membership is a binary search in a few
//! ids and a dictionary with thousands of flags (Serbian: 1,903) costs no more per lookup.
//!
//! Lookups never panic: stems sit in one string, found through an open-addressing hash table, and
//! a probe allocates only small scratch strings for the candidate roots it tries.

use std::collections::HashMap;
use std::hash::Hasher;

use crate::patterns::{FxHasher, FxMap};

const MAGIC: &str = "WCAFF1";
/// Most bytes the compact format inflates to, written or read.
const MAX_INFLATED: usize = 64 << 20;
/// Longest word (in bytes) looked up; longer input is never a dictionary word.
pub const MAX_WORD_BYTES: usize = 120;
/// Empty slot in the stem hash table.
const EMPTY: u32 = u32::MAX;
/// Longest input converted (`ICONV`) and stripped of `IGNORE` characters before a lookup: longer
/// input can't come down to [`MAX_WORD_BYTES`] in a real dictionary, so it is no word and is
/// never converted (the conversion would cost time on hostile input for nothing).
const MAX_INPUT_BYTES: usize = 4 * MAX_WORD_BYTES;
/// Most pairs an `ICONV` or `OCONV` table may hold (the Korean dictionary's hold 11,172 each).
const MAX_CONV_PAIRS: usize = 1 << 16;
/// Longest text on either side of an `ICONV` or `OCONV` pair, in bytes.
const MAX_CONV_BYTES: usize = 64;
/// Most flags one `.dic` entry or one continuation class field may carry. LibreOffice's Turkish
/// dictionary has 920 entries over 1,024 flags (`abart`, `savaş`: up to 3,926), so the cap sits
/// well above that; a longer field is hostile and is refused (`.aff`) or skipped (`.dic`, though
/// its `FORBIDDENWORD` and `NOSUGGEST` flags still apply to the word). What all the flags of a
/// dictionary may cost together is bounded by `MAX_SET_DATA`.
pub const MAX_FLAGS_PER_ENTRY: usize = 8192;
/// Most flags one stem may carry once its homonyms (the same word on several `.dic` lines) are
/// merged, and so most flags a stem line of the compact format may carry. Larger than
/// [`MAX_FLAGS_PER_ENTRY`] because merged sets aren't `.dic` fields; a homonym that would take the
/// set past it adds only its `FORBIDDENWORD` and `NOSUGGEST` flags, as an exclusion is never lost.
pub const MAX_FLAGS_PER_STEM: usize = 4 * MAX_FLAGS_PER_ENTRY;
/// Most `AF` aliases, and most bytes of alias text in all: each alias is read once and shared by
/// the stems using it, so these bound the flags an `AF` table can make a dictionary hold
/// (the largest Language pack's table: 1,596 aliases, 19 KB).
const MAX_ALIASES: usize = 1 << 16;
const MAX_ALIAS_BYTES: usize = 4 << 20;
/// Most flags all distinct flag sets (stems' and continuation classes') may hold together: a lookup
/// tests each distinct continuation set it meets at most once, so this bounds its cost.
const MAX_SET_DATA: usize = 1 << 23;

/// Most distinct flags a dictionary may use: each gets a `u16` id.
const MAX_DISTINCT_FLAGS: usize = 1 << 16;

/// Directives that change which words are correct in ways this engine doesn't implement.
const UNSUPPORTED: &[&str] = &[
    "COMPLEXPREFIXES",
    "COMPOUNDBEGIN",
    "COMPOUNDEND",
    "COMPOUNDFIRST",
    "COMPOUNDFLAG",
    "COMPOUNDLAST",
    "COMPOUNDMIDDLE",
    "COMPOUNDPERMITFLAG",
    "PSEUDOROOT",
];

/// Most `COMPOUNDRULE` patterns kept, and most tokens in one: a hostile `.aff` can't make a
/// compound check cost more than a few thousand stem lookups per word.
const MAX_COMPOUND_RULES: usize = 64;
const MAX_COMPOUND_TOKENS: usize = 32;
/// Longest word (in bytes) tried as a compound: each try looks up every substring.
const MAX_COMPOUND_BYTES: usize = 48;

/// How often a `COMPOUNDRULE` token's flag may repeat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Repeat {
    Once,
    /// `?`
    Optional,
    /// `*`
    Any,
}

/// A `COMPOUNDRULE` pattern as written (`n*1t`, or `(aa)*(bb)` with long or numeric flags): its
/// tokens, each a flag and how often it repeats.
fn parse_compound_rule(pattern: &str, mode: FlagMode) -> Result<Vec<(FlagKey, Repeat)>, String> {
    let mut out: Vec<(FlagKey, Repeat)> = Vec::new();
    let mut chars = pattern.chars().filter(|&c| c != EMOJI_SELECTOR);
    while let Some(c) = chars.next() {
        match c {
            '*' | '?' => {
                let Some(last) = out.last_mut().filter(|t| t.1 == Repeat::Once) else {
                    return Err(format!("COMPOUNDRULE `{pattern}`: `{c}` follows no flag"));
                };
                last.1 = if c == '*' { Repeat::Any } else { Repeat::Optional };
            }
            '(' => {
                let mut inner = String::new();
                loop {
                    match chars.next() {
                        Some(')') => break,
                        Some(c) if inner.len() < MAX_COMPOUND_BYTES => inner.push(c),
                        _ => return Err(format!("COMPOUNDRULE `{pattern}`: `(` is not closed")),
                    }
                }
                let key = mode.parse_one(&inner).map_err(|e| format!("COMPOUNDRULE `{pattern}`: {e}"))?;
                out.push((key, Repeat::Once));
            }
            ')' => return Err(format!("COMPOUNDRULE `{pattern}`: `)` closes no group")),
            c if mode == FlagMode::Char => out.push((FlagKey::from(c), Repeat::Once)),
            _ => return Err(format!("COMPOUNDRULE `{pattern}`: long and numeric flags are written in ( )")),
        }
        if out.len() > MAX_COMPOUND_TOKENS {
            return Err(format!("COMPOUNDRULE `{pattern}`: more than {MAX_COMPOUND_TOKENS} flags"));
        }
    }
    if out.is_empty() {
        return Err("COMPOUNDRULE with no flags".into());
    }
    Ok(out)
}

/// One character class of a rule's condition.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Cond {
    Any,
    Char(char),
    Set { negated: bool, chars: Box<[char]> },
}

impl Cond {
    fn matches(&self, c: char) -> bool {
        match self {
            Cond::Any => true,
            Cond::Char(x) => *x == c,
            Cond::Set { negated, chars } => chars.contains(&c) != *negated,
        }
    }
}

/// Parse a Hunspell condition (`.`, `[^i]e`, `[km]anie`…).
fn parse_cond(text: &str) -> Result<Box<[Cond]>, String> {
    if text == "." {
        return Ok(Box::new([]));
    }
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '.' => out.push(Cond::Any),
            '[' => {
                let mut set = Vec::new();
                let mut negated = false;
                let mut closed = false;
                let mut first = true;
                for d in chars.by_ref() {
                    if d == ']' {
                        closed = true;
                        break;
                    }
                    if first && d == '^' {
                        negated = true;
                    } else {
                        set.push(d);
                    }
                    first = false;
                }
                if !closed {
                    return Err(format!("unclosed `[` in condition `{text}`"));
                }
                out.push(Cond::Set { negated, chars: set.into_boxed_slice() });
            }
            ']' => return Err(format!("stray `]` in condition `{text}`")),
            c => out.push(Cond::Char(c)),
        }
    }
    Ok(out.into_boxed_slice())
}

/// How many suffixes a word may carry in a row through continuation classes (`work` +
/// `able/B` + `s` → `workables` is two). Hunspell strips at most two suffixes, so a third
/// chained suffix is not a word; the bound also keeps hostile input from walking deep recursions.
const MAX_CHAIN: u8 = 2;
/// How many suffixes may sit under the outermost one: the depth `form_has_flag` walks.
const INNER_CHAIN: u8 = MAX_CHAIN - 1;
/// Most `ss` pairs of a word in capitals that CHECKSHARPS reads as `ß` (each combination is tried:
/// at most 31 variants).
const MAX_SHARPS: usize = 5;
/// Most forms [`AffixDict::forms`] lists for one stem (real stems have a few hundred at most).
pub const MAX_FORMS: usize = 4096;
/// Most rules [`AffixDict::forms`] tries along continuation chains and prefix–suffix cross
/// products for one stem: real dictionaries need a few thousand, hostile ones are cut off.
const MAX_FORMS_WORK: usize = 1 << 18;

/// One prefix or suffix rule.
#[derive(Clone, Debug)]
struct Rule {
    /// The rule's flag (a dense id, see [`AffixDict::flag_keys`]).
    flag: u16,
    cross: bool,
    strip: Box<str>,
    add: Box<str>,
    /// Continuation classes (`SFX A 0 able/B`): flags the derived form carries, licensing
    /// further affixes, as a flag set id (0: none). Chained application is bounded by [`MAX_CHAIN`].
    cont: u32,
    cond: Box<[Cond]>,
    cond_text: Box<str>,
}

impl Rule {
    /// Does `root` end (suffix rule) or start (prefix rule) as the condition says?
    fn cond_matches(&self, root: &str, suffix: bool) -> bool {
        if self.cond.is_empty() {
            return true;
        }
        if suffix {
            let mut it = root.chars().rev();
            self.cond.iter().rev().all(|c| it.next().is_some_and(|x| c.matches(x)))
        } else {
            let mut it = root.chars();
            self.cond.iter().all(|c| it.next().is_some_and(|x| c.matches(x)))
        }
    }
}

/// Suffix rules sharing an appended string and a stripped one: one stem lookup serves them all.
#[derive(Clone, Debug, Default)]
struct SfxGroup {
    strip: Box<str>,
    rules: Vec<u32>,
    /// The rules of `rules` with continuation classes: the only ones a chain can go through.
    cont_rules: Vec<u32>,
}

/// A stem list with affix rules (see the module docs).
#[derive(Clone, Debug, Default)]
pub struct AffixDict {
    /// All stems, concatenated, sorted by bytes.
    words: String,
    offs: Vec<u32>,
    /// Per stem: its flag set id.
    flags: Vec<u32>,
    /// Distinct flag sets, each a sorted run of flag ids in `set_data`: set `i` is
    /// `set_data[set_offs[i]..set_offs[i + 1]]`. Set 0 is the empty set.
    set_offs: Vec<u32>,
    set_data: Vec<u16>,
    /// The `FORBIDDENWORD` flag; stems carrying it are not words.
    forbidden: Option<u16>,
    /// Stems with NOSUGGEST are correct, but none of their forms are suggested.
    no_suggest: Option<u16>,
    /// Stems with SUBSTANDARD are correct too, and likewise never suggested.
    substandard: Option<u16>,
    /// Open-addressing hash table of stem indices (`EMPTY` = free).
    table: Vec<u32>,
    /// How the source wrote flags (kept for the compact format).
    flag_mode: FlagMode,
    /// Each flag id's flag as the source wrote it, sorted (the id is the index).
    flag_keys: Vec<FlagKey>,
    /// Whether any rule has continuation classes (lookups skip the chain walk otherwise).
    has_cont: bool,
    /// Per flag id: whether some rule's continuation classes hold it, so that a form (not only
    /// a stem) can carry it.
    cont_flag: Vec<bool>,
    prefixes: Vec<Rule>,
    suffixes: Vec<Rule>,
    /// Appended string → groups of suffix rules by stripped string.
    sfx_index: FxMap<Box<str>, Vec<SfxGroup>>,
    /// Per flag id: the suffix rules of that flag, in rule order.
    sfx_by_flag: Vec<Vec<u32>>,
    /// Longest appended suffix, in chars.
    max_sfx_chars: usize,
    try_chars: String,
    key: String,
    map: Vec<Vec<char>>,
    rep: Vec<(String, String)>,
    /// The `KEEPCASE` flag: such stems are words only in their own letter case.
    keepcase: Option<u16>,
    /// `NEEDAFFIX`: a stem carrying it is not a word on its own, and a rule carrying it in its
    /// continuation classes gives forms that need one more affix.
    need_affix: Option<u16>,
    /// `CIRCUMFIX`: a prefix or suffix carrying it in its continuation classes is only valid
    /// together with a suffix or prefix that carries it too.
    circumfix: Option<u16>,
    /// `FULLSTRIP`: an affix may strip (and so replace) a whole stem.
    full_strip: bool,
    /// `CHECKSHARPS`: in a word in capitals, `SS` may stand for `ß` (`STRASSE` for `Straße`), and
    /// a `KEEPCASE` word with `ß` may be capitalized and written in capitals with `SS`.
    check_sharps: bool,
    /// `ICONV`: input conversions applied before a lookup.
    iconv: Conv,
    /// `OCONV`: output conversions applied to suggestions.
    oconv: Conv,
    /// `IGNORE`: characters removed from input words (and from the stems and appended texts).
    ignore: Box<[char]>,
    /// Provenance and licence lines, kept through [`AffixDict::to_bytes`].
    comments: Vec<String>,
    /// `COMPOUNDRULE` patterns: per token, the flag's id (`None`: a flag no stem carries, which
    /// matches no part) and how often it may repeat.
    compound_rules: Vec<Vec<(Option<u16>, Repeat)>>,
    /// The same patterns as written, for [`AffixDict::to_bytes`].
    compound_text: Vec<String>,
    /// The `ONLYINCOMPOUND` flag: such stems are words only as parts of a compound.
    only_in_compound: Option<u16>,
    /// `COMPOUNDMIN`, in characters.
    compound_min: usize,
    /// The first characters of the stems a compound part can be (their flags are in a
    /// `COMPOUNDRULE`), so a word starting otherwise is no compound: SCOWL's ordinals start with
    /// a digit, and a misspelled word costs one lookup here instead of one per cut.
    compound_firsts: Vec<char>,
}

/// An `ICONV` or `OCONV` table: `(from, to)` pairs in source order, found by the first character
/// of `from` so that converting a word costs a look at the few pairs that could start at each
/// character, not at the whole table (the Korean tables hold 11,172 pairs each).
#[derive(Clone, Debug, Default)]
struct Conv {
    pairs: Vec<(String, String)>,
    /// `from` → index into `pairs` (of equal ones, the later pair: it is the one that wins).
    by_from: FxMap<String, u32>,
    /// Longest `from`, in bytes.
    longest: usize,
}

impl Conv {
    /// The table of `pairs` (checked by [`Rules::push_conv`]: non-empty `from`, bounded sizes).
    fn new(pairs: Vec<(String, String)>) -> Conv {
        let mut by_from: FxMap<String, u32> = FxMap::default();
        for (i, (from, _)) in pairs.iter().enumerate() {
            if let Ok(i) = u32::try_from(i) {
                by_from.insert(from.clone(), i);
            }
        }
        let longest = pairs.iter().map(|p| p.0.len()).max().unwrap_or(0);
        Conv { pairs, by_from, longest }
    }

    fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// The pair to apply at the start of `rest`: the longest `from` that starts it. One hash
    /// lookup per prefix of `rest` up to the longest `from` ([`MAX_CONV_BYTES`]), however many
    /// pairs there are (craft-libs#489 review: 65,536 pairs on one first character).
    fn at(&self, rest: &str) -> Option<&(String, String)> {
        let ends = rest.char_indices().map(|(i, _)| i).skip(1).chain(std::iter::once(rest.len()));
        let ends: Vec<usize> = ends.take_while(|&e| e <= self.longest).collect();
        ends.iter().rev().find_map(|&e| self.by_from.get(rest.get(..e)?)).and_then(|&i| self.pairs.get(i as usize))
    }

    /// `w` converted: at each position the longest matching `from` is replaced by its `to`, and
    /// characters in `drop` are removed (outside the replacements too). Borrowed when nothing
    /// changes.
    fn apply<'a>(&self, w: &'a str, drop: &[char]) -> std::borrow::Cow<'a, str> {
        if self.is_empty() && drop.is_empty() {
            return std::borrow::Cow::Borrowed(w);
        }
        // One pass: the input is copied only from the first change on.
        let mut out: Option<String> = None;
        let mut rest = w;
        while let Some(c) = rest.chars().next() {
            let done = w.get(..w.len() - rest.len()).unwrap_or_default();
            match self.at(rest) {
                Some((from, to)) => {
                    out.get_or_insert_with(|| done.to_string()).extend(to.chars().filter(|c| !drop.contains(c)));
                    rest = rest.get(from.len()..).unwrap_or_default();
                }
                None => {
                    if drop.contains(&c) {
                        out.get_or_insert_with(|| done.to_string());
                    } else if let Some(o) = out.as_mut() {
                        o.push(c);
                    }
                    rest = rest.get(c.len_utf8()..).unwrap_or_default();
                }
            }
        }
        out.map_or(std::borrow::Cow::Borrowed(w), std::borrow::Cow::Owned)
    }
}

fn hash_str(s: &str) -> u64 {
    let mut h = FxHasher::default();
    h.write(s.as_bytes());
    h.finish()
}

/// Letter case of a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Casing {
    /// No capitals (`kot`).
    Lower,
    /// First letter capital, no other (`Kot`).
    Initial,
    /// Every cased letter a capital, at least two (`KOT`).
    Upper,
    /// Anything else (`McDonald`, `kOT`).
    Mixed,
}

/// The casing of `w`.
///
/// `ß` has no capital of its own in Hunspell's tables and counts as neither case, as there:
/// `STRAßE` is in capitals.
pub fn casing(w: &str) -> Casing {
    let (mut upper, mut lower, mut first_upper) = (0usize, 0usize, false);
    for (i, c) in w.chars().enumerate() {
        if c == 'ß' {
            continue;
        }
        if c.is_uppercase() {
            upper += 1;
            if i == 0 {
                first_upper = true;
            }
        } else if c.is_lowercase() {
            lower += 1;
        }
    }
    if upper == 0 {
        Casing::Lower
    } else if upper == 1 && first_upper {
        Casing::Initial
    } else if lower == 0 {
        Casing::Upper
    } else {
        Casing::Mixed
    }
}

/// `w` with its first letter a capital.
pub fn capitalize(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Parsed rules and tables, shared by the Hunspell reader and the compact format.
#[derive(Default)]
struct Rules {
    prefixes: Vec<RawRule>,
    suffixes: Vec<RawRule>,
    try_chars: String,
    key: String,
    map: Vec<Vec<char>>,
    rep: Vec<(String, String)>,
    /// The `FORBIDDENWORD` flag: stems carrying it are listed but never correct.
    forbidden: Option<FlagKey>,
    no_suggest: Option<FlagKey>,
    substandard: Option<FlagKey>,
    /// The `KEEPCASE` flag.
    keepcase: Option<FlagKey>,
    need_affix: Option<FlagKey>,
    circumfix: Option<FlagKey>,
    full_strip: bool,
    check_sharps: bool,
    iconv: Vec<(String, String)>,
    oconv: Vec<(String, String)>,
    ignore: Vec<char>,
    /// `COMPOUNDRULE` patterns, as written (`n*1t`); read in the `FLAG` mode by
    /// [`AffixDict::build`], since the `FLAG` line may come after them.
    compound_rules: Vec<String>,
    /// The `ONLYINCOMPOUND` flag.
    only_in_compound: Option<FlagKey>,
    /// `COMPOUNDMIN`: the shortest part of a compound, in characters (Hunspell's default is 3).
    compound_min: Option<usize>,
    /// `AF` flag aliases: alias `n` (1-based) stands for `aliases[n - 1]`.
    aliases: Vec<String>,
    /// The `AF` table opened with its count line, so flag fields are alias numbers. A table
    /// without one is rejected, as in Hunspell: its lines are ignored.
    aliases_on: bool,
    /// Total length of the alias texts, capped at [`MAX_ALIAS_BYTES`].
    alias_bytes: usize,
    /// How flags are written, from the `FLAG` line.
    mode: FlagMode,
    /// Whether a flag has been read: a later `FLAG` line can't change how the earlier ones read.
    flag_read: bool,
}

impl Rules {
    /// A `FLAG` line. One that changes the mode after a flag was read is refused: Hunspell loads
    /// such a file, reading the earlier flags in the earlier mode, but no real dictionary does
    /// this, and refusing it is safe where half-reading it would not be.
    fn set_mode(&mut self, name: &str) -> Result<(), String> {
        let mode = FlagMode::from_name(name).ok_or_else(|| format!("FLAG {name} is not supported"))?;
        if self.flag_read && mode != self.mode {
            return Err(format!("FLAG {name} after flags were read"));
        }
        self.mode = mode;
        Ok(())
    }

    /// Add a pair to the `ICONV` (`output` false) or `OCONV` table: at most [`MAX_CONV_PAIRS`]
    /// pairs of at most [`MAX_CONV_BYTES`] each side. A pair with nothing to convert is skipped.
    fn push_conv(&mut self, output: bool, from: &str, to: &str) -> Result<(), String> {
        let (name, table) = if output { ("OCONV", &mut self.oconv) } else { ("ICONV", &mut self.iconv) };
        if from.len() > MAX_CONV_BYTES || to.len() > MAX_CONV_BYTES {
            return Err(format!("{name} pair longer than {MAX_CONV_BYTES} bytes"));
        }
        if table.len() >= MAX_CONV_PAIRS {
            return Err(format!("{name} table larger than {MAX_CONV_PAIRS} pairs"));
        }
        if !from.is_empty() {
            table.push((from.to_string(), to.to_string()));
        }
        Ok(())
    }

    /// The one flag of a rule header or a single-flag directive.
    fn one_flag(&mut self, field: &str) -> Result<FlagKey, String> {
        self.flag_read = true;
        self.mode.parse_one(field)
    }

    /// Flags as written in a `.dic` entry or a continuation class: the flag characters
    /// themselves, or with an `AF` table the alias its leading digits number, as Hunspell reads
    /// them (`2x` is alias 2). A field without digits or an undefined alias reads as no flags:
    /// the stem is still a word, the rule still applies, without the flags (Hunspell warns).
    fn flags<'a>(&'a self, written: &'a str) -> &'a str {
        if !self.aliases_on {
            return written;
        }
        self.alias_index(written).and_then(|i| self.aliases.get(i)).map_or("", String::as_str)
    }

    /// With an `AF` table, the index in `aliases` of the alias `written` numbers, if defined.
    fn alias_index(&self, written: &str) -> Option<usize> {
        if !self.aliases_on {
            return None;
        }
        let digits = written.bytes().take_while(u8::is_ascii_digit).count();
        let i = written.get(..digits)?.parse::<usize>().ok()?.checked_sub(1)?;
        (i < self.aliases.len()).then_some(i)
    }

    /// `w` without the `IGNORE` characters.
    fn strip_ignored(&self, w: &str) -> String {
        if self.ignore.is_empty() { w.to_string() } else { w.chars().filter(|c| !self.ignore.contains(c)).collect() }
    }
}

/// The fields of an `.aff`/`.dic` line: Hunspell separates them with spaces and tabs only, so a
/// flag byte read as U+0085 or U+00A0 (see [`crate::hunspell::decode`]) stays a flag.
fn split_fields(line: &str) -> impl Iterator<Item = &str> {
    line.split([' ', '\t']).filter(|f| !f.is_empty())
}

/// A rule as parsed from text, before flags get ids: `(flag, cross, strip, add, cont, cond)`.
type RawRule = (FlagKey, bool, String, String, String, String);

/// Most distinct `TRY` characters kept; a hostile `.aff` could otherwise make every suggestion
/// round cost time and memory proportional to the file size.
const MAX_TRY_CHARS: usize = 128;

/// Keeps the first [`MAX_TRY_CHARS`] distinct characters of a `TRY` value, in order.
fn cap_try(s: &str) -> String {
    let mut out = String::new();
    let mut n = 0;
    for c in s.chars() {
        if n >= MAX_TRY_CHARS {
            break;
        }
        if !out.contains(c) {
            out.push(c);
            n += 1;
        }
    }
    out
}

/// `form`, unless it is empty: no word is, and `FULLSTRIP` can strip a stem down to nothing.
fn non_empty(form: String) -> Option<String> {
    Some(form).filter(|f| !f.is_empty())
}

/// `0` means empty in Hunspell's strip and append fields.
fn zero_empty(s: &str) -> String {
    if s == "0" { String::new() } else { s.to_string() }
}

/// U+FE0F VARIATION SELECTOR-16 asks for the emoji form of the character before it. A dictionary
/// that uses an emoji as a flag may write it after the flag (RLA-ES Spanish: `PFX ☎️`, `banco/S☎️`),
/// but it is not a flag of its own: it is dropped wherever flags are read.
const EMOJI_SELECTOR: char = '\u{fe0f}';

/// A flag as written in the source, before it gets a dense id: a character (one-character mode),
/// two characters (`FLAG long`, the first in the high 32 bits) or a number (`FLAG num`).
type FlagKey = u64;

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
    fn from_name(name: &str) -> Option<FlagMode> {
        match name {
            "UTF-8" | "DEFAULT" | "utf-8" | "default" | "UTF8" | "" => Some(FlagMode::Char),
            "long" | "LONG" => Some(FlagMode::Long),
            "num" | "NUM" => Some(FlagMode::Num),
            _ => None,
        }
    }

    /// The `FLAG` value the compact format writes (one-character mode writes no line).
    fn name(self) -> Option<&'static str> {
        match self {
            FlagMode::Char => None,
            FlagMode::Long => Some("long"),
            FlagMode::Num => Some("num"),
        }
    }

    /// The flags of a field (`.dic` flags, continuation classes), in source order, at most
    /// [`MAX_FLAGS_PER_ENTRY`]. An empty field has none.
    fn parse(self, field: &str) -> Result<Vec<FlagKey>, String> {
        self.parse_at_most(field, MAX_FLAGS_PER_ENTRY, false)
    }

    /// The flags of a field, at most `cap` of them. `lenient` (a `.dic` entry's flags) reads
    /// malformed items as Hunspell does, keeping the word (checked black-box with the `hunspell`
    /// CLI): under `FLAG num` an item that is no number from 0 to 65,535 (empty, `x2`, `-7`,
    /// `70000`) is flag 0, and under `FLAG long` an odd last character is dropped. Otherwise
    /// (continuation classes, which Hunspell drops the rule for, and the compact format) they are
    /// an error.
    fn parse_at_most(self, field: &str, cap: usize, lenient: bool) -> Result<Vec<FlagKey>, String> {
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
    fn parse_one(self, field: &str) -> Result<FlagKey, String> {
        match self.parse(field)?.as_slice() {
            [k] => Ok(*k),
            _ => Err(format!("`{field}` is not one flag")),
        }
    }

    /// Append the flags `keys` to `out` as the source writes them.
    fn write(self, keys: impl Iterator<Item = FlagKey>, out: &mut String) {
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
#[derive(Clone, Copy, Debug, Default)]
struct Excluded([Option<u16>; 5]);

impl Excluded {
    /// Does the flag set `set` carry an excluded flag?
    fn hits(self, set: &[u16]) -> bool {
        self.0.iter().flatten().any(|f| has(set, *f))
    }

    /// These exclusions and `flag` too.
    fn and(mut self, flag: Option<u16>) -> Excluded {
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
enum FlagProbe<'a> {
    Few(&'a [u16]),
    Many(Vec<u64>),
}

impl<'a> FlagProbe<'a> {
    fn new(flags: &'a [u16]) -> FlagProbe<'a> {
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
    fn meets(&self, set: &[u16]) -> bool {
        match self {
            FlagProbe::Few(flags) => intersects(set, flags),
            FlagProbe::Many(bits) => set.iter().any(|&f| bits.get(usize::from(f) >> 6).is_some_and(|w| w & (1 << (f & 63)) != 0)),
        }
    }
}

/// Does the sorted flag set `set` hold `flag`?
fn has(set: &[u16], flag: u16) -> bool {
    set.binary_search(&flag).is_ok()
}

/// Do two sorted flag sets share a flag? (A binary search per flag of the smaller one.)
fn intersects(a: &[u16], b: &[u16]) -> bool {
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    small.iter().any(|f| has(large, *f))
}

/// Directives an English dictionary may carry that this engine doesn't implement but that don't
/// change which English words are right, ignored by [`AffixDict::from_hunspell_english`]:
/// flag-based compounding (English dictionaries compound only ordinals, by `COMPOUNDRULE`, which
/// is read). Output conversion (`OCONV`) is read, for English as for every language: suggestions
/// are written as the dictionary says (`don’t`).
const IGNORED_IN_ENGLISH: &[&str] =
    &["COMPOUNDBEGIN", "COMPOUNDEND", "COMPOUNDFIRST", "COMPOUNDFLAG", "COMPOUNDLAST", "COMPOUNDMIDDLE", "COMPOUNDPERMITFLAG"];

/// Read Hunspell `.aff` text (already decoded to UTF-8). `english`: ignore the directives in
/// [`IGNORED_IN_ENGLISH`] instead of refusing them.
fn parse_aff(aff: &str, english: bool) -> Result<Rules, String> {
    let mut r = Rules::default();
    let mut cross: HashMap<(bool, FlagKey), bool> = HashMap::new();
    let mut map_header = false;
    let mut rep_header = false;
    let mut af_header = false;
    let mut iconv_header = false;
    let mut compound_header = false;
    let mut oconv_header = false;
    for (n, line) in aff.lines().enumerate() {
        let f: Vec<&str> = split_fields(line).collect();
        let Some(&dir) = f.first() else { continue };
        if dir.starts_with('#') {
            continue;
        }
        let at = |k: usize| f.get(k).copied().unwrap_or("");
        match dir {
            "SET" | "LANG" | "NAME" | "VERSION" | "HOME" | "WORDCHARS" | "MAXNGRAMSUGS" | "MAXDIFF" | "ONLYMAXDIFF" | "NOSPLITSUGS"
            | "SUGSWITHDOTS" | "WARN" | "FORBIDWARN" | "MAXCPDSUGS" => {}
            // Morphological aliases, and a flag that only matters for morphological generation:
            // the words it marks are correct.
            "AM" | "LEMMA_PRESENT" => {}
            "SUBSTANDARD" => r.substandard = Some(r.one_flag(at(1)).map_err(|e| format!("line {}: {dir} needs one flag: {e}", n + 1))?),
            // The first `AF`, `ICONV` or `OCONV` line is the table's count, as in Hunspell, even with a
            // trailing comment (`AF 1559 # aliases`).
            "AF" => {
                if !af_header {
                    af_header = true;
                    // A count below 1 rejects the table too: flags stay literal (`AF 0`).
                    r.aliases_on = at(1).parse::<usize>().is_ok_and(|n| n > 0);
                } else if r.aliases_on {
                    r.alias_bytes = r.alias_bytes.saturating_add(at(1).len());
                    if r.aliases.len() >= MAX_ALIASES || r.alias_bytes > MAX_ALIAS_BYTES {
                        return Err(format!("line {}: AF table over {MAX_ALIASES} aliases or {MAX_ALIAS_BYTES} bytes", n + 1));
                    }
                    r.aliases.push(at(1).to_string());
                }
            }
            "ICONV" | "OCONV" => {
                let header = if dir == "OCONV" { &mut oconv_header } else { &mut iconv_header };
                if !*header && at(1).parse::<usize>().is_ok() {
                    *header = true;
                } else if f.len() >= 3 {
                    r.push_conv(dir == "OCONV", at(1), at(2)).map_err(|e| format!("line {}: {e}", n + 1))?;
                }
            }
            "IGNORE" => r.ignore = at(1).chars().collect(),
            "KEEPCASE" => r.keepcase = Some(r.one_flag(at(1)).map_err(|e| format!("line {}: {dir} needs one flag: {e}", n + 1))?),
            // The first `COMPOUNDRULE` line is the table's count, as in Hunspell.
            "COMPOUNDRULE" => {
                if !compound_header && at(1).parse::<usize>().is_ok() {
                    compound_header = true;
                } else {
                    if r.compound_rules.len() >= MAX_COMPOUND_RULES {
                        return Err(format!("line {}: more than {MAX_COMPOUND_RULES} COMPOUNDRULE patterns", n + 1));
                    }
                    r.compound_rules.push(at(1).to_string());
                }
            }
            "ONLYINCOMPOUND" => {
                r.only_in_compound = Some(r.one_flag(at(1)).map_err(|e| format!("line {}: {dir} needs one flag: {e}", n + 1))?);
            }
            "COMPOUNDMIN" => {
                let min = at(1).parse::<usize>().map_err(|_| format!("line {}: COMPOUNDMIN needs a number", n + 1))?;
                r.compound_min = Some(min.clamp(1, MAX_COMPOUND_BYTES));
            }
            "NEEDAFFIX" => r.need_affix = Some(r.one_flag(at(1)).map_err(|e| format!("line {}: {dir} needs one flag: {e}", n + 1))?),
            "CIRCUMFIX" => r.circumfix = Some(r.one_flag(at(1)).map_err(|e| format!("line {}: {dir} needs one flag: {e}", n + 1))?),
            "FULLSTRIP" => r.full_strip = true,
            "CHECKSHARPS" => r.check_sharps = true,
            "FLAG" => r.set_mode(at(1)).map_err(|e| format!("line {}: {e}", n + 1))?,
            "FORBIDDENWORD" => r.forbidden = Some(r.one_flag(at(1)).map_err(|e| format!("line {}: {dir} needs one flag: {e}", n + 1))?),
            "NOSUGGEST" => r.no_suggest = Some(r.one_flag(at(1)).map_err(|e| format!("line {}: {dir} needs one flag: {e}", n + 1))?),
            "TRY" => r.try_chars = cap_try(at(1)),
            "KEY" => r.key = at(1).to_string(),
            "MAP" => {
                if !map_header && at(1).parse::<usize>().is_ok() {
                    map_header = true;
                } else if !at(1).contains('(') {
                    r.map.push(at(1).chars().collect());
                }
            }
            "REP" => {
                if !rep_header && at(1).parse::<usize>().is_ok() {
                    rep_header = true;
                } else if f.len() >= 3 {
                    r.rep.push((at(1).replace('_', " "), at(2).replace('_', " ")));
                }
            }
            "PFX" | "SFX" => {
                let suffix = dir == "SFX";
                let flag = r.one_flag(at(1)).map_err(|e| format!("line {}: {e}", n + 1))?;
                // Only a header ignores trailing fields; a rule condition beginning with # is literal.
                let is_header = f.len() >= 4 && matches!(at(2), "Y" | "N") && at(3).parse::<usize>().is_ok() && !cross.contains_key(&(suffix, flag));
                if is_header {
                    cross.insert((suffix, flag), at(2) == "Y");
                    continue;
                }
                if f.len() < 4 {
                    return Err(format!("line {}: short affix rule", n + 1));
                }
                // `able/B`: the appended text and its continuation classes.
                let (add, cont) = match at(3).split_once('/') {
                    Some((a, c)) => (a, c),
                    None => (at(3), ""),
                };
                let cond = if f.len() >= 5 { at(4) } else { "." };
                parse_cond(cond).map_err(|e| format!("line {}: {e}", n + 1))?;
                let c = cross.get(&(suffix, flag)).copied().unwrap_or(false);
                let rule: RawRule = (flag, c, zero_empty(at(2)), zero_empty(add), cont.to_string(), cond.to_string());
                if suffix {
                    r.suffixes.push(rule);
                } else {
                    r.prefixes.push(rule);
                }
            }
            d if english && IGNORED_IN_ENGLISH.contains(&d) => {}
            d if UNSUPPORTED.contains(&d) => return Err(format!("line {}: {d} is not supported", n + 1)),
            _ => {}
        }
    }
    // Aliases may be declared after the rules that use them; resolve once all are read.
    // Hunspell removes `IGNORE` characters from the appended text only: a stripped text that
    // holds one never matches a stem (stems lose them too), so the rule never applies.
    let resolve = |rules: &mut Vec<RawRule>, r: &Rules| {
        for rule in rules.iter_mut() {
            rule.4 = r.flags(&rule.4).to_string();
            rule.3 = r.strip_ignored(&rule.3);
        }
    };
    let (mut pfx, mut sfx) = (std::mem::take(&mut r.prefixes), std::mem::take(&mut r.suffixes));
    resolve(&mut pfx, &r);
    resolve(&mut sfx, &r);
    (r.prefixes, r.suffixes) = (pfx, sfx);
    Ok(r)
}

impl AffixDict {
    /// Build from Hunspell `.aff` and `.dic` text (decoded to UTF-8; the `.dic` count line and
    /// morphological fields are ignored). `comments` (provenance, licence) are kept with the data.
    pub fn from_hunspell(aff: &str, dic: &str, comments: &[&str]) -> Result<AffixDict, String> {
        AffixDict::from_hunspell_in(aff, dic, comments, false)
    }

    /// [`AffixDict::from_hunspell`] for an English dictionary: flag-based compounding
    /// (`COMPOUNDFLAG`…), which this engine doesn't implement and English spelling doesn't need,
    /// is ignored rather than refused. `COMPOUNDRULE`, `ONLYINCOMPOUND` and `COMPOUNDMIN`
    /// (ordinals: `21st`) and `OCONV` are read, as for every language.
    pub fn from_hunspell_english(aff: &str, dic: &str, comments: &[&str]) -> Result<AffixDict, String> {
        AffixDict::from_hunspell_in(aff, dic, comments, true)
    }

    fn from_hunspell_in(aff: &str, dic: &str, comments: &[&str], english: bool) -> Result<AffixDict, String> {
        // Some dictionaries open with a UTF-8 BOM; it must not glue itself to the first
        // directive or turn the `.dic` count line into a stem.
        let aff = aff.strip_prefix('\u{feff}').unwrap_or(aff);
        let dic = dic.strip_prefix('\u{feff}').unwrap_or(dic);
        let rules = parse_aff(aff, english)?;
        let mut stems: Vec<(String, String)> = Vec::new();
        for (n, line) in dic.lines().enumerate() {
            // The entry may hold spaces (`ad hoc/B\tpo:adv`); it ends where Hunspell ends it.
            // Leading blanks belong to it, as in Hunspell (`  lab` is no `lab`).
            let tok = line.get(..crate::hunspell::dic_entry_end(line.as_bytes())).unwrap_or_default();
            if tok.is_empty() {
                continue;
            }
            if n == 0 && tok.parse::<usize>().is_ok() {
                continue;
            }
            // `\/` is a literal slash in a word; the first plain `/` starts the flags.
            let mut word = String::new();
            let mut flags = String::new();
            let mut chars = tok.chars().peekable();
            let mut in_flags = false;
            while let Some(c) = chars.next() {
                if in_flags {
                    if c != EMOJI_SELECTOR {
                        flags.push(c);
                    }
                } else if c == '\\' && chars.peek() == Some(&'/') {
                    word.push('/');
                    chars.next();
                } else if c == '/' {
                    in_flags = true;
                } else {
                    word.push(c);
                }
            }
            let word = rules.strip_ignored(&word);
            if word.is_empty() || word.len() > MAX_WORD_BYTES || word.contains(['\n', '/']) {
                continue;
            }
            // With an `AF` table the field stays an alias number here: build() reads each alias
            // once, and every stem using it shares those flags.
            stems.push((word, flags));
        }
        AffixDict::build(rules, stems, comments.iter().map(ToString::to_string).collect(), MAX_FLAGS_PER_ENTRY)
    }

    /// Build from parsed rules and stems whose flag fields carry at most `entry_cap` flags each
    /// ([`MAX_FLAGS_PER_ENTRY`] for `.dic` entries, [`MAX_FLAGS_PER_STEM`] for compact data).
    fn build(rules: Rules, stems: Vec<(String, String)>, comments: Vec<String>, entry_cap: usize) -> Result<AffixDict, String> {
        let mode = rules.mode;
        // A `.dic` entry's malformed flag items read as Hunspell reads them ([`FlagMode`]); an
        // entry with more than MAX_FLAGS_PER_STEM flags, or compact data whose flags don't read,
        // is skipped. One with more than `entry_cap` flags is not a word either, but its
        // FORBIDDENWORD and NOSUGGEST flags still apply to its homonyms: an exclusion is never lost.
        let mut skipped = 0usize;
        let exclusion_keys = [rules.forbidden, rules.no_suggest, rules.substandard];
        let is_exclusion_key = |k: &FlagKey| exclusion_keys.contains(&Some(*k));
        // Each stem's flags are a range of `stem_keys`: one buffer, not one allocation per stem.
        // The flag says whether the entry is a word (false: it only carries exclusions).
        let mut stem_keys: Vec<FlagKey> = Vec::new();
        // `.dic` entries (not compact data) read malformed flag items as Hunspell does.
        let lenient = entry_cap == MAX_FLAGS_PER_ENTRY;
        // With an `AF` table: each alias read once, on first use, as its range of `stem_keys`
        // (and whether it is a word's flags); the stems using it share the range, so a big alias
        // on many stems costs its flags once (no copy per stem).
        let mut alias_ranges: FxMap<usize, Option<(usize, usize, bool)>> = FxMap::default();
        let read = |f: &str, stem_keys: &mut Vec<FlagKey>| -> Option<(usize, usize, bool)> {
            let keys = mode.parse_at_most(f, MAX_FLAGS_PER_STEM.max(entry_cap), lenient).ok()?;
            let start = stem_keys.len();
            let word = keys.len() <= entry_cap;
            if word {
                stem_keys.extend(keys);
            } else {
                stem_keys.extend(keys.into_iter().filter(is_exclusion_key));
            }
            Some((start, stem_keys.len(), word))
        };
        let mut stems: Vec<(String, usize, usize, bool)> = stems
            .into_iter()
            .filter_map(|(w, f)| {
                let range = match rules.alias_index(&f) {
                    Some(i) => *alias_ranges.entry(i).or_insert_with(|| read(rules.aliases.get(i).map_or("", String::as_str), &mut stem_keys)),
                    None if rules.aliases_on => Some((0, 0, true)),
                    None => read(&f, &mut stem_keys),
                };
                if range.is_none_or(|r| !r.2) {
                    skipped += 1;
                }
                range.map(|(a, b, word)| (w, a, b, word))
            })
            .collect();
        if skipped > 0 {
            log::debug!("affix dictionary: {skipped} entries with unreadable or too many flags skipped");
        }
        let conts = |rs: &[RawRule]| rs.iter().map(|r| mode.parse(&r.4)).collect::<Result<Vec<_>, _>>();
        let (pfx_cont, sfx_cont) = (conts(&rules.prefixes)?, conts(&rules.suffixes)?);
        // Flags: every one a rule, a stem or a directive uses gets an id, in key order.
        let mut distinct: FxMap<FlagKey, u16> = FxMap::default();
        let all = rules.prefixes.iter().chain(&rules.suffixes).map(|r| r.0);
        let all = all.chain(pfx_cont.iter().chain(&sfx_cont).flatten().copied()).chain(stem_keys.iter().copied());
        // NEEDAFFIX and CIRCUMFIX flags may only ever appear in rules' continuation classes.
        let directives =
            [rules.forbidden, rules.no_suggest, rules.substandard, rules.keepcase, rules.only_in_compound, rules.need_affix, rules.circumfix];
        for k in all.chain(directives.into_iter().flatten()) {
            distinct.entry(k).or_insert(0);
            if distinct.len() > MAX_DISTINCT_FLAGS {
                return Err(format!("more than {MAX_DISTINCT_FLAGS} distinct flags"));
            }
        }
        let mut flag_keys: Vec<FlagKey> = distinct.keys().copied().collect();
        flag_keys.sort_unstable();
        // At most 2^16 keys, so every index fits a u16.
        for (i, k) in flag_keys.iter().enumerate() {
            if let (Some(id), Ok(i)) = (distinct.get_mut(k), u16::try_from(i)) {
                *id = i;
            }
        }
        let id_of = |k: FlagKey| distinct.get(&k).copied();
        // Stems sorted by bytes; homonyms (the same word twice) are merged.
        stems.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut d = AffixDict {
            words: String::with_capacity(stems.iter().map(|s| s.0.len()).sum()),
            offs: Vec::with_capacity(stems.len() + 1),
            set_offs: vec![0, 0],
            forbidden: rules.forbidden.and_then(id_of),
            no_suggest: rules.no_suggest.and_then(id_of),
            substandard: rules.substandard.and_then(id_of),
            keepcase: rules.keepcase.and_then(id_of),
            need_affix: rules.need_affix.and_then(id_of),
            circumfix: rules.circumfix.and_then(id_of),
            full_strip: rules.full_strip,
            check_sharps: rules.check_sharps,
            flag_mode: mode,
            iconv: Conv::new(rules.iconv),
            oconv: Conv::new(rules.oconv),
            ignore: rules.ignore.into_boxed_slice(),
            try_chars: rules.try_chars,
            key: rules.key,
            map: rules.map,
            rep: rules.rep,
            comments,
            // A compound rule's flag no stem carries gets no id: its token matches no part.
            compound_rules: rules
                .compound_rules
                .iter()
                .map(|p| parse_compound_rule(p, mode).map(|t| t.into_iter().map(|(k, rep)| (id_of(k), rep)).collect()))
                .collect::<Result<_, _>>()?,
            compound_text: rules.compound_rules,
            only_in_compound: rules.only_in_compound.and_then(id_of),
            compound_min: rules.compound_min.unwrap_or(3),
            ..AffixDict::default()
        };
        d.offs.push(0);
        let compound_flags: Vec<u16> = d.compound_rules.iter().flatten().filter_map(|(f, _)| *f).collect();
        let mut set_ids: FxMap<Vec<u16>, u32> = FxMap::default();
        set_ids.insert(Vec::new(), 0);
        let exclusions: Vec<u16> = exclusion_keys.into_iter().flatten().filter_map(id_of).collect();
        let mut i = 0;
        let mut dropped = 0usize;
        let mut merged: Vec<u16> = Vec::new();
        let mut fresh: Vec<u16> = Vec::new();
        let mut carried: Vec<u16> = Vec::new();
        while i < stems.len() {
            let Some((word, ..)) = stems.get(i) else { break };
            merged.clear();
            carried.clear();
            let mut is_word = false;
            // NEEDAFFIX belongs to its homonym: the word stands alone if any homonym does
            // (`foo/NA` and `foo/B`: `foo` is right).
            let mut all_need = true;
            let mut j = i;
            while let Some((w, a, b, entry_is_word)) = stems.get(j)
                && w == word
            {
                j += 1;
                fresh.clear();
                fresh.extend(stem_keys.get(*a..*b).unwrap_or_default().iter().filter_map(|&k| id_of(k)));
                for &f in &fresh {
                    if exclusions.contains(&f) && !carried.contains(&f) {
                        carried.push(f);
                    }
                }
                if !entry_is_word {
                    continue;
                }
                is_word = true;
                all_need &= d.need_affix.is_some_and(|n| fresh.contains(&n));
                fresh.sort_unstable();
                fresh.dedup();
                fresh.retain(|&f| !has(&merged, f));
                // Homonyms can add up to more flags than one entry holds; past MAX_FLAGS_PER_STEM
                // an entry's other flags are dropped (its exclusions are carried below), so the
                // compact data never holds a set its reader refuses.
                if fresh.is_empty() {
                    // A repeated homonym adds nothing: no sort (200,000 duplicate lines of a
                    // stem with thousands of flags would each pay one).
                } else if merged.len().saturating_add(fresh.len()) <= MAX_FLAGS_PER_STEM {
                    merged.extend_from_slice(&fresh);
                    // A sorted run and a sorted tail: the stable sort merges them in linear time.
                    merged.sort();
                } else {
                    dropped += 1;
                }
            }
            if !is_word {
                i = j;
                continue;
            }
            if !all_need && let Some(n) = d.need_affix {
                merged.retain(|&f| f != n);
            }
            for &f in &carried {
                if has(&merged, f) {
                    continue;
                }
                // Make room by dropping a flag that excludes nothing (there are at most three
                // exclusion flags, so a full set always has one).
                if merged.len() >= MAX_FLAGS_PER_STEM
                    && let Some(at) = merged.iter().rposition(|g| !exclusions.contains(g))
                {
                    merged.remove(at);
                }
                if let Err(at) = merged.binary_search(&f) {
                    merged.insert(at, f);
                }
            }
            let id = d.intern(&mut set_ids, &mut merged)?;
            if merged.iter().any(|f| compound_flags.contains(f))
                && let Some(c) = word.chars().next()
                && !d.compound_firsts.contains(&c)
            {
                d.compound_firsts.push(c);
            }
            d.words.push_str(word);
            d.offs.push(u32::try_from(d.words.len()).map_err(|_| "stems too large".to_string())?);
            d.flags.push(id);
            i = j;
        }
        if dropped > 0 {
            log::debug!("affix dictionary: {dropped} homonym entries past {MAX_FLAGS_PER_STEM} merged flags kept only their exclusions");
        }
        let mut build_rules = |raw: Vec<RawRule>, conts: Vec<Vec<FlagKey>>, d: &mut AffixDict| -> Result<Vec<Rule>, String> {
            raw.into_iter()
                .zip(conts)
                .map(|((flag, cross, strip, add, _, cond), cont)| {
                    let flag = id_of(flag).ok_or("affix flag without an id")?;
                    let cont = d.intern(&mut set_ids, &mut cont.iter().filter_map(|&k| id_of(k)).collect())?;
                    Ok(Rule { flag, cross, strip: strip.into(), add: add.into(), cont, cond: parse_cond(&cond)?, cond_text: cond.into() })
                })
                .collect()
        };
        d.prefixes = build_rules(rules.prefixes, pfx_cont, &mut d)?;
        d.suffixes = build_rules(rules.suffixes, sfx_cont, &mut d)?;
        d.flag_keys = flag_keys;
        d.index();
        Ok(d)
    }

    /// The id of the flag set `flags` (sorted and deduplicated here), adding it if it is new.
    fn intern(&mut self, ids: &mut FxMap<Vec<u16>, u32>, flags: &mut Vec<u16>) -> Result<u32, String> {
        flags.sort_unstable();
        flags.dedup();
        if let Some(&id) = ids.get(flags.as_slice()) {
            return Ok(id);
        }
        let id = u32::try_from(self.set_offs.len().saturating_sub(1)).map_err(|_| "too many distinct flag sets".to_string())?;
        if self.set_data.len().saturating_add(flags.len()) > MAX_SET_DATA {
            return Err(format!("distinct flag sets hold more than {MAX_SET_DATA} flags in all"));
        }
        self.set_data.extend_from_slice(flags);
        self.set_offs.push(u32::try_from(self.set_data.len()).map_err(|_| "flag sets too large".to_string())?);
        ids.insert(flags.clone(), id);
        Ok(id)
    }

    /// The flag set `id`, sorted (empty when unknown).
    fn set(&self, id: u32) -> &[u16] {
        let i = id as usize;
        match (self.set_offs.get(i), self.set_offs.get(i + 1)) {
            (Some(&a), Some(&b)) => self.set_data.get(a as usize..b as usize).unwrap_or_default(),
            _ => &[],
        }
    }

    /// Build the suffix index and the stem hash table.
    fn index(&mut self) {
        let mut idx: FxMap<Box<str>, Vec<SfxGroup>> = FxMap::default();
        // Each (appended, stripped) pair's group, found by hash rather than by scanning the
        // appended string's groups: a dictionary with many strips per suffix stays linear.
        let mut group_of: FxMap<(&str, &str), usize> = FxMap::default();
        let mut by_flag: Vec<Vec<u32>> = vec![Vec::new(); self.flag_keys.len()];
        for (i, r) in self.suffixes.iter().enumerate() {
            let groups = idx.entry(r.add.clone()).or_default();
            let Ok(i) = u32::try_from(i) else { break };
            let g = match group_of.get(&(&*r.add, &*r.strip)).copied() {
                Some(g) => g,
                None => {
                    group_of.insert((&r.add, &r.strip), groups.len());
                    groups.push(SfxGroup { strip: r.strip.clone(), ..SfxGroup::default() });
                    groups.len() - 1
                }
            };
            if let Some(g) = groups.get_mut(g) {
                g.rules.push(i);
                if r.cont != 0 {
                    g.cont_rules.push(i);
                }
            }
            if let Some(list) = by_flag.get_mut(usize::from(r.flag)) {
                list.push(i);
            }
        }
        self.sfx_by_flag = by_flag;
        let mut cont_flag = vec![false; self.flag_keys.len()];
        for r in self.prefixes.iter().chain(&self.suffixes) {
            for &f in self.set(r.cont) {
                if let Some(c) = cont_flag.get_mut(usize::from(f)) {
                    *c = true;
                }
            }
        }
        self.has_cont = cont_flag.contains(&true);
        self.cont_flag = cont_flag;
        self.max_sfx_chars = self.suffixes.iter().map(|r| r.add.chars().count()).max().unwrap_or(0);
        self.sfx_index = idx;
        let n = self.len();
        let size = (n.max(1) * 2).next_power_of_two();
        let mut table = vec![EMPTY; size];
        let mask = size - 1;
        for i in 0..n {
            let Some(w) = self.word(i) else { continue };
            let mut slot = hash_str(w) as usize & mask;
            while table.get(slot).is_some_and(|&t| t != EMPTY) {
                slot = (slot + 1) & mask;
            }
            if let (Some(t), Ok(i)) = (table.get_mut(slot), u32::try_from(i)) {
                *t = i;
            }
        }
        self.table = table;
    }

    /// Number of stems.
    pub fn len(&self) -> usize {
        self.flags.len()
    }

    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }

    /// Number of prefix and suffix rules.
    pub fn rule_count(&self) -> usize {
        self.prefixes.len() + self.suffixes.len()
    }

    fn word(&self, i: usize) -> Option<&str> {
        let a = *self.offs.get(i)? as usize;
        let b = *self.offs.get(i + 1)? as usize;
        self.words.get(a..b)
    }

    /// Append the flags of the set `ids` (or one flag) to `out` as the source wrote them.
    fn write_flags(&self, ids: &[u16], out: &mut String) {
        self.flag_mode.write(ids.iter().filter_map(|&i| self.flag_keys.get(usize::from(i)).copied()), out);
    }

    /// The stems, in byte order.
    pub fn stems(&self) -> impl Iterator<Item = &str> + '_ {
        (0..self.len()).filter_map(|i| self.word(i))
    }

    /// The stems starting with `prefix`, in byte order (a binary search, then a scan).
    pub fn stems_with_prefix<'a>(&'a self, prefix: &str) -> impl Iterator<Item = &'a str> + use<'a> {
        let prefix = prefix.to_string();
        let first = {
            let (mut lo, mut hi) = (0usize, self.len());
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if self.word(mid).is_some_and(|w| w.as_bytes() < prefix.as_bytes()) {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            lo
        };
        (first..self.len()).map_while(move |i| self.word(i).filter(|w| w.starts_with(prefix.as_str())))
    }

    /// What lookups exclude: forbidden words, and NOSUGGEST and SUBSTANDARD stems too for
    /// suggestions.
    fn excluded(&self, suggestible: bool) -> Excluded {
        let e = Excluded::default().and(self.forbidden);
        if suggestible { e.and(self.no_suggest).and(self.substandard) } else { e }
    }

    /// Flag set of a stem, if `w` is one.
    fn stem_flags(&self, w: &str) -> Option<&[u16]> {
        self.stem_flags_except(w, self.excluded(false))
    }

    /// Flag set of a stem, if `w` is one and carries none of the `excluded` flags. A stem
    /// carrying FORBIDDENWORD (listed, not a word) or ONLYINCOMPOUND (a word only inside a
    /// compound, [`Self::compound`]) is no hit, and (this is the only stem lookup outside
    /// compounds) no affixed form is built on it either.
    fn stem_flags_except(&self, w: &str, excluded: Excluded) -> Option<&[u16]> {
        self.stem_set(w).filter(|set| !excluded.hits(set) && !self.only_in_compound.is_some_and(|f| has(set, f)))
    }

    /// Flag set of the stem `w`, whatever flags it carries.
    fn stem_set(&self, w: &str) -> Option<&[u16]> {
        if self.table.is_empty() {
            return None;
        }
        let mask = self.table.len() - 1;
        let mut slot = hash_str(w) as usize & mask;
        for _ in 0..self.table.len() {
            let t = *self.table.get(slot)?;
            if t == EMPTY {
                return None;
            }
            if self.word(t as usize) == Some(w) {
                return Some(self.set(*self.flags.get(t as usize)?));
            }
            slot = (slot + 1) & mask;
        }
        None
    }

    /// Do the continuation classes of `r` hold `flag`?
    fn cont_holds(&self, r: &Rule, flag: Option<u16>) -> bool {
        flag.is_some_and(|f| has(self.set(r.cont), f))
    }

    /// Can a derived form carry `flag` (does some rule's continuation classes hold it)?
    fn is_cont_flag(&self, flag: u16) -> bool {
        self.cont_flag.get(usize::from(flag)).copied().unwrap_or(false)
    }

    /// Does `w` carry any flag of `flags` — as a stem, or as the form of a suffix rule whose
    /// continuation classes include one (chained suffixation, depth-bounded)?
    ///
    /// The rules of a group that could lead to `w` are merged into one set of their flags (sorted)
    /// and followed once, not once per rule: the answer is the same, but thousands of rules of one
    /// flag cost one walk, not one each (#3613).
    fn form_has_flag(&self, w: &str, flags: &[u16], depth: u8, excluded: Excluded) -> bool {
        if self.stem_flags_except(w, excluded).is_some_and(|f| intersects(f, flags)) {
            return true;
        }
        // Without continuation classes no rule leads from a form to another.
        if depth == 0 || !self.has_cont {
            return false;
        }
        let mut root = String::with_capacity(w.len() + 8);
        let mut inner: Vec<u16> = Vec::new();
        // Does a continuation set hold one of `flags`? Answered once per distinct set (rules
        // share interned sets) and, for many flags, through a bitmap: a lookup costs at most the
        // total size of the distinct sets it meets, not rules × set size.
        let probe = FlagProbe::new(flags);
        let mut seen: FxMap<u32, bool> = FxMap::default();
        let cuts = std::iter::once(w.len()).chain(w.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0 || self.full_strip));
        for b in cuts.take(self.max_sfx_chars + 1) {
            let (Some(base), Some(add)) = (w.get(..b), w.get(b..)) else { continue };
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups.iter().filter(|g| !g.cont_rules.is_empty()) {
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                inner.clear();
                for r in g.cont_rules.iter().filter_map(|&i| self.suffixes.get(i as usize)) {
                    let meets = *seen.entry(r.cont).or_insert_with(|| probe.meets(self.set(r.cont)));
                    if meets && !self.cont_holds(r, self.circumfix) && r.cond_matches(&root, true) {
                        inner.push(r.flag);
                    }
                }
                inner.sort_unstable();
                inner.dedup();
                if !inner.is_empty() && self.form_has_flag(&root, &inner, depth - 1, excluded) {
                    return true;
                }
            }
        }
        false
    }

    /// Is `w` exactly (case and all) a stem or an affixed form of one? `ICONV` and `IGNORE`
    /// apply first.
    pub fn contains(&self, w: &str) -> bool {
        self.contains_except(&self.normalize(w), self.excluded(false))
    }

    /// Exactly spelled dictionary words eligible for suggestions (NOSUGGEST and SUBSTANDARD
    /// stems excluded).
    pub fn contains_suggestible(&self, w: &str) -> bool {
        self.contains_except(&self.normalize(w), self.excluded(true))
    }

    /// `w` after the `ICONV` conversions (at each position the longest match wins) and without
    /// the `IGNORE` characters. Input longer than [`MAX_INPUT_BYTES`] is no word and comes back
    /// unchanged, so the length check after this rejects it without converting it.
    fn normalize<'a>(&self, w: &'a str) -> std::borrow::Cow<'a, str> {
        if w.len() > MAX_INPUT_BYTES {
            return std::borrow::Cow::Borrowed(w);
        }
        self.iconv.apply(w, &self.ignore)
    }

    /// A suggestion as it is shown: `w` after the `OCONV` conversions (the French dictionary
    /// writes `’` for the `'` its stems use).
    pub fn output<'a>(&self, w: &'a str) -> std::borrow::Cow<'a, str> {
        self.oconv.apply(w, &[])
    }

    /// [`AffixDict::contains`] on normalized input, where stems carrying an `excluded` flag
    /// don't count (forbidden words; case variants of `KEEPCASE` stems).
    fn contains_except(&self, w: &str, excluded: Excluded) -> bool {
        if w.is_empty() || w.len() > MAX_WORD_BYTES {
            return false;
        }
        // A stem that needs an affix is no word on its own.
        if self.stem_flags_except(w, excluded.and(self.need_affix)).is_some() || self.suffixed(w, None, false, false, excluded) {
            return true;
        }
        let mut root = String::with_capacity(w.len() + 8);
        for r in &self.prefixes {
            let Some(rest) = w.strip_prefix(&*r.add) else { continue };
            if rest.is_empty() && !self.full_strip {
                continue;
            }
            root.clear();
            root.push_str(&r.strip);
            root.push_str(rest);
            if !r.cond_matches(&root, false) {
                continue;
            }
            // A circumfix prefix works only with its suffix, and one that needs another affix
            // only with a suffix too.
            let circ = self.cont_holds(r, self.circumfix);
            let needs = self.cont_holds(r, self.need_affix);
            if !circ && !needs {
                if self.stem_flags_except(&root, excluded).is_some_and(|f| has(f, r.flag)) {
                    return true;
                }
                if self.is_cont_flag(r.flag) && self.form_has_flag(&root, &[r.flag], INNER_CHAIN, excluded) {
                    return true;
                }
            }
            if r.cross && self.suffixed(&root, Some(r.flag), circ, needs, excluded) {
                return true;
            }
        }
        self.compound(w, excluded)
    }

    /// Is `w` a compound of two or more stems (no affixes) whose flags spell out one of the
    /// `COMPOUNDRULE` patterns, each part at least `COMPOUNDMIN` characters? `21st` is `2/m` +
    /// `1st/p` under `n*mp`. A dynamic programme over (byte offset, token, parts so far), so a
    /// hostile pattern costs at most a few thousand lookups. A forbidden word is never rebuilt
    /// from its parts.
    fn compound(&self, w: &str, excluded: Excluded) -> bool {
        if self.compound_rules.is_empty()
            || w.len() > MAX_COMPOUND_BYTES
            || !w.chars().next().is_some_and(|c| self.compound_firsts.contains(&c))
            || self.forbidden.is_some_and(|f| self.stem_set(w).is_some_and(|set| has(set, f)))
        {
            return false;
        }
        // FORBIDDENWORD and NEEDAFFIX stems are no compound part (Hunspell rejects a compound
        // with a NEEDAFFIX part), checked here so the `excluded` slots can't run out.
        let bare = |set: &[u16]| [self.forbidden, self.need_affix].into_iter().flatten().all(|f| !has(set, f));
        // Char boundaries of `w`, `0` and `w.len()` included.
        let cuts: Vec<usize> = w.char_indices().map(|(i, _)| i).chain(std::iter::once(w.len())).collect();
        let n = cuts.len();
        // Flag set of each part `w[cuts[a]..cuts[b]]`, by (a, b), looked up when a rule first
        // reaches cut `a` (`None`: not looked up yet; `Some(None)`: not a stem). Only the parts
        // after a cut a rule reaches are looked up, so a word with no first part costs `n`.
        let mut parts: Vec<Option<Option<&[u16]>>> = vec![None; n * n];
        let min = self.compound_min;
        let mut part = |a: usize, b: usize| -> Option<&[u16]> {
            if b < a.saturating_add(min) {
                return None;
            }
            let slot = parts.get_mut(a * n + b)?;
            *slot.get_or_insert_with(|| {
                let (Some(&x), Some(&y)) = (cuts.get(a), cuts.get(b)) else { return None };
                w.get(x..y).and_then(|p| self.stem_set(p)).filter(|set| !excluded.hits(set) && bare(set))
            })
        };
        self.compound_rules.iter().any(|rule| compound_matches(rule, &mut part, n))
    }

    /// Is `w` a stem with one suffix? With `need` set (a prefix was taken off), the prefix's flag
    /// must come from the stem or from the suffix's continuation classes (`leg` + `nagy` + `obb/A`)
    /// and the suffix must allow cross products. `circ` says whether that prefix is a circumfix
    /// one: the suffix must be one too, and otherwise must not be. `pfx_needs`: the prefix needs
    /// another affix (NEEDAFFIX), so the suffix may not need one too (Hunspell rejects `reworks`
    /// from `re/N` and `s/N`).
    fn suffixed(&self, w: &str, need: Option<u16>, circ: bool, pfx_needs: bool, excluded: Excluded) -> bool {
        // Fits the suffix rule `r`: circumfix alike, and in need of another affix only when a
        // prefix that doesn't is there.
        let fits = |r: &Rule| self.cont_holds(r, self.circumfix) == circ && (!self.cont_holds(r, self.need_affix) || (need.is_some() && !pfx_needs));
        let mut root = String::with_capacity(w.len() + 8);
        let mut flags: Vec<u16> = Vec::new();
        // Each split point from the end: the appended string is `w[b..]`, at most the longest
        // suffix, and something must stay before it.
        let cuts = std::iter::once(w.len()).chain(w.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0 || self.full_strip));
        for b in cuts.take(self.max_sfx_chars + 1) {
            let (Some(base), Some(add)) = (w.get(..b), w.get(b..)) else { continue };
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups {
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                let rules = || g.rules.iter().filter_map(|&i| self.suffixes.get(i as usize));
                // The stem path: `root` is a stem carrying the rule's flag (and the prefix's,
                // when a cross product is being checked).
                if let Some(f) = self.stem_flags_except(&root, excluded)
                    && rules().any(|r| {
                        has(f, r.flag)
                            && need.is_none_or(|n| has(f, n) || self.cont_holds(r, Some(n)))
                            && (need.is_none() || r.cross)
                            && fits(r)
                            && r.cond_matches(&root, true)
                    })
                {
                    return true;
                }
                // Continuation classes: `root` need not be a stem carrying this rule's flag —
                // it may be a form that gained the flag from another rule's continuation. The
                // matching rules' flags are looked for together, in one walk. Without continuation
                // classes that walk would only repeat the stem path above.
                if need.is_none() && self.has_cont {
                    flags.clear();
                    // Only a flag some continuation class holds can come from another rule. The
                    // outer suffix of a chain has another affix under it, so its NEEDAFFIX is met,
                    // and Hunspell ignores its CIRCUMFIX there (`workables` from `s/c`).
                    flags.extend(rules().filter(|r| self.is_cont_flag(r.flag) && r.cond_matches(&root, true)).map(|r| r.flag));
                    flags.sort_unstable();
                    flags.dedup();
                    if !flags.is_empty() && self.form_has_flag(&root, &flags, INNER_CHAIN, excluded) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Is `w` correct, Hunspell style: as written, or — for a capitalized word or one in capitals
    /// — in lower case (and capitalized), since a sentence may start with any word and headings
    /// may be in capitals. Lower-case input never matches a capitalized stem (`warszawa`), and a
    /// `KEEPCASE` stem matches only as written.
    pub fn check(&self, w: &str) -> bool {
        self.check_except(w, self.excluded(false))
    }

    /// Whether a suggestion is correct in its displayed case and has an eligible stem.
    pub fn can_suggest(&self, w: &str) -> bool {
        self.check_except(w, self.excluded(true))
    }

    /// Whether `w` is a word only through a `KEEPCASE` stem, so it is correct in this letter case
    /// alone (`km` from `km/K`, which `Km` and `KM` are not).
    pub(crate) fn case_locked(&self, w: &str) -> bool {
        self.keepcase.is_some() && self.contains(w) && !self.contains_except(&self.normalize(w), self.excluded(false).and(self.keepcase))
    }

    fn check_except(&self, w: &str, excluded: Excluded) -> bool {
        let w = self.normalize(w);
        if self.contains_except(&w, excluded) {
            return true;
        }
        // Over this no case variant can be a word either.
        if w.len() > MAX_INPUT_BYTES {
            return false;
        }
        let keep = excluded.and(self.keepcase);
        match casing(&w) {
            Casing::Lower | Casing::Mixed => false,
            Casing::Initial => {
                let lower = w.to_lowercase();
                // CHECKSHARPS: a KEEPCASE word with `ß` may be capitalized (`Gruß` for `Gruß/k`).
                let ex = if self.check_sharps && lower.contains('ß') { excluded } else { keep };
                self.contains_except(&lower, ex)
            }
            Casing::Upper => {
                let lower = w.to_lowercase();
                self.contains_except(&capitalize(&lower), keep)
                    || self.contains_except(&lower, keep)
                    || (self.check_sharps && self.sharps(&lower, excluded))
            }
        }
    }

    /// CHECKSHARPS: is `lower` (a word in capitals, lowercased) a word with some of its `ss` read
    /// as `ß`, in lower case or capitalized? Such a form may come from a `KEEPCASE` stem (in
    /// capitals a word with `ß` is written with `SS`). The first [`MAX_SHARPS`] `ss` pairs are
    /// tried, in every combination.
    fn sharps(&self, lower: &str, excluded: Excluded) -> bool {
        let at: Vec<usize> = lower.match_indices("ss").map(|(i, _)| i).take(MAX_SHARPS).collect();
        let mut variant = String::with_capacity(lower.len());
        for pick in 1u32..(1 << at.len()) {
            variant.clear();
            let mut from = 0;
            for (k, &i) in at.iter().enumerate() {
                if pick & (1 << k) != 0 {
                    variant.push_str(lower.get(from..i).unwrap_or_default());
                    variant.push('ß');
                    from = i + 2;
                }
            }
            variant.push_str(lower.get(from..).unwrap_or_default());
            if self.contains_except(&variant, excluded) || self.contains_except(&capitalize(&variant), excluded) {
                return true;
            }
        }
        false
    }

    /// Every form of the stem `stem` (itself included), unsorted, possibly with repeats; at most
    /// [`MAX_FORMS`], and at most [`MAX_FORMS_WORK`] rules tried along the continuation chains and
    /// cross products, so a dictionary with thousands of self-continuing rules can't make one stem
    /// cost millions of strings (#3613).
    pub fn forms(&self, stem: &str) -> Vec<String> {
        let Some(f) = self.stem_flags(stem) else { return Vec::new() };
        let mut out = Vec::new();
        if !self.need_affix.is_some_and(|n| has(f, n)) {
            out.push(stem.to_string());
        }
        // Suffix forms a prefix may be added to: the form, whether its suffix is a circumfix one,
        // whether it needs another affix, and its continuation classes (which may allow a prefix
        // the stem doesn't).
        let mut cross_sfx: Vec<(String, bool, bool, u32)> = Vec::new();
        // Suffix forms queued with the continuation classes they carry, for chained expansion;
        // each (form, classes) pair is expanded once.
        let mut chain: Vec<(String, u32)> = Vec::new();
        let mut queued: std::collections::HashSet<(String, u32)> = std::collections::HashSet::new();
        let mut work = 0usize;
        for r in &self.suffixes {
            if !has(f, r.flag) || !r.cond_matches(stem, true) {
                continue;
            }
            let Some(base) = stem.strip_suffix(&*r.strip) else { continue };
            if base.is_empty() && !self.full_strip {
                continue;
            }
            let form = format!("{base}{}", r.add);
            let circ = self.cont_holds(r, self.circumfix);
            if r.cross {
                cross_sfx.push((form.clone(), circ, self.cont_holds(r, self.need_affix), r.cont));
            }
            // A circumfix suffix needs its prefix, so what it ends is never chained without one.
            if r.cont != 0 && !circ && queued.insert((form.clone(), r.cont)) {
                chain.push((form.clone(), r.cont));
            }
            // A circumfix suffix or one that needs another affix is no word on its own.
            // (An empty form still feeds the cross products and chains above; it is no word.)
            if !circ && !self.cont_holds(r, self.need_affix) {
                out.extend(non_empty(form));
            }
            if out.len() >= MAX_FORMS {
                return out;
            }
        }
        // Continuation classes chain suffixes (`workable/B` → `workables`), up to [`MAX_CHAIN`]
        // suffixes in all — the same bound the checker walks.
        'chain: for _ in 0..INNER_CHAIN {
            let level = std::mem::take(&mut chain);
            if level.is_empty() {
                break;
            }
            for (form, flags) in level {
                for r in self.suffixes_with(flags).into_iter().filter_map(|i| self.suffixes.get(i as usize)) {
                    work += 1;
                    if work > MAX_FORMS_WORK {
                        break 'chain;
                    }
                    if !r.cond_matches(&form, true) {
                        continue;
                    }
                    let Some(base) = form.strip_suffix(&*r.strip) else { continue };
                    if base.is_empty() && !self.full_strip {
                        continue;
                    }
                    let next = format!("{base}{}", r.add);
                    if r.cont != 0 && queued.insert((next.clone(), r.cont)) {
                        chain.push((next.clone(), r.cont));
                    }
                    // The outer suffix of a chain has an affix under it, so its NEEDAFFIX is met,
                    // and Hunspell ignores its CIRCUMFIX there (`workables` from `s/c`).
                    out.extend(non_empty(next));
                    if out.len() >= MAX_FORMS {
                        return out;
                    }
                }
            }
        }
        for r in &self.prefixes {
            let on_stem = has(f, r.flag);
            let circ = self.cont_holds(r, self.circumfix);
            let needs = self.cont_holds(r, self.need_affix);
            if on_stem && !circ && !needs {
                out.extend(self.prefixed(r, stem));
            }
            if r.cross {
                // Every suffix form looked at counts against the work budget, matching or not.
                for (s, s_circ, s_needs, s_cont) in &cross_sfx {
                    work += 1;
                    if out.len() >= MAX_FORMS || work > MAX_FORMS_WORK {
                        return out;
                    }
                    // The prefix's flag comes from the stem or the suffix's continuation classes;
                    // circumfix alike; not both in need of another affix.
                    if !(on_stem || has(self.set(*s_cont), r.flag)) || *s_circ != circ || (needs && *s_needs) {
                        continue;
                    }
                    out.extend(self.prefixed(r, s));
                }
            }
            if out.len() >= MAX_FORMS {
                return out;
            }
        }
        out
    }

    /// `root` with the prefix rule `r` added, when the checker takes it back off: the rule's
    /// condition holds for `root` (the stem, or a suffix form in a cross product), and something
    /// stays after the strip unless `FULLSTRIP`.
    fn prefixed(&self, r: &Rule, root: &str) -> Option<String> {
        let rest = root.strip_prefix(&*r.strip)?;
        if (rest.is_empty() && !self.full_strip) || !r.cond_matches(root, false) {
            return None;
        }
        non_empty(format!("{}{rest}", r.add))
    }

    /// The suffix rules carrying any flag of the set `set`, in rule order.
    fn suffixes_with(&self, set: u32) -> Vec<u32> {
        let mut rules: Vec<u32> = Vec::new();
        let mut lists = 0;
        for &flag in self.set(set) {
            if let Some(list) = self.sfx_by_flag.get(usize::from(flag))
                && !list.is_empty()
            {
                rules.extend_from_slice(list);
                lists += 1;
            }
        }
        if lists > 1 {
            rules.sort_unstable();
        }
        rules
    }

    /// Whether the dictionary sets `CHECKSHARPS` (`SS` may stand for `ß` in capitals).
    pub(crate) fn check_sharps(&self) -> bool {
        self.check_sharps
    }

    /// The `TRY` letters, most frequent first (suggestions try these).
    pub fn try_chars(&self) -> &str {
        &self.try_chars
    }

    /// The `KEY` keyboard rows, separated by `|` (adjacent letters are likely mistypes).
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The `MAP` groups: letters often confused with each other (`aą`, `zżź`, `oóu`…).
    pub fn map(&self) -> &[Vec<char>] {
        &self.map
    }

    /// The `REP` table: common misspellings, `(wrong, right)`.
    pub fn rep(&self) -> &[(String, String)] {
        &self.rep
    }

    /// The provenance and licence lines.
    pub fn comments(&self) -> &[String] {
        &self.comments
    }

    /// Serialize to WordCraft's compact format: raw-deflate of `WCAFF1`, a `FLAG` line when flags
    /// aren't one character each, the comments, the tables and rules as text lines, a `WORDS`
    /// line, then one line per stem, front-coded against the previous one (a byte with the shared
    /// prefix length in bytes, the rest of the word, `/` and its flags if any). Flags are written
    /// as the `.aff` wrote them, so one-character data is the same as before `FLAG` lines existed.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let mut out = String::new();
        out.push_str(MAGIC);
        out.push('\n');
        if let Some(name) = self.flag_mode.name() {
            out.push_str(&format!("FLAG {name}\n"));
        }
        for c in &self.comments {
            out.push_str("# ");
            out.push_str(c.trim());
            out.push('\n');
        }
        if !self.try_chars.is_empty() {
            out.push_str(&format!("TRY {}\n", self.try_chars));
        }
        if !self.key.is_empty() {
            out.push_str(&format!("KEY {}\n", self.key));
        }
        for m in &self.map {
            out.push_str(&format!("MAP {}\n", m.iter().collect::<String>()));
        }
        for (a, b) in &self.rep {
            out.push_str(&format!("REP {} {}\n", a.replace(' ', "_"), b.replace(' ', "_")));
        }
        // In one-character mode a directive flag that no stem or rule uses isn't written, as
        // before flag ids existed, so those dictionaries' compact data stays byte for byte the same.
        let used = |flag: u16| {
            self.flags.iter().any(|&id| self.set(id).contains(&flag))
                || self.prefixes.iter().chain(&self.suffixes).any(|r| r.flag == flag || self.set(r.cont).contains(&flag))
        };
        if self.full_strip {
            out.push_str("FULLSTRIP\n");
        }
        if self.check_sharps {
            out.push_str("CHECKSHARPS\n");
        }
        for (directive, flag) in [
            ("FORBIDDENWORD", self.forbidden),
            ("NOSUGGEST", self.no_suggest),
            ("SUBSTANDARD", self.substandard),
            ("KEEPCASE", self.keepcase),
            ("NEEDAFFIX", self.need_affix),
            ("CIRCUMFIX", self.circumfix),
        ] {
            if let Some(flag) = flag.filter(|&f| self.flag_mode != FlagMode::Char || used(f)) {
                out.push_str(directive);
                out.push(' ');
                self.write_flags(&[flag], &mut out);
                out.push('\n');
            }
        }
        for (name, table) in [("ICONV", &self.iconv), ("OCONV", &self.oconv)] {
            for (a, b) in &table.pairs {
                out.push_str(&format!("{name} {a} {b}\n"));
            }
        }
        if !self.ignore.is_empty() {
            out.push_str(&format!("IGNORE {}\n", self.ignore.iter().collect::<String>()));
        }
        // ONLYINCOMPOUND goes after COMPOUNDMIN, as before flag ids, so one-character data stays
        // byte for byte the same; without compound rules it is still written (its stems are no
        // words on their own).
        let only_in_compound = self.only_in_compound.filter(|&f| self.flag_mode != FlagMode::Char || used(f));
        let write_only_in_compound = |out: &mut String| {
            if let Some(flag) = only_in_compound {
                out.push_str("ONLYINCOMPOUND ");
                self.write_flags(&[flag], out);
                out.push('\n');
            }
        };
        if self.compound_text.is_empty() {
            write_only_in_compound(&mut out);
        } else {
            out.push_str(&format!("COMPOUNDMIN {}\n", self.compound_min));
            write_only_in_compound(&mut out);
            for p in &self.compound_text {
                out.push_str(&format!("COMPOUNDRULE {p}\n"));
            }
        }
        let z = |s: &str| if s.is_empty() { "0".to_string() } else { s.to_string() };
        for (kind, rules) in [("PFX", &self.prefixes), ("SFX", &self.suffixes)] {
            for r in rules {
                let y = if r.cross { "Y" } else { "N" };
                // Continuation classes ride on the appended field, as in the `.aff` source.
                let mut add = z(&r.add);
                let cont = self.set(r.cont);
                if !cont.is_empty() {
                    add.push('/');
                    self.write_flags(cont, &mut add);
                }
                let mut flag = String::new();
                self.write_flags(&[r.flag], &mut flag);
                out.push_str(&format!("{kind} {flag} {y} {} {} {}\n", z(&r.strip), add, r.cond_text));
            }
        }
        out.push_str("WORDS\n");
        let mut raw = out.into_bytes();
        let mut prev: &[u8] = &[];
        for i in 0..self.len() {
            let Some(w) = self.word(i) else { continue };
            let b = w.as_bytes();
            let p = prev.iter().zip(b).take_while(|(x, y)| x == y).count().min(255);
            raw.push(p as u8);
            raw.extend_from_slice(b.get(p..).unwrap_or_default());
            let set = self.flags.get(i).map_or(&[][..], |&id| self.set(id));
            if !set.is_empty() {
                let mut flags = String::from("/");
                self.write_flags(set, &mut flags);
                raw.extend_from_slice(flags.as_bytes());
            }
            raw.push(b'\n');
            prev = b;
        }
        // What [`AffixDict::from_bytes`] would refuse to inflate is refused here.
        if raw.len() > MAX_INFLATED {
            return Err(format!("affix dictionary: {} bytes uncompressed, over the {MAX_INFLATED} the reader takes", raw.len()));
        }
        Ok(miniz_oxide::deflate::compress_to_vec(&raw, 10))
    }

    /// Read data written by [`AffixDict::to_bytes`].
    pub fn from_bytes(data: &[u8]) -> Result<AffixDict, String> {
        let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(data, MAX_INFLATED)
            .map_err(|e| format!("affix dictionary: inflate failed: {:?}", e.status))?;
        let words_at = raw.windows(7).position(|w| w == b"\nWORDS\n").ok_or("affix dictionary: no WORDS section")?;
        let head = std::str::from_utf8(raw.get(..words_at).unwrap_or_default()).map_err(|_| "affix dictionary: bad utf-8 in the rules")?;
        let mut lines = head.lines();
        if lines.next() != Some(MAGIC) {
            return Err("affix dictionary: bad header".into());
        }
        let mut rules = Rules::default();
        let mut comments = Vec::new();
        for line in lines {
            if let Some(c) = line.strip_prefix("# ") {
                comments.push(c.to_string());
                continue;
            }
            let f: Vec<&str> = split_fields(line).collect();
            let at = |k: usize| f.get(k).copied().unwrap_or("");
            match at(0) {
                "FLAG" => rules.set_mode(at(1)).map_err(|e| format!("affix dictionary: {e}"))?,
                "TRY" => rules.try_chars = cap_try(at(1)),
                "KEY" => rules.key = at(1).to_string(),
                "FULLSTRIP" => rules.full_strip = true,
                "CHECKSHARPS" => rules.check_sharps = true,
                directive @ ("FORBIDDENWORD" | "NOSUGGEST" | "SUBSTANDARD" | "KEEPCASE" | "ONLYINCOMPOUND" | "NEEDAFFIX" | "CIRCUMFIX") => {
                    let c = rules.one_flag(at(1)).map_err(|e| format!("affix dictionary: bad {directive} in `{line}`: {e}"))?;
                    match directive {
                        "FORBIDDENWORD" => rules.forbidden = Some(c),
                        "NOSUGGEST" => rules.no_suggest = Some(c),
                        "SUBSTANDARD" => rules.substandard = Some(c),
                        "KEEPCASE" => rules.keepcase = Some(c),
                        "ONLYINCOMPOUND" => rules.only_in_compound = Some(c),
                        "NEEDAFFIX" => rules.need_affix = Some(c),
                        _ => rules.circumfix = Some(c),
                    }
                }
                conv @ ("ICONV" | "OCONV") if f.len() == 3 => {
                    rules.push_conv(conv == "OCONV", at(1), at(2)).map_err(|e| format!("affix dictionary: {e}"))?;
                }
                "IGNORE" => rules.ignore = at(1).chars().collect(),
                "COMPOUNDMIN" => {
                    let min = at(1).parse::<usize>().map_err(|_| format!("affix dictionary: bad `{line}`"))?;
                    rules.compound_min = Some(min.clamp(1, MAX_COMPOUND_BYTES));
                }
                "COMPOUNDRULE" => {
                    if rules.compound_rules.len() >= MAX_COMPOUND_RULES {
                        return Err("affix dictionary: too many COMPOUNDRULE patterns".into());
                    }
                    rules.compound_rules.push(at(1).to_string());
                }
                "MAP" => rules.map.push(at(1).chars().collect()),
                "REP" => rules.rep.push((at(1).replace('_', " "), at(2).replace('_', " "))),
                kind @ ("PFX" | "SFX") => {
                    let flag = rules.one_flag(at(1)).map_err(|e| format!("affix dictionary: bad flag in `{line}`: {e}"))?;
                    if f.len() != 6 {
                        return Err(format!("affix dictionary: bad rule `{line}`"));
                    }
                    // Continuation classes ride on the appended field, as in the `.aff` source.
                    let (add, cont) = match at(4).split_once('/') {
                        Some((a, c)) => (zero_empty(a), c.to_string()),
                        None => (zero_empty(at(4)), String::new()),
                    };
                    let rule: RawRule = (flag, at(2) == "Y", zero_empty(at(3)), add, cont, at(5).to_string());
                    if kind == "SFX" {
                        rules.suffixes.push(rule);
                    } else {
                        rules.prefixes.push(rule);
                    }
                }
                "" => {}
                other => return Err(format!("affix dictionary: unknown line `{other}`")),
            }
        }
        let body = raw.get(words_at + 7..).unwrap_or_default();
        let mut stems: Vec<(String, String)> = Vec::new();
        let mut cur: Vec<u8> = Vec::new();
        let mut i = 0;
        while let Some(&p) = body.get(i) {
            let rest = body.get(i + 1..).unwrap_or_default();
            let len = rest.iter().position(|&b| b == b'\n').ok_or("affix dictionary: truncated")?;
            let line = rest.get(..len).unwrap_or_default();
            i += len + 2;
            let p = p as usize;
            if p > cur.len() {
                return Err("affix dictionary: bad prefix".into());
            }
            let (tail, flags) = match line.iter().position(|&b| b == b'/') {
                Some(k) => (line.get(..k).unwrap_or_default(), line.get(k + 1..).unwrap_or_default()),
                None => (line, &[][..]),
            };
            cur.truncate(p);
            cur.extend_from_slice(tail);
            let word = std::str::from_utf8(&cur).map_err(|_| "affix dictionary: bad utf-8 in a stem")?;
            let flags = std::str::from_utf8(flags).map_err(|_| "affix dictionary: bad utf-8 in flags")?;
            if word.is_empty() || word.len() > MAX_WORD_BYTES {
                return Err("affix dictionary: bad stem".into());
            }
            stems.push((word.to_string(), flags.to_string()));
        }
        AffixDict::build(rules, stems, comments, MAX_FLAGS_PER_STEM)
    }
}

/// Does a compound rule's token list match a split of the word into two or more parts? `part(a,
/// b)` is the flags of the stem between char cuts `a` and `b` (`None`: not a stem). States are
/// (cut, token, parts so far capped at 2), each visited once.
fn compound_matches<'a>(rule: &[(Option<u16>, Repeat)], part: &mut impl FnMut(usize, usize) -> Option<&'a [u16]>, n: usize) -> bool {
    let t = rule.len();
    let idx = |cut: usize, tok: usize, k: usize| (cut * (t + 1) + tok) * 3 + k;
    let mut seen = vec![false; n * (t + 1) * 3];
    let mut stack = vec![(0usize, 0usize, 0usize)];
    while let Some((cut, tok, k)) = stack.pop() {
        match seen.get_mut(idx(cut, tok, k)) {
            Some(s) if !*s => *s = true,
            _ => continue,
        }
        if tok == t {
            if cut + 1 == n && k >= 2 {
                return true;
            }
            continue;
        }
        let Some(&(flag, repeat)) = rule.get(tok) else { continue };
        if repeat != Repeat::Once {
            stack.push((cut, tok + 1, k));
        }
        for next in cut + 1..n {
            if let Some(flag) = flag
                && part(cut, next).is_some_and(|set| has(set, flag))
            {
                let k = (k + 1).min(2);
                stack.push((next, if repeat == Repeat::Any { tok } else { tok + 1 }, k));
            }
        }
    }
    false
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const AFF: &str = "SET UTF-8\nTRY aeoiułkr\n\nPFX b Y 1\nPFX b   0   nie   .\n\nSFX a Y 3\nSFX a   0   y     [^y]\nSFX a   ek  ka    ek\nSFX a   0   em    .\n\nSFX c N 1\nSFX c   a   ą     a\n\nMAP 2\nMAP aą\nMAP oóu\nREP 2\nREP ż rz\nREP u ó\n";
    const DIC: &str = "4\nkot/ab\npiesek/a\nkrowa/c\nWarszawa/c\n";

    fn dict() -> AffixDict {
        AffixDict::from_hunspell(AFF, DIC, &["test data"]).unwrap()
    }

    #[test]
    fn affix_headers_allow_comments_but_rule_conditions_stay_literal() {
        let aff = "SET UTF-8\nPFX X Y 1 # prefix comment\nPFX X 0 ne . # rule comment\nSFX y Y 1 # suffix comment\nSFX y 0 y . # rule comment\n";
        let d = AffixDict::from_hunspell(aff, "1\nstrom/Xy\n", &[]).unwrap();
        assert_eq!(d.rule_count(), 2, "headers must not become affix rules");
        for w in ["strom", "stromy", "nestrom", "nestromy"] {
            assert!(d.contains(w), "{w}");
        }
        for w in ["stromY1", "stromN1", "X", "y"] {
            assert!(!d.contains(w), "{w}");
        }
        let hash = AffixDict::from_hunspell("SFX H Y 1\nSFX H 0 # .\nTRY a#b\nKEY a#b\n", "1\nstrom/H\n", &[]).unwrap();
        assert!(hash.contains("strom#"), "a hash in an affix field is literal");
        let literal = AffixDict::from_hunspell(
            "PFX P Y 1 # header\nPFX P 0 pre #\nSFX S Y 1 # header\nSFX S 0 post #\n",
            "3\n#stem/P\nstem#/S\nstem/PS\n",
            &[],
        )
        .unwrap();
        assert!(literal.contains("pre#stem") && literal.contains("stem#post"));
        assert!(!literal.contains("prestem") && !literal.contains("stempost"), "# must remain a literal condition");

        assert_eq!(hash.try_chars(), "a#b");
        assert_eq!(hash.key(), "a#b");
        assert_eq!(AffixDict::from_bytes(&hash.to_bytes().unwrap()).unwrap().key(), "a#b");
    }

    #[test]
    fn try_alphabet_is_capped_and_deduplicated() {
        let aff = format!("TRY {}\n", "abcdefghijklmnopqrstuvwxyz".repeat(10_000));
        let d = AffixDict::from_hunspell(&aff, "1\nkot\n", &[]).unwrap();
        assert_eq!(d.try_chars(), "abcdefghijklmnopqrstuvwxyz");
        let compact = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert_eq!(compact.try_chars(), "abcdefghijklmnopqrstuvwxyz");

        // More distinct characters than the cap, each repeated: keep the first 128 in order.
        let many: String = (0x100u32..0x100 + 300).filter_map(char::from_u32).collect();
        let aff = format!("TRY {many}{many}\n");
        let want: String = many.chars().take(128).collect();
        let d = AffixDict::from_hunspell(&aff, "1\nkot\n", &[]).unwrap();
        assert_eq!(d.try_chars(), want);
        let compact = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert_eq!(compact.try_chars(), want);

        // An oversized TRY line fed straight to the compact reader is capped as well.
        let raw = format!("{MAGIC}\nTRY {many}{many}\nWORDS\n\0kot\n");
        let bytes = miniz_oxide::deflate::compress_to_vec(raw.as_bytes(), 6);
        assert_eq!(AffixDict::from_bytes(&bytes).unwrap().try_chars(), want);
    }

    /// Shared hand-written fixture: collectors must reject every path to NOSUGGEST forms.
    pub(crate) fn assert_nosuggest_is_honoured(suggest: impl Fn(&AffixDict, &str, usize) -> Vec<String>) {
        let aff = "SET UTF-8\nNOSUGGEST N\nFORBIDDENWORD F\nTRY abeklmnostxy\nMAP 1\nMAP ae\nKEY ak\nREP 2\nREP xy toka\nREP zz normal\nPFX X Y 1\nPFX X 0 pre .\nSFX A Y 1\nSFX A 0 ble/B .\nSFX B Y 1\nSFX B 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "4\ntoka/NXA\nnormal/XA\nTkname/N\nbanned/F\n", &[]).unwrap();
        for word in d.forms("toka").into_iter().chain(["TOKA".into(), "Toka".into(), "Tkname".into(), "TKNAME".into()]) {
            assert!(d.check(&word), "NOSUGGEST is still correct: {word}");
            assert!(!d.can_suggest(&word), "NOSUGGEST form: {word}");
        }
        for input in ["tokax", "tokablesx", "pretokax", "pretokablesx", "toka", "TOKA", "tkname", "teka", "xy", "normaltoka", "toak", "taak"] {
            let results = suggest(&d, input, 32);
            assert!(results.iter().all(|word| word.split(' ').all(|part| d.can_suggest(part))), "{input}: {results:?}");
            assert!(results.iter().all(|word| !word.to_lowercase().contains("toka") && word.to_lowercase() != "tkname"), "{input}: {results:?}");
        }
        assert!(suggest(&d, "norrmal", 8).contains(&"normal".into()));
        assert!(suggest(&d, "zz", 8).contains(&"normal".into()));
        // Filtering happens before display casing: ordinary mixed-case stems retain the
        // existing all-caps suggestion behaviour, with and without a NOSUGGEST declaration.
        for aff in ["", "NOSUGGEST N\n"] {
            let ordinary = AffixDict::from_hunspell(aff, "2\nmixCase\nhidden/N\n", &[]).unwrap();
            assert!(suggest(&ordinary, "MIXCASX", 8).contains(&"MIXCASE".into()));
        }
    }

    #[test]
    fn nosuggest_is_correct_but_not_suggestible_and_survives_compact_format() {
        let d = AffixDict::from_hunspell("NOSUGGEST N\nSFX S Y 1\nSFX S 0 s .\n", "2\nsecret/NS\nnormal/S\n", &[]).unwrap();
        for dict in [&d, &AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap()] {
            for word in ["secret", "secrets", "Secret", "SECRETS"] {
                assert!(dict.check(word), "{word}");
                assert!(!dict.can_suggest(word), "{word}");
            }
            assert!(dict.contains_suggestible("normal") && dict.can_suggest("NORMALS"));
        }
        assert!(AffixDict::from_hunspell("NOSUGGEST NN\n", "1\nword\n", &[]).is_err());
    }

    #[test]
    fn stems_suffixes_prefixes_and_cross_products() {
        let d = dict();
        assert_eq!(d.len(), 4);
        for w in ["kot", "koty", "kotem", "piesek", "pieska", "piesekem", "krowa", "krową", "niekot", "niekoty", "niekotem", "Warszawa", "Warszawą"]
        {
            assert!(d.contains(w), "{w}");
        }
        for w in ["kotą", "pieseka", "pieskem", "niepiesek", "niekrową", "krowy", "warszawa", "", "nie", "y", "ka"] {
            assert!(!d.contains(w), "{w}");
        }
    }

    #[test]
    fn case_variants() {
        let d = dict();
        assert!(d.check("Kot") && d.check("KOT") && d.check("Koty") && d.check("WARSZAWĄ") && d.check("Warszawą"));
        assert!(!d.check("warszawa") && !d.check("kOT") && !d.check("KoT"));
        assert_eq!(casing("Kot"), Casing::Initial);
        assert_eq!(casing("KOT"), Casing::Upper);
        assert_eq!(casing("K"), Casing::Initial);
        assert_eq!(casing("kot"), Casing::Lower);
        assert_eq!(casing("McDonald"), Casing::Mixed);
        assert_eq!(capitalize("żółw"), "Żółw");
    }

    #[test]
    fn forms_are_all_accepted() {
        let d = dict();
        let mut forms = d.forms("kot");
        forms.sort();
        forms.dedup();
        assert_eq!(forms, ["kot", "kotem", "koty", "niekot", "niekotem", "niekoty"]);
        for stem in d.stems().map(str::to_string).collect::<Vec<_>>() {
            for f in d.forms(&stem) {
                assert!(d.contains(&f), "{stem} → {f}");
            }
        }
    }

    #[test]
    fn compact_format_round_trips() {
        let d = dict();
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert_eq!(back.len(), d.len());
        assert_eq!(back.rule_count(), d.rule_count());
        assert_eq!(back.comments(), ["test data"]);
        assert_eq!(back.rep(), d.rep());
        assert_eq!(back.map(), d.map());
        assert_eq!(back.try_chars(), "aeoiułkr");
        for w in ["kotem", "niekoty", "pieska", "krową", "Warszawą"] {
            assert!(back.contains(w), "{w}");
        }
        assert!(!back.contains("kotą"));
    }

    #[test]
    fn flag_utf8_and_default_modes_are_accepted() {
        // `FLAG UTF-8` (VERO pt_BR) and `FLAG DEFAULT` are per-character flag modes: the same
        // mode the reader assumes without a FLAG line.
        for mode in ["UTF-8", "DEFAULT"] {
            let aff = format!("SET UTF-8\nFLAG {mode}\nSFX a Y 1\nSFX a   0   y   [^y]\n");
            let d = AffixDict::from_hunspell(&aff, "1\nkot/a\n", &[]).unwrap();
            assert!(d.contains("kot") && d.contains("koty"), "{mode}");
        }
        assert!(AffixDict::from_hunspell("FLAG UTF-8\n", "0\n", &[]).is_ok(), "FLAG alone is harmless");
    }

    #[test]
    fn emoji_flags_with_a_variation_selector_load() {
        // RLA-ES (the Spanish dictionaries in LibreOffice and most Linux distributions) writes its
        // telephone flag as ☎ followed by U+FE0F, in the rule and on the stems.
        let aff = "SET UTF-8\nFLAG UTF-8\nPFX \u{260e}\u{fe0f} Y 1\nPFX \u{260e}\u{fe0f} 0 tele .\nSFX S Y 1\nSFX S 0 s [aeiou]\n";
        let d = AffixDict::from_hunspell(aff, "2\nbanco/S\u{260e}\u{fe0f}\nbasura/S\n", &[]).unwrap();
        assert!(d.contains("banco") && d.contains("bancos"));
        assert!(d.contains("telebanco") && d.contains("telebancos"), "the ☎ prefix applies");
        assert!(d.contains("basuras") && !d.contains("telebasura"), "only to stems that carry it");
        assert!(!d.flag_keys.contains(&FlagKey::from(EMOJI_SELECTOR)), "the selector is not a flag");
    }

    #[test]
    fn emoji_forbiddenword_and_nosuggest_flags_load() {
        // The same selector after the flag of the directives that name one flag.
        let aff = "SET UTF-8\nFLAG UTF-8\nFORBIDDENWORD \u{260e}\u{fe0f}\nNOSUGGEST \u{1f6ab}\u{fe0f}\n";
        let d = AffixDict::from_hunspell(aff, "3\nbanco\nbanko/\u{260e}\u{fe0f}\nbancu/\u{1f6ab}\u{fe0f}\n", &[]).unwrap();
        assert!(d.contains("banco") && d.contains("bancu"));
        assert!(!d.contains("banko"), "a stem with the FORBIDDENWORD flag is not a word");
    }

    #[test]
    fn more_than_64_distinct_flags_load() {
        // VERO pt_BR uses 106 distinct flags (FLAG UTF-8). Build one with every basic Latin
        // letter, the digits and eight accented letters: 70 flags, more than 64.
        let mut flags = String::new();
        let mut aff = String::from("SET UTF-8\nFLAG UTF-8\n");
        let chars: Vec<char> = ('a'..='z').chain('A'..='Z').chain('0'..='9').chain(['á', 'é', 'í', 'ó', 'ú', 'ç', 'ã', 'õ']).collect();
        assert_eq!(chars.len(), 70);
        for c in chars {
            aff.push_str(&format!("SFX {c} N 1\nSFX {c}   0   {c}   .\n"));
            flags.push(c);
        }
        let d = AffixDict::from_hunspell(&aff, &format!("1\nverbo/{flags}\n"), &[]).unwrap();
        assert!(d.contains("verbo"));
        assert!(d.contains("verboa") && d.contains("verboz") && d.contains("verbo9"));
        assert!(d.contains("verboç") && d.contains("verboã"), "multi-byte UTF-8 flags");
        assert!(!d.contains("verboß"), "ß is not a flag and not a stem");
    }

    #[test]
    fn more_than_128_distinct_flags_load() {
        // The Hungarian dictionary uses 158 single-byte flags: 200 flags here, U+0080 up.
        let flags: String = (0x80u32..0x80 + 200).filter_map(char::from_u32).collect();
        let mut aff = String::from("SET UTF-8\n");
        for (i, c) in flags.chars().enumerate() {
            aff.push_str(&format!("SFX {c} N 1\nSFX {c} 0 x{i} .\n"));
        }
        let half: String = flags.chars().step_by(2).collect();
        let d = AffixDict::from_hunspell(&aff, &format!("1\nszó/{half}\n"), &[]).unwrap();
        assert!(d.contains("szóx0") && d.contains("szóx198") && d.contains("szóx130"));
        assert!(!d.contains("szóx1") && !d.contains("szóx199") && !d.contains("szóx131"));
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(back.contains("szóx198") && !back.contains("szóx199"));
        // 257 distinct flags load too now (each flag gets a u16 id).
        let many: String = (0x100u32..0x100 + 257).filter_map(char::from_u32).collect();
        assert!(AffixDict::from_hunspell("SET UTF-8\n", &format!("1\nszó/{many}\n"), &[]).is_ok());
    }

    #[test]
    fn forbidden_words_are_rejected() {
        // VERO pt_BR marks spellings it must reject (flag ý under FLAG UTF-8) as stems carrying
        // FORBIDDENWORD: listed, but never correct and never inflected.
        let aff = "SET UTF-8\nFLAG UTF-8\nFORBIDDENWORD ý\nSFX a Y 1\nSFX a   0   y   [^y]\n";
        let d = AffixDict::from_hunspell(aff, "2\nkot/a\nimundo/ý\n", &[]).unwrap();
        assert!(d.check("kot") && d.check("koty") && d.check("Kot") && d.check("KOT"));
        assert!(!d.check("imundo") && !d.check("Imundo") && !d.check("IMUNDO"));
        assert!(!d.contains("imundoy"), "a forbidden stem must not inflect");
        assert!(d.forms("imundo").is_empty());
    }

    #[test]
    fn continuation_classes_chain_suffixes() {
        // `SFX A 0 able/B`: the form derived by A carries flag B, so a suffix of class B may
        // follow (workable → workables). The bare stem has A only, so `works` stays wrong.
        let aff = "SET UTF-8\nFLAG UTF-8\nSFX A N 1\nSFX A   0   able/B   .\nSFX B N 1\nSFX B   0   s   .\n";
        let d = AffixDict::from_hunspell(aff, "1\nwork/A\n", &[]).unwrap();
        assert!(d.contains("work") && d.contains("workable") && d.contains("workables"));
        assert!(!d.contains("works"), "the stem has A, not B");
        assert!(!d.contains("workabless"), "B declares no continuation");
        assert!(d.check("Workable") && d.check("WORKABLES"));
    }

    #[test]
    fn continuation_chains_stop_at_two_suffixes() {
        // Hunspell strips at most two suffixes: work + able/B + s/C is a word, a third (C) is not.
        let aff = "SET UTF-8\nFLAG UTF-8\nSFX A N 1\nSFX A   0   able/B   .\nSFX B N 1\nSFX B   0   s/C   .\nSFX C N 1\nSFX C   0   x   .\n";
        let d = AffixDict::from_hunspell(aff, "1\nwork/A\n", &[]).unwrap();
        assert!(d.contains("workable") && d.contains("workables"));
        assert!(!d.contains("workablesx"), "three chained suffixes exceed the limit");
        assert!(!d.forms("work").iter().any(|f| f == "workablesx"));
    }

    #[test]
    fn forms_include_continuation_chains() {
        let aff = "SET UTF-8\nFLAG UTF-8\nSFX A N 1\nSFX A   0   able/B   .\nSFX B N 1\nSFX B   0   s   .\n";
        let d = AffixDict::from_hunspell(aff, "1\nwork/A\n", &[]).unwrap();
        let mut f = d.forms("work");
        f.sort();
        f.dedup();
        assert_eq!(f, ["work", "workable", "workables"]);
        for form in d.forms("work") {
            assert!(d.contains(&form), "every listed form is accepted: {form}");
        }
    }

    #[test]
    fn continuation_classes_merge_the_flags_of_a_group() {
        // `s` under B and under C share one group (same append, same strip): a form carrying
        // either flag takes it, and the stem carrying neither doesn't.
        let aff = "SET UTF-8\nSFX A N 1\nSFX A 0 able/C .\nSFX D N 1\nSFX D 0 er/B .\nSFX B N 1\nSFX B 0 s .\nSFX C N 1\nSFX C 0 s [e]\n";
        let d = AffixDict::from_hunspell(aff, "2\nwork/A\nplay/D\n", &[]).unwrap();
        assert!(d.contains("workables") && d.contains("players"));
        assert!(!d.contains("works") && !d.contains("plays") && !d.contains("workabless"));
        assert!(d.forms("work").iter().any(|f| f == "workables") && d.forms("play").iter().any(|f| f == "players"));
    }

    /// `n` suffix rules of flag A on one stem, `rule(i)` giving each rule's strip, append and condition.
    fn many_rules(n: usize, rule: impl Fn(usize) -> String) -> AffixDict {
        let mut aff = format!("SET UTF-8\nSFX A Y {n}\n");
        for i in 0..n {
            aff.push_str(&format!("SFX A {}\n", rule(i)));
        }
        AffixDict::from_hunspell(&aff, "1\nkot/A\n", &[]).unwrap()
    }

    #[test]
    fn self_continuing_rules_check_in_linear_time() {
        // Thousands of empty-append rules that continue with their own flag: each check used to
        // follow every rule from every rule, millions of steps a word (#3613). The checks take
        // about a millisecond; the old checker took 14 s for two of them at 16,000 rules, so at
        // 40,000 (quadratic: minutes) the 10 s bound leaves a busy machine a wide margin.
        let d = many_rules(40_000, |_| "0 0/A .".into());
        let t = std::time::Instant::now();
        assert!(d.check("kot") && !d.check("pies") && !d.check("kotx"));
        assert!(t.elapsed().as_secs_f64() < 10.0, "{:?}", t.elapsed());
    }

    #[test]
    fn forms_of_self_continuing_rules_are_bounded() {
        // Every rule's form continues with every rule: millions of strings for one stem (#3613).
        let same = many_rules(3000, |_| "0 0/A .".into());
        let distinct = many_rules(3000, |i| format!("0 x{i}/A ."));
        let t = std::time::Instant::now();
        let (a, b) = (same.forms("kot"), distinct.forms("kot"));
        // The cap on the count is what tells the old code (9 million forms each) apart; the
        // generous time bound (tens of ms here) only backs up the rule-visit cap.
        assert!(t.elapsed().as_secs_f64() < 10.0, "{:?}", t.elapsed());
        assert!(a.len() <= MAX_FORMS && b.len() <= MAX_FORMS, "{} {}", a.len(), b.len());
        assert!(b.iter().any(|f| f == "kotx0x1"), "chained forms are still listed");
    }

    #[test]
    fn many_strips_per_suffix_index_in_linear_time() {
        // One append with thousands of strips: the index used to scan the append's groups for
        // each rule. Building 200,000 rules takes well under a second; the old index took 3 s at
        // 30,000 and over 10 minutes at 300,000.
        let n = 200_000;
        let t = std::time::Instant::now();
        let d = many_rules(n, |i| format!("k{i} y ."));
        assert!(t.elapsed().as_secs_f64() < 10.0, "{:?}", t.elapsed());
        assert_eq!(d.sfx_index.get("y").map(Vec::len), Some(n));
        assert!(!d.contains("koty"));
    }

    #[test]
    fn compact_format_round_trips_continuations_and_forbidden() {
        let aff = "SET UTF-8\nFLAG UTF-8\nFORBIDDENWORD ý\nSFX A N 1\nSFX A   0   able/B   .\nSFX B N 1\nSFX B   0   s   .\n";
        let d = AffixDict::from_hunspell(aff, "2\nwork/A\nimundo/ý\n", &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(back.contains("work") && back.contains("workable") && back.contains("workables"));
        assert!(!back.contains("works"), "the stem has A, not B, after a round trip");
        assert!(!back.check("imundo") && !back.contains("imundoy"), "forbidden survives a round trip");
    }

    #[test]
    fn dic_bom_glued_to_the_count_line_is_skipped() {
        // VERO's .dic opens with a UTF-8 BOM glued to its count line; it must not become a stem.
        let aff = "SET UTF-8\nSFX a Y 1\nSFX a   0   y   [^y]\n";
        let d = AffixDict::from_hunspell(aff, "\u{feff}1\nkot/a\n", &[]).unwrap();
        assert_eq!(d.len(), 1);
        assert!(d.contains("kot") && d.contains("koty"));
    }

    #[test]
    fn flag_aliases_stand_for_flag_sets() {
        // `AF`: `/1` in the `.dic` and in a continuation class means the first alias's flags.
        // Morphological aliases (`AM`) and the fields after a stem are skipped.
        let aff = "SET UTF-8\nAF 2\nAF ab # 1\nAF c # 2\nAM 1\nAM po:noun\nSFX a Y 1\nSFX a 0 ok/2 . 1\nSFX b Y 1\nSFX b 0 at . 1\nSFX c Y 1\nSFX c 0 ban . 1\n";
        let d = AffixDict::from_hunspell(aff, "1\nház/1\t1\n", &[]).unwrap();
        assert!(d.contains("ház") && d.contains("házok") && d.contains("házat") && d.contains("házokban"));
        assert!(!d.contains("házban"), "the stem has a and b, not c");
        // An undefined alias reads as no flags, as in Hunspell: the stem loads, without affixes.
        let undefined = AffixDict::from_hunspell(aff, "2\nház/3\nkert/1\n", &[]).unwrap();
        assert!(undefined.check("ház") && !undefined.check("házok"), "alias 3 is not defined");
        assert!(undefined.check("kertok"), "the other entries keep their aliases");
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(back.contains("házokban") && !back.contains("házban"), "aliases are expanded in the compact format");
    }

    #[test]
    fn iconv_and_ignore_normalize_input() {
        // Ligatures read as their letters; `IGNORE` characters vanish from input and stems.
        let aff = "SET UTF-8\nICONV 2\nICONV ﬁ fi\nICONV ﬄ ffl\nIGNORE ()\nSFX a Y 1\nSFX a 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "2\nfine/a\nwaf(fle)/a\n", &[]).unwrap();
        assert!(d.check("ﬁne") && d.check("ﬁnes") && d.check("Fine") && d.contains("ﬁne"));
        assert!(d.check("waffle") && d.check("waﬄes") && d.check("wa(ffle)"));
        assert!(!d.check("ﬁner"));
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(back.check("ﬁnes") && back.check("waﬄe") && back.check("waf(fle)"));
    }

    #[test]
    fn alias_numbers_are_read_as_hunspell_reads_them() {
        // Checked with the Hunspell 1.7 command line. With an `AF` table a flag field is the alias
        // its leading digits number: `fal/2x` is alias 2, `lab/b` has no flags.
        let aff = "SET UTF-8\nAF 2\nAF B\nAF C\nSFX B Y 1\nSFX B 0 s .\nSFX C Y 1\nSFX C 0 x .\nSFX b Y 1\nSFX b 0 q .\n";
        let d = AffixDict::from_hunspell(aff, "3\nlab/b\nfal/2x\nkut/1\n", &[]).unwrap();
        assert!(d.check("lab") && !d.check("labq") && !d.check("labs"));
        assert!(d.check("falx") && !d.check("fals") && d.check("kuts"));
        // A table without its count line is rejected: no alias applies.
        let broken = AffixDict::from_hunspell("SET UTF-8\nAF B\nAF C\nSFX B Y 1\nSFX B 0 s .\n", "2\nlab/1\nfal/2\n", &[]).unwrap();
        assert!(broken.check("lab") && !broken.check("labs") && !broken.check("fals"));
        // `AF 0` is rejected the same way: `lab/B` keeps its literal flag.
        let zero = AffixDict::from_hunspell("SET UTF-8\nAF 0\nSFX B Y 1\nSFX B 0 s .\n", "1\nlab/B\n", &[]).unwrap();
        assert!(zero.check("lab") && zero.check("labs"));
        // An undefined alias in a continuation class leaves the rule in force, without it.
        let cont = AffixDict::from_hunspell("SET UTF-8\nAF 1\nAF a\nSFX a Y 1\nSFX a 0 ok/7 .\n", "1\nhaz/1\n", &[]).unwrap();
        assert!(cont.check("haz") && cont.check("hazok"));
    }

    #[test]
    fn a_rep_count_line_may_carry_a_comment() {
        let d = AffixDict::from_hunspell("SET UTF-8\nREP 1 # confusions\nREP x y\n", "1\nay\n", &[]).unwrap();
        assert_eq!(d.rep(), [("x".to_string(), "y".to_string())]);
    }

    #[test]
    fn count_lines_may_carry_a_comment() {
        // Hunspell reads the first `AF` and `ICONV` line as the count, comment and all: `AF 2 #`
        // is no alias, and `ICONV 1 # …` doesn't turn `1` into `#`.
        let aff = "SET UTF-8\nAF 2 # aliases\nAF a # 1\nAF b # 2\nICONV 1 # ligatures\nICONV q y\nSFX a Y 1\nSFX a 0 ok .\nSFX b Y 1\nSFX b 0 at .\n";
        let d = AffixDict::from_hunspell(aff, "3\nhaz/1\nkert/2\na1\n", &[]).unwrap();
        assert!(d.check("hazok") && !d.check("hazat") && d.check("kertat") && !d.check("kertok"), "alias 1 is `a`");
        assert!(d.check("a1") && !d.check("a#"));
        // The UTF-8 reader keeps such a line as it is, so both readers agree on it.
        let (a, dic) = crate::hunspell::decode(aff.as_bytes(), "1\nhaz/1\n".as_bytes()).unwrap();
        assert!(a.contains("AF 2 # aliases") && a.contains("ICONV 1 # ligatures"), "{a}");
        assert!(AffixDict::from_hunspell(&a, &dic, &[]).unwrap().check("hazok"));
    }

    #[test]
    fn dic_entries_may_hold_spaces() {
        // `ad hoc/B` is one entry, as in Hunspell; `ad` alone is no word.
        let aff = "SET UTF-8\nSFX B Y 1\nSFX B 0 s .\n";
        // Checked with the Hunspell 1.7 command line: without a tab or a morphological field the
        // whole line is the entry (`kot/B 12` has the flag `B`), and leading blanks belong to it
        // (`  lab` is no `lab`).
        let d = AffixDict::from_hunspell(aff, "5\nad hoc/B\tpo:adv\nword/B po:noun\nkot/B 12\n  lab/B\nés így/B\n", &[]).unwrap();
        assert!(d.contains("ad hoc") && d.contains("ad hocs") && d.contains("és így") && d.contains("és ígys"));
        assert!(!d.contains("ad") && !d.contains("hoc") && !d.contains("ads") && !d.contains("így"));
        assert!(d.contains("words") && d.contains("kots") && !d.contains("kot/B"));
        assert!(!d.contains("lab") && !d.contains("labs"));
    }

    #[test]
    fn ignored_characters_leave_the_stripped_text_alone() {
        // Hunspell removes `IGNORE` characters from the appended text, not from the stripped
        // one: `SFX a bx c` never matches the stem `abx` (stored as `ab`), so `ac` is wrong,
        // while `SFX a b cx` appends `c` (checked with the Hunspell 1.7 command line).
        let strip = AffixDict::from_hunspell("SET UTF-8\nIGNORE x\nSFX a Y 1\nSFX a bx c .\n", "1\nabx/a\n", &[]).unwrap();
        assert!(strip.check("ab") && strip.check("abx") && !strip.check("ac"));
        let add = AffixDict::from_hunspell("SET UTF-8\nIGNORE x\nSFX a Y 1\nSFX a b cx .\n", "1\nab/a\n", &[]).unwrap();
        assert!(add.check("ac") && add.check("acx"));
        for d in [&strip, &add] {
            let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
            assert_eq!(back.check("ac"), d.check("ac"));
        }
    }

    /// CHECKSHARPS answers as Hunspell 1.7 gives them on the same files (`*` correct, `&` not).
    #[test]
    fn checksharps_reads_ss_as_sharp_s_in_capitals() {
        let dic = "6\nstraße\nMaße\nfußball/k\nmesse\nGruß/k\nSchloss\n";
        let words = [
            ("straße", true, true),
            ("Straße", true, true),
            ("STRASSE", false, true),
            ("STRAßE", true, true),
            ("STRAẞE", true, true),
            ("Strasse", false, false),
            ("strasse", false, false),
            ("MASSE", false, true),
            ("MAßE", true, true),
            ("maße", false, false),
            ("FUSSBALL", false, true),
            ("fußball", true, true),
            ("Fußball", false, true),
            ("FUßBALL", false, false),
            ("MESSE", true, true),
            ("MEßE", false, false),
            ("GRUSS", false, true),
            ("Gruß", true, true),
            ("gruß", false, false),
            ("GRUß", false, false),
            ("SCHLOSS", true, true),
            ("SCHLOß", false, false),
        ];
        for (sharps, aff) in [(false, "SET UTF-8\nKEEPCASE k\n"), (true, "SET UTF-8\nCHECKSHARPS\nKEEPCASE k\n")] {
            let d = AffixDict::from_hunspell(aff, dic, &[]).unwrap();
            let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
            for (w, without, with) in words {
                let want = if sharps { with } else { without };
                assert_eq!(d.check(w), want, "{w}, CHECKSHARPS {sharps}");
                assert_eq!(back.check(w), want, "{w}, CHECKSHARPS {sharps}, compact");
            }
        }
        assert_eq!(casing("STRAßE"), Casing::Upper);
        assert_eq!(casing("Maß"), Casing::Initial);
        // Many `SS` pairs: only the first MAX_SHARPS are varied, and a long word costs nothing.
        let d = AffixDict::from_hunspell("CHECKSHARPS\n", "1\naßaßaßaßaß\n", &[]).unwrap();
        assert!(d.check("ASSASSASSASSASS") && !d.check(&"SS".repeat(60)));
    }

    #[test]
    fn oconv_converts_suggestions_only() {
        // Answers as Hunspell 1.7 gives them on the same files: `kbt` → `k’t`.
        let aff = "SET UTF-8\nTRY ab\nOCONV 1\nOCONV a ’\n";
        let d = AffixDict::from_hunspell(aff, "1\nkat\n", &[]).unwrap();
        assert!(d.check("kat") && !d.check("k’t"), "OCONV doesn't change what is correct");
        assert_eq!(d.output("kat"), "k’t");
        assert!(matches!(d.output("kot"), std::borrow::Cow::Borrowed("kot")));
        assert_eq!(crate::affix_suggest::suggest_in(&d, "kbt", 4, &crate::affix_suggest::Tuning::DEFAULT), ["k’t"]);
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert_eq!(back.output("kat"), "k’t");
        // The count line may carry a comment, as for ICONV.
        let d = AffixDict::from_hunspell("OCONV 1 # apostrophes\nOCONV ' ’\n", "1\nl'eau\n", &[]).unwrap();
        assert_eq!(d.output("l'eau"), "l’eau");
    }

    #[test]
    fn conversions_take_the_longest_match_and_the_later_of_equal_ones() {
        let aff = "ICONV 3\nICONV a x\nICONV ab y\nICONV a z\n";
        let d = AffixDict::from_hunspell(aff, "2\ny\nzc\n", &[]).unwrap();
        assert_eq!(d.normalize("ab"), "y");
        assert_eq!(d.normalize("ac"), "zc");
        assert!(d.check("ab") && d.check("ac"));
    }

    #[test]
    fn conversion_tables_are_capped() {
        let pairs = |n: usize| (0..n).map(|i| format!("ICONV q{i} r\n")).collect::<String>();
        let ok = AffixDict::from_hunspell(&format!("ICONV {MAX_CONV_PAIRS}\n{}", pairs(MAX_CONV_PAIRS)), "1\nr\n", &[]).unwrap();
        assert!(ok.check("q65535") && !ok.check("q65536"));
        let err = AffixDict::from_hunspell(&format!("ICONV 1\n{}", pairs(MAX_CONV_PAIRS + 1)), "1\nr\n", &[]).unwrap_err();
        assert!(err.contains("ICONV table larger"), "{err}");
        let long = "x".repeat(MAX_CONV_BYTES + 1);
        for aff in [format!("ICONV 1\nICONV {long} y\n"), format!("OCONV 1\nOCONV y {long}\n")] {
            assert!(AffixDict::from_hunspell(&aff, "1\nr\n", &[]).is_err(), "{aff}");
        }
        let raw = format!("{MAGIC}\nOCONV {long} y\nWORDS\n\0kot\n");
        assert!(AffixDict::from_bytes(&miniz_oxide::deflate::compress_to_vec(raw.as_bytes(), 6)).is_err());
    }

    #[test]
    fn long_input_is_rejected_before_it_is_converted() {
        let d = AffixDict::from_hunspell("ICONV 1\nICONV ﬁ fi\nIGNORE -\n", "1\nfine\n", &[]).unwrap();
        // Ignored characters may take a word past MAX_WORD_BYTES and still leave a word.
        let spaced = "f-i-n-e".to_string() + &"-".repeat(MAX_WORD_BYTES);
        assert!(spaced.len() > MAX_WORD_BYTES && spaced.len() <= MAX_INPUT_BYTES && d.check(&spaced));
        // Past MAX_INPUT_BYTES the input is no word and is returned as it is, unconverted.
        let long = "ﬁ".repeat(MAX_INPUT_BYTES);
        assert!(matches!(d.normalize(&long), std::borrow::Cow::Borrowed(_)));
        assert!(!d.check(&long) && !d.contains(&long) && !d.can_suggest(&long));
    }

    #[test]
    fn normalizing_borrows_when_nothing_changes() {
        let d = AffixDict::from_hunspell("SET UTF-8\nICONV 1\nICONV ﬁ fi\nIGNORE ()\n", "1\nfine\n", &[]).unwrap();
        assert!(matches!(d.normalize("fine"), std::borrow::Cow::Borrowed("fine")));
        assert_eq!(d.normalize("(ﬁ)ne"), "fine");
        assert!(matches!(dict().normalize("kot"), std::borrow::Cow::Borrowed("kot")));
    }

    #[test]
    fn keepcase_stems_match_only_as_written() {
        let aff = "SET UTF-8\nKEEPCASE k\nSFX a Y 1\nSFX a 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "3\npH/k\nkm/ka\nkot/a\n", &[]).unwrap();
        assert!(d.check("pH") && d.check("km") && d.check("kms"));
        assert!(!d.check("PH") && !d.check("Km") && !d.check("KM") && !d.check("Kms"));
        assert!(d.check("Kot") && d.check("KOTS"), "other stems still follow the sentence and heading case");
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(back.check("km") && !back.check("Km"));
    }

    #[test]
    fn substandard_and_lemma_present_words_are_correct() {
        // `SUBSTANDARD` and `LEMMA_PRESENT` steer morphological generation, and a SUBSTANDARD
        // stem is never suggested; the stems and rules they mark are still correct.
        let aff = "SET UTF-8\nSUBSTANDARD x\nLEMMA_PRESENT l\nSFX a Y 2\nSFX a 0 y .\nSFX a 0 em/x .\n";
        let d = AffixDict::from_hunspell(aff, "2\nkot/a\npies/axl\n", &[]).unwrap();
        for dict in [&d, &AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap()] {
            assert!(dict.check("kot") && dict.check("koty") && dict.check("kotem"));
            assert!(dict.check("pies") && dict.check("piesy") && dict.check("Piesem"));
            assert!(dict.can_suggest("koty") && dict.can_suggest("kotem") && dict.contains_suggestible("kot"));
            assert!(!dict.can_suggest("piesy") && !dict.can_suggest("Pies") && !dict.contains_suggestible("pies"));
        }
    }

    #[test]
    fn unsupported_directives_are_refused_and_the_rest_load() {
        // A dictionary that needs a directive outside this engine fails to load rather than
        // accepting the wrong words.
        for dir in UNSUPPORTED {
            assert!(AffixDict::from_hunspell(&format!("SET UTF-8\n{dir} X\n"), DIC, &[]).is_err(), "{dir} should be refused");
        }
        // The directives this engine reads, with no other change to the dictionary (`AF` would
        // turn the flags of `DIC` into alias numbers; it has tests of its own).
        for line in ["AM 1\nAM po:noun", "ICONV 1\nICONV ﬁ fi", "IGNORE ()", "KEEPCASE k", "SUBSTANDARD x", "LEMMA_PRESENT l"] {
            let d = AffixDict::from_hunspell(&format!("{AFF}{line}\n"), DIC, &[]).unwrap_or_else(|e| panic!("{line}: {e}"));
            assert!(d.check("koty") && d.check("niekoty") && !d.check("kotx"), "{line}");
        }
    }

    /// SCOWL's ordinal rules (its English `.aff`, verbatim): `21st`, `112th` and `1000th` are
    /// compounds of digit stems, `1th` is only a part, and nothing else compounds.
    const ORDINALS_AFF: &str = "SET UTF-8\nCOMPOUNDMIN 1\nONLYINCOMPOUND c\nCOMPOUNDRULE 2\nCOMPOUNDRULE n*1t\nCOMPOUNDRULE n*mp\nWORDCHARS 0123456789\nSFX S Y 1\nSFX S 0 s .\n";
    const ORDINALS_DIC: &str = "13\n0/nm\n0th/pt\n1/n1\n1st/p\n1th/tc\n2/nm\n2nd/p\n2th/tc\n3/nm\n3rd/p\n3th/tc\n4/nm\n4th/pt\ncat/S\n";

    #[test]
    fn compound_rules_make_ordinals() {
        let d = AffixDict::from_hunspell(ORDINALS_AFF, ORDINALS_DIC, &[]).unwrap();
        for w in ["1st", "2nd", "4th", "21st", "22nd", "23rd", "11th", "12th", "13th", "111th", "1000th", "104th", "cats"] {
            assert!(d.check(w), "{w}");
        }
        for w in ["1th", "2th", "11st", "12nd", "13rd", "21th", "catcat", "cat1st", "1stcat", "1st1st", "", "x"] {
            assert!(!d.check(w), "{w}");
        }
        // Only-in-compound parts are never suggested on their own.
        assert!(!d.can_suggest("1th") && d.forms("1th").is_empty());
        // The compact format keeps the rules.
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(back.check("21st") && back.check("111th") && !back.check("1th") && !back.check("21th"));
    }

    #[test]
    fn compound_rules_need_two_parts_and_compoundmin() {
        // `COMPOUNDMIN` defaults to 3; a pattern a lone stem matches still needs two parts.
        let aff = "SET UTF-8\nONLYINCOMPOUND c\nCOMPOUNDRULE a*b?\n";
        let d = AffixDict::from_hunspell(aff, "3\nfoo/a\nbar/ab\nqux/c\n", &[]).unwrap();
        assert!(d.check("foobar") && d.check("foofoo") && d.check("foofoofoobar") && d.check("foo"));
        assert!(!d.check("qux") && !d.check("quxfoo") && !d.check("fooqux") && !d.check("foobarx"));
        let d = AffixDict::from_hunspell(aff, "2\nab/ab\ncd/a\n", &[]).unwrap();
        assert!(!d.check("abcd"), "parts shorter than COMPOUNDMIN 3");
        let d = AffixDict::from_hunspell(&format!("{aff}COMPOUNDMIN 2\n"), "2\nab/ab\ncd/a\n", &[]).unwrap();
        assert!(d.check("cdab"));
    }

    /// craft-libs#483 review: a forbidden word isn't rebuilt from its parts.
    #[test]
    fn compound_never_rebuilds_a_forbidden_word() {
        let d = AffixDict::from_hunspell("SET UTF-8\nFORBIDDENWORD X\nCOMPOUNDMIN 1\nCOMPOUNDRULE a*\n", "2\nx/a\nxx/X\n", &[]).unwrap();
        assert!(!d.check("xx") && d.check("xxx") && d.check("x"));
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(!back.check("xx") && back.check("xxx"));
    }

    /// craft-libs#483 review: `ONLYINCOMPOUND` survives a round trip without compound rules too.
    #[test]
    fn onlyincompound_round_trips_without_compound_rules() {
        let d = AffixDict::from_hunspell("SET UTF-8\nONLYINCOMPOUND c\n", "2\nfoo\nqux/c\n", &[]).unwrap();
        assert!(d.check("foo") && !d.check("qux"));
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(back.check("foo") && !back.check("qux"), "a part-only stem stays a part");
    }

    /// With long or numeric flags a `COMPOUNDRULE` writes each flag in parentheses, as Hunspell
    /// reads them (checked black-box against the `hunspell` CLI); a bare flag is refused.
    #[test]
    fn compound_rules_read_long_and_numeric_flags() {
        let aff = "SET UTF-8\nFLAG long\nCOMPOUNDMIN 1\nONLYINCOMPOUND cc\nCOMPOUNDRULE 1\nCOMPOUNDRULE (nn)*(mm)(pp)\n";
        let d = AffixDict::from_hunspell(aff, "4\n1/nnmm\n2/nnmm\n1st/pp\n1th/ppcc\n", &[]).unwrap();
        for w in ["1st", "21st", "11st", "221st", "21th"] {
            assert!(d.check(w), "{w}");
        }
        assert!(!d.check("1th") && !d.check("12"));
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(back.check("221st") && !back.check("1th"));
        let aff = "SET UTF-8\nFLAG num\nCOMPOUNDMIN 1\nONLYINCOMPOUND 9\nCOMPOUNDRULE 1\nCOMPOUNDRULE (1)*(2)(3)\n";
        let d = AffixDict::from_hunspell(aff, "3\n1/1,2\n2/1,2\n1st/3\n", &[]).unwrap();
        assert!(d.check("21st") && d.check("221st") && !d.check("12st"));
        for bad in ["FLAG long\nCOMPOUNDRULE nn*", "FLAG long\nCOMPOUNDRULE (n)", "FLAG num\nCOMPOUNDRULE (1", "FLAG num\nCOMPOUNDRULE (x)"] {
            assert!(AffixDict::from_hunspell(&format!("SET UTF-8\n{bad}\n"), "1\nx\n", &[]).is_err(), "{bad}");
        }
    }

    /// A NEEDAFFIX stem is no part of a `COMPOUNDRULE` compound (as the `hunspell` CLI answers).
    #[test]
    fn needaffix_stems_are_no_compound_part() {
        let d = AffixDict::from_hunspell("SET UTF-8\nNEEDAFFIX N\nCOMPOUNDRULE 1\nCOMPOUNDRULE ab\n", "2\nfoo/a\nbar/bN\n", &[]).unwrap();
        assert!(d.check("foo") && !d.check("bar") && !d.check("foobar"));
    }

    /// craft-libs#488 review: NEEDAFFIX, CIRCUMFIX and FULLSTRIP as Hunspell 1.7.5 reads them.
    /// Each case is a reviewer's repro `.aff`/`.dic`; the right and wrong words are the `hunspell`
    /// CLI's answers on the same files (black-box) for every prefix/stem/suffix combination of
    /// them, and `forms()` lists exactly the right ones.
    #[test]
    fn needaffix_circumfix_and_fullstrip_match_hunspell() {
        const CASES: &[(&str, &str, &str, &str)] = &[
            // rv488-hs/circ
            (
                "SET UTF-8\nCIRCUMFIX c\nPFX P Y 1\nPFX P 0 ne/c .\nSFX S Y 1\nSFX S 0 y/c .\nSFX T Y 1\nSFX T 0 z .\n",
                "1\nstrom/PST\n",
                "nestromy strom stromz",
                "nenestrom nenestromy nenestromyy nenestromyz nenestromz nenestromzy nenestromzz nestrom nestromyy nestromyz nestromz nestromzy nestromzz stromy stromyy stromyz stromzy stromzz",
            ),
            // rv488-hs/circ_chain
            (
                "SET UTF-8\nCIRCUMFIX c\nPFX P Y 1\nPFX P 0 ne/c .\nSFX A Y 1\nSFX A 0 able/Bc .\nSFX B Y 1\nSFX B 0 s .\n",
                "1\nwork/AP\n",
                "neworkable work",
                "nenework neneworkable neneworkableable neneworkables neneworks neneworksable neneworkss nework neworkableable neworks neworksable neworkss workable workableable workables works worksable workss",
            ),
            // rv488-hs/circN
            (
                "SET UTF-8\nCIRCUMFIX c\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 ne/cN .\nSFX S Y 1\nSFX S 0 y/c .\n",
                "1\nstrom/PS\n",
                "nestromy strom",
                "nenestrom nenestromy nenestromyy nestrom nestromyy stromy stromyy",
            ),
            // rv488-hs/circ_nocross
            (
                "SET UTF-8\nCIRCUMFIX c\nPFX P Y 1\nPFX P 0 ne/c .\nSFX S N 1\nSFX S 0 y/c .\n",
                "1\nstrom/PS\n",
                "strom",
                "nenestrom nenestromy nenestromyy nestrom nestromy nestromyy stromy stromyy",
            ),
            // rv488-hs/comp
            ("SET UTF-8\nNEEDAFFIX N\nCOMPOUNDRULE 1\nCOMPOUNDRULE ab\n", "2\nfoo/a\nbar/bN\n", "foo", "bar"),
            // rv488-hs/full
            ("SET UTF-8\nFULLSTRIP\nSFX A Y 1\nSFX A ab x .\nPFX B Y 1\nPFX B ab y .\n", "1\nab/AB\n", "ab x y", ""),
            // rv488-hs/full_pfxsfx
            ("SET UTF-8\nFULLSTRIP\nCIRCUMFIX c\nPFX P Y 1\nPFX P ab y/c .\nSFX S Y 1\nSFX S ab z/c .\n", "1\nab/PS\n", "ab", "y z"),
            // rv488-hs/homo2
            ("SET UTF-8\nNEEDAFFIX N\nSFX A Y 1\nSFX A 0 s .\n", "2\nfoo/NA\nfoo\n", "foo foos", "fooss"),
            // rv488-hs/homo
            ("SET UTF-8\nNEEDAFFIX N\nSFX A Y 1\nSFX A 0 s .\n", "2\nfoo/NA\nfoo/B\n", "foo foos", "fooss"),
            // rv488-hs/pfxna
            (
                "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re/N .\nSFX S Y 1\nSFX S 0 s/N .\n",
                "1\nwork/PS\n",
                "work",
                "rerework rereworks rereworkss rework reworks reworkss works workss",
            ),
            // rv488-hs/pfxstem
            ("SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re .\n", "1\nbaz/NP\n", "rebaz", "baz rerebaz"),
            // rv488-hs/pN_s
            (
                "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re/N .\nSFX S Y 1\nSFX S 0 s .\n",
                "1\nwork/PS\n",
                "reworks work works",
                "rerework rereworks rereworkss rework reworkss workss",
            ),
            // rv488-hs/p_sN
            (
                "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re .\nSFX S Y 1\nSFX S 0 s/N .\n",
                "1\nwork/PS\n",
                "rework reworks work",
                "rerework rereworks rereworkss reworkss works workss",
            ),
            // rv488-hs/sN_chain
            ("SET UTF-8\nNEEDAFFIX N\nSFX B Y 1\nSFX B 0 x/AN .\nSFX A Y 1\nSFX A 0 s .\n", "1\nbar/B\n", "bar barxs", "bars barss barsx barx barxx"),
            // rv488-hs/sNnocross
            (
                "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re .\nSFX S N 1\nSFX S 0 s/N .\n",
                "1\nwork/PS\n",
                "rework work",
                "rerework rereworks rereworkss reworks reworkss works workss",
            ),
            // rv488-hs/stemN_ps
            (
                "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re .\nSFX S Y 1\nSFX S 0 s .\n",
                "1\nwork/PSN\n",
                "rework reworks works",
                "rerework rereworks rereworkss reworkss work workss",
            ),
            // rv488-hs/zeroaff
            ("SET UTF-8\nNEEDAFFIX N\nSFX A Y 1\nSFX A 0 0 .\n", "1\nfoo/NA\n", "foo", ""),
            // rv488-second2/t11
            (
                "SET UTF-8\nFULLSTRIP\nSFX A Y 1\nSFX A ab x .\nPFX B Y 1\nPFX B ab y .\nSFX C Y 1\nSFX C 0 z .\n",
                "1\nab/ABC\n",
                "ab abz x y yz",
                "abzz xz yzz",
            ),
            // rv488-second2/t12
            ("SET UTF-8\nFULLSTRIP\nSFX A Y 1\nSFX A x 0 .\nPFX P Y 1\nPFX P 0 q .\n", "1\nx/AP\n", "q qx x", "qq qqx"),
            // rv488-second2/t13
            ("SET UTF-8\nFULLSTRIP\nSFX A Y 1\nSFX A x 0/B .\nSFX B Y 1\nSFX B 0 y .\n", "1\nx/A\n", "x y", "xy xyy"),
            // rv488-second2/t1
            ("SET UTF-8\nNEEDAFFIX N\nSFX A Y 1\nSFX A 0 s .\n", "2\nfoo/N\nfoo/A\n", "foo foos", "fooss"),
            // rv488-second2/t2
            (
                "SET UTF-8\nCIRCUMFIX X\nPFX A Y 1\nPFX A 0 leg/X .\nPFX B Y 1\nPFX B 0 legesleg/X .\nSFX C Y 3\nSFX C 0 obb .\nSFX C 0 obb/AX .\nSFX C 0 obb/BX .\n",
                "1\nnagy/C\n",
                "legeslegnagyobb legnagyobb nagy nagyobb",
                "legesleglegeslegnagy legesleglegeslegnagyobb legesleglegeslegnagyobbobb legesleglegnagy legesleglegnagyobb legesleglegnagyobbobb legeslegnagy legeslegnagyobbobb leglegeslegnagy leglegeslegnagyobb leglegeslegnagyobbobb leglegnagy leglegnagyobb leglegnagyobbobb legnagy legnagyobbobb nagyobbobb",
            ),
            // rv488-second2/t30
            (
                "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re .\nSFX B Y 1\nSFX B 0 x/N .\n",
                "1\nbar/BP\n",
                "bar rebar rebarx",
                "barx barxx rebarxx rerebar rerebarx rerebarxx",
            ),
            // rv488-second2/t31
            (
                "SET UTF-8\nCIRCUMFIX c\nPFX P Y 1\nPFX P 0 ne .\nPFX Q Y 1\nPFX Q 0 qe/c .\nSFX S Y 1\nSFX S 0 y/c .\nSFX T Y 1\nSFX T 0 z .\n",
                "1\nstrom/PQST\n",
                "nestrom nestromz qestromy strom stromz",
                "nenestrom nenestromy nenestromyy nenestromyz nenestromz nenestromzy nenestromzz neqestrom neqestromy neqestromyy neqestromyz neqestromz neqestromzy neqestromzz nestromy nestromyy nestromyz nestromzy nestromzz qenestrom qenestromy qenestromyy qenestromyz qenestromz qenestromzy qenestromzz qeqestrom qeqestromy qeqestromyy qeqestromyz qeqestromz qeqestromzy qeqestromzz qestrom qestromyy qestromyz qestromz qestromzy qestromzz stromy stromyy stromyz stromzy stromzz",
            ),
            // rv488-second2/t32
            (
                "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re/N .\nSFX B Y 1\nSFX B 0 x/N .\n",
                "1\nbar/BP\n",
                "bar",
                "barx barxx rebar rebarx rebarxx rerebar rerebarx rerebarxx",
            ),
            // rv488-second2/t3
            (
                "SET UTF-8\nPFX A Y 1\nPFX A 0 leg .\nSFX C Y 1\nSFX C 0 obb/A .\n",
                "1\nnagy/C\n",
                "legnagyobb nagy nagyobb",
                "leglegnagy leglegnagyobb leglegnagyobbobb legnagy legnagyobbobb nagyobbobb",
            ),
            // rv488-second2/t4
            (
                "SET UTF-8\nCIRCUMFIX X\nPFX A Y 1\nPFX A 0 leg/X .\nSFX C Y 2\nSFX C 0 obb .\nSFX C 0 obb/X .\n",
                "1\nnagy/AC\n",
                "legnagyobb nagy nagyobb",
                "leglegnagy leglegnagyobb leglegnagyobbobb legnagy legnagyobbobb nagyobbobb",
            ),
            // rv488-second2/t5
            (
                "SET UTF-8\nNEEDAFFIX N\nPFX A Y 1\nPFX A 0 leg/N .\nSFX C Y 1\nSFX C 0 obb/A .\n",
                "1\nnagy/C\n",
                "legnagyobb nagy nagyobb",
                "leglegnagy leglegnagyobb leglegnagyobbobb legnagy legnagyobbobb nagyobbobb",
            ),
            // rv488-second2/t6
            (
                "SET UTF-8\nNEEDAFFIX N\nPFX A Y 1\nPFX A 0 leg .\nSFX C Y 1\nSFX C 0 obb/AN .\n",
                "1\nnagy/C\n",
                "legnagyobb nagy",
                "leglegnagy leglegnagyobb leglegnagyobbobb legnagy legnagyobbobb nagyobb nagyobbobb",
            ),
            // rv488-second2/t7
            (
                "SET UTF-8\nNEEDAFFIX N\nSFX A Y 1\nSFX A 0 able/B .\nSFX B Y 1\nSFX B 0 s/N .\n",
                "1\nwork/A\n",
                "work workable workables",
                "workableable works worksable workss",
            ),
            // rv488-second2/t8
            (
                "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re .\nSFX Z Y 1\nSFX Z 0 0 .\nSFX S Y 1\nSFX S 0 s .\n",
                "2\nfoo/NPS\nbar/NZ\n",
                "bar foos refoo refoos",
                "bars barss foo fooss rebar rebars rebarss refooss rerebar rerebars rerebarss rerefoo rerefoos rerefooss",
            ),
            // rv488-second2/t9
            (
                "SET UTF-8\nCIRCUMFIX c\nPFX P Y 1\nPFX P 0 ne/c .\nSFX A Y 1\nSFX A 0 able/B .\nSFX B Y 1\nSFX B 0 s/c .\n",
                "1\nwork/AP\n",
                "work workable workables",
                "nenework neneworkable neneworkableable neneworkables neneworks neneworksable neneworkss nework neworkable neworkableable neworkables neworks neworksable neworkss workableable works worksable workss",
            ),
        ];
        for (aff, dic, right, wrong) in CASES {
            let d = AffixDict::from_hunspell(aff, dic, &[]).unwrap();
            for w in right.split_whitespace() {
                assert!(d.check(w), "{w} is right:\n{aff}{dic}");
            }
            for w in wrong.split_whitespace() {
                assert!(!d.check(w), "{w} is wrong:\n{aff}{dic}");
            }
            let mut forms: Vec<String> = dic.lines().skip(1).flat_map(|l| d.forms(l.split('/').next().unwrap_or(""))).collect();
            forms.sort();
            forms.dedup();
            let mut want: Vec<&str> = right.split_whitespace().collect();
            want.sort_unstable();
            assert_eq!(forms, want, "forms():\n{aff}{dic}");
        }
    }

    /// A circumfix prefix with two suffixes (`ne` + `work` + `able/Bc` + `s`): Hunspell 1.7.5
    /// takes `neworkables`; this engine allows a prefix with one suffix only.
    #[test]
    #[ignore = "prefix + two suffixes is not supported (craft-libs#488 review P3)"]
    fn a_circumfix_prefix_with_a_suffix_chain() {
        let aff = "SET UTF-8\nCIRCUMFIX c\nPFX P Y 1\nPFX P 0 ne/c .\nSFX A Y 1\nSFX A 0 able/Bc .\nSFX B Y 1\nSFX B 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "1\nwork/AP\n", &[]).unwrap();
        assert!(d.check("neworkables"));
    }

    #[test]
    fn hostile_compound_rules_are_refused_or_cheap() {
        for bad in ["COMPOUNDRULE *a", "COMPOUNDRULE (ab)", "COMPOUNDRULE a**", "COMPOUNDMIN x", "ONLYINCOMPOUND ab"] {
            assert!(AffixDict::from_hunspell(&format!("SET UTF-8\n{bad}\n"), DIC, &[]).is_err(), "{bad}");
        }
        let many: String = (0..100).map(|_| "COMPOUNDRULE a*\n").collect();
        assert!(AffixDict::from_hunspell(&format!("SET UTF-8\n{many}"), DIC, &[]).is_err());
        // Every part a stem and a pattern of stars: still answers at once.
        let star: String = std::iter::repeat_n("a*", 16).collect();
        let d = AffixDict::from_hunspell(&format!("SET UTF-8\nCOMPOUNDMIN 1\nCOMPOUNDRULE {star}b\n"), "1\nx/a\n", &[]).unwrap();
        let t = std::time::Instant::now();
        assert!(!d.check(&"x".repeat(48)) && !d.check(&"x".repeat(200)));
        assert!(t.elapsed().as_secs() < 2);
    }

    #[test]
    fn fields_split_on_spaces_and_tabs_only() {
        // A flag byte read as U+00A0 (no-break space) or U+0085 is a flag, not a separator.
        let aff = "SET UTF-8\nSFX \u{a0} Y 1\nSFX \u{a0} 0 y .\nSFX \u{85} Y 1\nSFX \u{85} 0 em .\n";
        let d = AffixDict::from_hunspell(aff, "1\nkot/\u{a0}\u{85}\n", &[]).unwrap();
        assert!(d.contains("koty") && d.contains("kotem"));
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(back.contains("koty") && back.contains("kotem"));
        // The same bytes as one-flag directives, as Hunspell reads them.
        let aff = "SET UTF-8\nFORBIDDENWORD \u{a0}\nKEEPCASE \u{85}\nSFX a Y 1\nSFX a 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "3\nkot/a\nrossz/\u{a0}\npH/\u{85}\n", &[]).unwrap();
        for dict in [&d, &AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap()] {
            assert!(dict.check("kots") && !dict.check("rossz"));
            assert!(dict.check("pH") && !dict.check("PH"));
        }
    }

    /// craft-libs#482 review: an English dictionary with compounding and output conversion, as the
    /// usual en_US and en_GB ones have, loads for English: `COMPOUNDRULE` makes its ordinals, its
    /// `ONLYINCOMPOUND` stems are words only inside them, `COMPOUNDFLAG` is ignored (other
    /// languages still refuse it) and `OCONV` is applied to suggestions. (A synthetic `.aff`
    /// written for this test.)
    #[test]
    fn english_dictionaries_ignore_compounding_and_output_conversion() {
        let aff = "SET UTF-8\nTRY esianrtolcdugmphbyfvkwz'\nICONV 1\nICONV ’ '\nOCONV 1\nOCONV ' ’\nNOSUGGEST !\n\
                   COMPOUNDMIN 1\nONLYINCOMPOUND c\nCOMPOUNDRULE 2\nCOMPOUNDRULE n*1t\nCOMPOUNDRULE n*mp\nCOMPOUNDFLAG Z\n\
                   WORDCHARS 0123456789\nSFX S Y 1\nSFX S 0 s [^sxzhy]\nSFX D Y 1\nSFX D 0 ed [^ey]\n";
        let dic = "7\nhouse/S\nwalk/SD\n0th/pc\n1st/p\n1th/tc\n1/n1\nthe\n";
        assert!(AffixDict::from_hunspell(aff, dic, &[]).is_err(), "refused for other languages");
        let d = AffixDict::from_hunspell_english(aff, dic, &[]).unwrap();
        for w in ["house", "houses", "walked", "walks", "the"] {
            assert!(d.check(w), "{w}");
        }
        assert!(!d.check("0th") && !d.check("1th"), "parts of compounds only");
        assert!(d.check("1st") && d.check("11th") && d.check("111th") && !d.check("11st"), "ordinals by COMPOUNDRULE");
        assert!(!d.check("housed") && !d.check("hous"));
        // OCONV is read and applied to suggestions, as Hunspell does: `'` is shown as `’`.
        let d = AffixDict::from_hunspell_english(aff, "2\ndon't\nwalk/S\n", &[]).unwrap();
        assert_eq!(d.output("don't"), "don’t");
        assert!(d.check("don't") && d.check("don’t"), "ICONV reads it back");
        assert!(crate::affix_suggest::suggest_in(&d, "donn't", 4, &crate::affix_suggest::Tuning::DEFAULT).contains(&"don’t".to_string()));
        // The directives that change which words are right are still refused.
        assert!(AffixDict::from_hunspell_english("PSEUDOROOT X\n", dic, &[]).is_err(), "PSEUDOROOT");
    }

    /// The English path reads `ONLYINCOMPOUND` in the dictionary's FLAG mode too.
    #[test]
    fn english_onlyincompound_reads_long_and_numeric_flags() {
        for (aff, dic) in [
            ("FLAG long\nONLYINCOMPOUND cc\nCOMPOUNDMIN 1\nSFX Ss Y 1\nSFX Ss 0 s .\n", "3\nhouse/Ss\n1st/cc\nnth/Sscc\n"),
            ("FLAG num\nONLYINCOMPOUND 7\nCOMPOUNDMIN 1\nSFX 1 Y 1\nSFX 1 0 s .\n", "3\nhouse/1\n1st/7\nnth/1,7\n"),
        ] {
            let d = AffixDict::from_hunspell_english(aff, dic, &[]).unwrap();
            assert!(d.check("house") && d.check("houses"), "{aff}");
            assert!(!d.check("1st") && !d.check("nth") && !d.check("nths"), "{aff}");
        }
    }

    #[test]
    fn unsupported_and_broken_input_is_an_error() {
        // `FLAG long` and `FLAG num` load now (see the tests below); an unknown mode doesn't.
        assert!(AffixDict::from_hunspell("FLAG long\n", DIC, &[]).is_ok());
        assert!(AffixDict::from_hunspell("FLAG num\n", DIC, &[]).is_ok());
        assert!(AffixDict::from_hunspell("FLAG bogus\n", DIC, &[]).is_err());
        assert!(AffixDict::from_hunspell("COMPOUNDFLAG X\n", DIC, &[]).is_err());
        assert!(AffixDict::from_hunspell("SFX a Y 1\nSFX a 0 y [ab\n", DIC, &[]).is_err());
        assert!(AffixDict::from_bytes(b"not deflate").is_err());
        let ok = dict().to_bytes().unwrap();
        assert!(AffixDict::from_bytes(&ok[..ok.len() / 2]).is_err());
        let garbage = miniz_oxide::deflate::compress_to_vec(b"WCAFF1\nWORDS\n\x09abc\n", 6);
        assert!(AffixDict::from_bytes(&garbage).is_err(), "prefix longer than the previous word");
        let empty = AffixDict::default();
        assert!(!empty.contains("kot") && !empty.check("Kot") && empty.forms("kot").is_empty());
    }

    /// A small `FLAG num` dictionary (made-up words): prefixes, suffixes, a continuation class,
    /// a cross product, the largest flag number, and numeric FORBIDDENWORD and NOSUGGEST flags.
    const NUM_AFF: &str = "SET UTF-8\nFLAG num\nFORBIDDENWORD 900\nNOSUGGEST 901\nPFX 7 Y 1\nPFX 7 0 ne .\nSFX 12 Y 2\nSFX 12 0 a .\nSFX 12 0 om/1503 .\nSFX 1503 N 1\nSFX 1503 0 e .\nSFX 65535 N 1\nSFX 65535 0 i .\n";
    const NUM_DIC: &str = "5\ngrad/12,7\nmost/65535\nkuca/12,901\ngradd/900\nlist/1503\n";

    fn assert_num_dict(d: &AffixDict) {
        for w in ["grad", "grada", "gradom", "gradome", "negrad", "negrada", "negradom", "most", "mosti", "list", "liste", "Grada", "MOSTI"] {
            assert!(d.check(w), "{w}");
        }
        // 1503 reaches `grad` only through `om`; 12 and 65535 belong to other stems; 7 is a prefix.
        for w in ["grade", "gradi", "mosta", "nemost", "lista", "listom", "gradomi", "7grad", "grad12"] {
            assert!(!d.check(w), "{w}");
        }
        assert!(!d.check("gradd") && !d.check("Gradd") && d.forms("gradd").is_empty(), "numeric FORBIDDENWORD");
        assert!(d.check("kuca") && d.check("kucaa") && !d.can_suggest("kuca") && !d.can_suggest("kucaom"), "numeric NOSUGGEST");
        assert!(d.can_suggest("gradome"));
        let mut forms = d.forms("grad");
        forms.sort();
        forms.dedup();
        assert_eq!(forms, ["grad", "grada", "gradom", "gradome", "negrad", "negrada", "negradom"]);
    }

    #[test]
    fn flag_num_dictionaries_load_and_check() {
        let d = AffixDict::from_hunspell(NUM_AFF, NUM_DIC, &[]).unwrap();
        assert_eq!((d.len(), d.rule_count()), (5, 5));
        assert_num_dict(&d);
    }

    #[test]
    fn flag_num_dictionaries_round_trip_the_compact_format() {
        let d = AffixDict::from_hunspell(NUM_AFF, NUM_DIC, &["num test"]).unwrap();
        let bytes = d.to_bytes().unwrap();
        let raw = miniz_oxide::inflate::decompress_to_vec(&bytes).unwrap();
        assert!(raw.starts_with(b"WCAFF1\nFLAG num\n"), "the mode is written before any flag");
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("FORBIDDENWORD 900\n") && text.contains("SFX 12 Y 0 om/1503 .\n") && text.contains("grad/7,12\n"));
        let back = AffixDict::from_bytes(&bytes).unwrap();
        assert_num_dict(&back);
        assert_eq!(back.comments(), ["num test"]);
        assert_eq!(back.to_bytes().unwrap(), bytes, "a second round trip writes the same data");
    }

    /// A small `FLAG long` dictionary: two-character flags that share characters with each other.
    const LONG_AFF: &str = "SET UTF-8\nFLAG long\nFORBIDDENWORD !!\nSFX Aa Y 1\nSFX Aa 0 s/Bb .\nSFX Bb N 1\nSFX Bb 0 ly .\nPFX Cc Y 1\nPFX Cc 0 un .\nSFX aA N 1\nSFX aA 0 er .\n";
    const LONG_DIC: &str = "5\nkind/AaCc\nhope/Bb\nbadd/!!\nodd/Aab\nwarm/aA\n";

    fn assert_long_dict(d: &AffixDict) {
        // `odd/Aab`: an odd last flag character is dropped, as Hunspell does (`odds` is right).
        for w in ["kind", "kinds", "kindsly", "unkind", "unkinds", "hope", "hopely", "warm", "warmer", "odd", "odds"] {
            assert!(d.check(w), "{w}");
        }
        for w in ["kindly", "kinder", "hopes", "unhope", "warms", "unwarm", "badd", "unodd"] {
            assert!(!d.check(w), "{w}");
        }
    }

    #[test]
    fn flag_aliases_hold_long_and_numeric_flags() {
        // `AF` aliases stand for flags written in the FLAG mode: `AaBb` is two long flags,
        // `12,7` two numbers. Answers match Hunspell 1.7's on the same files.
        let aff = "SET UTF-8\nFLAG long\nAF 2\nAF AaBb\nAF Bb\nSFX Aa Y 1\nSFX Aa 0 s/2 .\nSFX Bb Y 1\nSFX Bb 0 ly .\n";
        let d = AffixDict::from_hunspell(aff, "2\nkind/1\nhope/2\n", &[]).unwrap();
        for w in ["kind", "kinds", "kindly", "kindsly", "hope", "hopely"] {
            assert!(d.check(w), "{w}");
        }
        assert!(!d.check("hopes"));
        let aff = "SET UTF-8\nFLAG num\nAF 1\nAF 12,7\nSFX 12 Y 1\nSFX 12 0 a .\nPFX 7 Y 1\nPFX 7 0 ne .\n";
        let d = AffixDict::from_hunspell(aff, "1\ngrad/1\n", &[]).unwrap();
        for w in ["grad", "grada", "negrad", "negrada"] {
            assert!(d.check(w), "{w}");
        }
        assert!(AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap().check("negrada"));
        // A stem's alias reads as its own flags would: an odd last long-flag character is
        // dropped (Hunspell: `odd` and `odds` right).
        let d = AffixDict::from_hunspell("FLAG long\nAF 1\nAF Aab\nSFX Aa Y 1\nSFX Aa 0 s .\n", "2\nodd/1\nkot\n", &[]).unwrap();
        assert!(d.check("odd") && d.check("odds") && d.check("kot"));
    }

    #[test]
    fn a_substandard_homonym_past_the_merged_cap_keeps_its_flag() {
        let n = MAX_FLAGS_PER_STEM / MAX_FLAGS_PER_ENTRY + 1;
        let x = 60_000;
        let d = AffixDict::from_hunspell(&format!("FLAG num\nSUBSTANDARD {x}\nSFX 1 Y 1\nSFX 1 0 a .\n"), &homonyms(n, &x.to_string()), &[]).unwrap();
        assert!(d.check("same") && d.check("samea") && !d.contains_suggestible("same"));
    }

    #[test]
    fn flag_long_dictionaries_load_check_and_round_trip() {
        let d = AffixDict::from_hunspell(LONG_AFF, LONG_DIC, &[]).unwrap();
        assert_eq!(d.len(), 5);
        assert_long_dict(&d);
        let bytes = d.to_bytes().unwrap();
        assert!(miniz_oxide::inflate::decompress_to_vec(&bytes).unwrap().starts_with(b"WCAFF1\nFLAG long\n"));
        let back = AffixDict::from_bytes(&bytes).unwrap();
        assert_long_dict(&back);
        assert_eq!(back.to_bytes().unwrap(), bytes);
        assert!(AffixDict::from_hunspell("FLAG long\nSFX A Y 1\nSFX A 0 s .\n", "0\n", &[]).is_err(), "a one-character long flag");
    }

    #[test]
    fn malformed_flag_numbers_are_refused_in_the_aff_and_read_as_hunspell_does_in_the_dic() {
        for rule in ["x", "65536", "99999999999999999999999", "+5", "-1", "1,2", " "] {
            let aff = format!("FLAG num\nSFX {rule} Y 1\nSFX {rule} 0 a .\n");
            assert!(AffixDict::from_hunspell(&aff, "1\nw/1\n", &[]).is_err(), "rule flag `{rule}`");
        }
        // Hunspell drops a rule whose continuation classes don't read; refusing the file is safe.
        for cont in ["2,,3", "2,", ",2", "a", "70000", "x2"] {
            let aff = format!("FLAG num\nSFX 1 Y 1\nSFX 1 0 a/{cont} .\n");
            assert!(AffixDict::from_hunspell(&aff, "1\nw/1\n", &[]).is_err(), "continuation `{cont}`");
        }
        for directive in ["FORBIDDENWORD 1,2", "FORBIDDENWORD abc", "NOSUGGEST -1", "NOSUGGEST", "FORBIDDENWORD 65536"] {
            assert!(AffixDict::from_hunspell(&format!("FLAG num\n{directive}\n"), "0\n", &[]).is_err(), "{directive}");
        }
        assert!(AffixDict::from_hunspell("SFX a Y 1\nSFX a 0 y .\nFLAG num\n", "0\n", &[]).is_err(), "FLAG after flags");
        assert!(AffixDict::from_hunspell("FLAG num\nFLAG num\nSFX 1 Y 1\nSFX 1 0 a .\n", "0\n", &[]).is_ok());
        // The .dic: as Hunspell 1.7 reads them (black-box, same files), an item that is no number
        // from 0 to 65,535 is flag 0 and the word stays: `1,,2` is 1, 0 and 2; `x2`, `-1` and
        // `70000` are 0; `000001` is 1. A real Nepali entry, `निजामती/I15,22`, keeps flag 22.
        let aff = "SET UTF-8\nFLAG num\nSFX 1 Y 1\nSFX 1 0 a .\nSFX 2 Y 1\nSFX 2 0 b .\nSFX 0 Y 1\nSFX 0 0 z .\n";
        let dic = "10\nok/1\nempty/1,,2\nbig/70000\nhuge/123456789012345678901234567890\nsign/-1\ncomma/1,\nletters/a1\nzeros/000001\nनिजामती/I15,2\nplain\n";
        let d = AffixDict::from_hunspell(aff, dic, &[]).unwrap();
        assert_eq!(d.len(), 10);
        for w in [
            "ok",
            "oka",
            "plain",
            "empty",
            "emptya",
            "emptyb",
            "emptyz",
            "bigz",
            "hugez",
            "signz",
            "comma",
            "commaa",
            "commaz",
            "lettersz",
            "zerosa",
            "निजामतीb",
            "निजामतीz",
        ] {
            assert!(d.check(w), "{w}");
        }
        for w in ["bigb", "biga", "signa", "lettersa", "zerosz", "निजामतीa", "plainz"] {
            assert!(!d.check(w), "{w}");
        }
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(back.check("emptyz") && back.check("निजामतीb") && !back.check("bigb"));
        // Text after a number's digits is ignored, as Hunspell does (Nepali's `0/17X`).
        let d = AffixDict::from_hunspell("FLAG num\nSFX 1 Y 1\nSFX 1 0 a/2X .\nSFX 2 N 1\nSFX 2 0 i .\n", "1\nw/1x,7\n", &[]).unwrap();
        assert!(d.check("wa") && d.check("wai") && !d.check("wi"));
        // Flag 0 is a flag, as Hunspell reads it (the Turkish dictionary's `SFX 0 N 1`).
        let d = AffixDict::from_hunspell("FLAG num\nSFX 0 N 1\nSFX 0 0 de .\n", "2\nev/0\nsu/0,1\n", &[]).unwrap();
        assert!(d.check("evde") && d.check("sude") && !d.check("kolde"));
        assert!(AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap().check("evde"));
    }

    /// craft-libs#487 review: 21,000 suffix rules whose continuation classes are one 1,024-flag
    /// `AF` alias made every lookup cost rules × set size (172 ms a `check()`). A set is tested
    /// once per lookup now, against a bitmap.
    #[test]
    fn big_continuation_sets_on_many_rules_stay_fast() {
        // Rules 1–1,024 continue with alias 1 (their own flags), so a chain step looks for 1,024
        // flags; the other 20,000 rules on the same suffix continue with alias 2, 1,024 flags none
        // of which is looked for: each of them is a full miss.
        let set = |from: usize| (from..from + 1024).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
        let mut aff = format!("SET UTF-8\nFLAG num\nAF 2\nAF {}\nAF {}\n", set(1), set(40_001));
        for n in 1..=21_024 {
            let alias = if n <= 1024 { 1 } else { 2 };
            aff.push_str(&format!("SFX {n} Y 1\nSFX {n} 0 a/{alias} .\n"));
        }
        let d = AffixDict::from_hunspell(&aff, "1\nstem/1\n", &[]).unwrap();
        assert!(d.check("stem") && d.check("stema") && d.check("stemaa"));
        let t = std::time::Instant::now();
        for w in ["qqaa", "qqaaa", "xaaa", "stemaaa", "zzzzaa", "qa", "aaaa", "bbaa"] {
            for _ in 0..5 {
                std::hint::black_box(d.check(w));
            }
        }
        assert!(t.elapsed().as_millis() < 2000, "{:?} for 40 checks", t.elapsed());
    }

    /// The same stem on 200,000 lines merges in linear time (no sort for a homonym that adds
    /// nothing).
    #[test]
    fn many_repeated_homonyms_load_fast() {
        let big: Vec<String> = (1..=1024).map(|n| n.to_string()).collect();
        let mut dic = String::from("200008\n");
        for k in 0..8 {
            let part: Vec<String> = (0..1024).map(|n| (k * 1024 + n + 1).to_string()).collect();
            dic.push_str(&format!("same/{}\n", part.join(",")));
        }
        for _ in 0..200_000 {
            dic.push_str("same\n");
        }
        let t = std::time::Instant::now();
        let d = AffixDict::from_hunspell("FLAG num\n", &dic, &[]).unwrap();
        assert!(d.check("same") && big.len() == 1024);
        assert!(t.elapsed().as_secs() < 5, "{:?}", t.elapsed());
    }

    /// craft-libs#487 review: an `AF` table is read in the final FLAG mode (Hunspell would read an
    /// alias before a `FLAG num` line one character at a time; no real dictionary does that), is
    /// shared by the stems using it rather than copied, and is capped.
    #[test]
    fn aliases_are_read_once_in_the_final_mode_and_capped() {
        let d = AffixDict::from_hunspell("AF 1\nAF 1,2\nFLAG num\nSFX 1 Y 1\nSFX 1 0 a .\nSFX 2 Y 1\nSFX 2 0 b .\n", "2\nw/1\nv/1\n", &[]).unwrap();
        assert!(d.check("wa") && d.check("wb") && d.check("va"));
        let many: String = (0..=MAX_ALIASES).map(|_| "AF 1\n").collect();
        assert!(AffixDict::from_hunspell(&format!("FLAG num\nAF {}\n{many}", MAX_ALIASES + 1), "0\n", &[]).is_err());
        let big = format!("AF {}\n", "1,".repeat(MAX_ALIAS_BYTES / 2 + 1));
        assert!(AffixDict::from_hunspell(&format!("FLAG num\nAF 1\n{big}"), "0\n", &[]).is_err());
    }

    /// ONLYINCOMPOUND survives the compact format without compound rules too, and with them it
    /// sits after COMPOUNDMIN, where one-character data has always written it.
    #[test]
    fn onlyincompound_round_trips_in_place() {
        let d = AffixDict::from_hunspell("ONLYINCOMPOUND c\n", "2\npart/c\nword\n", &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(back.check("word") && !back.check("part"));
        let d = AffixDict::from_hunspell("COMPOUNDMIN 1\nONLYINCOMPOUND c\nCOMPOUNDRULE 1\nCOMPOUNDRULE ab\n", "3\nx/a\ny/b\nz/bc\n", &[]).unwrap();
        let raw = String::from_utf8(miniz_oxide::inflate::decompress_to_vec(&d.to_bytes().unwrap()).unwrap()).unwrap();
        assert!(raw.contains("COMPOUNDMIN 1\nONLYINCOMPOUND c\nCOMPOUNDRULE ab\n"), "{raw}");
    }

    #[test]
    fn thousands_of_flags_load() {
        // Serbian uses 1,903 numeric flags, Belarusian and Sinhala 5,000, each stem one or two:
        // 3,000 flags here, all appending `a` (one suffix group of 3,000 rules), plus flags up to
        // the largest number.
        let mut aff = String::from("SET UTF-8\nFLAG num\n");
        let mut dic = String::from("3002\n");
        for n in 1..=3000 {
            aff.push_str(&format!("SFX {n} N 1\nSFX {n} 0 a [^q]\n"));
            dic.push_str(&format!("w{n}/{n}\n"));
        }
        aff.push_str("SFX 65000 N 1\nSFX 65000 0 i .\n");
        dic.push_str("dva/3000,65000\nq/1\n");
        let d = AffixDict::from_hunspell(&aff, &dic, &[]).unwrap();
        assert_eq!(d.rule_count(), 3001);
        for w in ["w1a", "w1500a", "w3000a", "dva", "dvaa", "dvai", "q"] {
            assert!(d.check(w), "{w}");
        }
        assert!(!d.check("qa") && !d.check("w1i") && d.check("w1"));
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(back.check("w2999a") && back.check("dvai") && !back.check("w7i"));

        // More than 128 one-character flags (#4966): 300 letters, each its own suffix.
        let letters: Vec<char> = (0x100u32..0x100 + 300).filter_map(char::from_u32).collect();
        let mut aff = String::from("SET UTF-8\nFLAG UTF-8\n");
        for c in &letters {
            aff.push_str(&format!("SFX {c} N 1\nSFX {c} 0 {c} .\n"));
        }
        let all: String = letters.iter().take(256).collect();
        let d = AffixDict::from_hunspell(&aff, &format!("2\nbase/{all}\nlast/{}\n", letters[299]), &[]).unwrap();
        assert!(d.check(&format!("base{}", letters[0])) && d.check(&format!("base{}", letters[255])));
        assert!(!d.check(&format!("base{}", letters[299])) && d.check(&format!("last{}", letters[299])));
    }

    #[test]
    fn flag_counts_are_capped() {
        let field = |n: usize| (1..=n).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let aff = "FLAG num\nSFX 1 Y 1\nSFX 1 0 a .\n";
        // At most MAX_FLAGS_PER_ENTRY flags per .dic entry: one more and the entry is skipped.
        let dic = format!("2\nfull/{}\nover/{}\n", field(MAX_FLAGS_PER_ENTRY), field(MAX_FLAGS_PER_ENTRY + 1));
        let d = AffixDict::from_hunspell(aff, &dic, &[]).unwrap();
        assert!(d.check("fulla") && !d.check("over") && !d.check("overa"));
        let cont = format!("FLAG num\nSFX 1 Y 1\nSFX 1 0 a/{} .\n", field(MAX_FLAGS_PER_ENTRY + 1));
        assert!(AffixDict::from_hunspell(&cont, "0\n", &[]).is_err(), "too many continuation classes");
        // More distinct flags than a u16 id holds (only one-character flags can get there): an
        // error, not a panic or a wrapped id.
        let chars: Vec<char> =
            (0x4e00u32..0xa000).chain(0xac00..0xd7a4).chain(0x20000..0x2a6e0).filter_map(char::from_u32).take(MAX_DISTINCT_FLAGS + 1).collect();
        assert_eq!(chars.len(), MAX_DISTINCT_FLAGS + 1);
        let mut dic = String::new();
        for (i, chunk) in chars.chunks(MAX_FLAGS_PER_ENTRY).enumerate() {
            dic.push_str(&format!("w{i}/{}\n", chunk.iter().collect::<String>()));
        }
        let err = AffixDict::from_hunspell("", &dic, &[]).unwrap_err();
        assert!(err.contains("distinct flags"), "{err}");
        // Homonyms are merged into one set. The per-entry cap is for .dic fields, not merged sets:
        // two entries adding up to more than MAX_FLAGS_PER_ENTRY are both kept (up to
        // MAX_FLAGS_PER_STEM), and the compact data reloads with the word and all its flags.
        let aff2 = format!("{aff}SFX {n} Y 1\nSFX {n} 0 b .\n", n = MAX_FLAGS_PER_ENTRY + 5);
        let first = field(MAX_FLAGS_PER_ENTRY);
        let second = (MAX_FLAGS_PER_ENTRY + 1..=MAX_FLAGS_PER_ENTRY + 10).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let dic = format!("2\nsame/{first}\nsame/{second}\n");
        let d = AffixDict::from_hunspell(&aff2, &dic, &[]).unwrap();
        assert!(d.check("same") && d.check("samea") && d.check("sameb"), "the second homonym's flags are merged");
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(back.check("same") && back.check("samea") && back.check("sameb"));
    }

    /// `n` homonym lines of `same`, each with MAX_FLAGS_PER_ENTRY distinct numeric flags (the
    /// first starting at 1), the last one ending with the flags `extra` (comma-separated).
    fn homonyms(n: usize, extra: &str) -> String {
        let mut dic = format!("{n}\n");
        for e in 0..n {
            let flags = e * MAX_FLAGS_PER_ENTRY + 1..=(e + 1) * MAX_FLAGS_PER_ENTRY;
            let mut flags: Vec<String> = flags.map(|i| i.to_string()).collect();
            if e + 1 == n && !extra.is_empty() {
                // Still exactly MAX_FLAGS_PER_ENTRY flags: the entry itself is within its cap.
                let extra: Vec<&str> = extra.split(',').collect();
                flags.truncate(MAX_FLAGS_PER_ENTRY - extra.len());
                flags.extend(extra.iter().map(ToString::to_string));
            }
            dic.push_str(&format!("same/{}\n", flags.join(",")));
        }
        dic
    }

    #[test]
    fn merged_homonyms_between_the_entry_and_stem_caps_round_trip() {
        // Eight full entries: exactly MAX_FLAGS_PER_STEM merged flags, the largest set build()
        // makes. The compact reader takes it back, word and flags.
        let n = MAX_FLAGS_PER_STEM / MAX_FLAGS_PER_ENTRY;
        let last = MAX_FLAGS_PER_STEM;
        let aff = format!("FLAG num\nSFX 1 Y 1\nSFX 1 0 a .\nSFX {last} Y 1\nSFX {last} 0 z .\n");
        let d = AffixDict::from_hunspell(&aff, &homonyms(n, ""), &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        for d in [&d, &back] {
            assert!(d.check("same") && d.check("samea") && d.check("samez"));
        }
        // One flag past the stem cap in compact data is refused, as an oversized .dic entry is.
        let flags = (1..=MAX_FLAGS_PER_STEM + 1).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let raw = format!("{MAGIC}\nFLAG num\nSFX 1 Y 0 a .\nWORDS\n\0same/{flags}\n\0kot\n");
        let d = AffixDict::from_bytes(&miniz_oxide::deflate::compress_to_vec(raw.as_bytes(), 6)).unwrap();
        assert!(!d.check("same") && d.check("kot"));
    }

    #[test]
    fn a_homonym_past_the_merged_cap_keeps_its_exclusions() {
        // Nine full homonyms: the ninth takes the merged set past MAX_FLAGS_PER_STEM, and it is
        // the only one with the exclusion flag. Its other flags are dropped; the exclusion isn't.
        let n = MAX_FLAGS_PER_STEM / MAX_FLAGS_PER_ENTRY + 1;
        let x = 60_000;
        let rules = "SFX 1 Y 1\nSFX 1 0 a .\n";
        let forbidden = AffixDict::from_hunspell(&format!("FLAG num\nFORBIDDENWORD {x}\n{rules}"), &homonyms(n, &x.to_string()), &[]).unwrap();
        let back = AffixDict::from_bytes(&forbidden.to_bytes().unwrap()).unwrap();
        for d in [&forbidden, &back] {
            assert!(!d.check("same") && !d.check("samea") && !d.contains("same"), "still forbidden");
            assert!(!d.contains_suggestible("same"));
        }
        let no_suggest = AffixDict::from_hunspell(&format!("FLAG num\nNOSUGGEST {x}\n{rules}"), &homonyms(n, &x.to_string()), &[]).unwrap();
        let back = AffixDict::from_bytes(&no_suggest.to_bytes().unwrap()).unwrap();
        for d in [&no_suggest, &back] {
            assert!(d.check("same") && d.check("samea"), "correct");
            assert!(!d.contains_suggestible("same") && !d.contains_suggestible("samea"), "but never suggested");
        }
        // Both exclusions on the overflowing homonym: both carried.
        let both = format!("FLAG num\nFORBIDDENWORD {x}\nNOSUGGEST {y}\n{rules}", y = x + 1);
        let d = AffixDict::from_hunspell(&both, &homonyms(n, &format!("{x},{}", x + 1)), &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        for d in [&d, &back] {
            assert!(!d.check("same") && !d.contains_suggestible("same"));
        }
        // A single entry over MAX_FLAGS_PER_ENTRY is not a word, but its exclusion still rules
        // its homonym out.
        let over = format!("{},{x}", (2..=MAX_FLAGS_PER_ENTRY + 1).map(|i| i.to_string()).collect::<Vec<_>>().join(","));
        let d = AffixDict::from_hunspell(&format!("FLAG num\nFORBIDDENWORD {x}\n{rules}"), &format!("2\nsame/1\nsame/{over}\n"), &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        for d in [&d, &back] {
            assert!(!d.check("same") && !d.check("samea"), "an oversized entry's FORBIDDENWORD applies");
        }
        // Alone, an oversized entry is still no word, exclusion or not.
        let d = AffixDict::from_hunspell(&format!("FLAG num\nNOSUGGEST {x}\n{rules}"), &format!("1\nsame/{over}\n"), &[]).unwrap();
        assert!(!d.check("same"));
    }

    #[test]
    fn an_unused_directive_flag_is_not_written_in_character_mode() {
        // FORBIDDENWORD names a flag no stem or rule uses: the compact data leaves it out, as it
        // did before flag ids, in one-character mode only.
        let d = AffixDict::from_hunspell("FORBIDDENWORD F\nNOSUGGEST N\nSFX a Y 1\nSFX a 0 y .\n", "1\nkot/a\n", &[]).unwrap();
        let raw = String::from_utf8(miniz_oxide::inflate::decompress_to_vec(&d.to_bytes().unwrap()).unwrap()).unwrap();
        assert!(!raw.contains("FORBIDDENWORD") && !raw.contains("NOSUGGEST"), "{raw}");
        // A used one is still written.
        let d = AffixDict::from_hunspell("FORBIDDENWORD F\nSFX a Y 1\nSFX a 0 y .\n", "2\nkot/a\nzly/F\n", &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(back.check("koty") && !back.check("zly"));
    }

    #[test]
    fn compact_data_without_a_flag_line_still_loads() {
        // Data written before the FLAG line existed: one-character flags, no FLAG line.
        let old = miniz_oxide::deflate::compress_to_vec(b"WCAFF1\n# old data\nFORBIDDENWORD F\nSFX a Y 0 y .\nWORDS\n\x00kot/a\n\x00zly/F\n", 6);
        let d = AffixDict::from_bytes(&old).unwrap();
        assert!(d.check("kot") && d.check("koty") && !d.check("zly"));
        // One-character dictionaries still write no FLAG line, so their data is unchanged.
        let raw = miniz_oxide::inflate::decompress_to_vec(&dict().to_bytes().unwrap()).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("\nFLAG"));
        // A FLAG line after a flag, or an unknown one, is bad data.
        for head in ["WCAFF1\nSFX a Y 0 y .\nFLAG num\n", "WCAFF1\nFLAG bogus\n"] {
            let bytes = miniz_oxide::deflate::compress_to_vec(format!("{head}WORDS\n\x00kot\n").as_bytes(), 6);
            assert!(AffixDict::from_bytes(&bytes).is_err(), "{head:?}");
        }
    }

    proptest::proptest! {
        #[test]
        fn lookups_never_panic(s in "\\PC{0,40}", bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..200)) {
            let d = dict();
            let _ = d.check(&s);
            let _ = d.forms(&s);
            let _ = AffixDict::from_bytes(&bytes);
            let _ = AffixDict::from_bytes(&miniz_oxide::deflate::compress_to_vec(&bytes, 1));
        }

        /// FULLSTRIP, NEEDAFFIX and CIRCUMFIX on hostile stems and words: no panic, and every
        /// form `forms()` lists is one the checker accepts.
        #[test]
        fn needaffix_circumfix_and_fullstrip_never_panic(stem in "[abx]{0,4}", word in "\\PC{0,12}") {
            let aff = "SET UTF-8\nFULLSTRIP\nNEEDAFFIX N\nCIRCUMFIX C\nPFX P Y 2\nPFX P a 0/C .\nPFX P 0 x/N a\nSFX S Y 3\nSFX S b 0/C b\nSFX S 0 x/NT .\nSFX S ab y .\nSFX T Y 1\nSFX T x 0 .\n";
            if let Ok(d) = AffixDict::from_hunspell(aff, &format!("2\n{stem}/PSN\n{stem}b/PS\n"), &[]) {
                let _ = d.check(&word);
                for s in [stem.clone(), format!("{stem}b")] {
                    for f in d.forms(&s) {
                        proptest::prop_assert!(!f.is_empty() && d.contains(&f), "{s} → {f:?}");
                    }
                }
            }
        }

        #[test]
        fn any_flag_field_reads_without_panicking(field in "[0-9,a-zA-Z!\u{fe0f}\u{1f6ab}]{0,24}", word in "\\PC{0,12}") {
            for mode in ["num", "long", "UTF-8"] {
                let aff = format!("FLAG {mode}
FORBIDDENWORD {field}
SFX {field} Y 1
SFX {field} 0 a/{field} .
");
                for aff in [aff.as_str(), &format!("FLAG {mode}
SFX 12 Y 1
SFX 12 0 a .
")] {
                    if let Ok(d) = AffixDict::from_hunspell(aff, &format!("2
w/{field}
x/12
"), &[]) {
                        let _ = (d.check(&word), d.check("wa"), d.forms("w"), d.forms("x"));
                        // Data the writer takes, the reader takes back.
                        if let Ok(bytes) = d.to_bytes() {
                            let back = AffixDict::from_bytes(&bytes);
                            proptest::prop_assert!(back.is_ok(), "{mode} {field:?}: {back:?}");
                        }
                    }
                }
            }
        }
    }
    /// `FULLSTRIP` lets an affix replace a whole stem; without it a stem must outlive its strip.
    #[test]
    fn fullstrip_allows_an_affix_to_replace_the_whole_stem() {
        let aff = "SET UTF-8\nSFX A Y 1\nSFX A ab x .\nPFX B Y 1\nPFX B ab y .\n";
        let dic = "1\nab/AB\n";
        let without = AffixDict::from_hunspell(aff, dic, &[]).unwrap();
        for w in ["ab", "x", "y"] {
            assert_eq!(without.contains(w), w == "ab", "{w} without FULLSTRIP");
        }
        let with = AffixDict::from_hunspell(&format!("FULLSTRIP\n{aff}"), dic, &[]).unwrap();
        for w in ["ab", "x", "y"] {
            assert!(with.contains(w), "{w} with FULLSTRIP");
        }
        assert!(with.forms("ab").iter().all(|f| with.contains(f)));
        assert!(!without.forms("ab").iter().any(|f| f == "x" || f == "y"));
        let back = AffixDict::from_bytes(&with.to_bytes().unwrap()).unwrap();
        assert!(back.contains("x") && back.contains("y"));
    }

    /// `NEEDAFFIX`: a stem or a suffix form that needs another affix is not a word without one.
    #[test]
    fn needaffix_stems_and_forms_need_one_more_affix() {
        let aff = "SET UTF-8\nNEEDAFFIX N\nSFX A Y 1\nSFX A 0 s .\nSFX B Y 1\nSFX B 0 x/AN .\nPFX P Y 1\nPFX P 0 re .\n";
        let d = AffixDict::from_hunspell(aff, "3\nfoo/NA\nbar/AB\nbaz/NP\n", &[]).unwrap();
        for w in ["foos", "bar", "bars", "barxs", "rebaz"] {
            assert!(d.contains(w), "{w}");
        }
        assert!(!d.contains("foo"), "a stem that needs an affix");
        assert!(!d.contains("baz"), "a stem that needs an affix");
        // `barx` carries NEEDAFFIX through its continuation class: it needs `s` after it.
        assert!(!d.contains("barx"), "a form that needs another affix");
        for stem in d.stems().map(str::to_string).collect::<Vec<_>>() {
            for f in d.forms(&stem) {
                assert!(d.contains(&f), "{stem} → {f}");
            }
        }
        assert!(!d.forms("foo").iter().any(|f| f == "foo") && d.forms("foo").iter().any(|f| f == "foos"));
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(!back.contains("foo") && back.contains("foos"));
    }

    /// `CIRCUMFIX`: a circumfix prefix and suffix only work as a pair.
    #[test]
    fn circumfix_prefix_and_suffix_only_work_together() {
        let aff = "SET UTF-8\nCIRCUMFIX c\nPFX P Y 1\nPFX P 0 ne/c .\nSFX S Y 1\nSFX S 0 y/c .\nSFX T Y 1\nSFX T 0 z .\n";
        let d = AffixDict::from_hunspell(aff, "1\nstrom/PST\n", &[]).unwrap();
        for w in ["strom", "stromz", "nestromy"] {
            assert!(d.contains(w), "{w}");
        }
        for w in ["stromy", "nestrom", "nestromz"] {
            assert!(!d.contains(w), "{w}");
        }
        for f in d.forms("strom") {
            assert!(d.contains(&f), "strom → {f}");
        }
        let mut forms = d.forms("strom");
        forms.sort();
        assert_eq!(forms, ["nestromy", "strom", "stromz"]);
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert!(back.contains("nestromy") && !back.contains("stromy") && !back.contains("nestrom"));
    }

    /// A chained suffix form is listed when the checker accepts it: also when its last suffix
    /// needs another affix or is a circumfix one (as hunspell 1.7.5 reads a chain), not when it
    /// would replace a whole stem without `FULLSTRIP`.
    #[test]
    fn chained_forms_follow_needaffix_circumfix_and_fullstrip() {
        let chain = |aff: &str, dic: &str, stem: &str| {
            let d = AffixDict::from_hunspell(aff, dic, &[]).unwrap();
            let mut forms = d.forms(stem);
            forms.sort();
            forms.dedup();
            for f in &forms {
                assert!(d.contains(f), "{stem} → {f}");
            }
            (d, forms)
        };
        let (d, forms) = chain("SET UTF-8\nNEEDAFFIX N\nSFX A Y 1\nSFX A 0 able/B .\nSFX B Y 1\nSFX B 0 s/N .\n", "1\nwork/A\n", "work");
        // The last suffix's NEEDAFFIX is met by the one under it (hunspell 1.7.5 takes it).
        assert_eq!(forms, ["work", "workable", "workables"]);
        assert!(d.contains("workables"), "its last suffix has another affix under it");
        let (d, forms) = chain("SET UTF-8\nCIRCUMFIX c\nSFX A Y 1\nSFX A 0 able/B .\nSFX B Y 1\nSFX B 0 s/c .\n", "1\nwork/A\n", "work");
        // Hunspell 1.7.5 ignores a chain's last suffix's CIRCUMFIX: `workables` is right.
        assert_eq!(forms, ["work", "workable", "workables"]);
        assert!(d.contains("workables"), "a chain's last suffix needs no prefix");
        let aff = "SFX A Y 1\nSFX A 0 y/B .\nSFX B Y 1\nSFX B xy z .\n";
        let (d, forms) = chain(&format!("SET UTF-8\n{aff}"), "1\nx/A\n", "x");
        assert_eq!(forms, ["x", "xy"]);
        assert!(!d.contains("z"), "a whole stem is replaced only with FULLSTRIP");
        let (d, forms) = chain(&format!("SET UTF-8\nFULLSTRIP\n{aff}"), "1\nx/A\n", "x");
        assert_eq!(forms, ["x", "xy", "z"]);
        assert!(d.contains("z"));
    }

    /// `forms()` never lists an empty form, which `FULLSTRIP` can produce when a suffix replaces
    /// the whole stem with nothing, at the first suffix or further along a chain.
    #[test]
    fn forms_never_list_an_empty_word() {
        let aff = "SET UTF-8\nFULLSTRIP\nSFX A Y 2\nSFX A x 0 .\nSFX A 0 y/B .\nSFX B Y 1\nSFX B xy 0 .\n";
        let d = AffixDict::from_hunspell(aff, "1\nx/A\n", &[]).unwrap();
        let mut forms = d.forms("x");
        forms.sort();
        assert_eq!(forms, ["x", "xy"]);
        assert!(!d.contains(""));
    }

    /// A circumfix suffix in the middle of a suffix chain still needs its prefix: `workable`
    /// carries the circumfix, so neither it nor `workables` is a word without `ne`.
    #[test]
    fn circumfix_suffix_inside_a_chain_still_needs_its_prefix() {
        let aff = "SET UTF-8\nCIRCUMFIX c\nPFX P Y 1\nPFX P 0 ne/c .\nSFX A Y 1\nSFX A 0 able/Bc .\nSFX B Y 1\nSFX B 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "1\nwork/AP\n", &[]).unwrap();
        let mut forms = d.forms("work");
        forms.sort();
        assert_eq!(forms, ["neworkable", "work"]);
        for f in &forms {
            assert!(d.contains(f), "{f}");
        }
        for w in ["workable", "workables", "nework"] {
            assert!(!d.contains(w), "{w}");
        }
    }

    /// An empty form (`FULLSTRIP` stripping a whole stem to nothing) is no word itself, but what
    /// a continuation class or a prefix adds to it is: `x` + `SFX A x 0/B`, `SFX B 0 y` makes `y`.
    #[test]
    fn an_empty_suffix_form_still_feeds_chains_and_prefixes() {
        let aff = "SET UTF-8\nFULLSTRIP\nSFX A Y 1\nSFX A x 0/B .\nSFX B Y 1\nSFX B 0 y .\n";
        let d = AffixDict::from_hunspell(aff, "1\nx/A\n", &[]).unwrap();
        let mut forms = d.forms("x");
        forms.sort();
        assert_eq!(forms, ["x", "y"]);
        assert!(d.contains("y") && !d.contains(""));
        let aff = "SET UTF-8\nFULLSTRIP\nSFX A Y 1\nSFX A x 0 .\nPFX P Y 1\nPFX P 0 q x\n";
        let d = AffixDict::from_hunspell(aff, "1\nx/AP\n", &[]).unwrap();
        let mut forms = d.forms("x");
        forms.sort();
        assert_eq!(forms, ["qx", "x"], "`q` over the empty form fails the condition `x`");
        assert!(d.contains("qx") && !d.contains("q"));
    }

    /// A prefix condition is checked on what the prefix is added to: the stem for the prefix
    /// alone, the suffix form in a cross product, as `contains()` does. Under the condition `ab`,
    /// `ac` + `-c+b` = `ab` takes `z`, and `ab` + `-b+c` = `ac` does not.
    #[test]
    fn prefix_conditions_apply_to_the_suffix_form_in_cross_products() {
        let aff = "SET UTF-8\nPFX P Y 1\nPFX P 0 z ab\nSFX A Y 2\nSFX A c b c\nSFX A b c b\n";
        let d = AffixDict::from_hunspell(aff, "2\nac/AP\nab/AP\n", &[]).unwrap();
        for stem in ["ac", "ab"] {
            for f in d.forms(stem) {
                assert!(d.contains(&f), "{stem} → {f}");
            }
        }
        assert!(d.forms("ac").iter().any(|f| f == "zab"), "the cross product meets the condition");
        assert!(!d.forms("ab").iter().any(|f| f == "zac") && !d.contains("zac"), "the cross product fails it");
    }
}
