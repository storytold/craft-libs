//! Proofing for the Craft Apps (moved from WordCraft's `wordcraft-proof`), per language. A run's language (its BCP 47 tag) picks the tools through
//! [`ProofLang`]; the `*_in` functions take it, the older English-only ones remain.
//!
//! - **English** ([`ProofLang::En`]; also text with no language): spelling against the SCOWL
//!   2026 Hunspell dictionary of the run's English ([`scowl`]; else the public-domain Moby word
//!   list, ~160k words, the same list that drives hyphenation) plus our
//!   supplement of modern words it lacks (craft-i18n's `language-data/en-us/en-extra.txt`: `desktop`, `email`,
//!   `download`…), with inflections (`-s`, `-es`, `-ed`, `-ing`, `-ly`, `'s`…), a user dictionary and ignore lists;
//!   suggestions by edit distance. Grammar checks that are cheap and reliable: repeated words,
//!   `a`/`an`, capital letter after a sentence end, spaces before punctuation, doubled spaces.
//! - **Polish** ([`ProofLang::Pl`], module [`pl`]): spelling against the Hunspell `pl_PL`
//!   dictionary installed on the system ([`hunspell`]; not bundled, so without it Polish has no
//!   spelling marks), read by the affix engine ([`affix`]), suggestions tuned for Polish
//!   (diacritics, `ż`/`rz`, `u`/`ó`, missing spaces), and Polish grammar rules with Polish
//!   messages.
//! - **Bulgarian** ([`ProofLang::Bg`], module [`bg`]): hyphenation by the Bulgarian syllable
//!   rules; no spelling or grammar marks yet.
//! - **Portuguese** ([`ProofLang::Pt`], module [`pt`]): spelling against the Hunspell `pt_BR`
//!   dictionary installed on the system (VERO and friends; not bundled), suggestions tuned for
//!   Portuguese (missing accents, `seje`/`seja` confusions, missing spaces), conservative
//!   grammar rules with Portuguese messages, and `hyph-utf8`'s Portuguese patterns.
//! - **Italian** ([`ProofLang::It`], module [`it`]): spelling against the Hunspell `it_IT`
//!   dictionary installed on the system (not bundled — the upstream file is GPL-licensed),
//!   suggestions tuned for Italian (missing accents, `REP` confusions, missing spaces),
//!   conservative grammar rules with Italian messages, and `hyph-utf8`'s Italian patterns
//!   (Beccari, MIT option).
//! - **Russian** ([`ProofLang::Ru`], module [`ru`]): spelling against the Hunspell `ru_RU`
//!   dictionary installed on the system (not bundled either), with `ё` optional and hyphenated
//!   words (`кто-то`, `по-русски`, `северо-западный`) understood, suggestions tuned for Russian
//!   (`е`/`ё`, `жи`/`ши`, `тся`/`ться`, voiced and voiceless pairs, missing spaces), Russian
//!   grammar rules with Russian messages, and Russian word division.
//! - **Czech** ([`ProofLang::Cs`], module [`cs`]): the installed Hunspell `cs_CZ` dictionary
//!   (not bundled), dictionary-driven suggestions and generic checks with Czech messages.
//!   Hyphenation uses the existing English fallback; no Czech patterns are bundled.
//! - **Indonesian** ([`ProofLang::Id`], module [`id`]): the installed Hunspell `id_ID`
//!   dictionary (not bundled), dictionary-driven suggestions and generic checks with Indonesian
//!   messages. Hyphenation uses the existing English fallback.
//! - **Spanish** ([`ProofLang::Es`], [`ProofLang::EsMx`], [`ProofLang::Es419`], [`ProofLang::EsUy`],
//!   module [`es`]): each variety's installed Hunspell dictionary (`es_ES`, `es_MX`, `es_419`,
//!   `es_UY`, with fallbacks; not bundled), dictionary-driven suggestions, Uruguayan `vos` forms,
//!   and generic checks plus the opening `¿`/`¡` with Spanish messages. Hyphenation uses the
//!   existing English fallback.
//! - **German** ([`ProofLang::De`]): hyphenation by `hyph-utf8`'s German patterns (reformed
//!   spelling, MIT); no spelling or grammar marks yet.
//! - **Any other language** ([`ProofLang::Pack`], module [`packs`]): spelling and suggestions
//!   from its Hunspell dictionary (an installed Language pack, the dictionaries folder or the
//!   system's) with default tuning, hyphenation from its pack, no grammar rules; without a
//!   dictionary no marks, like Word without the language's proofing tools.
//! - **No language** ([`ProofLang::Other`]: `x-none`, `zxx`, malformed tags): no marks.
//! - **Hyphenation** ([`hyphen`]): dictionary, Liang patterns (English, Polish, Portuguese,
//!   Italian, German), Bulgarian syllable rules, Russian word-division rules, heuristic.
//!
//! [`check_text`] checks a paragraph whose runs are in different languages.
//!
//! **Language data** (the SCOWL English dictionaries, the Moby word list, the pattern files, our
//! word supplements) is embedded at build time from craft-i18n's `language-data/`
//! (`CRAFT_I18N_DIR`, see `build.rs`). English spelling never goes unchecked: a dictionaries-folder
//! English first, then the embedded SCOWL dictionary, then an English Hunspell dictionary
//! installed on the system, then the whole Moby word list built into the crate (see
//! [`english_source`]). Hyphenation that needs the data is off without it; grammar and the
//! rule-based hyphenation (Bulgarian, Turkish, Russian) are code and still work.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod affix;
mod affix_suggest;
/// The Language data embedded from craft-i18n by `build.rs` (each constant empty when it isn't).
mod data {
    include!(concat!(env!("OUT_DIR"), "/language_data.rs"));
}
pub mod bg;
pub mod cs;
pub mod dict;
pub mod en_variants;
mod english;
pub mod es;
mod grammar_en;
pub mod hr;
pub mod hunspell;
pub mod hyphen;
pub mod id;
pub mod it;
pub mod lang;
mod languages;
mod pack_lang;
pub mod packs;
pub mod patterns;
pub mod pl;
pub mod pt;
pub mod ru;
pub mod scowl;
mod slot;
mod spelling;
pub mod tr;
mod user_dict;
pub mod vi;

