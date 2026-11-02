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
pub mod hr;
pub mod hunspell;
pub mod hyphen;
pub mod id;
pub mod it;
pub mod lang;
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
pub mod vi;

use std::collections::HashSet;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};

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
pub use lang::{PackLocale, ProofLang};

/// A problem found in text: byte range, kind, message and suggestions.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub start: usize,
    pub end: usize,
    pub kind: IssueKind,
    /// In the language of the text it was found in (Polish rules explain in Polish).
    pub message: String,
    pub suggestions: Vec<String>,
    /// The language the text was checked in: spelling suggestions come from its dictionary
    /// ([`suggest_in`]).
    pub lang: ProofLang,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueKind {
    Spelling,
    Grammar,
}

fn user_words() -> &'static RwLock<HashSet<String>> {
    static U: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();
    U.get_or_init(|| RwLock::new(HashSet::new()))
}

/// Bumped whenever the user dictionary changes, so cached proofing results can be dropped.
static DICTIONARY_GENERATION: AtomicU64 = AtomicU64::new(0);

/// The key a word has in the user dictionary and the Ignore All list: lower case and composed
/// (NFC), as spelling looks words up ([`composed`]). A word added, ignored, removed or loaded
/// decomposed (`zqprue\u{301}ba`) is then the same entry as its composed form (`zqpruéba`), and
/// the other way round (#3562 review).
fn dictionary_key(w: &str) -> String {
    composed(&w.to_lowercase()).into_owned()
}

