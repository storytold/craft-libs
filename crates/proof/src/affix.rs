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
//! compound are stems, without affixes, as in Hunspell. Flag-based compounds (`COMPOUNDFLAG`,
//! `COMPOUNDBEGIN`/`MIDDLE`/`END`, `COMPOUNDPERMITFLAG`, `COMPOUNDFORBIDFLAG`, `COMPOUNDWORDMAX`,
//! the `CHECKCOMPOUND…` checks, `FORCEUCASE`…): see `compound`.
//! Morphological data (`AM` aliases, the fields after a stem) is skipped. Rejected: the other
//! directives that change which words are correct (`COMPLEXPREFIXES`, `COMPOUNDMORESUFFIXES`…).
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

mod build;
mod check;
mod compact;
mod compound;
mod flags;
mod forms;
mod parse;
#[cfg(test)]
mod tests;

use build::*;
pub(crate) use compound::{MAX_SUGGEST_COMPOUND_WORK, Pieces};
pub use flags::FlagMode;
pub(crate) use flags::SuggestSteps;
use flags::*;
use parse::*;

const MAGIC: &str = "WCAFF2";
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
/// Most flags all stems may carry together before their sets are merged and interned (the same
/// set on many stems counts each time; an `AF` alias counts once). Checked while reading, so a
/// huge `.dic` is refused before its flags fill memory, not after (#495: a 760 MB `.dic` once
/// held 3 GB before [`MAX_SET_DATA`] refused it).
const MAX_STEM_FLAGS: usize = 4 * MAX_SET_DATA;
/// Most bytes of flag text per flag of [`MAX_STEM_FLAGS`] the stem lines may hold in all while
/// they are read (`FLAG num`: `65535,` is six).
const FLAG_TEXT_PER_FLAG: usize = 6;

/// Most distinct flags a dictionary may use: each gets a `u16` id.
const MAX_DISTINCT_FLAGS: usize = 1 << 16;

/// Directives that change which words are correct in ways this engine doesn't implement.
const UNSUPPORTED: &[&str] = &["COMPLEXPREFIXES", "PSEUDOROOT"];

/// Most `COMPOUNDRULE` patterns kept, and most tokens in one: a hostile `.aff` can't make a
/// compound check cost more than a few thousand stem lookups per word.
const MAX_COMPOUND_RULES: usize = 64;
const MAX_COMPOUND_TOKENS: usize = 32;
/// Longest word (in bytes) tried as a compound: each try looks up every substring.
const MAX_COMPOUND_BYTES: usize = 48;

/// Steps one simple-word check takes at most, all prefixes and suffix chains together. A step is
/// about one rule tried: a rule, a character class of its condition, [`FLAGS_PER_STEP`] flags
/// of a set looked through, and [`GROUP_STEPS`] for each group of rules a split of the word leads to.
const MAX_SIMPLE_WORK: usize = 1 << 16;
/// Flags of a set looked through for one step.
const FLAGS_PER_STEP: usize = 16;
/// Steps a group of rules costs before its rules (the root it leaves is built and looked up).
const GROUP_STEPS: usize = 4;
/// Steps one simple-word check of a suggestion candidate takes at most.
const MAX_SUGGEST_SIMPLE_WORK: usize = 1 << 13;
/// Steps the simple-word checks of one suggestion call take in all (real packs' longest typos
/// need about 2^24.5; hostile rules stop here in under a second).
const MAX_SUGGEST_STEPS: usize = 1 << 25;
/// Most homonyms of one stem kept apart ([`AffixDict::homonyms`]); past it their flags count merged.
const MAX_HOMONYMS: usize = 16;
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
    /// The rules of `rules` whose flag some continuation class holds: the only ones that can be
    /// the outer suffix of two in a row.
    outer_rules: Vec<u32>,
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
    /// Flag-based compounding (`COMPOUNDFLAG`…), if the dictionary compounds by flags.
    cpd: Option<Box<compound::Compounding>>,
    /// Stems with `ONLYINCOMPOUND` homonyms beside others: stem index → those homonyms' flags
    /// (compound parts only), while `flags` holds the others'.
    cpd_sets: FxMap<u32, u32>,
    /// Those stems' sets with all their homonyms' flags, for `COMPOUNDRULE` parts.
    rule_sets: FxMap<u32, u32>,
    /// Stems listed more than once with different flags (not the `ONLYINCOMPOUND` homonyms
    /// beside others): stem index → each homonym's flags. `flags` holds them merged, which
    /// answers for one affix; a prefix with a suffix must come from one homonym (Dutch `stop/Pj…`
    /// takes `af`, `stop/Ca…` takes `je`: `afstopje` is wrong in the CLI).
    homonyms: FxMap<u32, Box<[u32]>>,
    /// Appended string → the prefix rules adding it, in rule order.
    pfx_index: FxMap<Box<str>, Vec<u32>>,
    /// Stems in mixed case (`kB`, `MSc`) by their lower case: a word in capitals may be one
    /// (`KB`, `KBS`), as in the CLI.
    mixed: FxMap<Box<str>, Vec<u32>>,
    /// Longest appended prefix, in chars.
    max_pfx_chars: usize,
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

/// `w` with its first letter in lower case.
fn decapitalize(w: &str) -> String {
    let mut c = w.chars();
    c.next().map(|f| f.to_lowercase().chain(c).collect()).unwrap_or_default()
}

/// Does the sorted flag set `set` hold `flag`?
fn has(set: &[u16], flag: u16) -> bool {
    set.binary_search(&flag).is_ok()
}

impl AffixDict {
    /// The flag set `id`, sorted (empty when unknown).
    fn set(&self, id: u32) -> &[u16] {
        let i = id as usize;
        match (self.set_offs.get(i), self.set_offs.get(i + 1)) {
            (Some(&a), Some(&b)) => self.set_data.get(a as usize..b as usize).unwrap_or_default(),
            _ => &[],
        }
    }

    /// Number of stems.
    pub fn len(&self) -> usize {
        self.flags.len()
    }

    /// Whether there are no stems.
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
}