use std::collections::HashSet;
use std::ops::Range;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// What the spelling check skips (File › Options › Proofing). Word's defaults: all on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Options {
    /// Words in capitals (`NASA`, `WORDCRAFT`) are taken as correct.
    pub ignore_uppercase: bool,
    /// Words containing digits (`B2B`, `mp3`) are taken as correct.
    pub ignore_numbers: bool,
    /// Web and email addresses and file paths (`https://…`, `a@b.org`, `www.…`) are skipped.
    pub ignore_internet: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { ignore_uppercase: true, ignore_numbers: true, ignore_internet: true }
    }
}

pub use english::EnglishSource;
pub use grammar_en::check_grammar;
use grammar_en::grammar_en;
pub use lang::{PackLocale, ProofLang};
pub use languages::{dictionary_load_error, english_source, installed_dictionaries, pack_dictionary_file, set_dictionaries_folder};
pub(crate) use languages::{language, reload_dictionaries};
pub(crate) use user_dict::in_user_dictionary;
pub use user_dict::{
    MAX_CUSTOM_WORD_CHARS, add_word, custom_word, ignore_word, load_user_dictionary, remove_word, save_user_dictionary, user_dictionary,
};

/// A problem found in text: byte range, kind, message and suggestions.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    /// Byte offset in the checked text where the problem starts.
    pub start: usize,
    /// Byte offset just past the problem's last byte.
    pub end: usize,
    #[allow(missing_docs)]
    pub kind: IssueKind,
    /// In the language of the text it was found in (Polish rules explain in Polish).
    pub message: String,
    #[allow(missing_docs)]
    pub suggestions: Vec<String>,
    /// The language the text was checked in: spelling suggestions come from its dictionary
    /// ([`suggest_in`]).
    pub lang: ProofLang,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The kind of a problem an [`Issue`] reports.
pub enum IssueKind {
    /// A word not in the dictionary.
    Spelling,
    /// A grammar or spacing problem.
    Grammar,
}

/// Bumped whenever the user dictionary changes, so cached proofing results can be dropped.
static DICTIONARY_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Each run of two or more spaces in `text`, once, over the whole run (a fix replaces it with one
/// space). A run of three is one issue over three, not a pair and a leftover.
pub(crate) fn space_runs(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes.get(i) != Some(&b' ') {
            i += 1;
            continue;
        }
        let start = i;
        while bytes.get(i) == Some(&b' ') {
            i += 1;
        }
        if i - start >= 2 {
            runs.push(start..i);
        }
    }
    runs
}