/// Add a word to the user dictionary ("Add to Dictionary"). It applies to every language.
pub fn add_word(w: &str) {
    if user_words().write().unwrap_or_else(std::sync::PoisonError::into_inner).insert(dictionary_key(w)) {
        DICTIONARY_GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Take a word out of the user dictionary (File › Options › Proofing). Returns whether it was there.
pub fn remove_word(w: &str) -> bool {
    let removed = user_words().write().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&dictionary_key(w));
    if removed {
        DICTIONARY_GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    removed
}

fn ignored_words() -> &'static RwLock<HashSet<String>> {
    static I: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();
    I.get_or_init(|| RwLock::new(HashSet::new()))
}

/// Ignore a word everywhere until the app closes ("Ignore All"). Unlike [`add_word`] it isn't
/// part of the user dictionary: it is never listed, saved or loaded, as in Word.
pub fn ignore_word(w: &str) {
    if ignored_words().write().unwrap_or_else(std::sync::PoisonError::into_inner).insert(dictionary_key(w)) {
        DICTIONARY_GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

/// A language proofed against an installed Hunspell dictionary: all the functions below need to
/// know about it, so a language joins with one row in [`LANGUAGES`].
struct Language {
    lang: ProofLang,
    slot: &'static dyn slot::Slot,
    /// The spelling mark's message, in the language.
    message: &'static str,
    is_correct: fn(&str, ProofLang, &Options) -> bool,
    suggest: fn(&str, usize, ProofLang) -> Vec<String>,
    /// Grammar issues; the flag says whether the first word must have a capital (not in a list
    /// item).
    grammar: fn(&str, ProofLang, bool) -> Vec<Issue>,
}

/// Every language proofed against a Hunspell dictionary, in the order File › Options › Proofing
/// lists them. English has a built-in word list and is handled apart.
static LANGUAGES: [Language; 12] = [
    Language {
        lang: ProofLang::Pl,
        slot: &pl::DICT,
        message: "Możliwy błąd pisowni",
        is_correct: |w, _, o| pl::is_correct_with_options(w, o),
        suggest: |w, n, _| pl::suggest(w, n),
        grammar: |t, _, c| pl::grammar::check_with(t, c),
    },
    Language {
        lang: ProofLang::Pt,
        slot: &pt::DICT,
        message: "Possível erro de ortografia",
        is_correct: |w, _, o| pt::is_correct_with_options(w, o),
        suggest: |w, n, _| pt::suggest(w, n),
        grammar: |t, _, c| pt::grammar::check_with(t, c),
    },
    Language {
        lang: ProofLang::It,
        slot: &it::DICT,
        message: "Possibile errore di ortografia",
        is_correct: |w, _, o| it::is_correct_with_options(w, o),
        suggest: |w, n, _| it::suggest(w, n),
        grammar: |t, _, c| it::grammar::check_with(t, c),
    },
    Language {
        lang: ProofLang::Hr,
        slot: &hr::DICT,
        message: "Moguća pravopisna pogreška",
        is_correct: |w, _, o| hr::is_correct_with_options(w, o),
        suggest: |w, n, _| hr::suggest(w, n),
        grammar: |t, _, c| hr::grammar::check_with(t, c),
    },
    Language {
        lang: ProofLang::Tr,
        slot: &tr::DICT,
        message: "Olası yazım hatası",
        is_correct: |w, _, o| tr::is_correct_with_options(w, o),
        suggest: |w, n, _| tr::suggest(w, n),
        grammar: |t, _, c| tr::grammar::check_with(t, c),
    },
    Language {
        lang: ProofLang::Ru,
        slot: &ru::DICT,
        message: "Возможна орфографическая ошибка",
        is_correct: |w, _, o| ru::is_correct_with_options(w, o),
        suggest: |w, n, _| ru::suggest(w, n),
        grammar: |t, _, c| ru::grammar::check_with(t, c),
    },
    // Czech, Indonesian and Spanish have no sentence-capital rule, so a list item needs no
    // exception.
    Language {
        lang: ProofLang::Cs,
        slot: &cs::DICT,
        message: "Možná pravopisná chyba",
        is_correct: |w, _, o| cs::is_correct_with_options(w, o),
        suggest: |w, n, _| cs::suggest(w, n),
        grammar: |t, _, _| cs::grammar::check(t),
    },
    Language {
        lang: ProofLang::Id,
        slot: &id::DICT,
        message: "Kemungkinan salah eja",
        is_correct: |w, _, o| id::is_correct_with_options(w, o),
        suggest: |w, n, _| id::suggest(w, n),
        grammar: |t, _, _| id::grammar::check(t),
    },
    Language {
        lang: ProofLang::Es,
        slot: &es::SPAIN_DICT,
        message: "Posible error ortográfico",
        is_correct: es::is_correct_with_options,
        suggest: es::suggest,
        grammar: |t, l, _| es::grammar::check(t, l),
    },
    Language {
        lang: ProofLang::EsMx,
        slot: &es::MEXICO_DICT,
        message: "Posible error ortográfico",
        is_correct: es::is_correct_with_options,
        suggest: es::suggest,
        grammar: |t, l, _| es::grammar::check(t, l),
    },
    Language {
        lang: ProofLang::Es419,
        slot: &es::LATIN_AMERICA_DICT,
        message: "Posible error ortográfico",
        is_correct: es::is_correct_with_options,
        suggest: es::suggest,
        grammar: |t, l, _| es::grammar::check(t, l),
    },
    Language {
        lang: ProofLang::EsUy,
        slot: &es::URUGUAY_DICT,
        message: "Posible error ortográfico",
        is_correct: es::is_correct_with_options,
        suggest: es::suggest,
        grammar: |t, l, _| es::grammar::check(t, l),
    },
];

/// `lang`'s row in [`LANGUAGES`]; `None` for English and the languages WordCraft doesn't proof.
fn language(lang: ProofLang) -> Option<&'static Language> {
    LANGUAGES.iter().find(|l| l.lang == lang)
}

/// Search `dir` for Hunspell dictionaries before the system locations (File › Options ›
/// Proofing › Dictionaries folder, #1063); `None` clears it. The dictionaries load again from
/// the new search path, and text is checked again.
pub fn set_dictionaries_folder(dir: Option<std::path::PathBuf>) {
    hunspell::set_dictionary_dir(dir);
    reload_dictionaries();
}

/// Forget every loaded dictionary, so the next check looks for them again (the dictionaries
/// folder or the installed Language packs changed), and check text again.
pub(crate) fn reload_dictionaries() {
    for l in &LANGUAGES {
        l.slot.reload();
    }
    english::reload();
    pack_lang::reload();
    DICTIONARY_GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// The `.dic` file a [`ProofLang::Pack`] language's (or German's or Bulgarian's) spelling is read from now (the dictionaries
/// folder, an installed Language pack or the system's); `None` when it has none (unchecked) or
/// `lang` isn't a pack language. Reads the dictionary once (it is cached afterwards).
pub fn pack_dictionary_file(lang: ProofLang) -> Option<std::path::PathBuf> {
    pack_lang::dictionary_file(lang.spelling_locale()?)
}

/// Each language that reads an installed Hunspell dictionary, with the `.dic` file found for it
/// now (`None`: not installed, so the language has no spelling marks).
pub fn installed_dictionaries() -> Vec<(ProofLang, Option<std::path::PathBuf>)> {
    LANGUAGES.iter().map(|l| (l.lang, l.slot.installed_path())).collect()
}

/// Why the dictionary found for `lang` failed to load (unsupported encoding, too large, a parse
/// error), if it was read and failed. Doesn't read anything itself: a dictionary nobody has
/// checked yet reports `None` (#2830). For an English, its embedded SCOWL dictionary or the
/// English Hunspell dictionary that failed; English then uses the next source ([`english_source`]).
pub fn dictionary_load_error(lang: ProofLang) -> Option<String> {
    if matches!(lang, ProofLang::En | ProofLang::EnGb | ProofLang::EnCa) {
        return english::load_error(lang);
    }
    if let Some(l) = lang.spelling_locale() {
        return pack_lang::load_error(l);
    }
    hunspell::load_error(language(lang)?.slot.name()).map(|(_, reason)| reason)
}

/// Where the spelling of an English `lang` comes from now (`None` for other languages), in this
/// order: a Hunspell dictionary of its own (`en_US`, `en_GB`, `en_CA`) in the dictionaries
/// folder, the SCOWL dictionary embedded at build time, one of its own on the system, else the
/// Moby word list built into the crate. Another English's dictionary is never used. Reads the
/// Hunspell dictionary once (it is cached afterwards).
pub fn english_source(lang: ProofLang) -> Option<EnglishSource> {
    lang.is_english().then(|| english::source(lang))
}

/// A number that changes whenever a word is added to or removed from the user dictionary, the
/// dictionaries folder changes, or the Language packs do ([`packs::set_packs_dir`]).
pub fn dictionary_generation() -> u64 {
    DICTIONARY_GENERATION.load(Ordering::Relaxed)
}

/// Is `lower` (a lower-case word) in the user dictionary or ignored? It is looked up composed, as
/// the entries are kept ([`dictionary_key`]).
pub(crate) fn in_user_dictionary(lower: &str) -> bool {
    let key = composed(lower);
    user_words().read().unwrap_or_else(std::sync::PoisonError::into_inner).contains(&*key)
        || ignored_words().read().unwrap_or_else(std::sync::PoisonError::into_inner).contains(&*key)
}

/// Words in the user dictionary.
pub fn user_dictionary() -> Vec<String> {
    let mut v: Vec<String> = user_words().read().unwrap_or_else(std::sync::PoisonError::into_inner).iter().cloned().collect();
    v.sort();
    v
}

/// Longest custom-dictionary word kept, in characters (mirrors `file.options` validation).
pub const MAX_CUSTOM_WORD_CHARS: usize = 64;
/// Most custom-dictionary words read from one file (caps a hostile file).
const MAX_CUSTOM_WORDS: usize = 100_000;
/// Largest custom-dictionary file read, in bytes (caps a hostile file).
const MAX_CUSTOM_FILE_BYTES: u64 = 1_000_000;

/// `w` as a custom-dictionary word: trimmed and lower-cased, or `None` when it isn't a single
/// word of at most [`MAX_CUSTOM_WORD_CHARS`] characters (empty, spaces, control characters, or
/// bytes that weren't UTF-8).
/// Adding and loading both use it, so a word that can be added is never dropped on the next load.
pub fn custom_word(w: &str) -> Option<String> {
    let w = w.trim();
    if w.is_empty()
        || w.chars().count() > MAX_CUSTOM_WORD_CHARS
        || w.chars().any(|c| c.is_whitespace() || c.is_control() || c == char::REPLACEMENT_CHARACTER)
    {
        return None;
    }
    Some(dictionary_key(w))
}

/// The valid words of the custom-dictionary file at `path`, reading at most
/// [`MAX_CUSTOM_FILE_BYTES`] of it and keeping at most [`MAX_CUSTOM_WORDS`]. A missing or
/// unreadable file reads as no words.
fn read_dictionary_file(path: &std::path::Path) -> Vec<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    let Ok(file) = std::fs::File::open(path) else { return Vec::new() };
    // `take` bounds the read itself, so a huge file is never read whole.
    if file.take(MAX_CUSTOM_FILE_BYTES).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    // Lossy: a cut multi-byte tail becomes U+FFFD and is skipped by validation below,
    // instead of discarding the whole file.
    String::from_utf8_lossy(&bytes).lines().filter_map(custom_word).take(MAX_CUSTOM_WORDS).collect()
}

/// Read custom-dictionary words from a file into the user dictionary (one word per line;
/// blank lines are skipped). A missing file reads as empty; a hostile file is capped,
/// never a panic and never an error that stops startup. Returns the words added.
pub fn load_user_dictionary(path: &std::path::Path) -> Vec<String> {
    let words = read_dictionary_file(path);
    let mut set = user_words().write().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut added = Vec::new();
    for w in words {
        if set.len() >= MAX_CUSTOM_WORDS {
            break;
        }
        if set.insert(w.clone()) {
            added.push(w);
        }
    }
    if !added.is_empty() {
        DICTIONARY_GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    added
}

/// Write the user dictionary to a file (one word per line, sorted), creating its folder.
///
/// Words another process (a second window, the MCP bridge) saved to the file since it was loaded
/// are merged in first, so neither loses the other's words; `removed` (words just taken out) stay
/// out. The file is replaced atomically: written beside it, then renamed over it. Errors (a
/// read-only folder, a full disk) go back to the caller; the in-memory words are kept.
pub fn save_user_dictionary(path: &std::path::Path, removed: &[String]) -> std::io::Result<()> {
    let removed: HashSet<String> = removed.iter().map(|w| dictionary_key(w)).collect();
    let on_disk = read_dictionary_file(path);
    {
        let mut set = user_words().write().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut changed = false;
        for w in on_disk {
            if set.len() >= MAX_CUSTOM_WORDS {
                break;
            }
            if !removed.contains(&w) && set.insert(w) {
                changed = true;
            }
        }
        for w in &removed {
            changed |= set.remove(w);
        }
        if changed {
            DICTIONARY_GENERATION.fetch_add(1, Ordering::Relaxed);
        }
    }
    let mut out = String::new();
    for w in user_dictionary() {
        out.push_str(&w);
        out.push('\n');
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "custom-dictionary.txt".into());
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let written = std::fs::write(&tmp, out).and_then(|()| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
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
        || (opts.ignore_uppercase && w.chars().any(char::is_uppercase) && w.chars().all(|c| !c.is_lowercase()))
        // The whole word: the languages split a hyphenated word into its parts, and an added
        // `Unigranrio-RJ` is none of them (#2082).
        || in_user_dictionary(&w.to_lowercase())
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
/// checked as its composed form ([`composed`]).
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
/// ([`composed`]) like the dictionaries' words.
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
        ProofLang::EnGb | ProofLang::EnCa => grammar_en(text, capital_first).into_iter().map(|i| Issue { lang, ..i }).collect(),
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

/// Grammar issues in a paragraph of English text.
pub fn check_grammar(text: &str) -> Vec<Issue> {
    grammar_en(text, true)
}

/// The English rules; `capital_first` is whether the text's first word must start a sentence
/// with a capital (false for a list item, which may be a fragment).
fn grammar_en(text: &str, capital_first: bool) -> Vec<Issue> {
    let mut v = Vec::new();
    let ws = words(text);
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if gap.trim().is_empty()
            && w0.eq_ignore_ascii_case(w1)
            && w0.chars().all(char::is_alphabetic)
            && !matches!(w0.to_lowercase().as_str(), "had" | "that" | "bye" | "so")
        {
            v.push(Issue {
                start: *a0,
                end: *b1,
                kind: IssueKind::Grammar,
                message: format!("Repeated word: \"{w1}\""),
                suggestions: vec![w0.to_string()],
                lang: ProofLang::En,
            });
        }
        if gap == " " && (w0 == "a" || w0 == "A") && w1.chars().next().is_some_and(|c| "aeiouAEIOU".contains(c)) && !starts_consonant_sound(w1) {
            v.push(Issue {
                start: *a0,
                end: *b0,
                kind: IssueKind::Grammar,
                message: "Use \"an\" before a vowel sound".into(),
                suggestions: vec![if w0 == "A" { "An" } else { "an" }.into()],
                lang: ProofLang::En,
            });
        }
        if gap == " "
            && (w0 == "an" || w0 == "An")
            && w1.chars().next().is_some_and(|c| c.is_alphabetic() && !"aeiouAEIOUhH".contains(c))
            && !w1.chars().all(char::is_uppercase)
        {
            v.push(Issue {
                start: *a0,
                end: *b0,
                kind: IssueKind::Grammar,
                message: "Use \"a\" before a consonant sound".into(),
                suggestions: vec![if w0 == "An" { "A" } else { "a" }.into()],
                lang: ProofLang::En,
            });
        }
    }
    // Space before punctuation; doubled spaces.
    for (i, _) in text.match_indices(" ,").chain(text.match_indices(" .")).chain(text.match_indices(" ;")) {
        if text.get(i + 2..).is_some_and(|r| r.starts_with('.') || r.starts_with(',')) {
            continue; // "..." or numbers like " .5"
        }
        v.push(Issue {
            start: i,
            end: i + 2,
            kind: IssueKind::Grammar,
            message: "Remove the space before the punctuation".into(),
            suggestions: vec![text.get(i + 1..i + 2).unwrap_or("").to_string()],
            lang: ProofLang::En,
        });
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(Issue {
            start: i,
            end: i + 2,
            kind: IssueKind::Grammar,
            message: "Extra space".into(),
            suggestions: vec![" ".into()],
            lang: ProofLang::En,
        });
    }
    // Sentence start capital.
    let mut after_end = capital_first;
    for (a, b) in &ws {
        let Some(w) = text.get(*a..*b) else { continue };
        if after_end && w.chars().next().is_some_and(char::is_lowercase) && w.chars().all(char::is_alphabetic) {
            let fixed: String = w.chars().next().map(|f| f.to_uppercase().chain(w.chars().skip(1)).collect()).unwrap_or_default();
            v.push(Issue {
                start: *a,
                end: *b,
                kind: IssueKind::Grammar,
                message: "Capitalize the first word of a sentence".into(),
                suggestions: vec![fixed],
                lang: ProofLang::En,
            });
        }
        let tail = text.get(*b..).unwrap_or("");
        let punctuation = tail.trim_start_matches(['"', '”', ')', '\'']);
        // Two or more dots are an ellipsis, not a full stop ending a sentence.
        // A Unicode ellipsis (U+2026) does not match sentence-ending punctuation either.
        after_end = matches!(punctuation.chars().next(), Some('.' | '!' | '?'))
            && !punctuation.starts_with("..")
            && punctuation.trim_start_matches(['.', '!', '?', '"', '”', ')']).starts_with(' ');
    }
    v.sort_by_key(|i| i.start);
    v
}

fn starts_consonant_sound(w: &str) -> bool {
    let l = w.to_lowercase();
    ["uni", "use", "usu", "eu", "one", "once", "ubiq", "uti", "ure", "uro"].iter().any(|p| l.starts_with(p))
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

/// The no-Language-data build (`build.rs` sets `craft_proof_no_data` when nothing is embedded).
#[cfg(all(test, craft_proof_no_data))]
mod no_data;

#[cfg(test)]
mod tests {
    #[test]
    fn missing_dictionary_is_named_only_for_hunspell_languages_without_one() {
        assert_eq!(dictionary_needed(ProofLang::Pl, false), Some("pl_PL"));
        assert_eq!(dictionary_needed(ProofLang::Pt, false), Some("pt_BR"));
        assert_eq!(dictionary_needed(ProofLang::Pl, true), None);
        assert_eq!(dictionary_needed(ProofLang::Pt, true), None);
        // Every Hunspell language is named by the first name its module looks for (#3150).
        for (l, name) in [
            (ProofLang::It, "it_IT"),
            (ProofLang::Hr, "hr_HR"),
            (ProofLang::Tr, "tr_TR"),
            (ProofLang::Ru, "ru_RU"),
            (ProofLang::Cs, "cs_CZ"),
            (ProofLang::Id, "id_ID"),
            // #3360: each Spanish variety names its own.
            (ProofLang::Es, "es_ES"),
            (ProofLang::EsMx, "es_MX"),
            (ProofLang::Es419, "es_419"),
            (ProofLang::EsUy, "es_UY"),
        ] {
            assert_eq!(dictionary_needed(l, false), Some(name), "{l:?}");
            assert_eq!(dictionary_needed(l, true), None, "{l:?}");
        }
        // Every proofed language other than English has a Hunspell dictionary to report.
        for l in ProofLang::PROOFED {
            let english = matches!(l, ProofLang::En | ProofLang::EnGb | ProofLang::EnCa);
            assert_eq!(dictionary_needed(l, false).is_some(), !english, "{l:?}");
        }
        for l in [ProofLang::En, ProofLang::Other] {
            assert_eq!(dictionary_needed(l, false), None);
        }
        // German and Bulgarian spell from a pack or Hunspell dictionary, and name it as the others do.
        assert_eq!((dictionary_needed(ProofLang::De, false), dictionary_needed(ProofLang::Bg, false)), (Some("de_DE"), Some("bg_BG")));
        // Installed dictionaries and the built-in English list: no hint.
        pl::test_dict::install();
        pt::test_dict::install();
        es::test_dict::install();
        for tag in [Some("pl-PL"), Some("pt-BR"), Some("es-ES"), Some("es-MX"), Some("es-419"), Some("es-UY"), Some("en-US"), None] {
            assert_eq!(missing_dictionary(tag), None, "{tag:?}");
        }
        // A pack language names its dictionary too, unless one is installed.
        assert!(missing_dictionary(Some("fr-FR")).is_none_or(|n| n == "fr_FR"));
    }

    use super::*;

    /// craft-libs#489 review: a suggestion an OCONV table wrote with `’` is still mapped to this
    /// dialect's spelling (`colour’s` → `color’s` in American English), and keeps its apostrophe.
    #[test]
    fn curly_suggestions_are_mapped_to_the_dialect() {
        if english_preferred("colour's", ProofLang::En).is_none() {
            return; // no dialect lists without CRAFT_I18N_DIR
        }
        assert_eq!(in_dialect("colour’s", "colours", ProofLang::En), "color’s");
        assert_eq!(in_dialect("colour's", "colours", ProofLang::En), "color's");
        assert_eq!(in_dialect("colour’s", "colours", ProofLang::EnGb), "colour’s");
    }

    /// The non-English test dictionaries every dictionaries-folder test puts in its folder, so
    /// tests running meanwhile see the same words whichever of those folders is set.
    fn write_test_dictionaries(dir: &std::path::Path) {
        std::fs::create_dir_all(dir).unwrap();
        for (name, aff, dic) in [
            ("pl_PL", pl::test_dict::AFF, pl::test_dict::DIC),
            ("pt_BR", pt::test_dict::AFF, pt::test_dict::DIC),
            ("it_IT", it::test_dict::AFF, it::test_dict::DIC),
            ("hr_HR", hr::test_dict::AFF, hr::test_dict::DIC),
            ("tr_TR", tr::test_dict::AFF, tr::test_dict::DIC),
            ("ru_RU", ru::test_dict::AFF, ru::test_dict::DIC),
            ("cs_CZ", cs::test_dict::AFF, cs::test_dict::DIC),
            ("id_ID", id::test_dict::AFF, id::test_dict::DIC),
            ("es_ES", es::test_dict::AFF, es::test_dict::DIC),
            ("es_MX", es::test_dict::AFF, es::test_dict::DIC_AMERICAS),
            ("es_419", es::test_dict::AFF, es::test_dict::DIC_AMERICAS),
            ("es_UY", es::test_dict::AFF, es::test_dict::DIC_AMERICAS),
        ] {
            std::fs::write(dir.join(format!("{name}.aff")), aff).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), dic).unwrap();
        }
    }

    /// Set `dir` (with [`write_test_dictionaries`] and `english`, `(name, .aff, .dic)` files) as
    /// the dictionaries folder; the English files are removed again by [`drop_english_files`].
    fn folder_with_english(dir: &std::path::Path, english: &[(&str, &[u8], &[u8])]) {
        write_test_dictionaries(dir);
        for (name, aff, dic) in english {
            std::fs::write(dir.join(format!("{name}.aff")), aff).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), dic).unwrap();
        }
        set_dictionaries_folder(Some(dir.to_path_buf()));
    }

    /// Remove the English files of `dir` and set it again: English goes back to what it was
    /// (the built-in list, or a dictionary installed on the system), for the tests that follow.
    fn drop_english_files(dir: &std::path::Path, english: &[(&str, &[u8], &[u8])]) {
        for (name, _, _) in english {
            let _ = std::fs::remove_file(dir.join(format!("{name}.aff")));
            let _ = std::fs::remove_file(dir.join(format!("{name}.dic")));
        }
        set_dictionaries_folder(Some(dir.to_path_buf()));
    }

    /// Language-data builds embed SCOWL: an English dictionary in the dictionaries folder would
    /// then come before it for every English test running meanwhile, so the folder tests below
    /// run without the data only; `english::tests` checks the order with the data.
    fn scowl_embedded() -> bool {
        scowl::dict(ProofLang::En).is_some()
    }

    /// R6.4, English never goes unchecked: an English Hunspell dictionary in the dictionaries
    /// folder is used before the word list built into the crate.
    #[test]
    fn english_hunspell_dictionary_is_used_before_the_built_in_list() {
        if scowl_embedded() {
            return;
        }
        crate::hold_dictionary_tests();
        let dir = std::env::temp_dir().join(format!("wc-proof-en-hunspell-{}", std::process::id()));
        // A tiny en_US. `hous` and `because` aren't words in it, so a marked `because` is this
        // dictionary answering (the built-in list knows it).
        let english: &[(&str, &[u8], &[u8])] =
            &[("en_US", b"SET UTF-8\nTRY esiarntolcdugmphbyfvkwz\nSFX S Y 1\nSFX S 0 s .\n", b"3\nteh\nthe\nhouse/S\n")];
        folder_with_english(&dir, english);
        {
            let dic = dir.join("en_US.dic");
            assert_eq!(english_source(ProofLang::En), Some(EnglishSource::Hunspell(dic.clone())));
            for w in ["teh", "Teh", "the", "house", "houses", "House"] {
                assert!(is_correct_in(w, ProofLang::En), "{w}");
            }
            for w in ["hous", "because", "housez"] {
                assert!(!is_correct_in(w, ProofLang::En), "{w}: the dictionary's words, not the built-in list's");
            }
            assert!(!is_correct("hous") && is_correct("teh"), "the older English-only functions too");
            assert_eq!(check_spelling_in("teh hous", ProofLang::En).len(), 1);
            assert!(suggest_in("hous", 3, ProofLang::En).contains(&"house".to_string()), "{:?}", suggest_in("hous", 3, ProofLang::En));
            // No en_GB or en_CA file: never the en_US one, which would mark their spellings
            // (craft-libs#482 re-review), but their own installed elsewhere or the built-in list.
            for lang in [ProofLang::EnGb, ProofLang::EnCa] {
                assert_ne!(english_source(lang), Some(EnglishSource::Hunspell(dic.clone())), "{lang:?}");
                if english_source(lang) == Some(EnglishSource::CoreList) {
                    for w in ["colour", "organise", "favourite", "centre", "travelled", "because"] {
                        assert!(is_correct_in(w, lang), "{w} {lang:?}");
                    }
                    assert!(!is_correct_in("becuase", lang), "{lang:?}");
                }
            }
        }
        // Text in a language without proofing is never checked with English.
        assert!(check_spelling_in("hous teh zxqv", ProofLang::Other).is_empty());
        assert!(check_text("hous teh", &[(0..8, ProofLang::De)]).is_empty());
        assert_eq!(english_source(ProofLang::Pl), None);
        drop_english_files(&dir, english);
        assert_ne!(english_source(ProofLang::En), Some(EnglishSource::Hunspell(dir.join("en_US.dic"))));
    }

    /// A document tagging thousands of languages keeps the remembered pack hyphenation lookups
    /// bounded, and stays quick.
    #[test]
    fn many_pack_languages_keep_the_hyphenation_cache_bounded() {
        crate::hold_dictionary_tests();
        set_dictionaries_folder(Some(shared_test_folder()));
        let root = std::env::temp_dir().join(format!("wc-proof-many-tags-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        packs::set_packs_dir(Some(root.clone()));
        let start = std::time::Instant::now();
        for i in 0..3000 {
            let lang = ProofLang::from_tag(Some(&format!(
                "q{}{}-{}",
                (b'a' + (i / 1000) as u8) as char,
                (b'a' + (i / 100 % 10) as u8) as char,
                100 + i % 100
            )));
            assert!(lang.pack_locale().is_some(), "{lang:?}");
            let _ = hyphen::hyphenate_word_in("hyphenation", &hyphen::Limits::default(), lang);
            assert!(packs::hyphenation_lookups() <= 256);
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(5), "{:?}", start.elapsed());
        packs::set_packs_dir(None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// CP4.2: with no embedded word list, English looks in the dictionaries folder, then an
    /// installed English Language pack, then the system's Hunspell, then the built-in list.
    #[test]
    fn an_english_pack_comes_after_the_dictionaries_folder() {
        // With SCOWL embedded, English reads only the dictionaries folder before it.
        if scowl_embedded() {
            return;
        }
        crate::hold_dictionary_tests();
        set_dictionaries_folder(Some(shared_test_folder()));
        let root = std::env::temp_dir().join(format!("wc-proof-en-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (dir, packs) = (root.join("dictionaries"), root.join("language-packs"));
        let aff: &[u8] = b"SET UTF-8\nSFX S Y 1\nSFX S 0 s .\n";
        let pack = write_pack(&packs, "en-us", &[("spelling.aff", aff), ("spelling.dic", b"2\nzzpackword\nhouse/S\n")]);
        let gb_pack = write_pack(&packs, "en-gb", &[("spelling.aff", aff), ("spelling.dic", b"2\nzzgbword\nhouse/S\n")]);
        packs::set_packs_dir(Some(packs.clone()));
        let english: &[(&str, &[u8], &[u8])] = &[("en_US", aff, b"2\nzzfolderword\nhouse/S\n")];
        folder_with_english(&dir, english);
        assert_eq!(english_source(ProofLang::En), Some(EnglishSource::Hunspell(dir.join("en_US.dic"))), "the folder first");
        assert!(is_correct_in("zzfolderword", ProofLang::En) && !is_correct_in("zzpackword", ProofLang::En));
        // Each English its own: the folder's en_US never stands in for en-GB, which takes its pack.
        assert_eq!(english_source(ProofLang::EnGb), Some(EnglishSource::Hunspell(gb_pack.join("spelling.dic"))));
        drop_english_files(&dir, english);
        assert_eq!(english_source(ProofLang::En), Some(EnglishSource::Hunspell(pack.join("spelling.dic"))), "then the pack");
        assert!(is_correct_in("zzpackword", ProofLang::En) && is_correct_in("houses", ProofLang::En));
        // Neither English takes the other dialect's pack: without its own, en-CA has none.
        std::fs::remove_dir_all(&gb_pack).unwrap();
        packs::reload();
        assert_ne!(english_source(ProofLang::EnGb), Some(EnglishSource::Hunspell(pack.join("spelling.dic"))), "never en-us for en-GB");
        assert_ne!(english_source(ProofLang::EnCa), Some(EnglishSource::Hunspell(pack.join("spelling.dic"))), "never en-us for en-CA");
        packs::set_packs_dir(None);
        assert_ne!(english_source(ProofLang::En), Some(EnglishSource::Hunspell(pack.join("spelling.dic"))), "a removed pack is not used");
        set_dictionaries_folder(Some(shared_test_folder()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// R6.4: an English Hunspell dictionary that is there but can't be read (an encoding we don't
    /// read, bytes that aren't UTF-8, no words) is an error reported for its English, never a
    /// panic, and that English falls back to the next dictionary or the built-in word list, which
    /// still marks `becuase`.
    #[test]
    fn unreadable_english_dictionary_is_reported_and_skipped() {
        if scowl_embedded() {
            return;
        }
        crate::hold_dictionary_tests();
        let dir = std::env::temp_dir().join(format!("wc-proof-en-unreadable-{}", std::process::id()));
        let english: &[(&str, &[u8], &[u8])] =
            &[("en_US", b"SET UTF-8\n", b"0\n"), ("en_GB", b"SET SHIFT_JIS\n", b"1\ncolour\n"), ("en_CA", b"SET UTF-8\n", b"1\n\xff\xfe\n")];
        folder_with_english(&dir, english);
        for (lang, reason) in [(ProofLang::En, "no words"), (ProofLang::EnGb, "unsupported encoding"), (ProofLang::EnCa, "not UTF-8")] {
            {
                // The built-in list, or a dictionary installed on the system (tried next).
                let source = english_source(lang);
                assert!(!matches!(&source, Some(EnglishSource::Hunspell(p)) if p.starts_with(&dir)), "{lang:?}: {source:?}");
                let error = dictionary_load_error(lang).unwrap_or_default();
                assert!(error.contains(reason), "{lang:?}: {error}");
            }
            assert!(!is_correct_in("becuase", lang) && is_correct_in("because", lang) && is_correct_in("houses", lang), "{lang:?}");
            assert!(suggest_in("becuase", 3, lang).contains(&"because".to_string()), "{lang:?}");
        }
        drop_english_files(&dir, english);
        assert_eq!(dictionary_load_error(ProofLang::EnGb), None, "a new folder forgets the old errors");
    }

    /// craft-libs#482 review: an English dictionary that can't be read is skipped for the next
    /// name or folder of the same English, never for another English's; and a dictionary with
    /// compounding and output conversion (as the usual en_US and en_GB ones have) is read. (Only
    /// American English gets a dictionary that loads: the British tests don't hold the lock.)
    #[test]
    fn english_tries_its_next_dictionary_and_reads_real_world_directives() {
        if scowl_embedded() {
            return;
        }
        crate::hold_dictionary_tests();
        let dir = std::env::temp_dir().join(format!("wc-proof-en-next-{}", std::process::id()));
        let aff: &[u8] = b"SET UTF-8\nICONV 1\nICONV \xe2\x80\x99 '\nOCONV 1\nOCONV ' \xe2\x80\x99\nCOMPOUNDMIN 1\nONLYINCOMPOUND c\n\
                  COMPOUNDRULE 1\nCOMPOUNDRULE n*1t\nSFX S Y 1\nSFX S 0 s .\n";
        let english: &[(&str, &[u8], &[u8])] = &[
            ("en_US", b"SET SHIFT_JIS\n", b"1\ncolor\n"),
            ("en-US", aff, b"5\nhouse/S\nthe\nqwzzy\nbecause\n1st/c\n"),
            ("en_GB", b"SET SHIFT_JIS\n", b"1\ncolour\n"),
        ];
        folder_with_english(&dir, english);
        {
            let us = Some(EnglishSource::Hunspell(dir.join("en-US.dic")));
            // A real en_US installed elsewhere (WORDCRAFT_DICTIONARIES, the system) comes first:
            // the same name in every folder before the next name.
            if english_source(ProofLang::En) == us {
                assert!(is_correct_in("qwzzy", ProofLang::En) && is_correct_in("houses", ProofLang::En));
                assert!(is_correct_in("because", ProofLang::En) && !is_correct_in("becuase", ProofLang::En) && !is_correct_in("hous", ProofLang::En));
            }
            for lang in [ProofLang::En, ProofLang::EnGb, ProofLang::EnCa] {
                let source = english_source(lang);
                assert!(source == us || !matches!(&source, Some(EnglishSource::Hunspell(p)) if p.starts_with(&dir)), "{lang:?}: {source:?}");
            }
            assert_ne!(english_source(ProofLang::EnGb), us, "never another English's dictionary");
            for lang in [ProofLang::En, ProofLang::EnGb] {
                let error = dictionary_load_error(lang).unwrap_or_default();
                assert!(error.contains("unsupported encoding"), "{lang:?}: the file it would rather use is reported: {error}");
            }
        }
        drop_english_files(&dir, english);
    }

    /// #1063: the dictionaries folder is searched before the system locations; the dictionaries
    /// in it are reported and used, and cached checks are dropped. The folder holds the same test
    /// dictionaries the other tests install, so tests running meanwhile see the same words; it
    /// stays set for the rest of the run for that reason.
    #[test]
    fn dictionaries_folder_is_searched_first() {
        crate::hold_dictionary_tests();
        let dir = std::env::temp_dir().join(format!("wc-proof-dictionaries-{}", std::process::id()));
        write_test_dictionaries(&dir);
        // #3252 review: force Russian and Czech to cache whatever they find before the folder is
        // set (empty, with no folder and nothing installed on CI), so the folder change below has
        // a stale cache to actually invalidate, not just an untouched `None`.
        ru::dictionary();
        cs::dictionary();
        let before = dictionary_generation();
        set_dictionaries_folder(Some(dir.clone()));
        assert!(dictionary_generation() > before, "cached checks are dropped");
        // Every language is listed, Russian and Czech too (#3226: they were once missing, so
        // neither showed up in File › Options › Proofing). A language reports the folder's file,
        // or none while a dictionary given with install_dictionary() is in use (#3252 review);
        // the languages' own tests install theirs concurrently, so an exact path would race them.
        // The folder is searched first whatever is in use.
        let found = installed_dictionaries();
        let names = ["pl_PL", "pt_BR", "it_IT", "hr_HR", "tr_TR", "ru_RU", "cs_CZ", "id_ID", "es_ES", "es_MX", "es_419", "es_UY"];
        assert_eq!(
            found.iter().map(|(lang, _)| *lang).collect::<Vec<_>>(),
            [
                ProofLang::Pl,
                ProofLang::Pt,
                ProofLang::It,
                ProofLang::Hr,
                ProofLang::Tr,
                ProofLang::Ru,
                ProofLang::Cs,
                ProofLang::Id,
                ProofLang::Es,
                ProofLang::EsMx,
                ProofLang::Es419,
                ProofLang::EsUy
            ]
        );
        for ((lang, path), name) in found.iter().zip(names) {
            let file = dir.join(format!("{name}.dic"));
            assert!(path.is_none() || path.as_ref() == Some(&file), "{lang:?}: {path:?}");
            assert_eq!(hunspell::find(&[name]).map(|(_, dic)| dic), Some(file), "{lang:?}");
        }
        assert!(is_correct_in("chrząszcz", ProofLang::Pl) && !is_correct_in("chrzonszcz", ProofLang::Pl));
        assert!(is_correct_in("tenés", ProofLang::EsUy) && !is_correct_in("prueva", ProofLang::EsUy));
        assert!(is_correct_in("дом", ProofLang::Ru) && !is_correct_in("дум", ProofLang::Ru));
    }

    /// The test dictionaries' folder the dictionaries-folder tests leave set (see
    /// [`dictionaries_folder_is_searched_first`]).
    fn shared_test_folder() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wc-proof-dictionaries-{}", std::process::id()));
        write_test_dictionaries(&dir);
        dir
    }

    /// Write a Language pack folder `<packs>/<locale>/` as the font store unpacks one.
    fn write_pack(packs: &std::path::Path, locale: &str, files: &[(&str, &[u8])]) -> std::path::PathBuf {
        let dir = packs.join(locale);
        std::fs::create_dir_all(dir.join("LICENCES")).unwrap();
        std::fs::write(dir.join("pack.toml"), format!("locale = \"{locale}\"\nname = \"Hrvatski (test)\"\nlicence = \"MIT\"\nclass = \"Embeddable\"\nsources = [\"https://example.org/{locale}\"]\n")).unwrap();
        std::fs::write(dir.join("LICENCES").join("COPYING"), "Test licence text.").unwrap();
        for (name, bytes) in files {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
        dir
    }

    /// CP4.2: an installed Language pack's spelling dictionary and hyphenation patterns are used
    /// (after the dictionaries folder, before the system's), without a restart; removing it turns
    /// them off again; English is unaffected. Croatian stands in for every pack language: its
    /// dictionary leaves the dictionaries folder while the pack holds the same test words (plus
    /// one), so Croatian tests running meanwhile see the same words.
    #[test]
    fn an_installed_pack_is_used_and_a_removed_one_is_not() {
        crate::hold_dictionary_tests();
        // The test dictionaries on disk first, so a reload never leaves other tests without them.
        set_dictionaries_folder(Some(shared_test_folder()));
        let root = std::env::temp_dir().join(format!("wc-proof-packs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (folder, packs) = (root.join("dictionaries"), root.join("language-packs"));
        write_test_dictionaries(&folder);
        std::fs::remove_file(folder.join("hr_HR.aff")).unwrap();
        std::fs::remove_file(folder.join("hr_HR.dic")).unwrap();
        let hyphen_before = hyphen::hyphenate_word_in("hyphenation", &hyphen::Limits::default(), ProofLang::Hr);
        let english_before = (
            english_source(ProofLang::En),
            is_correct_in("becuase", ProofLang::En),
            hyphen::hyphenate_word("hyphenation", &hyphen::Limits::default()),
        );
        let dic = format!("{}zzpaket\n", hr::test_dict::DIC);
        let pack = write_pack(
            &packs,
            "hr",
            &[
                ("spelling.aff", hr::test_dict::AFF.as_bytes()),
                ("spelling.dic", dic.as_bytes()),
                // Liang's example patterns: `hy-phen-ation`.
                ("hyphenation.pat", b"% test patterns\nhy3ph he2n hena4 hen5at 1na n2at 1tio 2io o2n\n"),
            ],
        );
        // The pack first, so Croatian always has its test words.
        let before = dictionary_generation();
        packs::set_packs_dir(Some(packs.clone()));
        assert!(dictionary_generation() > before, "cached checks are dropped");
        set_dictionaries_folder(Some(folder.clone()));
        assert_eq!(packs::packs_dir(), Some(packs.clone()));
        let installed = packs::installed();
        assert_eq!(installed.len(), 1);
        assert_eq!(
            installed[0],
            packs::InstalledPack { locale: "hr".into(), name: "Hrvatski (test)".into(), licence: "MIT".into(), path: pack.clone() }
        );
        assert!(packs::load_errors().is_empty(), "{:?}", packs::load_errors());
        assert!(is_correct_in("zzpaket", ProofLang::Hr) && is_correct_in("kuća", ProofLang::Hr), "the pack's dictionary");
        assert!(!is_correct_in("zzpaketx", ProofLang::Hr));
        let found = installed_dictionaries();
        assert!(found.iter().any(|(l, p)| *l == ProofLang::Hr && p.as_ref() == Some(&pack.join("spelling.dic"))), "{found:?}");
        assert_eq!(hyphen::hyphenate_word_in("hyphenation", &hyphen::Limits::default(), ProofLang::Hr), "hy-phen-ation");
        let licences = packs::licence_texts();
        assert!(licences.iter().any(|(t, x)| t == "Hrvatski (test) (hr): COPYING" && x == "Test licence text."), "{licences:?}");
        assert!(licences.iter().any(|(t, _)| t.contains("English")), "the embedded notices come first");
        // The dictionaries folder still comes first.
        std::fs::write(folder.join("hr_HR.aff"), hr::test_dict::AFF).unwrap();
        std::fs::write(folder.join("hr_HR.dic"), hr::test_dict::DIC).unwrap();
        packs::reload();
        assert!(!is_correct_in("zzpaket", ProofLang::Hr) && is_correct_in("kuća", ProofLang::Hr), "the folder's dictionary");
        assert_eq!(hyphen::hyphenate_word_in("hyphenation", &hyphen::Limits::default(), ProofLang::Hr), "hy-phen-ation", "hyphenation is the pack's");
        // Removed: the pack is no longer read.
        std::fs::remove_dir_all(&pack).unwrap();
        packs::reload();
        assert!(packs::installed().is_empty());
        assert!(!is_correct_in("zzpaket", ProofLang::Hr) && is_correct_in("kuća", ProofLang::Hr));
        assert_eq!(hyphen::hyphenate_word_in("hyphenation", &hyphen::Limits::default(), ProofLang::Hr), hyphen_before);
        let english_after = (
            english_source(ProofLang::En),
            is_correct_in("becuase", ProofLang::En),
            hyphen::hyphenate_word("hyphenation", &hyphen::Limits::default()),
        );
        assert_eq!(english_after, english_before, "English is unaffected");
        packs::set_packs_dir(None);
        set_dictionaries_folder(Some(shared_test_folder()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// German, Bulgarian and pack languages mark spelling in their own language where we have
    /// the words for it, like the built-in ones; English otherwise.
    #[test]
    fn spelling_messages_are_in_the_language() {
        assert_eq!(spelling_message(ProofLang::De), "Möglicher Rechtschreibfehler");
        assert_eq!(spelling_message(ProofLang::Bg), "Възможна правописна грешка");
        assert_eq!(spelling_message(ProofLang::from_tag(Some("fr-CA"))), "Faute d’orthographe possible");
        assert_eq!(spelling_message(ProofLang::Pl), "Możliwy błąd pisowni");
        assert_eq!(spelling_message(ProofLang::from_tag(Some("xh"))), "Possible spelling mistake");
        assert_eq!(spelling_message(ProofLang::En), "Possible spelling mistake");
    }

    /// CP4.2: a language with no tools of its own (Dutch) is spell-checked, given suggestions and
    /// hyphenated from its installed Language pack; removed, it is unchecked again; English is
    /// unaffected. No grammar rules for it.
    #[test]
    fn a_pack_proofs_a_language_without_its_own_tools() {
        crate::hold_dictionary_tests();
        // The test dictionaries on disk first, so a reload never leaves other tests without them.
        set_dictionaries_folder(Some(shared_test_folder()));
        let root = std::env::temp_dir().join(format!("wc-proof-nl-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let nl = ProofLang::from_tag(Some("nl-NL"));
        assert_eq!(nl.pack_locale().map(|l| l.to_string()).as_deref(), Some("nl-nl"));
        let text = "Het huis en de fietz.";
        let english_before = check_text(text, &[]);
        let unchecked = |lang: ProofLang| check_text(text, &[(0..text.len(), lang)]).iter().all(|i| i.kind == IssueKind::Grammar);
        let before = (is_correct_in("fietz", nl), hyphen::hyphenate_word_in("xxnaxx", &hyphen::Limits::default(), nl), pack_dictionary_file(nl));
        let pack = write_pack(
            &root,
            "nl",
            &[
                ("spelling.aff", b"SET UTF-8\nTRY esianrtolducgmphbykfvwjzxq\nSFX E Y 1\nSFX E 0 en .\n"),
                ("spelling.dic", b"5\nhet\nhuis/E\nen\nde\nfiets/E\n"),
                ("hyphenation.pat", b"1na xx\n"),
            ],
        );
        packs::set_packs_dir(Some(root.clone()));
        assert!(nl.is_proofed() && has_dictionary(Some("nl-BE")) && !has_dictionary(Some("fr-FR")));
        assert_eq!(pack_dictionary_file(nl), Some(pack.join("spelling.dic")));
        assert_eq!(pack_dictionary_file(ProofLang::from_tag(Some("nl"))), Some(pack.join("spelling.dic")), "a region falls back to the pack");
        for w in ["huis", "Huis", "huizen", "huisen", "fietsen", "HUIS"] {
            assert_eq!(is_correct_in(w, nl), w != "huizen", "{w}");
        }
        let issues = check_text(text, &[(0..text.len(), nl)]);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!((issues[0].kind, &text[issues[0].start..issues[0].end], issues[0].lang), (IssueKind::Spelling, "fietz", nl));
        assert_eq!(issues[0].message, "Mogelijke spelfout", "the language's own message");
        assert_eq!((missing_dictionary(Some("nl-NL")), unreadable_dictionary(Some("nl-NL"))), (None, None));
        let missing_fr = missing_dictionary(Some("fr-FR"));
        assert!(missing_fr.is_none_or(|n| n == "fr_FR"), "{missing_fr:?}");
        assert!(suggest_in("fietz", 5, nl).contains(&"fiets".to_string()), "{:?}", suggest_in("fietz", 5, nl));
        assert!(check_grammar_in("de de huis", nl).is_empty(), "no grammar rules");
        assert_eq!(hyphen::hyphenate_word_in("xxnaxx", &hyphen::Limits::default(), nl), "xx-naxx");
        assert!(unchecked(ProofLang::from_tag(Some("fr-FR"))), "a language with no pack stays unchecked");
        assert_eq!(check_text(text, &[]), english_before, "English is unaffected");
        // A broken dictionary is reported, never a panic.
        std::fs::write(pack.join("spelling.aff"), "SET SHIFT_JIS\n").unwrap();
        packs::reload();
        assert!(is_correct_in("fietz", nl) && !nl.is_proofed());
        assert!(dictionary_load_error(nl).unwrap_or_default().contains("unsupported encoding"));
        let (name, file, reason) = unreadable_dictionary(Some("nl-NL")).unwrap();
        assert_eq!((name, file), ("nl_NL", pack.join("spelling.dic")));
        assert!(reason.contains("unsupported encoding"), "{reason}");
        assert_eq!(missing_dictionary(Some("nl-NL")), Some("nl_NL"), "as for the other languages");
        // Removed: unchecked again.
        std::fs::remove_dir_all(&pack).unwrap();
        packs::reload();
        assert_eq!(
            (is_correct_in("fietz", nl), hyphen::hyphenate_word_in("xxnaxx", &hyphen::Limits::default(), nl), pack_dictionary_file(nl)),
            before
        );
        assert!(unchecked(nl) || before.2.is_some());
        assert_eq!(check_text(text, &[]), english_before);
        packs::set_packs_dir(None);
        set_dictionaries_folder(Some(shared_test_folder()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Second review of #485: "Ignore words in UPPERCASE" never skips a word with no cased
    /// letter, so a misspelt Hebrew or Arabic word is still marked.
    #[test]
    fn caseless_scripts_are_not_read_as_uppercase() {
        crate::hold_dictionary_tests();
        set_dictionaries_folder(Some(shared_test_folder()));
        let root = std::env::temp_dir().join(format!("wc-proof-caseless-packs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        write_pack(&root, "he", &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", "1\nשלום\n".as_bytes())]);
        write_pack(&root, "ar", &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", "1\nسلام\n".as_bytes())]);
        packs::set_packs_dir(Some(root.clone()));
        let (he, ar) = (ProofLang::from_tag(Some("he-IL")), ProofLang::from_tag(Some("ar-EG")));
        assert!(Options::default().ignore_uppercase);
        assert!(is_correct_in("שלום", he) && !is_correct_in("שלוםם", he));
        assert!(is_correct_in("سلام", ar) && !is_correct_in("سلامم", ar));
        let text = "שלום שלוםם";
        let issues = check_text(text, &[(0..text.len(), he)]);
        assert_eq!(issues.iter().map(|i| &text[i.start..i.end]).collect::<Vec<_>>(), ["שלוםם"], "{issues:?}");
        packs::set_packs_dir(None);
        set_dictionaries_folder(Some(shared_test_folder()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The `.dic` file `lang`'s spelling is read from with the installed packs and `system` as the
    /// system's only dictionary folder.
    fn chosen_dictionary(lang: ProofLang, system: &std::path::Path) -> Option<std::path::PathBuf> {
        let names: Vec<String> = match lang.spelling_locale() {
            Some(l) => pack_lang::names(l),
            None => language(lang).map(|l| l.slot.names().iter().map(|n| (*n).to_string()).collect()).unwrap_or_default(),
        };
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (region, other) = packs::spelling_dirs(&refs);
        hunspell::ordered(&refs, &[], &region, &[system.to_path_buf()], &other).into_iter().next().map(|(_, dic)| dic)
    }

    /// Second review of #485: the bare `es` and `de` packs are Spain's and Germany's, a region's
    /// text reads its own country's pack, and the system's dictionary of the right region beats
    /// another region's pack.
    #[test]
    fn regional_text_reads_its_own_regions_pack() {
        crate::hold_dictionary_tests();
        set_dictionaries_folder(Some(shared_test_folder()));
        let root = std::env::temp_dir().join(format!("wc-proof-region-packs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (packs_dir, system) = (root.join("packs"), root.join("system"));
        std::fs::create_dir_all(&system).unwrap();
        let sys = |name: &str| {
            std::fs::write(system.join(format!("{name}.aff")), "SET UTF-8\n").unwrap();
            std::fs::write(system.join(format!("{name}.dic")), "1\nx\n").unwrap();
            system.join(format!("{name}.dic"))
        };
        let pack =
            |locale: &str| write_pack(&packs_dir, locale, &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", b"1\nx\n")]).join("spelling.dic");
        let tag = |t: &str| ProofLang::from_tag(Some(t));
        packs::set_packs_dir(Some(packs_dir.clone()));

        // es + es-ar + es-mx: each country follows its own list; the bare pack is Spain's.
        let (es, es_ar, es_mx) = (pack("es"), pack("es-ar"), pack("es-mx"));
        packs::reload();
        assert_eq!(chosen_dictionary(tag("es-UY"), &system), Some(es_ar.clone()), "Uruguay: its own, then Argentina's");
        assert_eq!(chosen_dictionary(tag("es-CO"), &system), Some(es_mx.clone()), "Latin America: Mexico's before Argentina's");
        assert_eq!(chosen_dictionary(tag("es-AR"), &system), Some(es_mx.clone()), "es-AR is Latin American Spanish");
        assert_eq!(chosen_dictionary(tag("es-MX"), &system), Some(es_mx.clone()));
        assert_eq!(chosen_dictionary(tag("es-ES"), &system), Some(es.clone()));
        std::fs::remove_dir_all(es_mx.parent().unwrap()).unwrap();
        packs::reload();
        for t in ["es-AR", "es-CO", "es-UY"] {
            assert_eq!(chosen_dictionary(tag(t), &system), Some(es_ar.clone()), "{t}: never Spain's bare pack first");
        }
        std::fs::remove_dir_all(&packs_dir).unwrap();

        // de + de-ch, system de_DE: Swiss text reads the de-ch pack; German text the bare one.
        let sys_de = sys("de_DE");
        let (de, de_ch) = (pack("de"), pack("de-ch"));
        packs::reload();
        assert_eq!(chosen_dictionary(tag("de-CH"), &system), Some(de_ch.clone()));
        assert_eq!(chosen_dictionary(tag("de-LI"), &system), Some(de_ch.clone()));
        assert_eq!(chosen_dictionary(tag("de-DE"), &system), Some(de.clone()), "the bare pack is Germany's: before the system's de_DE");
        std::fs::remove_dir_all(de.parent().unwrap()).unwrap();
        packs::reload();
        assert_eq!(chosen_dictionary(tag("de-AT"), &system), Some(sys_de), "never the Swiss pack");
        std::fs::remove_dir_all(&packs_dir).unwrap();

        // pt-br + pt-pt, or system pt_BR: European text reads the pt-pt pack.
        let (pt_br, pt_pt) = (pack("pt-br"), pack("pt-pt"));
        packs::reload();
        assert_eq!(chosen_dictionary(tag("pt-PT"), &system), Some(pt_pt.clone()));
        assert_eq!(chosen_dictionary(tag("pt-BR"), &system), Some(pt_br.clone()));
        std::fs::remove_dir_all(pt_br.parent().unwrap()).unwrap();
        let sys_pt_br = sys("pt_BR");
        packs::reload();
        assert_eq!(chosen_dictionary(tag("pt-PT"), &system), Some(pt_pt.clone()));
        assert_eq!(chosen_dictionary(tag("pt-BR"), &system), Some(sys_pt_br), "another region's pack after the system's right region");
        std::fs::remove_dir_all(&packs_dir).unwrap();

        // fr pack + system fr_CA: the system's exact region beats another region's pack.
        let sys_fr_ca = sys("fr_CA");
        let fr = pack("fr");
        packs::reload();
        assert_eq!(chosen_dictionary(tag("fr-CA"), &system), Some(sys_fr_ca));
        assert_eq!(chosen_dictionary(tag("fr-FR"), &system), Some(fr.clone()));
        assert_eq!(chosen_dictionary(tag("fr"), &system), Some(fr));

        // Swiss text spells from its pack, hyphenates as German and reads German messages.
        std::fs::remove_dir_all(&packs_dir).unwrap();
        write_pack(&packs_dir, "de", &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", "1\nStraße\n".as_bytes())]);
        write_pack(&packs_dir, "de-ch", &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", "1\nStrasse\n".as_bytes())]);
        packs::reload();
        let (ch, de) = (tag("de-CH"), tag("de-DE"));
        assert!(is_correct_in("Strasse", ch) && !is_correct_in("Straße", ch));
        assert!(is_correct_in("Straße", de) && !is_correct_in("Strasse", de));
        assert_eq!(
            hyphen::hyphenate_word_in("Silbentrennung", &hyphen::Limits::default(), ch),
            hyphen::hyphenate_word_in("Silbentrennung", &hyphen::Limits::default(), de)
        );
        assert_eq!(spelling_message(ch), "Möglicher Rechtschreibfehler");
        packs::set_packs_dir(None);
        set_dictionaries_folder(Some(shared_test_folder()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Second review of #485: past [`MAX_INTERNED`] names the hint still names a dictionary.
    #[test]
    fn interned_names_fall_back_to_a_generic_one() {
        let mut names = Vec::new();
        for i in 0..MAX_INTERNED {
            assert_eq!(interned_in(&mut names, &format!("xx_{i}")), format!("xx_{i}"));
        }
        assert_eq!(interned_in(&mut names, "xx_0"), "xx_0", "a kept name is still given");
        assert_eq!(interned_in(&mut names, "yy_YY"), UNINTERNED);
        assert_eq!(names.len(), MAX_INTERNED);
    }

    /// Review of #485: German and Bulgarian, which have hyphenation of their own but no
    /// dictionary slot, are spelled from their Language pack (or a Hunspell dictionary) too.
    #[test]
    fn german_and_bulgarian_are_spelled_from_their_packs() {
        crate::hold_dictionary_tests();
        set_dictionaries_folder(Some(shared_test_folder()));
        let root = std::env::temp_dir().join(format!("wc-proof-de-bg-packs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let hyphen_before = hyphen::hyphenate_word_in("Silbentrennung", &hyphen::Limits::default(), ProofLang::De);
        write_pack(&root, "de", &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", "2\nHaus\nund\n".as_bytes())]);
        write_pack(&root, "bg", &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", "1\nкъща\n".as_bytes())]);
        packs::set_packs_dir(Some(root.clone()));
        let de = ProofLang::from_tag(Some("de-AT"));
        assert_eq!(de, ProofLang::De);
        assert!(is_correct_in("Haus", de) && !is_correct_in("Hauz", de));
        assert!(suggest_in("Hauz", 5, de).contains(&"Haus".to_string()));
        assert_eq!(check_text("Haus und Hauz", &[(0..13, de)]).len(), 1);
        assert!(de.is_proofed() && has_dictionary(Some("de-CH")));
        assert_eq!(pack_dictionary_file(de), Some(root.join("de").join("spelling.dic")));
        assert!(is_correct_in("къща", ProofLang::Bg) && !is_correct_in("къшта", ProofLang::Bg), "any script, not only Latin");
        assert_eq!(
            hyphen::hyphenate_word_in("Silbentrennung", &hyphen::Limits::default(), ProofLang::De),
            hyphen_before,
            "German keeps its own hyphenation"
        );
        packs::set_packs_dir(None);
        assert!(is_correct_in("Hauz", de) || pack_dictionary_file(de).is_some(), "no dictionary: unchecked");
        set_dictionaries_folder(Some(shared_test_folder()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Review of #485: a tag naming its language's default script reads the script-less pack
    /// (`sr-Cyrl-RS` reads `sr`, `bs-Latn-BA` reads `bs`); another script never does (`sr-Latn`
    /// never reads `sr`, `sr` never reads `sr-latn`).
    #[test]
    fn packs_follow_the_script() {
        crate::hold_dictionary_tests();
        set_dictionaries_folder(Some(shared_test_folder()));
        let root = std::env::temp_dir().join(format!("wc-proof-script-packs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dic = |w: &str| format!("1\n{w}\n");
        let sr = write_pack(&root, "sr", &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", dic("кућа").as_bytes())]);
        let bs = write_pack(&root, "bs", &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", dic("kuća").as_bytes())]);
        packs::set_packs_dir(Some(root.clone()));
        let file = |tag: &str| pack_dictionary_file(ProofLang::from_tag(Some(tag)));
        assert_eq!(file("sr-Cyrl-RS"), Some(sr.join("spelling.dic")));
        assert_eq!(file("sr-RS"), Some(sr.join("spelling.dic")));
        assert_eq!(file("bs-Latn-BA"), Some(bs.join("spelling.dic")));
        assert_eq!(file("sr-Latn-RS"), None, "Latin Serbian never reads the Cyrillic pack");
        assert!(!is_correct_in("кућаа", ProofLang::from_tag(Some("sr-Cyrl"))));
        let latin = write_pack(&root, "sr-latn", &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", dic("kuća").as_bytes())]);
        std::fs::remove_dir_all(&sr).unwrap();
        packs::reload();
        assert_eq!(file("sr-Latn-RS"), Some(latin.join("spelling.dic")));
        assert_eq!(file("sr"), None, "Serbian (Cyrillic) never reads the Latin pack");
        assert_eq!(file("sr-Cyrl-RS"), None);
        packs::set_packs_dir(None);
        set_dictionaries_folder(Some(shared_test_folder()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Review of #485: a pack's patterns hyphenate only words in the language's script: a Latin
    /// word in a Russian or Dutch run is hyphenated as it was without the pack.
    #[test]
    fn pack_patterns_only_hyphenate_their_script() {
        crate::hold_dictionary_tests();
        set_dictionaries_folder(Some(shared_test_folder()));
        let root = std::env::temp_dir().join(format!("wc-proof-script-hyphen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let lim = hyphen::Limits::default();
        let (ru, el) = (ProofLang::Ru, ProofLang::from_tag(Some("el")));
        let before = [hyphen::hyphenate_word_in("hyphenation", &lim, ru), hyphen::hyphenate_word_in("hyphenation", &lim, el)];
        write_pack(&root, "ru", &[("hyphenation.pat", "1на 1ка\n".as_bytes())]);
        write_pack(&root, "el", &[("hyphenation.pat", "1να 1κα 1κά\n".as_bytes())]);
        packs::set_packs_dir(Some(root.clone()));
        assert_eq!([hyphen::hyphenate_word_in("hyphenation", &lim, ru), hyphen::hyphenate_word_in("hyphenation", &lim, el)], before);
        assert_eq!(hyphen::hyphenate_word_in("канака", &lim, ru), "ка-на-ка", "a Russian word uses the pack");
        assert_eq!(hyphen::hyphenate_word_in("κανακα", &lim, el), "κα-να-κα");
        // Decomposed text is composed for the patterns, as spelling does, and stays as it was
        // (patterns no fallback would give: `καν-ακά`).
        write_pack(&root, "grc", &[("hyphenation.pat", "ν1α ν1ά 2κ ά2\n".as_bytes())]);
        packs::reload();
        let grc = ProofLang::from_tag(Some("grc"));
        assert_eq!(hyphen::hyphenate_word_in("κανακά", &lim, grc), "καν-ακά");
        assert_eq!(hyphen::hyphenate_word_in("κανακα\u{301}", &lim, grc), "καν-ακα\u{301}");
        assert!(hyphen::hyphenate_word_in("κανάκανα", &lim, grc).starts_with("καν-ά"), "{}", hyphen::hyphenate_word_in("κανάκανα", &lim, grc));
        assert_eq!(
            hyphen::hyphenate_word_in("κανα\u{301}κανα", &lim, grc),
            hyphen::hyphenate_word_in("κανάκανα", &lim, grc).replace('ά', "α\u{301}")
        );
        packs::set_packs_dir(None);
        set_dictionaries_folder(Some(shared_test_folder()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Review of #485: licence texts are read up to a total, however many packs hold big files.
    #[test]
    fn licence_texts_are_capped_in_all() {
        crate::hold_dictionary_tests();
        set_dictionaries_folder(Some(shared_test_folder()));
        let root = std::env::temp_dir().join(format!("wc-proof-licence-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let big = vec![b'x'; 4 << 20];
        for l in ["qa", "qb", "qc", "qd", "qe"] {
            let dir = write_pack(&root, l, &[]);
            std::fs::write(dir.join("LICENCES").join("BIG"), &big).unwrap();
        }
        packs::set_packs_dir(Some(root.clone()));
        let before = packs::LICENCE_BYTES_READ.load(std::sync::atomic::Ordering::Relaxed);
        let from_packs: usize = packs::licence_texts().iter().filter(|(t, _)| t.contains(": BIG")).map(|(_, x)| x.len()).sum();
        assert!(from_packs > 0 && from_packs <= 16 << 20, "{from_packs}");
        let read = packs::LICENCE_BYTES_READ.load(std::sync::atomic::Ordering::Relaxed) - before;
        assert!(read <= 16 << 20, "{read} bytes read: a file over what is left is never read");
        packs::set_packs_dir(None);
        set_dictionaries_folder(Some(shared_test_folder()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Pack languages for the cache tests: `qa`, `qb`… each knowing `word<locale>`.
    fn many_packs(root: &std::path::Path, n: usize) -> Vec<ProofLang> {
        (0..n)
            .map(|i| {
                let l = format!("q{}", (b'a' + i as u8) as char);
                write_pack(root, &l, &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", format!("1\nword{l}\n").as_bytes())]);
                ProofLang::from_tag(Some(&l))
            })
            .collect()
    }

    /// Review of #485: ten pack languages used in turn stay loaded (a memory budget, not eight
    /// entries), so switching between them never reads a dictionary again.
    #[test]
    fn many_pack_languages_load_once() {
        crate::hold_dictionary_tests();
        set_dictionaries_folder(Some(shared_test_folder()));
        let root = std::env::temp_dir().join(format!("wc-proof-many-packs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let langs = many_packs(&root, 10);
        packs::set_packs_dir(Some(root.clone()));
        let loads = pack_lang::loads();
        let start = std::time::Instant::now();
        // A paragraph of 20 words over the ten languages, checked 50 times.
        let mut text = String::new();
        let mut spans = Vec::new();
        for k in 0..20 {
            let lang = langs[k % 10];
            let w = format!("word{} ", lang.pack_locale().unwrap());
            spans.push((text.len()..text.len() + w.len(), lang));
            text.push_str(&w);
        }
        for _ in 0..50 {
            assert!(check_text(&text, &spans).is_empty());
            for lang in &langs {
                assert!(!is_correct_in("wordzz", *lang));
            }
        }
        assert_eq!(pack_lang::loads() - loads, 10, "each dictionary is read once");
        assert!(start.elapsed() < std::time::Duration::from_secs(10), "{:?}", start.elapsed());
        // A language with no dictionary takes no room and is not looked up again.
        assert!(is_correct_in("anything", ProofLang::from_tag(Some("qz"))));
        packs::set_packs_dir(None);
        set_dictionaries_folder(Some(shared_test_folder()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Review of #485: over the memory budget, a dictionary used during the current paragraph is
    /// never evicted, so the paragraph loads each once.
    #[test]
    fn a_paragraph_never_evicts_its_own_dictionaries() {
        crate::hold_dictionary_tests();
        set_dictionaries_folder(Some(shared_test_folder()));
        let root = std::env::temp_dir().join(format!("wc-proof-budget-packs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let langs = many_packs(&root, 2);
        packs::set_packs_dir(Some(root.clone()));
        pack_lang::set_budget(1);
        let (a, b) = (langs[0], langs[1]);
        let text = "wordqa wordqb wordqa wordqb wordqa wordqb";
        let spans: Vec<(Range<usize>, ProofLang)> = (0..6).map(|k| (k * 7..k * 7 + 6, if k % 2 == 0 { a } else { b })).collect();
        let loads = pack_lang::loads();
        assert!(check_text(text, &spans).is_empty());
        assert_eq!(pack_lang::loads() - loads, 2);
        pack_lang::set_budget(pack_lang::MAX_LOADED_BYTES);
        packs::set_packs_dir(None);
        set_dictionaries_folder(Some(shared_test_folder()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// CP4.2, hostile packs: a missing or broken `pack.toml`, a path outside the pack, a folder
    /// that isn't a language tag, a symbolic link, patterns that aren't UTF-8 or a dictionary that
    /// can't be read are reported, never a panic, and the pack (or that file) is skipped.
    #[test]
    fn corrupt_packs_are_reported_and_skipped() {
        crate::hold_dictionary_tests();
        // The test dictionaries on disk first, so a reload never leaves other tests without them.
        set_dictionaries_folder(Some(shared_test_folder()));
        let root = std::env::temp_dir().join(format!("wc-proof-bad-packs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // Good spelling (the test words, so Croatian tests running meanwhile are unaffected), bad
        // patterns.
        write_pack(
            &root,
            "hr",
            &[("spelling.aff", hr::test_dict::AFF.as_bytes()), ("spelling.dic", hr::test_dict::DIC.as_bytes()), ("hyphenation.pat", b"\xff\xfe a1b")],
        );
        let traversal = write_pack(&root, "nl", &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", b"1\nhuis\n")]);
        std::fs::write(traversal.join("pack.toml"), "locale = \"nl\"\nlicence = \"MIT\"\n[[file]]\npath = \"../../../etc/passwd\"\n").unwrap();
        let wrong = write_pack(&root, "fr", &[]);
        std::fs::write(wrong.join("pack.toml"), "locale = \"de\"\nlicence = \"MIT\"\n").unwrap();
        let broken = write_pack(&root, "sk", &[]);
        std::fs::write(broken.join("pack.toml"), "locale = \"sk\n").unwrap();
        std::fs::create_dir_all(root.join("ca")).unwrap();
        std::fs::create_dir_all(root.join("Bad_Name")).unwrap();
        std::fs::create_dir_all(root.join(".tmp-el-1")).unwrap();
        let huge = write_pack(&root, "lt", &[]);
        std::fs::write(huge.join("pack.toml"), vec![b'#'; 2 << 20]).unwrap();
        #[cfg(unix)]
        {
            let linked = write_pack(&root, "sl", &[("hyphenation.pat", b"a1b")]);
            std::fs::remove_file(linked.join("hyphenation.pat")).unwrap();
            std::os::unix::fs::symlink("/etc/passwd", linked.join("hyphenation.pat")).unwrap();
            std::os::unix::fs::symlink(&linked, root.join("sv")).unwrap();
        }
        packs::set_packs_dir(Some(root.clone()));
        let locales: Vec<String> = packs::installed().into_iter().map(|p| p.locale).collect();
        assert_eq!(locales, if cfg!(unix) { vec!["hr", "sl"] } else { vec!["hr"] });
        let _ = hyphen::hyphenate_word_in("hyphenation", &hyphen::Limits::default(), ProofLang::Hr);
        let errors = packs::load_errors();
        let reason = |l: &str| errors.iter().find(|(k, _)| k == l).map(|(_, e)| e.clone()).unwrap_or_default();
        assert!(reason("nl").contains("outside the pack"), "{errors:?}");
        assert!(reason("fr").contains("names locale"), "{errors:?}");
        assert!(reason("sk").starts_with("pack.toml"), "{errors:?}");
        assert!(reason("ca").contains("no pack.toml"), "{errors:?}");
        assert!(reason("Bad_Name").contains("language tag"), "{errors:?}");
        assert!(reason("lt").contains("too large"), "{errors:?}");
        assert!(reason("hr").contains("not UTF-8"), "{errors:?}");
        assert!(!errors.iter().any(|(k, _)| k.starts_with('.')), "temporary folders are not packs");
        #[cfg(unix)]
        {
            assert!(reason("sv").contains("not a folder"), "{errors:?}");
            assert!(hunspell::find(&["sl"]).is_none_or(|(_, d)| !d.starts_with(&root)), "a link is never read");
        }
        assert!(is_correct_in("kuća", ProofLang::Hr), "the pack's spelling still works");
        // A dictionary in a pack that can't be read is reported like any other.
        let bad = write_pack(&root, "hr", &[("spelling.aff", b"SET SHIFT_JIS\n"), ("spelling.dic", b"1\nkuca\n")]);
        let (aff, dic) = hunspell::find_in(&bad, &["spelling"]).unwrap();
        assert!(hunspell::load(&aff, &dic).unwrap_err().contains("unsupported encoding"));
        packs::set_packs_dir(None);
        set_dictionaries_folder(Some(shared_test_folder()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A dictionary that fails to load says why in File › Options › Proofing in every language
    /// that reads one; Russian and Czech were missing from the list that named their dictionaries
    /// there.
    #[test]
    fn every_dictionary_language_reports_its_load_error() {
        crate::hold_dictionary_tests();
        for (lang, _) in installed_dictionaries() {
            let name = dictionary_needed(lang, false).unwrap();
            let reason = format!("zq unreadable {name}");
            let _ = hunspell::note_load::<()>(name, std::path::Path::new("zq.dic"), Err(reason.clone()));
            assert_eq!(dictionary_load_error(lang), Some(reason), "{lang:?}");
            hunspell::clear_load_error(name);
        }
    }

    /// Every proofed language other than English has a row in the language table, and only those.
    #[test]
    fn the_language_table_covers_the_proofed_languages() {
        for l in ProofLang::PROOFED {
            assert_eq!(language(l).is_some(), !l.is_english(), "{l:?}");
        }
        for (i, row) in LANGUAGES.iter().enumerate() {
            assert!(row.lang.is_proofed(), "{:?}", row.lang);
            assert!(LANGUAGES.iter().skip(i + 1).all(|other| other.lang != row.lang), "{:?} twice", row.lang);
        }
    }

    /// #1741: a list item may be a fragment, so its first word needs no capital in any language
    /// with the rule; a plain paragraph's does, and later sentences in the item are still checked.
    #[test]
    fn list_item_first_word_needs_no_capital() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        let grammar = |t: &str, lang: ProofLang, list: bool| -> Vec<String> {
            check_text_with(t, &[(0..t.len(), lang)], &Options::default(), true, list)
                .into_iter()
                .filter(|i| i.kind == IssueKind::Grammar)
                .filter_map(|i| t.get(i.start..i.end).map(str::to_string))
                .collect()
        };
        for (lang, text) in [
            (ProofLang::En, "item one"),
            (ProofLang::Pl, "jabłka i gruszki"),
            (ProofLang::Hr, "jabuke i kruške"),
            (ProofLang::It, "mele e pere"),
            (ProofLang::Tr, "elma ve armut"),
            (ProofLang::Pt, "maçãs e peras"),
            (ProofLang::Ru, "яблоки и груши"),
        ] {
            let first = text.split(' ').next().unwrap();
            assert!(grammar(text, lang, true).is_empty(), "{lang:?} list item: {:?}", grammar(text, lang, true));
            assert_eq!(grammar(text, lang, false), [first], "{lang:?} plain paragraph");
        }
        assert_eq!(grammar("hello world.", ProofLang::En, false), ["hello"]);
        // Later sentences in a list item are still checked.
        assert_eq!(grammar("item one. then two", ProofLang::En, true), ["then"]);
        assert_eq!(grammar("elma ve armut. sonra kiraz", ProofLang::Tr, true), ["sonra"]);
    }

    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn dictionary_loads() {
        assert!(dict::Dictionary::en_us().len() > 100_000);
    }

    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn spelling() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        for w in [
            "if",
            "we",
            "hello",
            "world",
            "running",
            "studios",
            "don't",
            "Thursday",
            "NASA",
            "2026",
            "co-operate",
            "unhappy",
            "colour",
            "it's",
            "stopped",
        ] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["helo", "wrold", "sentense", "recieve"] {
            assert!(!is_correct(w), "{w}");
        }
        let s = suggest("recieve", 5);
        assert!(s.contains(&"receive".to_string()), "{s:?}");
        assert!(suggest("Wrold", 3).contains(&"World".to_string()));
    }

    /// #1694: `-ically` adverbs come from their `-ic` adjective (specific → specifically).
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn ically_adverbs_are_known() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        for w in ["specifically", "scientifically", "automatically", "dramatically", "Specifically"] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["specificaly", "specifcally", "zzzically"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    /// #1694 review: only `-ic` adjectives take `-ally`; other known words (dictionary or
    /// supplement) don't.
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn non_ic_words_do_not_take_ally() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        for (base, wrong) in [("desktop", "desktopally"), ("house", "houseally"), ("final", "finalally"), ("green", "greenally")] {
            assert!(is_correct(base), "{base}");
            assert!(!is_correct(wrong), "{wrong}");
        }
    }

    /// #520: common modern words (and their inflections) are known; misspellings still flagged.
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn common_modern_words() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        for w in [
            "desktop",
            "Desktops",
            "laptop",
            "download",
            "downloaded",
            "uploading",
            "email",
            "emailing",
            "website",
            "online",
            "offline",
            "smartphones",
            "login",
            "username",
            "spreadsheet",
            "database",
            "screenshots",
            "blogging",
            "inbox",
            "workplace",
            "uploader",
        ] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["dekstop", "laptp", "dowload", "spreadshet"] {
            assert!(!is_correct(w), "{w}");
        }
        assert!(suggest("dekstop", 5).contains(&"desktop".to_string()), "{:?}", suggest("dekstop", 5));
        assert!(check_spelling("My desktop and laptop download email online.").is_empty());
    }

    /// The supplement stays a sorted list of lowercase base forms.
    /// #40: British spellings and their inflections are not flagged.
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn british_spellings_are_known() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        for w in [
            "judgement",
            "defence",
            "licence",
            "programme",
            "catalogue",
            "behaviour",
            "colour",
            "colourful",
            "colourless",
            "honour",
            "honoured",
            "honouring",
            "honours",
            "honourable",
            "rigour",
            "glamour",
            "clamour",
            "endeavour",
            "paediatric",
            "prioritise",
            "prioritised",
            "prioritising",
            "privatise",
            "privatised",
            "synthesise",
            "travelogue",
            "manoeuvrable",
        ] {
            assert!(is_correct(w), "{w}");
        }
        let text = "The programme honoured her rigour and prioritised colourful, paediatric care.";
        for lang in [ProofLang::EnGb, ProofLang::EnCa] {
            let flagged: Vec<&str> = check_spelling_in(text, lang).iter().map(|i| &text[i.start..i.end]).collect();
            let expect: &[&str] = if lang == ProofLang::EnCa { &["prioritised"] } else { &[] };
            assert_eq!(flagged, expect, "{lang:?}");
        }
        // American English flags the British spellings (#1281).
        let flagged: Vec<&str> = check_spelling(text).iter().map(|i| &text[i.start..i.end]).collect();
        assert_eq!(flagged, ["programme", "honoured", "rigour", "prioritised", "colourful", "paediatric"]);
    }

    /// Each English accepts its own spellings and flags the other dialect's, suggesting its own
    /// first, as Word does (#1281, #1045).
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn english_dialects_flag_each_others_spellings() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        let (us, gb, ca) = (ProofLang::En, ProofLang::EnGb, ProofLang::EnCa);
        // (word, right in en-US, right in en-CA, right in en-GB)
        let cases = [
            ("color", true, false, false),
            ("colour", false, true, true),
            ("colours", false, true, true),
            ("coloured", false, true, true),
            ("Colourful", false, true, true),
            ("colour's", false, true, true),
            ("honorable", true, false, false),
            ("neighbourhood", false, true, true),
            ("center", true, false, false),
            ("centre", false, true, true),
            ("centred", false, true, true),
            ("centring", false, true, true),
            ("centering", true, false, false),
            ("theater", true, false, false),
            ("theatre", true, true, true),
            ("organize", true, true, true),
            ("organise", false, false, true),
            ("organisation", false, false, true),
            ("organisations", false, false, true),
            ("realised", false, false, true),
            ("recognising", false, false, true),
            ("analyze", true, true, false),
            ("analyse", false, true, true),
            ("paralysed", false, true, true),
            ("defense", true, false, false),
            ("defence", false, true, true),
            ("licence", false, true, true),
            ("license", true, true, true),
            ("practise", false, true, true),
            ("traveled", true, false, false),
            ("travelled", true, true, true),
            ("catalog", true, false, false),
            ("catalogue", true, true, true),
            ("anaemia", false, true, true),
            ("anemia", true, true, false),
            ("tire", true, true, true),
            ("tyre", false, false, true),
            ("programme", false, true, true),
            ("program", true, true, true),
            ("meter", true, true, true),
            ("metre", false, true, true),
            ("plough", false, true, true),
            ("colour-blind", false, true, true),
            // Words spelled alike everywhere, and real misspellings, are unaffected.
            ("humorous", true, true, true),
            ("honorary", true, true, true),
            ("tour", true, true, true),
            ("four", true, true, true),
            ("programmed", true, true, true),
            ("colourr", false, false, false),
            ("colorr", false, false, false),
            ("organisationz", false, false, false),
        ];
        for (w, in_us, in_ca, in_gb) in cases {
            assert_eq!(is_correct_in(w, us), in_us, "en-US: {w}");
            assert_eq!(is_correct_in(w, ca), in_ca, "en-CA: {w}");
            assert_eq!(is_correct_in(w, gb), in_gb, "en-GB: {w}");
            assert_eq!(is_correct_for(w, Some("en-AU")), in_gb, "en-AU follows British spelling: {w}");
        }
        // The dialect's own spelling comes first, in the word's case.
        assert_eq!(suggest_in("colour", 5, us).first().map(String::as_str), Some("color"));
        assert_eq!(suggest_in("Colours", 5, us).first().map(String::as_str), Some("Colors"));
        assert_eq!(suggest_in("color", 5, gb).first().map(String::as_str), Some("colour"));
        assert_eq!(suggest_in("organisation", 5, us).first().map(String::as_str), Some("organization"));
        assert_eq!(suggest_in("organisation", 5, ca).first().map(String::as_str), Some("organization"));
        assert_eq!(suggest_in("analyze", 5, gb).first().map(String::as_str), Some("analyse"));
        assert_eq!(suggest_in("CENTER", 5, gb).first().map(String::as_str), Some("CENTRE"));
        assert_eq!(suggest_in("colour's", 5, us).first().map(String::as_str), Some("color's"));
        assert_eq!(suggest_in("colour-blind", 5, us).first().map(String::as_str), Some("color-blind"));
        // Shared suggestions come in the dialect's spelling, never the other one's.
        let gb_sugg = suggest_in("colouur", 8, gb);
        assert!(gb_sugg.contains(&"colour".to_string()) && !gb_sugg.contains(&"color".to_string()), "{gb_sugg:?}");
        let us_sugg = suggest_in("colouur", 8, us);
        assert!(us_sugg.contains(&"color".to_string()) && !us_sugg.contains(&"colour".to_string()), "{us_sugg:?}");
        // A word in the user dictionary is right in every English.
        add_word("zzcolourzz");
        assert!(is_correct_in("zzcolourzz", us));
        // Issues carry the dialect, so suggestions on demand come from the same one.
        let issues = check_text("The color is nice.", &[(0..18, gb)]);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!((issues[0].lang, &"The color is nice."[issues[0].start..issues[0].end]), (gb, "color"));
        let grammar = check_text("the colour is nice.", &[(0..19, gb)]);
        assert!(grammar.iter().any(|i| i.kind == IssueKind::Grammar && i.lang == gb), "{grammar:?}");
    }

    /// The words #40 (folded into #1281) reported as flagged in British English, with their
    /// inflections: right in en-GB, and in en-US flagged or not as `en-variants.txt` decides.
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn british_words_reported_in_issue_40_are_right_in_en_gb() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        let (us, gb, ca) = (ProofLang::En, ProofLang::EnGb, ProofLang::EnCa);
        // (word, right in en-US, right in en-CA, right in en-GB)
        let cases = [
            // Both spellings are right everywhere: `judgment` is also British legal usage, and
            // American dictionaries list `judgement` as a variant.
            ("judgement", true, true, true),
            ("judgements", true, true, true),
            ("judgment", true, true, true),
            ("judgments", true, true, true),
            ("defence", false, true, true),
            ("defences", false, true, true),
            ("defense", true, false, false),
            // `license` is the British verb, so only American English flags `licence`.
            ("licence", false, true, true),
            ("licences", false, true, true),
            ("license", true, true, true),
            ("licensed", true, true, true),
            ("licenses", true, true, true),
            // `program` is British for computing.
            ("programme", false, true, true),
            ("programmes", false, true, true),
            ("program", true, true, true),
            ("programs", true, true, true),
            // American English accepts both.
            ("catalogue", true, true, true),
            ("catalogues", true, true, true),
            ("catalogued", true, true, true),
            ("cataloguing", true, true, true),
            ("catalog", true, false, false),
            ("catalogs", true, false, false),
        ];
        for (w, in_us, in_ca, in_gb) in cases {
            assert_eq!(is_correct_in(w, us), in_us, "en-US: {w}");
            assert_eq!(is_correct_in(w, ca), in_ca, "en-CA: {w}");
            assert_eq!(is_correct_in(w, gb), in_gb, "en-GB: {w}");
        }
        let text = "The judgement on the defence licence put the programme in the catalogue.";
        assert!(check_spelling_in(text, gb).is_empty(), "{:?}", check_spelling_in(text, gb));
        assert_eq!(suggest_in("licences", 5, us).first().map(String::as_str), Some("licenses"));
        assert_eq!(suggest_in("programmes", 5, us).first().map(String::as_str), Some("programs"));
    }

    /// More British spellings that must stay right in en-GB and en-AU (#1281): -re, single l, ae,
    /// with their inflections.
    #[test]
    fn more_british_spellings_are_right_in_en_gb() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        let gb = ProofLang::EnGb;
        assert_eq!(ProofLang::from_tag(Some("en-AU")), gb);
        for w in [
            "goitre",
            "goitres",
            "reconnoitre",
            "sceptre",
            "sceptres",
            "enrol",
            "enrols",
            "enrolled",
            "enrolment",
            "fulfil",
            "fulfils",
            "fulfilled",
            "fulfilment",
            "instalment",
            "instalments",
            "skilful",
            "skilfully",
            "wilful",
            "wilfully",
            "encyclopaedia",
            "encyclopaedias",
        ] {
            assert!(is_correct_in(w, gb), "en-GB: {w}");
        }
        let text = "The wilful knight took the sceptre to fulfil his enrolment, paid the instalment and read the encyclopaedia.";
        assert!(check_spelling_in(text, gb).is_empty(), "{:?}", check_spelling_in(text, gb));
    }

    /// English spelling reads SCOWL 2026 through the affix engine: one dictionary per English,
    /// its notice embedded, and its ordinals (`COMPOUNDRULE`) understood.
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn english_reads_scowl() {
        for l in [ProofLang::En, ProofLang::EnGb, ProofLang::EnCa] {
            let d = scowl::dict(l).unwrap_or_else(|| panic!("{l:?}: {:?}", scowl::load_error(l)));
            assert!(d.len() > 40_000, "{l:?}: {} stems", d.len());
            assert_eq!(english_source(l), Some(EnglishSource::Embedded));
            assert_eq!(dictionary_load_error(l), None);
        }
        assert!(std::ptr::eq(scowl::dict(ProofLang::EnGb).unwrap(), scowl::dict(ProofLang::from_tag(Some("en-AU"))).unwrap()), "en-AU reads en-GB");
        let notice = scowl::notice().unwrap();
        for credit in ["Kevin Atkinson", "Geoff Kuenning", "clause 4 removed", "Benjamin Titze"] {
            assert!(notice.contains(credit), "{credit}");
        }
        // Ordinals are compounds of digit stems; `1th` is only a part.
        let strict = Options { ignore_numbers: false, ..Options::default() };
        for w in ["1st", "21st", "102nd", "33rd", "11th", "112th", "2026th"] {
            assert!(is_correct_in_with(w, ProofLang::En, &strict), "{w}");
        }
        for w in ["1th", "21th", "11st", "3nd"] {
            assert!(!is_correct_in_with(w, ProofLang::En, &strict), "{w}");
        }
        // Affixed forms and possessives come from the affix rules.
        for w in ["unrecognizable", "reorganization", "children's", "Mississippi's", "can't", "can’t"] {
            assert!(is_correct_in(w, ProofLang::En), "{w}");
        }
        assert!(!is_correct_in("mississippi", ProofLang::En), "a proper noun in lower case");
        // Suggestions come from the run's own dictionary.
        assert!(suggest_in("colur", 5, ProofLang::EnGb).contains(&"colour".to_string()), "{:?}", suggest_in("colur", 5, ProofLang::EnGb));
        assert!(suggest_in("colr", 5, ProofLang::En).contains(&"color".to_string()), "{:?}", suggest_in("colr", 5, ProofLang::En));
        assert_eq!(suggest_in("organisation", 5, ProofLang::En).first().map(String::as_str), Some("organization"));
        assert_eq!(suggest_in("color", 5, ProofLang::EnGb).first().map(String::as_str), Some("colour"));
    }

    /// craft-libs#483 review: English suggestions offer a missing space only between two
    /// lower-case words of three letters or more, after every one-word candidate, and no word the
    /// dictionary only has capitalized (`Col`, `Wis`) for a lower-case word.
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn english_suggestions_skip_junk_splits_and_names() {
        for (w, l) in [
            ("colour", ProofLang::En),
            ("theatr", ProofLang::En),
            ("publically", ProofLang::En),
            ("cheque", ProofLang::En),
            ("colr", ProofLang::En),
            ("wiss", ProofLang::En),
            ("theatr", ProofLang::EnGb),
        ] {
            let got = suggest_in(w, 8, l);
            assert!(!got.is_empty(), "{w}");
            assert!(got.iter().all(|s| !s.chars().any(char::is_uppercase)), "{w} in {l:?}: {got:?}");
            let first_split = got.iter().position(|s| s.contains(' ')).unwrap_or(got.len());
            assert!(got.iter().skip(first_split).all(|s| s.contains(' ')), "{w}: a split before a word: {got:?}");
            assert!(got.iter().filter(|s| s.contains(' ')).all(|s| s.split(' ').all(|p| p.chars().count() >= 3)), "{w}: {got:?}");
        }
        assert_eq!(suggest_in("publically", 5, ProofLang::En).first().map(String::as_str), Some("publicly"));
        assert_eq!(suggest_in("colour", 5, ProofLang::En).first().map(String::as_str), Some("color"));
        assert_eq!(suggest_in("theatr", 5, ProofLang::EnGb).first().map(String::as_str), Some("theatre"));
        assert!(suggest_in("Londn", 5, ProofLang::En).contains(&"London".to_string()), "a capitalized word may get a name");
    }

    /// `en-variants.txt` stays with SCOWL: SCOWL's size-60 lists keep one spelling per dialect,
    /// and Word accepts more (both `-ize` and `-ise` in British English, `judgement` in American).
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn dialect_policy_widens_scowl() {
        let gb = scowl::dict(ProofLang::EnGb).unwrap();
        let us = scowl::dict(ProofLang::En).unwrap();
        assert!(!gb.check("organize") && gb.check("organise") && !us.check("judgement"), "SCOWL alone is stricter");
        assert!(is_correct_in("organize", ProofLang::EnGb) && is_correct_in("organise", ProofLang::EnGb));
        assert!(is_correct_in("judgement", ProofLang::En) && !is_correct_in("colour", ProofLang::En));
        assert!(is_correct_in("encyclopaedia", ProofLang::EnGb) && is_correct_in("encyclopedia", ProofLang::EnGb));
    }

    /// craft-libs#483 review: each English checks against its own SCOWL dictionary only. A stem
    /// one English's dictionary has and another's lacks (`colourblind`, `greyscale` in en-GB;
    /// `colorblind`, `esophagi` in en-US) doesn't cross, unless our dialect policy or the `-ize`
    /// spellings of British English say so. Sampled from every pair of Englishes.
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn scowl_dialects_dont_cross() {
        let langs = [
            (ProofLang::En, crate::data::EN_US_SPELLING),
            (ProofLang::EnGb, crate::data::EN_GB_SPELLING),
            (ProofLang::EnCa, crate::data::EN_CA_SPELLING),
        ];
        for (from, dic) in langs {
            for (to, _) in langs {
                if from == to {
                    continue;
                }
                let d = scowl::dict(to).unwrap();
                let widened = |w: &str| {
                    let x = english::ise_spelling(w);
                    (scowl::is_british(to) && x.as_deref().is_some_and(|x| d.check(x)))
                        || en_variants::known_variant(w, to, |x| d.check(x))
                        || known_in_supplement(w)
                };
                let only: Vec<&str> = dic
                    .lines()
                    .skip(1)
                    .map(|l| l.split('/').next().unwrap_or_default())
                    .filter(|w| w.len() > 3 && w.chars().all(|c| c.is_ascii_lowercase()) && !d.check(w) && !widened(w))
                    .collect();
                assert!(only.len() >= 40, "{from:?} → {to:?}: {}", only.len());
                let step = only.len() / 40;
                let crossed: Vec<&str> = only.iter().step_by(step.max(1)).copied().filter(|w| is_correct_in(w, to)).collect();
                assert!(crossed.is_empty(), "{from:?} words right in {to:?}: {crossed:?}");
            }
        }
        for (w, l) in [("colourblind", ProofLang::En), ("greyscale", ProofLang::En), ("colorblind", ProofLang::EnGb), ("esophagi", ProofLang::EnGb)] {
            assert!(!is_correct_in(w, l), "{w} in {l:?}");
        }
        for (w, l) in
            [("colorblind", ProofLang::En), ("colourblind", ProofLang::EnGb), ("organizational", ProofLang::EnGb), ("realized", ProofLang::EnGb)]
        {
            assert!(is_correct_in(w, l), "{w} in {l:?}");
        }
    }

    /// craft-libs#483 re-review: British English takes the `-ize` spelling of an `-ise` word only
    /// when American or Canadian English spells it so; a misspelling whose `-s-` form is a word
    /// (`wize`, `advize`) stays wrong, as in Hunspell's en-GB.
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn british_ize_needs_a_real_ize_word() {
        let wrong = [
            "wize", "rize", "noize", "dizease", "advize", "rizing", "praize", "raize", "poize", "wizer", "surprize", "arize", "revize", "devize",
            "mizer", "concize", "precize", "exercize", "chaize",
        ];
        for l in [ProofLang::EnGb, ProofLang::from_tag(Some("en-AU"))] {
            for w in wrong.iter().chain(&["colorize"]) {
                assert!(!is_correct_in(w, l), "{w} in {l:?}");
            }
            for w in ["organize", "realization", "colourize", "organizing", "realizes"] {
                assert!(is_correct_in(w, l), "{w} in {l:?}");
            }
        }
    }

    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn user_dictionary_accepts_the_other_dialects_spelling() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        add_word("savour");
        assert!(is_correct_in("savour", ProofLang::En), "a word the user added is never flagged");
        assert!(!is_correct_in("savor", ProofLang::EnGb));
        remove_word("savour");
        assert!(!is_correct_in("savour", ProofLang::En));
    }

    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn supplement_is_sorted_lowercase() {
        let words: Vec<&str> = EXTRA_EN.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        assert!(words.len() > 100);
        assert!(words.windows(2).all(|p| p[0] < p[1]), "not sorted / duplicate");
        assert!(words.iter().all(|w| w.chars().all(|c| c.is_lowercase() || c == '\'' || c == '-')), "not lowercase");
    }

    /// Without the Language data too: the installed English dictionary or the built-in word list
    /// marks both misspellings (R6.4).
    #[test]
    fn spelling_issues_in_text() {
        crate::hold_dictionary_tests();
        let t = "This sentense has a mispelled word, see https://example.com/xyzq.";
        let v = check_spelling(t);
        let bad: Vec<&str> = v.iter().map(|i| &t[i.start..i.end]).collect();
        assert_eq!(bad, vec!["sentense", "mispelled"]);
    }

    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn address_token_covers_every_word_split_out_of_it() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        // #1335: one whitespace-delimited token, however punctuation splits it, has one
        // web/e-mail classification, shared by all its words.
        let t = "zxq@mail.example.com,blorp;qwirpy wugwug";
        let bad: Vec<&str> = check_spelling(t).iter().map(|i| &t[i.start..i.end]).collect();
        assert_eq!(bad, vec!["wugwug"]);
        let opts = Options { ignore_internet: false, ..Options::default() };
        let bad: Vec<&str> = check_spelling_with(t, &opts).iter().map(|i| &t[i.start..i.end]).collect();
        assert_eq!(bad.first(), Some(&"zxq"), "{bad:?}");
        assert!(bad.contains(&"qwirpy"), "{bad:?}");
        assert_eq!(bad.last(), Some(&"wugwug"), "{bad:?}");
    }

    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn address_after_a_wide_space_is_an_internet_token() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        // A web or e-mail address after a no-break space, a narrow no-break space, a figure space
        // or an ideographic space is skipped, as it is after a plain space.
        let t = "see\u{a0}https://xyzq.example/blorp and\u{3000}www.qwirpy.example or\u{a0}zxq@mail.example \
                 at\u{202f}https://frobq.example/zibb by\u{2007}snorf@mail.example wugwug";
        let bad: Vec<&str> = check_spelling(t).iter().map(|i| &t[i.start..i.end]).collect();
        assert_eq!(bad, vec!["wugwug"]);
        let opts = Options { ignore_internet: false, ..Options::default() };
        let bad: Vec<&str> = check_spelling_with(t, &opts).iter().map(|i| &t[i.start..i.end]).collect();
        assert!(["xyzq", "blorp", "qwirpy", "zxq", "frobq", "zibb", "snorf", "wugwug"].iter().all(|w| bad.contains(w)), "{bad:?}");
    }

    #[test]
    fn spelling_scales_over_one_long_token() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        // #1335: thousands of punctuation-separated words inside a single
        // whitespace-free token: one classification for the token, one issue per
        // misspelled word, through every public spelling entry point. Without the Language data
        // too (craft-libs#482 review: the old core list took 14 s here).
        let started = std::time::Instant::now();
        let word = |i: usize| {
            let mut w = String::from("zxq");
            let mut n = i;
            loop {
                w.push((b'a' + (n % 26) as u8) as char);
                n /= 26;
                if n == 0 {
                    break;
                }
            }
            w
        };
        let t = (0..2000).map(word).collect::<Vec<_>>().join(",");
        assert_eq!(check_spelling(&t).len(), 2000);
        assert_eq!(check_spelling_in(&t, ProofLang::En).len(), 2000);
        let opts = Options { ignore_internet: false, ..Options::default() };
        assert_eq!(check_spelling_with(&t, &opts).len(), 2000);
        assert_eq!(check_text(&t, &[]).into_iter().filter(|i| i.kind == IssueKind::Spelling).count(), 2000);
        // Debug builds on a loaded CI machine: a bound against blowups, not a benchmark.
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    }

    #[test]
    fn grammar_issues() {
        let t = "this is is a apple , and an car.  Done";
        let v = check_grammar(t);
        let msgs: Vec<&str> = v.iter().map(|i| i.message.as_str()).collect();
        assert!(msgs.iter().any(|m| m.contains("Repeated")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("\"an\"")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("\"a\" before")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("space before")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("Capitalize")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("Extra space")), "{msgs:?}");
        assert!(check_grammar("A university is an hour away.").is_empty(), "{:?}", check_grammar("A university is an hour away."));
    }

    #[test]
    fn ellipses_do_not_start_sentences() {
        for text in [
            "I saw... a figure right outside.",
            "I saw… a figure right outside.",
            "I saw.... a figure right outside.",
            "I saw... \"a figure\" right outside.",
            "I saw… “a figure” right outside.",
        ] {
            let capitals: Vec<_> =
                check_grammar(text).into_iter().filter(|issue| issue.message == "Capitalize the first word of a sentence").collect();
            assert!(capitals.is_empty(), "{text}: {capitals:?}");
        }

        for text in ["I saw. a figure.", "I saw! a figure.", "I saw? a figure.", "I saw.\" a figure."] {
            let capitals: Vec<_> =
                check_grammar(text).into_iter().filter(|issue| issue.message == "Capitalize the first word of a sentence").collect();
            assert_eq!(capitals.len(), 1, "{text}: {capitals:?}");
            assert_eq!(capitals[0].suggestions, ["A"], "{text}");
        }
    }

    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn user_dictionary_works() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        assert!(!is_correct("zxqwordcrafty"));
        add_word("zxqwordcrafty");
        assert!(is_correct("zxqwordcrafty"));
    }

    /// #40: the user dictionary survives a save and a fresh load.
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn user_dictionary_persists_through_a_file() {
        crate::hold_dictionary_tests();
        let dir = std::env::temp_dir().join(format!("wc-dict-{}-{}", std::process::id(), "persist"));
        let _ = std::fs::remove_dir_all(&dir);
        // The folder is created on the first save.
        let path = dir.join("sub/custom-dictionary.txt");
        add_word("zxqpersistone");
        add_word("zxqpersisttwo");
        save_user_dictionary(&path, &[]).expect("save works");
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.lines().any(|w| w == "zxqpersistone") && saved.lines().any(|w| w == "zxqpersisttwo"), "{saved}");
        // Written atomically: no temporary file is left beside it.
        let names: Vec<_> = std::fs::read_dir(dir.join("sub")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["custom-dictionary.txt"]);
        remove_word("zxqpersistone");
        assert!(!is_correct("zxqpersistone"));
        load_user_dictionary(&path);
        assert!(is_correct("zxqpersistone"), "loading brings the word back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #40: a hostile file is capped: invalid lines are skipped, and only the first megabyte is read.
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn hostile_dictionary_files_are_capped() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        let dir = std::env::temp_dir().join(format!("wc-dict-{}-{}", std::process::id(), "hostile"));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("custom-dictionary.txt");
        let mut hostile = vec![0xff, 0xfe, b'\n'];
        hostile.extend(b"zxqhostileok\n  zxqhostiletrim  \nzxq hostile space\nzxq\x07bell\n");
        hostile.extend("zxq".repeat(30).bytes()); // 90 characters: too long
        hostile.push(b'\n');
        // Pad past the 1 MB cap with blank lines; the word after it is never read.
        hostile.resize(1_000_100, b'\n');
        hostile.extend(b"zxqhostilepastcap\n");
        std::fs::write(&path, &hostile).unwrap();
        let added = load_user_dictionary(&path);
        assert!(added.contains(&"zxqhostileok".to_string()) && added.contains(&"zxqhostiletrim".to_string()), "{added:?}");
        assert!(!added.iter().any(|w| w.contains("space") || w.contains("bell") || w.chars().count() > MAX_CUSTOM_WORD_CHARS), "{added:?}");
        assert!(!is_correct("zxqhostilepastcap"), "the file is read only up to the cap");
        assert_eq!(read_dictionary_file(&path).len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #40: saving merges in words another process saved, and leaves out the ones just removed.
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn saving_merges_with_the_file() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        let dir = std::env::temp_dir().join(format!("wc-dict-{}-{}", std::process::id(), "merge"));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("custom-dictionary.txt");
        std::fs::write(&path, "zxqmergeother\nzxqmergegone\n").unwrap();
        add_word("zxqmergemine");
        save_user_dictionary(&path, &["zxqmergegone".to_string()]).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.lines().any(|w| w == "zxqmergeother"), "the other process's word is kept: {saved}");
        assert!(saved.lines().any(|w| w == "zxqmergemine"), "{saved}");
        assert!(!saved.lines().any(|w| w == "zxqmergegone"), "a removed word stays out: {saved}");
        assert!(is_correct("zxqmergeother") && !is_correct("zxqmergegone"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Ignore All lasts for the session: the word is accepted but isn't in the dictionary.
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn ignored_words_are_not_dictionary_words() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        assert!(!is_correct("zxqignoredword"));
        ignore_word("ZxqIgnoredWord");
        assert!(is_correct("zxqignoredword"));
        assert!(!user_dictionary().contains(&"zxqignoredword".to_string()));
    }

    #[test]
    fn custom_words_are_single_short_words() {
        assert_eq!(custom_word("  Hello "), Some("hello".into()));
        for bad in ["", "   ", "two words", "tab\tbed", "bell\u{7}", "bad\u{FFFD}", &"x".repeat(MAX_CUSTOM_WORD_CHARS + 1)] {
            assert_eq!(custom_word(bad), None, "{bad:?}");
        }
        assert!(custom_word(&"é".repeat(MAX_CUSTOM_WORD_CHARS)).is_some(), "the cap counts characters, not bytes");
    }

    /// #40: a missing file loads as empty, never an error.
    #[test]
    fn missing_dictionary_file_loads_empty() {
        let path = std::env::temp_dir().join(format!("wc-dict-{}-missing.txt", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert!(load_user_dictionary(&path).is_empty());
    }

    /// #100: runs are proofed when their language has a dictionary (the default language too)
    /// and skipped otherwise.
    #[test]
    fn dictionary_lang_tags() {
        for l in [None, Some(""), Some("  "), Some("en"), Some("en-US"), Some("en-GB"), Some("EN-us"), Some("en_US")] {
            assert!(has_dictionary(l), "{l:?}");
        }
        for l in [Some("lt-LT"), Some("fr-FR"), Some("de"), Some("uk"), Some("-US"), Some("e")] {
            assert!(!has_dictionary(l), "{l:?}");
        }
        // The gate follows the dictionaries, not English: every proofed language is proofed.
        for d in ProofLang::PROOFED.map(ProofLang::code) {
            assert!(has_dictionary(Some(d)), "{d}");
            assert!(has_dictionary(Some(&format!("{}-XX", d.to_ascii_uppercase()))), "{d}");
        }
    }

    /// Combining marks stay in their word (#3562 review): a decomposed word is one word, with the
    /// byte range of what was typed, and it is checked as its composed form in every language.
    #[test]
    fn decomposed_words_stay_whole() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        let text = "tambie\u{301}n, cafe\u{301}  \u{301}x";
        let ws: Vec<&str> = words(text).into_iter().map(|(a, b)| &text[a..b]).collect();
        assert_eq!(ws, ["tambie\u{301}n", "cafe\u{301}", "\u{301}x"]);
        assert_eq!(words("tambie\u{301}n"), [(0, 9)]);
        assert_eq!(&*composed("tambie\u{301}n"), "también");
        assert!(matches!(composed("también"), std::borrow::Cow::Borrowed(_)) && matches!(composed("plain"), std::borrow::Cow::Borrowed(_)));
        pt::test_dict::install();
        assert!(is_correct_in("corac\u{327}a\u{303}o", ProofLang::Pt), "Portuguese gets the same composition");
    }

    /// #3562 review: Add to Dictionary, Ignore All, removal and the dictionary file use the same
    /// composed key as spelling, so a word added or ignored decomposed is accepted in both forms,
    /// an NFD entry in a saved file loads (and saves) composed, and either form removes it.
    #[test]
    fn user_dictionary_and_ignore_all_match_both_normalization_forms() {
        crate::hold_dictionary_tests();
        es::test_dict::install();
        let es = ProofLang::Es;
        let (nfd, nfc) = ("zqprue\u{301}ba", "zqpruéba");
        let text = format!("esto {nfd}");
        let issues = check_text(&text, &[(0..text.len(), es)]);
        assert_eq!(issues.iter().map(|i| (i.start, i.end)).collect::<Vec<_>>(), [(5, text.len())], "flagged at the typed bytes");
        assert!(!is_correct_in(nfc, es));
        add_word(nfd);
        assert!(is_correct_in(nfd, es) && is_correct_in(nfc, es));
        assert!(check_text(&text, &[(0..text.len(), es)]).is_empty());
        assert!(user_dictionary().contains(&nfc.to_string()) && !user_dictionary().contains(&nfd.to_string()));
        assert!(remove_word(nfc), "the composed form removes the entry added decomposed");
        assert!(!is_correct_in(nfd, es) && !is_correct_in(nfc, es));
        // Ignore All, either way round.
        ignore_word("zqigno\u{301}ro");
        assert!(is_correct_in("zqignóro", es) && is_correct_in("zqigno\u{301}ro", es));
        ignore_word("zqcallé");
        assert!(is_correct_in("zqcalle\u{301}", es));
        // A dictionary file with a decomposed entry loads composed and is saved composed.
        let path = std::env::temp_dir().join(format!("wc-nfd-dictionary-{}.txt", std::process::id()));
        std::fs::write(&path, "zqguarda\u{301}do\nzqotra\n").unwrap();
        let added = load_user_dictionary(&path);
        assert!(added.contains(&"zqguardádo".to_string()) && !added.iter().any(|w| w.contains('\u{301}')), "{added:?}");
        assert!(is_correct_in("zqguardádo", es) && is_correct_in("zqguarda\u{301}do", es));
        save_user_dictionary(&path, &["zqotra".to_string()]).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.lines().any(|l| l == "zqguardádo") && !saved.contains('\u{301}') && !saved.lines().any(|l| l == "zqotra"), "{saved}");
        // Removing the decomposed form takes out the composed entry, and the file follows.
        assert!(remove_word("zqguarda\u{301}do"));
        save_user_dictionary(&path, &["zqguarda\u{301}do".to_string()]).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().lines().any(|l| l == "zqguardádo"));
        assert!(!is_correct_in("zqguardádo", es));
        let _ = std::fs::remove_file(&path);
    }

    /// #100: runs in a language without a dictionary are never flagged; English still is.
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn non_english_words_are_not_flagged() {
        for w in ["ąžuolas", "élève", "über", "labas", "bonjour"] {
            assert!(is_correct_for(w, Some("lt-LT")), "{w}");
            assert!(is_correct_for(w, Some("fr-FR")), "{w}");
        }
        assert!(!is_correct_for("helo", Some("en-US")), "English typos still flagged");
        assert!(!is_correct_for("helo", None), "default language still proofed");
        assert!(is_correct_for("hello", Some("en-US")));
    }

    /// File › Options › Proofing (#485): each "Ignore" choice turned off makes those words count.
    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn ignore_options_decide_what_is_checked() {
        let t = "SENTENSE helo2 www.exampel.org";
        let bad = |o: Options| check_spelling_with(t, &o).iter().map(|i| t[i.start..i.end].to_string()).collect::<Vec<_>>();
        assert!(bad(Options::default()).is_empty(), "Word's defaults skip all three");
        let all = Options { ignore_uppercase: false, ignore_numbers: false, ignore_internet: false };
        assert_eq!(bad(all), ["SENTENSE", "helo2", "www", "exampel"]);
        assert_eq!(bad(Options { ignore_uppercase: false, ..Options::default() }), ["SENTENSE"]);
        assert!(is_correct_with("NASA", &all), "capitals that are a word stay correct");
    }

    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn hyphenation() {
        let l = hyphen::Limits::default();
        assert!(hyphen::hyphenate_word("hyphenation", &l).matches('-').count() >= 2);
    }

    fn flagged(t: &str, v: &[Issue]) -> Vec<String> {
        v.iter().filter(|i| i.kind == IssueKind::Spelling).map(|i| t[i.start..i.end].to_string()).collect()
    }

    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn each_language_uses_its_own_tools() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        pl::test_dict::install();
        let pl = "Zażółć gęślą jaźń, chrząszcz brzmi w trzcinie.";
        assert!(check_spelling_in(pl, ProofLang::Pl).is_empty(), "{:?}", check_spelling_in(pl, ProofLang::Pl));
        assert!(check_spelling_in(pl, ProofLang::En).len() >= 3, "Polish words are English misspellings");
        assert!(check_spelling_in("Nous écrivons en français.", ProofLang::Other).is_empty());
        assert!(check_grammar_in("this is is wrong", ProofLang::Other).is_empty());
        assert!(is_correct_in("anything", ProofLang::Other) && suggest_in("xyzzy", 5, ProofLang::Other).is_empty());
        let v = check_spelling_in("Płynie żeka.", ProofLang::Pl);
        assert_eq!(flagged("Płynie żeka.", &v), ["żeka"]);
        assert_eq!((v[0].lang, v[0].message.as_str()), (ProofLang::Pl, "Możliwy błąd pisowni"));
        assert!(suggest_in("żeka", 5, ProofLang::Pl).contains(&"rzeka".to_string()));
        assert!(suggest_in("recieve", 5, ProofLang::En).contains(&"receive".to_string()));
        // Polish grammar speaks Polish and has no English a/an rule.
        let g = check_grammar_in("Wiem że to jest a apple.", ProofLang::Pl);
        assert!(g.iter().any(|i| i.message == "Brak przecinka przed „że”"), "{g:?}");
        assert!(!g.iter().any(|i| i.message.contains("\"an\"")), "{g:?}");
    }

    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn italian_uses_its_own_tools() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        it::test_dict::install();
        let it = "La città vecchia ha un bel caffè in piazza.";
        assert!(check_spelling_in(it, ProofLang::It).is_empty(), "{:?}", check_spelling_in(it, ProofLang::It));
        assert!(check_spelling_in(it, ProofLang::En).len() >= 3, "Italian words are English misspellings");
        let v = check_spelling_in("Vado a Roma con lbiro.", ProofLang::It);
        assert_eq!(flagged("Vado a Roma con lbiro.", &v), ["lbiro"]);
        assert_eq!((v[0].lang, v[0].message.as_str()), (ProofLang::It, "Possibile errore di ortografia"));
        assert!(suggest_in("caffe", 5, ProofLang::It).contains(&"caffè".to_string()));
        // Italian grammar speaks Italian and has no English a/an rule.
        let g = check_grammar_in("questo questo è un test", ProofLang::It);
        assert!(g.iter().any(|i| i.message.contains("ripetuta")), "{g:?}");
    }

    #[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
    #[test]
    fn mixed_language_paragraphs_check_each_run_in_its_language() {
        // English results depend on the dictionaries other tests install.
        crate::hold_dictionary_tests();
        pl::test_dict::install();
        // A Polish paragraph quoting English: English spelling and rules inside the quote only.
        let t = "Napisał „the quick brown fox is is here” i poszedł spać, bo żeka wylała.";
        let (a, b) = (t.find("the").unwrap(), t.find('”').unwrap());
        let spans = [(0..a, ProofLang::Pl), (a..b, ProofLang::En), (b..t.len(), ProofLang::Pl)];
        let v = check_text(t, &spans);
        assert_eq!(flagged(t, &v), ["żeka"], "{v:?}");
        let rep = v.iter().find(|i| i.kind == IssueKind::Grammar).unwrap();
        assert_eq!((rep.lang, rep.message.as_str()), (ProofLang::En, "Repeated word: \"is\""), "{v:?}");
        // The English rules don't run over the Polish text (no "Capitalize" for the lower-case
        // Polish words), and the Polish rules don't run over the English quote.
        assert!(v.iter().all(|i| i.lang == ProofLang::Pl || (a..b).contains(&i.start)), "{v:?}");
        // Text outside every span is English; no spans at all is English.
        assert_eq!(flagged("helo świat", &check_text("helo świat", &[])), ["helo", "świat"]);
        assert_eq!(flagged("helo świat", &check_text("helo świat", &[(5..12, ProofLang::Pl)])), ["helo"]);
        // A language without tools: nothing flagged there.
        assert!(check_text("Bonjour mes amis", &[(0..16, ProofLang::Other)]).is_empty());
        // Portuguese checks in their span only, with the Portuguese message.
        pt::test_dict::install();
        let t = "Ele disse que coraçao e frequencia existem.";
        let v = check_text(t, &[(0..t.len(), ProofLang::Pt)]);
        assert_eq!(flagged(t, &v), ["coraçao", "frequencia"], "{v:?}");
        assert!(v.iter().all(|i| i.lang == ProofLang::Pt && i.message == "Possível erro de ortografia"), "{v:?}");
        // Italian checks in its span only, with the Italian message.
        it::test_dict::install();
        let t = "Vado a Roma con lbiro e caffe.";
        let v = check_text(t, &[(0..t.len(), ProofLang::It)]);
        assert_eq!(flagged(t, &v), ["lbiro", "caffe"], "{v:?}");
        assert!(v.iter().all(|i| i.lang == ProofLang::It && i.message == "Possibile errore di ortografia"), "{v:?}");
    }

    proptest::proptest! {
        #[test]
        fn never_panics(s in "\\PC{0,80}") {
            let _ = check_spelling(&s);
            let _ = check_grammar(&s);
            let _ = suggest(&s, 3);
            let _ = has_dictionary(Some(&s));
            let _ = is_correct_for(&s, Some(&s));
            let _ = hyphen::hyphen_points(&s, &hyphen::Limits::default());
            for lang in [ProofLang::En, ProofLang::EnGb, ProofLang::EnCa, ProofLang::Pl, ProofLang::Pt, ProofLang::It, ProofLang::Hr, ProofLang::Tr, ProofLang::Ru, ProofLang::Cs, ProofLang::Id, ProofLang::De, ProofLang::Es, ProofLang::EsMx, ProofLang::Es419, ProofLang::EsUy, ProofLang::Other] {
                let _ = check_spelling_in(&s, lang);
                let _ = check_grammar_in(&s, lang);
                let _ = is_correct_in(&s, lang);
                let _ = hyphen::hyphen_points_in(&s, &hyphen::Limits::default(), lang);
            }
            let _ = lang::normalize_tag(&s);
            let _ = ProofLang::from_tag(Some(&s));
        }

        #[test]
        fn polish_suggestions_never_panic(s in "[a-ząćęłńóśźżA-ZĄĆĘŁŃÓŚŹŻ'’ -]{0,14}") {
            let _ = suggest_in(&s, 4, ProofLang::Pl);
        }

        #[test]
        fn portuguese_suggestions_never_panic(s in "[a-záàâãéêíóôõúüçA-ZÁÀÂÃÉÊÍÓÔÕÚÜÇ'’ -]{0,14}") {
            let _ = suggest_in(&s, 4, ProofLang::Pt);
        }

        #[test]
        fn italian_suggestions_never_panic(s in "[a-zàèéìòóùA-ZÀÈÉÌÒÓÙ'’ -]{0,14}") {
            let _ = suggest_in(&s, 4, ProofLang::It);
        }

        #[test]
        fn turkish_suggestions_never_panic(s in "[a-zçğıöşüA-ZÇĞİÖŞÜ'’ -]{0,14}") {
            let _ = suggest_in(&s, 4, ProofLang::Tr);
        }

        #[test]
        fn czech_suggestions_never_panic(s in "[a-zěščřžýáíéůúĚŠČŘŽÝÁÍÉŮÚ'’ -]{0,14}") {
            cs::test_dict::install();
            let _ = suggest_in(&s, 4, ProofLang::Cs);
        }

        #[test]
        fn indonesian_suggestions_never_panic(s in "[a-zA-Zé'’ -]{0,14}") {
            id::test_dict::install();
            let _ = suggest_in(&s, 4, ProofLang::Id);
            let _ = is_correct_in(&s, ProofLang::Id);
        }

        #[test]
        fn spanish_suggestions_never_panic(s in "[a-zA-ZáéíóúüñÁÉÍÓÚÜÑ¿¡'’ -]{0,14}") {
            es::test_dict::install();
            for lang in [ProofLang::Es, ProofLang::EsMx, ProofLang::Es419, ProofLang::EsUy] {
                let _ = suggest_in(&s, 4, lang);
                let _ = is_correct_in(&s, lang);
            }
        }

        #[test]
        fn mixed_spans_never_panic(s in "\\PC{0,60}", cuts in proptest::collection::vec((0usize..80, 0usize..80, 0u8..15), 0..6)) {
            let langs = [ProofLang::En, ProofLang::EnGb, ProofLang::EnCa, ProofLang::Pl, ProofLang::Pt, ProofLang::It, ProofLang::Tr, ProofLang::Ru, ProofLang::Cs, ProofLang::Id, ProofLang::Es, ProofLang::EsMx, ProofLang::Es419, ProofLang::EsUy, ProofLang::Other];
            let spans: Vec<(Range<usize>, ProofLang)> = cuts.iter().map(|&(a, b, l)| (a.min(b)..a.max(b), langs[l as usize % langs.len()])).collect();
            for i in check_text(&s, &spans) {
                proptest::prop_assert!(i.start <= i.end && i.end <= s.len());
            }
        }
    }

    proptest::proptest! {
        // Suggestions scan the whole word list: fewer cases keep the test quick.
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(24))]
        #[test]
        fn english_dialect_suggestions_never_panic(s in "[a-zA-Zéß'’ -]{0,16}|(colou?r|cent(er|re)|organi[sz]ation)['’s-]{0,4}") {
            for lang in [ProofLang::En, ProofLang::EnGb, ProofLang::EnCa] {
                let _ = suggest_in(&s, 4, lang);
                let _ = is_correct_in(&s, lang);
                let _ = en_variants::preferred(&s, lang);
                let _ = en_variants::known_variant(&s, lang, |_| true);
            }
        }
    }
}