/// A number that changes whenever a word is added to or removed from the user dictionary, the
/// dictionaries folder changes, or the Language packs do ([`packs::set_packs_dir`]).
pub fn dictionary_generation() -> u64 {
    DICTIONARY_GENERATION.load(Ordering::Relaxed)
}

/// Common words the source list lacks (modern vocabulary, contractions).
const COMMON: &str = "a an the and or but if of to in on at by for with from as is am are was were be been being it its we us our you your he him his she her they them their i me my mine this that these those there here what which who whom whose when where why how all any both each few more most other some such no nor not only own same so than too very can will just should now do does did done has have had having may might must shall would could up down out over under again further then once off into onto upon about above below between through during before after until while because per via vs etc ok yes yeah oh ah hi hey bye one two three four five six seven eight nine ten first last next new old get got go goes went gone see saw seen say said make made take took come came give gave know knew think thought look want need use used find found tell told ask asked feel felt try left keep let put mean set run show turn move live play pay hear hold bring begin seem help talk start might";

/// Is `w` (lower case) one of [`COMMON`]'s words?
pub(crate) fn is_common_word(w: &str) -> bool {
    COMMON.split(' ').any(|c| c == w)
}

/// Our supplement of common words the Moby list lacks (`desktop`, `email`, `download`…): one
/// lowercase base form per line, `#` comments. Inflections come from [`known_with`].
use data::EXTRA_EN;

/// The supplement's words: the Language data's copy, else the crate's own ([`english::EXTRA`]).
fn extra_words() -> &'static HashSet<&'static str> {
    static E: OnceLock<HashSet<&str>> = OnceLock::new();
    let extra = if EXTRA_EN.is_empty() { english::EXTRA } else { EXTRA_EN };
    E.get_or_init(|| extra.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect())
}

/// Is `w` (lower case) a word of the built-in Moby list or the supplement, with the inflection
/// rules ([`known_with`])?
pub(crate) fn known_in_word_list(w: &str) -> bool {
    known_with(w, &|s| english::word_list_contains(s) || extra_words().contains(s))
}

/// Is `w` (lower case) a word of the supplement alone, with the inflection rules (the SCOWL
/// dictionaries take it as an addition)?
pub(crate) fn known_in_supplement(w: &str) -> bool {
    known_with(w, &|s| extra_words().contains(s))
}

/// Is `w` (lower case) a word, by `base_known` (a base form in the word list), the common words,
/// the user dictionary and the inflection and prefix rules?
fn known_with(w: &str, base_known: &dyn Fn(&str) -> bool) -> bool {
    if base_known(w) || COMMON.split(' ').any(|c| c == w) {
        return true;
    }
    if in_user_dictionary(w) {
        return true;
    }
    // Inflections.
    let stems: [(&str, &[&str]); 13] = [
        ("'s", &[""]),
        ("s", &[""]),
        ("es", &["", "e"]),
        ("ies", &["y"]),
        ("ed", &["", "e"]),
        ("ied", &["y"]),
        ("ing", &["", "e"]),
        ("ly", &["", "le"]),
        ("ily", &["y"]),
        // specifically → specific (#1694): only -ic adjectives, so "desktopally" stays wrong.
        ("ically", &["ic"]),
        ("er", &["", "e"]),
        ("est", &["", "e"]),
        ("ness", &[""]),
    ];
    for (suf, adds) in stems {
        if let Some(stem) = w.strip_suffix(suf)
            && stem.len() >= 2
        {
            for a in adds {
                let s = format!("{stem}{a}");
                if base_known(&s) {
                    return true;
                }
            }
            // Doubled consonant: running → run, stopped → stop.
            let b = stem.as_bytes();
            if (suf == "ing" || suf == "ed" || suf == "er" || suf == "est")
                && b.len() >= 3
                && b.last() == b.get(b.len() - 2)
                && let Some(s) = stem.get(..stem.len() - 1)
                && base_known(s)
            {
                return true;
            }
        }
    }
    // Prefixes.
    for pre in ["un", "re", "non", "pre", "over", "under", "co", "mis"] {
        if let Some(rest) = w.strip_prefix(pre)
            && rest.len() >= 3
            && base_known(rest)
        {
            return true;
        }
    }
    false
}

/// Is there a proofing dictionary for this BCP 47 language tag? `None` (and empty tags) mean the
/// document default, which is en-US. Runs in a language without one are skipped (#100) instead
/// of flagging every word against another language's dictionary; see [`ProofLang::from_tag`].
pub fn has_dictionary(lang: Option<&str>) -> bool {
    ProofLang::from_tag(lang).is_proofed()
}

/// The Hunspell dictionary name (`pl_PL`, `it_IT`, `nl_NL`) a proofing language needs but doesn't
/// have installed, for a status-bar hint (#1167). `None` when the language's dictionary is
/// available, when it needs none (English has a built-in word list), or for a tag that is no
/// language (`x-none`, `zxx`). Any other language is named by its tag, even one no Hunspell
/// dictionary exists for (`ja`, `zh`, `th`): the caller decides whether to show the hint. Reads
/// the dictionary once (it is cached afterwards).
pub fn missing_dictionary(lang: Option<&str>) -> Option<&'static str> {
    let lang = ProofLang::from_tag(lang);
    dictionary_needed(lang, dictionary_loaded(lang)?)
}

/// The Hunspell name, `.dic` file and reason of a dictionary that is installed for this language
/// but couldn't be read (unsupported `SET` encoding, too large, a parse error). `None` when it
/// loaded, or when no file exists (then [`missing_dictionary`] names it). Reads the dictionary
/// once (it is cached afterwards).
pub fn unreadable_dictionary(lang: Option<&str>) -> Option<(&'static str, std::path::PathBuf, String)> {
    let lang = ProofLang::from_tag(lang);
    let name = dictionary_needed(lang, dictionary_loaded(lang)?)?;
    hunspell::load_error(name).map(|(file, reason)| (name, file, reason))
}

/// Did the Hunspell dictionary of `lang` load? `None` for languages that use none (English).
fn dictionary_loaded(lang: ProofLang) -> Option<bool> {
    if let Some(l) = lang.spelling_locale() {
        return Some(pack_lang::has_dictionary(l));
    }
    language(lang).map(|l| l.slot.has_dictionary())
}

/// The Hunspell name `lang` needs when `installed` is false (the first name its module looks for;
/// a Spanish variety's is its own: `es_ES`, `es_MX`, `es_419`, `es_UY`).
fn dictionary_needed(lang: ProofLang, installed: bool) -> Option<&'static str> {
    if installed {
        return None;
    }
    if let Some(l) = lang.spelling_locale() {
        return pack_lang::names(l).first().map(|n| interned(n));
    }
    language(lang).map(|l| l.slot.name())
}

/// `name` as a `&'static str` (the API hands out Hunspell names that way), for at most
/// [`MAX_INTERNED`] names; after that, [`UNINTERNED`], so the hint still shows.
fn interned(name: &str) -> &'static str {
    // ponytail: leaks each distinct pack dictionary name once, capped; a `Cow` API if more matter.
    static NAMES: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    interned_in(&mut NAMES.lock().unwrap_or_else(std::sync::PoisonError::into_inner), name)
}

/// [`interned`] with the names kept so far.
fn interned_in(names: &mut Vec<&'static str>, name: &str) -> &'static str {
    if let Some(n) = names.iter().find(|n| **n == name) {
        return n;
    }
    if names.len() >= MAX_INTERNED {
        return UNINTERNED;
    }
    let n: &'static str = Box::leak(name.to_string().into_boxed_str());
    names.push(n);
    n
}

/// Most pack dictionary names [`interned`] keeps.
const MAX_INTERNED: usize = 256;

/// The name [`interned`] gives once it keeps no more.
const UNINTERNED: &str = "a Hunspell dictionary";

/// Is `word` spelled correctly in the run's proofing language? A language without a dictionary
/// can't be checked, so every word counts as correct (no red underlines for it).
pub fn is_correct_for(word: &str, lang: Option<&str>) -> bool {
    is_correct_in(word, ProofLang::from_tag(lang))
}

/// Is `word` spelled correctly? Numbers, single letters, ALL-CAPS acronyms, URLs and words with
/// digits are accepted.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(word, &Options::default())
}

/// The words every language's check accepts under `opts`: single letters, words without
/// letters, words with digits (Ignore words that contain numbers), words in capitals
/// (Ignore words in UPPERCASE) and words in the user dictionary.
pub(crate) fn accepted_by_options(word: &str, opts: &Options) -> bool {
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').trim_matches('\'');
    w.chars().count() <= 1
        || !w.chars().any(char::is_alphabetic)
        || (opts.ignore_numbers && w.chars().any(|c| c.is_ascii_digit()))
        // Only a word with a cased letter is in capitals: a Hebrew or Thai word has none.
        || (opts.ignore_uppercase && spelling::in_capitals(w))
        // The whole word: the languages split a hyphenated word into its parts, and an added
        // `Unigranrio-RJ` is none of them (#2082).
        || in_user_dictionary(&w.to_lowercase())
}

/// Is the space at byte `space` of `text` the first of a score or ratio's `3 : 1`, which keeps
/// its spaces (the grammar rules' space-before-punctuation check)?
pub(crate) fn score_colon(text: &str, space: usize) -> bool {
    text.get(..space).and_then(|t| t.chars().next_back()).is_some_and(|c| c.is_ascii_digit())
        && text.get(space..).is_some_and(|t| t.starts_with(" : ") && t.get(3..).and_then(|r| r.chars().next()).is_some_and(|c| c.is_ascii_digit()))
}

/// [`is_correct`] with the File › Options › Proofing choices: English with no dialect, so a word
/// any English spells so is right (`color` and `colour`; American English's dictionary when it
/// isn't SCOWL: [`english_source`]), without the dialect policy.
pub fn is_correct_with(word: &str, opts: &Options) -> bool {
    english::is_correct(word, ProofLang::En, opts, false)
}

/// Spelling suggestions for a misspelled word (best first, at most `max`).
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    english::suggest(word, max, ProofLang::En)
}

/// `w` (lower case) in the case of `word`: all capitals, an initial capital, or as it is.
fn match_case(word: &str, w: String) -> String {
    let cap = word.chars().next().is_some_and(char::is_uppercase);
    let upper = word.chars().all(|c| !c.is_lowercase());
    if upper && word.chars().count() > 1 {
        w.to_uppercase()
    } else if cap {
        let mut c = w.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or(w)
    } else {
        w
    }
}

/// `lower` in the spelling `lang` (an English) wants, when it or a part of a hyphenated compound
/// is the other dialect's spelling (`colour-blind` → `color-blind` in American English).
fn english_preferred(lower: &str, lang: ProofLang) -> Option<String> {
    let mut changed = false;
    let parts: Vec<String> = lower
        .split('-')
        .map(|p| match en_variants::preferred(p, lang) {
            Some(n) => {
                changed = true;
                n
            }
            None => p.to_string(),
        })
        .collect();
    changed.then(|| parts.join("-"))
}

/// Is `word` spelled right in the English `lang` ([`ProofLang::En`], [`ProofLang::EnGb`],
/// [`ProofLang::EnCa`])? Its dictionary's words ([`english`]), less the other dialect's spellings
/// (`colour` in American English, `color` in British), plus this dialect's spellings of the
/// dictionary's words ([`en_variants`]). The user dictionary overrules both.
fn is_correct_english(word: &str, lang: ProofLang, opts: &Options) -> bool {
    english::is_correct(word, lang, opts, true)
}

/// Spelling suggestions in the English `lang`: this dialect's spelling first when `word` is the
/// other dialect's, then the shared suggestions in this dialect's spelling.
fn suggest_english(word: &str, max: usize, lang: ProofLang) -> Vec<String> {
    let lower = word.to_lowercase().replace('’', "'");
    let mut out: Vec<String> = Vec::new();
    if let Some(p) = english_preferred(&lower, lang) {
        out.push(match_case(word, p));
    }
    let shared = english::suggest(word, max.saturating_add(2), lang);
    for s in shared {
        let s = in_dialect(&s, word, lang);
        // `color's` and `color’s` are one suggestion (a dictionary's OCONV writes the second).
        let key = s.replace('’', "'");
        if !out.iter().any(|o| o.replace('’', "'") == key) {
            out.push(s);
        }
    }
    out.truncate(max);
    out
}

/// A suggestion `s` for `word` in `lang`'s spelling. Suggestions come as the dictionary's OCONV
/// writes them (`colour’s`); the dialect lists use `'`, so the mapping reads `'` and the result
/// keeps the dictionary's apostrophe (`color’s`).
fn in_dialect(s: &str, word: &str, lang: ProofLang) -> String {
    let curly = s.contains('’');
    match english_preferred(&s.to_lowercase().replace('’', "'"), lang) {
        Some(p) => {
            let p = match_case(word, p);
            if curly { p.replace('\'', "’") } else { p }
        }
        None => s.to_string(),
    }
}

/// Words in `text`: (byte start, byte end). Combining marks belong to the word they follow, so a
/// word typed decomposed (`tambie\u{301}n`, NFD) is one word, as its precomposed form is.
pub fn words(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let is_w = |c: char| c.is_alphanumeric() || c == '\'' || c == '’' || c == '-' || unicode_normalization::char::is_combining_mark(c);
    for (i, c) in text.char_indices() {
        match (start, is_w(c)) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    // Trim surrounding apostrophes/hyphens.
    out.into_iter()
        .filter_map(|(a, b)| {
            let w = text.get(a..b)?;
            let lead = w.len() - w.trim_start_matches(['\'', '’', '-']).len();
            let trail = w.len() - w.trim_end_matches(['\'', '’', '-']).len();
            (a + lead < b - trail).then_some((a + lead, b - trail))
        })
        .collect()
}

/// Spelling issues in a paragraph of English text (no suggestions; ask [`suggest`] on demand).
pub fn check_spelling(text: &str) -> Vec<Issue> {
    check_spelling_with(text, &Options::default())
}

/// [`check_spelling`] with the File › Options › Proofing choices.
pub fn check_spelling_with(text: &str, opts: &Options) -> Vec<Issue> {
    spelling_issues(text, |_| ProofLang::En, opts)
}

/// Spelling issues in a paragraph of text in `lang` (none for a language without a dictionary;
/// no suggestions: ask [`suggest_in`] on demand).
pub fn check_spelling_in(text: &str, lang: ProofLang) -> Vec<Issue> {
    if !lang.is_proofed() {
        return Vec::new();
    }
    spelling_issues(text, |_| lang, &Options::default())
}

/// Spelling issues of every word of `text`, each word spelled in `lang(word_start)`.
///
/// The web/e-mail classification of the whitespace-delimited token around a word is computed
/// once per token and shared by every word punctuation splits out of it. Classifying per word
/// rescanned the token each time, so a long whitespace-free run of punctuation-separated words
/// cost quadratic scanning (#1335).
fn spelling_issues(text: &str, lang: impl Fn(usize) -> ProofLang, opts: &Options) -> Vec<Issue> {
    let mut token = (0, 0, false);
    words(text)
        .into_iter()
        .filter_map(|(a, b)| {
            if a < token.0 || a >= token.1 {
                token = enclosing_token(text, a, b);
            }
            spelling_issue(text, a, b, lang(a), opts, token.2)
        })
        .collect()
}

/// The whitespace-delimited token enclosing the word at `a..b` of `text`: its byte range, and
/// whether it looks like a web or e-mail address.
fn enclosing_token(text: &str, a: usize, b: usize) -> (usize, usize, bool) {
    // The token starts after the last white-space character, whatever its width (a no-break
    // space is two bytes and an ideographic space is three).
    let ts = text.get(..a).and_then(|t| t.char_indices().rev().find(|(_, c)| c.is_whitespace())).map(|(i, c)| i + c.len_utf8()).unwrap_or(0);
    let te = text.get(b..).and_then(|t| t.find(char::is_whitespace)).map(|i| b + i).unwrap_or(text.len());
    (ts, te, text.get(ts..te).is_some_and(is_address))
}

/// Does a whitespace-delimited token look like a web or e-mail address (`https://…`, `www.…`,
/// `a@b.org`)? Spelling skips its words, and Spanish grammar its punctuation.
pub(crate) fn is_address(token: &str) -> bool {
    token.contains("://") || token.contains('@') || token.starts_with("www.")
}

/// The spelling mark's message in `lang`: a built-in language's own, a pack language's when we
/// have it (German, Bulgarian and other common ones), else English.
fn spelling_message(lang: ProofLang) -> &'static str {
    if let Some(l) = language(lang.built_in()) {
        return l.message;
    }
    let pack = lang.pack_locale();
    let primary = match lang {
        ProofLang::De => "de",
        ProofLang::Bg => "bg",
        _ => pack.as_ref().map_or("", PackLocale::primary),
    };
    match primary {
        "de" => "Möglicher Rechtschreibfehler",
        "bg" => "Възможна правописна грешка",
        "nl" => "Mogelijke spelfout",
        "fr" => "Faute d’orthographe possible",
        "sv" => "Möjligt stavfel",
        "da" => "Mulig stavefejl",
        "nb" | "nn" | "no" => "Mulig stavefeil",
        "fi" => "Mahdollinen kirjoitusvirhe",
        "uk" => "Можлива орфографічна помилка",
        "el" => "Πιθανό ορθογραφικό λάθος",
        "ca" => "Possible error ortogràfic",
        "ro" => "Posibilă greșeală de ortografie",
        "hu" => "Lehetséges helyesírási hiba",
        "sk" => "Možná pravopisná chyba",
        "sl" => "Možna pravopisna napaka",
        _ => "Possible spelling mistake",
    }
}

/// The spelling issue of the word at `a..b` of `text`, if it is misspelled in `lang`. Words in
/// web and e-mail addresses are skipped (unless `opts` says otherwise); `internet` is the
/// classification of the word's enclosing token, from [`enclosing_token`].
fn spelling_issue(text: &str, a: usize, b: usize, lang: ProofLang, opts: &Options, internet: bool) -> Option<Issue> {
    let w = text.get(a..b)?;
    // A word with a Vietnamese-only letter (a name like Nguyễn in English text) isn't English.
    if (opts.ignore_internet && internet) || (lang.is_english() && vi::is_vietnamese_word(w)) || is_correct_in_with(w, lang, opts) {
        return None;
    }
    let message = spelling_message(lang);
    Some(Issue { start: a, end: b, kind: IssueKind::Spelling, message: message.into(), suggestions: Vec::new(), lang })
}

/// Is `word` spelled correctly in `lang`? Always true in a language without a dictionary.
pub fn is_correct_in(word: &str, lang: ProofLang) -> bool {
    is_correct_in_with(word, lang, &Options::default())
}

/// `word` composed (NFC), as the dictionaries spell their words: `tambie\u{301}n` is looked up as
/// `también`. Words already composed (all ASCII ones among them) are borrowed as they are.
fn composed(word: &str) -> std::borrow::Cow<'_, str> {
    use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfc_quick};
    if word.is_ascii() || is_nfc_quick(word.chars()) == IsNormalized::Yes {
        std::borrow::Cow::Borrowed(word)
    } else {
        std::borrow::Cow::Owned(word.nfc().collect())
    }
}

/// [`is_correct_in`] with the File › Options › Proofing choices. A word with combining marks is
/// checked as its composed form (`composed`).
pub fn is_correct_in_with(word: &str, lang: ProofLang, opts: &Options) -> bool {
    let word = &*composed(word);
    if lang.is_english() {
        return is_correct_english(word, lang, opts);
    }
    if let Some(l) = lang.spelling_locale() {
        return accepted_by_options(word, opts) || pack_lang::is_correct(word, l, opts);
    }
    // A language WordCraft doesn't proof (Bulgarian, German, any other) has no spelling marks.
    language(lang).is_none_or(|l| accepted_by_options(word, opts) || (l.is_correct)(word, lang, opts))
}

/// Spelling suggestions for a word misspelled in `lang` (best first, at most `max`), composed
/// (`composed`) like the dictionaries' words.
pub fn suggest_in(word: &str, max: usize, lang: ProofLang) -> Vec<String> {
    let word = &*composed(word);
    if lang.is_english() {
        return suggest_english(word, max, lang);
    }
    if let Some(l) = lang.spelling_locale() {
        return pack_lang::suggest(word, max, l);
    }
    language(lang).map_or_else(Vec::new, |l| (l.suggest)(word, max, lang))
}

/// Grammar issues in a paragraph of text in `lang`: the English rules, the Polish ones
/// ([`pl::grammar`]), the Portuguese ones ([`pt::grammar`]), the Italian ones ([`it::grammar`]),
/// the Turkish ones ([`tr::grammar`]), Czech and Indonesian generic checks ([`cs::grammar`], [`id::grammar`]), the Spanish ones
/// ([`es::grammar`]), or none for a language WordCraft doesn't proof.
pub fn check_grammar_in(text: &str, lang: ProofLang) -> Vec<Issue> {
    grammar_in(text, lang, true)
}

/// [`check_grammar_in`] where `capital_first` says whether the first word must be capitalized
/// (false for a list item).
fn grammar_in(text: &str, lang: ProofLang, capital_first: bool) -> Vec<Issue> {
    match lang {
        ProofLang::En => grammar_en(text, capital_first),
        ProofLang::EnGb | ProofLang::EnAu | ProofLang::EnCa => grammar_en(text, capital_first).into_iter().map(|i| Issue { lang, ..i }).collect(),
        // `pt-pt` shares Portuguese's rules, its issues in its own language.
        _ => language(lang.built_in())
            .map_or_else(Vec::new, |l| (l.grammar)(text, lang, capital_first).into_iter().map(|i| Issue { lang, ..i }).collect()),
    }
}

/// Spelling and grammar issues in a paragraph whose text is in several languages, in order.
/// `spans` gives the language of byte ranges of `text` (in order, not overlapping); text outside
/// them is English. Each word is spelled in the language of its first letter. Each language's
/// grammar rules read the whole paragraph (so sentence boundaries are seen) but report only
/// inside that language's spans: an English quotation in a Polish paragraph gets English
/// spelling and is never told a Polish rule.
pub fn check_text(text: &str, spans: &[(Range<usize>, ProofLang)]) -> Vec<Issue> {
    check_text_with(text, spans, &Options::default(), true, false)
}

/// [`check_text`] with the File › Options › Proofing choices: what spelling skips, and whether
/// grammar is checked at all.
///
/// `list_item` marks a list paragraph: items are often fragments, so the English rule asking for
/// a capital on the paragraph's first word is off for it (later sentences are still checked).
pub fn check_text_with(text: &str, spans: &[(Range<usize>, ProofLang)], opts: &Options, grammar: bool, list_item: bool) -> Vec<Issue> {
    // A paragraph in many pack languages loads each dictionary once.
    pack_lang::in_one_check(|| check_text_once(text, spans, opts, grammar, list_item))
}

/// [`check_text_with`], inside one [`pack_lang::in_one_check`].
fn check_text_once(text: &str, spans: &[(Range<usize>, ProofLang)], opts: &Options, grammar: bool, list_item: bool) -> Vec<Issue> {
    // Sorted, empty ones dropped, neighbours of one language merged.
    let mut sorted: Vec<&(Range<usize>, ProofLang)> = spans.iter().filter(|(r, _)| r.start < r.end).collect();
    sorted.sort_by_key(|s| s.0.start);
    let mut merged: Vec<(Range<usize>, ProofLang)> = Vec::new();
    for (r, l) in sorted {
        match merged.last_mut() {
            Some((m, ml)) if *ml == *l && m.end == r.start => m.end = r.end,
            _ => merged.push((r.clone(), *l)),
        }
    }
    // A paragraph typed in Vietnamese under the default English language is Vietnamese, which
    // has no proofing tools: no marks (Word detects the language the same way).
    let default = if vi::looks_vietnamese(text) {
        for (_, l) in merged.iter_mut().filter(|(_, l)| l.is_english()) {
            *l = ProofLang::Other;
        }
        ProofLang::Other
    } else {
        ProofLang::En
    };
    let lang_at = |i: usize| -> ProofLang {
        let k = merged.partition_point(|(r, _)| r.end <= i);
        merged.get(k).filter(|(r, _)| r.start <= i).map_or(default, |s| s.1)
    };
    let mut v: Vec<Issue> = spelling_issues(text, lang_at, opts);
    if !grammar {
        v.sort_by_key(|i| (i.start, i.end));
        return v;
    }
    let mut langs: Vec<ProofLang> = merged.iter().map(|s| s.1).collect();
    let mut covered = 0;
    for (r, _) in &merged {
        if r.start > covered {
            break;
        }
        covered = covered.max(r.end);
    }
    if covered < text.len() || merged.is_empty() {
        langs.push(default);
    }
    langs.sort();
    langs.dedup();
    for lang in langs {
        let found = grammar_in(text, lang, !list_item);
        v.extend(found.into_iter().filter(|i| lang_at(i.start) == lang));
    }
    v.sort_by_key(|i| (i.start, i.end));
    v
}

/// Held by tests that change the dictionaries folder or rely on what it holds, so a folder change
/// can't land mid-test, and by those that save the whole user dictionary and load it back, which
/// would bring back a word another test just removed.
#[cfg(test)]
pub(crate) static DICTIONARY_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Hold [`DICTIONARY_TESTS`] until this test ends (libtest runs each test on its own thread, and
/// the guard drops with the thread); a second call on the thread does nothing. The tests that
/// change the dictionaries folder or the packs call it, and so does every in-memory test
/// dictionary's `install`, which a reload elsewhere would otherwise drop mid-test.
#[cfg(test)]
pub(crate) fn hold_dictionary_tests() {
    thread_local! {
        static HELD: std::cell::RefCell<Option<std::sync::MutexGuard<'static, ()>>> = const { std::cell::RefCell::new(None) };
    }
    HELD.with(|held| {
        let mut held = held.borrow_mut();
        if held.is_none() {
            *held = Some(DICTIONARY_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        }
    });
}

#[cfg(test)]
mod tests;
