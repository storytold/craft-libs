//! Proofing for any language by its locale ([`crate::ProofLang::Pack`]): its Hunspell dictionary (the
//! dictionaries folder, then an installed Language pack, then the system's), read by the shared
//! affix engine, with default suggestion tuning and no grammar rules.
//!
//! The loaded dictionaries share a memory budget ([`MAX_LOADED_BYTES`], estimated from their
//! files); the one used least recently goes first, but never one used during the current
//! [`in_one_check`] call, so a paragraph in many languages loads each dictionary once. Locales
//! with no dictionary are remembered apart and take no room.
//!
//! A word none of whose letters is in a script the dictionary's words are written in (a Cyrillic
//! word in a Dutch run) isn't this language's: it is never marked, as Word leaves a word in another
//! script alone. A word in the dictionary's script is always checked, whatever its letters.

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use crate::affix::AffixDict;
use crate::affix_suggest::Tuning;
use crate::lang::PackLocale;

/// The memory budget of the loaded dictionaries, in estimated bytes.
pub(crate) const MAX_LOADED_BYTES: u64 = 256 << 20;
/// Estimated memory per byte of `.aff` and `.dic` file.
const BYTES_PER_FILE_BYTE: u64 = 6;
/// Most locales remembered as having no dictionary (the oldest is forgotten first when full).
const MAX_MISSING: usize = 256;

/// The budget in force (tests lower it).
static BUDGET: AtomicU64 = AtomicU64::new(MAX_LOADED_BYTES);
/// Dictionaries loaded from disk so far (tests count them).
static LOADS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// The clock when this thread's current [`in_one_check`] call started.
    static CHECK_FROM: Cell<Option<u64>> = const { Cell::new(None) };
}

struct Loaded {
    locale: PackLocale,
    dict: Arc<AffixDict>,
    /// The scripts its words are written in ([`scripts`]), sorted.
    scripts: Arc<[u32]>,
    /// The `.dic` it came from.
    file: Option<PathBuf>,
    /// Estimated memory.
    bytes: u64,
    used: u64,
}

struct Cache {
    loaded: Vec<Loaded>,
    /// Locales with no dictionary (or one that failed to load).
    missing: Vec<PackLocale>,
    clock: u64,
    /// Counts reloads, so a load that started before one is not kept.
    reloads: u64,
}

static CACHE: RwLock<Cache> = RwLock::new(Cache { loaded: Vec::new(), missing: Vec::new(), clock: 0, reloads: 0 });

/// Forget every loaded dictionary (the dictionaries folder or the packs changed).
pub(crate) fn reload() {
    let mut c = CACHE.write().unwrap_or_else(PoisonError::into_inner);
    c.loaded.clear();
    c.missing.clear();
    c.reloads = c.reloads.wrapping_add(1);
}

/// Run `f` as one check (a paragraph): no dictionary it uses is evicted before it returns, even
/// over the budget.
pub(crate) fn in_one_check<R>(f: impl FnOnce() -> R) -> R {
    let outer = CHECK_FROM.with(Cell::get);
    let _end = outer.is_none().then(|| {
        let now = CACHE.read().unwrap_or_else(PoisonError::into_inner).clock;
        CHECK_FROM.with(|c| c.set(Some(now.wrapping_add(1))));
        EndCheck
    });
    f()
}

/// Ends the outermost [`in_one_check`] when dropped: also when `f` panics (`Session::run` catches
/// it), or every dictionary would stay pinned on this thread and the budget never hold again.
struct EndCheck;

impl Drop for EndCheck {
    fn drop(&mut self) {
        CHECK_FROM.with(|c| c.set(None));
    }
}

/// Remember that `locale` has no dictionary; when [`MAX_MISSING`] are remembered, the oldest is
/// forgotten (clearing them all would look every other one up again).
fn remember_missing(missing: &mut Vec<PackLocale>, locale: PackLocale) {
    if missing.len() >= MAX_MISSING {
        missing.remove(0);
    }
    missing.push(locale);
}

/// The script of the letter `c`, as a number: the main alphabetic scripts by their Unicode blocks
/// (Latin with its extensions, Greek, Cyrillic, Armenian, Hebrew, Arabic, Georgian, Hangul, kana,
/// Han, Ethiopic); any other letter by its 128-character block. Only told apart, never named.
fn script_of(c: char) -> u32 {
    const SCRIPTS: &[(u32, u32, u32)] = &[
        (0x0041, 0x024F, 1),
        (0x0250, 0x02AF, 1),
        (0x1E00, 0x1EFF, 1),
        (0x2C60, 0x2C7F, 1),
        (0xA720, 0xA7FF, 1),
        (0xAB30, 0xAB6F, 1),
        (0xFF21, 0xFF5A, 1),
        (0x0370, 0x03FF, 2),
        (0x1F00, 0x1FFF, 2),
        (0x0400, 0x052F, 3),
        (0x1C80, 0x1C8F, 3),
        (0x2DE0, 0x2DFF, 3),
        (0xA640, 0xA69F, 3),
        (0x0530, 0x058F, 4),
        (0xFB13, 0xFB17, 4),
        (0x0590, 0x05FF, 5),
        (0xFB1D, 0xFB4F, 5),
        (0x0600, 0x06FF, 6),
        (0x0750, 0x077F, 6),
        (0x08A0, 0x08FF, 6),
        (0xFB50, 0xFDFF, 6),
        (0xFE70, 0xFEFF, 6),
        (0x10A0, 0x10FF, 7),
        (0x1C90, 0x1CBF, 7),
        (0x2D00, 0x2D2F, 7),
        (0x1100, 0x11FF, 8),
        (0x3130, 0x318F, 8),
        (0xAC00, 0xD7AF, 8),
        (0x3040, 0x30FF, 9),
        (0x3400, 0x4DBF, 10),
        (0x4E00, 0x9FFF, 10),
        (0x1200, 0x139F, 11),
    ];
    let u = u32::from(c);
    SCRIPTS.iter().find(|(lo, hi, _)| (*lo..=*hi).contains(&u)).map_or(0x100 + (u >> 7), |(_, _, s)| *s)
}

/// The scripts the words of `d` are written in ([`script_of`] of their letters), sorted.
fn scripts(d: &AffixDict) -> Arc<[u32]> {
    let mut seen = std::collections::BTreeSet::new();
    for c in d.stems().flat_map(str::chars).filter(|c| c.is_alphabetic()) {
        seen.insert(script_of(c));
    }
    seen.into_iter().collect()
}

/// Is `word` in another script than the dictionary's (`scripts`): none of its letters is in one
/// of them? A word in the dictionary's script is never, whatever its letters (`zzzz` in Dutch).
fn other_script(word: &str, scripts: &[u32]) -> bool {
    !scripts.is_empty() && !word.chars().filter(|c| c.is_alphabetic()).any(|c| scripts.binary_search(&script_of(c)).is_ok())
}

#[cfg(test)]
pub(crate) fn loads() -> u64 {
    LOADS.load(Ordering::Relaxed)
}

#[cfg(test)]
pub(crate) fn set_budget(bytes: u64) {
    BUDGET.store(bytes, Ordering::Relaxed);
}

/// Hunspell file names for `locale`, best first: `fr-fr` → `fr_FR`, `fr-FR`, `fr`;
/// `sr-latn` → `sr_Latn`, `sr-Latn` (never plain `sr`, which is Cyrillic); `sr-cyrl-rs` →
/// `sr_Cyrl_RS`, `sr-Cyrl-RS`, `sr_Cyrl`, `sr-Cyrl`, `sr_RS`, `sr-RS`, `sr` (Cyrillic is Serbian's
/// default script, [`crate::lang::default_script`]); `sr` → `sr`, `sr_Cyrl`, `sr-Cyrl`.
pub(crate) fn names(locale: PackLocale) -> Vec<String> {
    let parts: Vec<&str> = locale.as_str().split('-').collect();
    let script = parts.iter().position(|p| crate::lang::is_script(p));
    let cased: Vec<String> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if i == 0 {
                (*p).to_string()
            } else if Some(i) == script {
                let mut c = p.chars();
                c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
            } else if p.len() == 2 {
                p.to_ascii_uppercase()
            } else {
                (*p).to_string()
            }
        })
        .collect();
    let mut out: Vec<String> = Vec::new();
    let mut push = |parts: &[String]| {
        for sep in ["_", "-"] {
            let n = parts.join(sep);
            if !out.contains(&n) {
                out.push(n);
            }
        }
    };
    let primary = locale.primary();
    let default = crate::lang::default_script(primary);
    // Each shorter prefix too (`sr-latn-rs` → `sr-latn`), never dropping the script subtag.
    let keep = script.map_or(1, |s| s + 1);
    for n in (keep..=cased.len()).rev() {
        push(cased.get(..n).unwrap_or_default());
    }
    match script {
        // The language's default script: its script-less names are in that script too.
        Some(s) if parts.get(s).is_some_and(|p| Some(*p) == default) => {
            let without: Vec<String> = cased.iter().enumerate().filter(|(i, _)| *i != s).map(|(_, p)| p.clone()).collect();
            for n in (1..=without.len()).rev() {
                push(without.get(..n).unwrap_or_default());
            }
        }
        Some(_) => {}
        // No script named: the language's default one may be spelled out (`sr-Cyrl`).
        None => {
            if let Some(d) = default {
                let mut c = d.chars();
                let d = c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default();
                push(&[primary.to_string(), d]);
            }
        }
    }
    out
}

/// `locale`'s dictionary, loaded on first use; an empty one when there is none.
fn get(locale: PackLocale) -> (Arc<AffixDict>, Option<PathBuf>) {
    let (d, file, _) = get_with_scripts(locale);
    (d, file)
}

/// [`get`], with the dictionary's [`scripts`].
fn get_with_scripts(locale: PackLocale) -> (Arc<AffixDict>, Option<PathBuf>, Arc<[u32]>) {
    let reloads = {
        let mut c = CACHE.write().unwrap_or_else(PoisonError::into_inner);
        c.clock = c.clock.wrapping_add(1);
        let now = c.clock;
        if let Some(l) = c.loaded.iter_mut().find(|l| l.locale == locale) {
            l.used = now;
            return (l.dict.clone(), l.file.clone(), l.scripts.clone());
        }
        if c.missing.contains(&locale) {
            return (Arc::default(), None, Arc::from([]));
        }
        c.reloads
    };
    let (dict, file, bytes) = load(locale);
    let scripts = scripts(&dict);
    let mut c = CACHE.write().unwrap_or_else(PoisonError::into_inner);
    if c.reloads != reloads || c.loaded.iter().any(|l| l.locale == locale) || c.missing.contains(&locale) {
        return (dict, file, scripts);
    }
    if dict.is_empty() {
        remember_missing(&mut c.missing, locale);
        return (dict, file, scripts);
    }
    LOADS.fetch_add(1, Ordering::Relaxed);
    let budget = BUDGET.load(Ordering::Relaxed);
    let pinned_from = CHECK_FROM.with(Cell::get).unwrap_or(u64::MAX);
    let mut total: u64 = c.loaded.iter().map(|l| l.bytes).sum();
    while total.saturating_add(bytes) > budget {
        // The least recently used dictionary not used during this check.
        let Some(oldest) = c.loaded.iter().enumerate().filter(|(_, l)| l.used < pinned_from).min_by_key(|(_, l)| l.used).map(|(i, _)| i) else {
            break;
        };
        let gone = c.loaded.swap_remove(oldest);
        total = total.saturating_sub(gone.bytes);
    }
    let used = c.clock;
    c.loaded.push(Loaded { locale, dict: dict.clone(), scripts: scripts.clone(), file: file.clone(), bytes, used });
    (dict, file, scripts)
}

/// Load `locale`'s dictionary, with its estimated memory.
fn load(locale: PackLocale) -> (Arc<AffixDict>, Option<PathBuf>, u64) {
    let names = names(locale);
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let key = names.first().map_or("", String::as_str);
    let Some((aff, dic)) = crate::hunspell::find(&refs) else {
        crate::hunspell::clear_load_error(key);
        return (Arc::default(), None, 0);
    };
    let size = |p: &std::path::Path| std::fs::metadata(p).map_or(0, |m| m.len());
    let bytes = size(&aff).saturating_add(size(&dic)).saturating_mul(BYTES_PER_FILE_BYTE).max(1);
    match crate::hunspell::note_load(key, &dic, crate::hunspell::load(&aff, &dic)) {
        Ok(d) => (Arc::new(d), Some(dic), bytes),
        Err(e) => {
            log::warn!("{locale} spelling: {e}");
            (Arc::default(), None, 0)
        }
    }
}

/// Is a dictionary for `locale` available (its spelling is checked)?
pub(crate) fn has_dictionary(locale: PackLocale) -> bool {
    !get(locale).0.is_empty()
}

/// The `.dic` file `locale`'s spelling is read from now.
pub(crate) fn dictionary_file(locale: PackLocale) -> Option<PathBuf> {
    let (d, file) = get(locale);
    file.filter(|_| !d.is_empty())
}

/// Why `locale`'s dictionary failed to load, if it did.
pub(crate) fn load_error(locale: PackLocale) -> Option<String> {
    crate::hunspell::load_error(names(locale).first()?).map(|(_, e)| e)
}

/// Is `word` right in `locale`? Every word is when there is no dictionary.
pub(crate) fn is_correct(word: &str, locale: PackLocale, opts: &crate::Options) -> bool {
    let (d, _, scripts) = get_with_scripts(locale);
    if d.is_empty() {
        return true;
    }
    let Some(w) = crate::spelling::word_to_check_in(&d, word, opts) else { return true };
    if other_script(w, &scripts) {
        return true;
    }
    let known = |w: &str| d.check(w) || crate::in_user_dictionary(&w.to_lowercase());
    if w.contains('-') && !known(w) {
        return w.split('-').all(|p| p.chars().count() <= 1 || known(p));
    }
    known(w)
}

/// Suggestions for `word` in `locale`, best first, at most `max`.
pub(crate) fn suggest(word: &str, max: usize, locale: PackLocale) -> Vec<String> {
    let (d, _) = get(locale);
    if d.is_empty() {
        return Vec::new();
    }
    crate::affix_suggest::suggest_in(&d, word, max, &Tuning::DEFAULT)
}

/// The pack locales whose hyphenation patterns serve a [`crate::ProofLang::Pack`] language: the locale,
/// then each shorter prefix not past a script subtag (`fr-ca` → `fr`).
pub(crate) fn hyphenation_locales(locale: PackLocale) -> Vec<String> {
    names(locale).into_iter().filter(|n| !n.contains('_')).map(|n| n.to_ascii_lowercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(t: &str) -> PackLocale {
        PackLocale::new(t).unwrap()
    }

    /// Review follow-up (craft-libs#485): a full list of locales with no dictionary forgets its
    /// oldest one, not all of them, so a document in more languages doesn't look every one up again.
    #[test]
    fn missing_locales_forget_the_oldest_first() {
        let tag = |i: usize| l(&format!("q{}{}", (b'a' + (i / 26 % 26) as u8) as char, (b'a' + (i % 26) as u8) as char));
        let mut missing = Vec::new();
        for i in 0..MAX_MISSING + 10 {
            remember_missing(&mut missing, tag(i));
            assert!(missing.len() <= MAX_MISSING);
        }
        assert_eq!(missing.len(), MAX_MISSING);
        assert!(!missing.contains(&tag(0)) && !missing.contains(&tag(9)), "the oldest went first");
        assert!(missing.contains(&tag(10)) && missing.contains(&tag(MAX_MISSING + 9)), "the rest stay");
    }

    /// Review follow-up (craft-libs#485): a panic inside a check (caught by the app) doesn't leave
    /// this thread's dictionaries pinned past the budget for good.
    #[test]
    fn a_panic_inside_a_check_ends_it() {
        let caught = std::panic::catch_unwind(|| in_one_check(|| panic!("a bug inside a check")));
        assert!(caught.is_err());
        assert_eq!(CHECK_FROM.with(Cell::get), None);
        in_one_check(|| {
            assert!(CHECK_FROM.with(Cell::get).is_some());
            in_one_check(|| ());
            assert!(CHECK_FROM.with(Cell::get).is_some(), "an inner check doesn't end the outer one");
        });
        assert_eq!(CHECK_FROM.with(Cell::get), None);
    }

    /// Review follow-up (craft-libs#485): a word with no letter in the dictionary's scripts is
    /// in another script (a Cyrillic word in a Dutch run); a word in its script is always checked,
    /// even with letters no dictionary word uses (CodeRabbit on #496: `zzzz`).
    #[test]
    fn words_in_another_script_are_not_the_dictionarys() {
        let nl = AffixDict::from_hunspell("SET UTF-8\n", "3\nhuis\nkat\nÉén\n", &[]).unwrap();
        let latin = scripts(&nl);
        assert_eq!(latin.len(), 1);
        for w in ["привет", "Привет", "αβγ", "שלום", "日本"] {
            assert!(other_script(w, &latin), "{w}");
        }
        for w in ["huus", "kät", "huisп", "ÉÉN", "zzzz", "qxqw", "ŋʒ"] {
            assert!(!other_script(w, &latin), "{w}");
        }
        assert!(!other_script("привет", &[]), "no scripts: every word is checked");
        let ru = AffixDict::from_hunspell("SET UTF-8\n", "1\nдом\n", &[]).unwrap();
        assert!(!other_script("щщщ", &scripts(&ru)) && other_script("dom", &scripts(&ru)));
        assert_ne!(script_of('ა'), script_of('a'));
        assert_eq!(script_of('ă'), script_of('Ạ'));
    }

    #[test]
    fn locales_and_file_names() {
        assert_eq!(l("nl-NL").as_str(), "nl-nl");
        assert_eq!(l("fr_FR.UTF-8").as_str(), "fr-fr");
        assert_eq!(l("ca-ES-valencia").as_str(), "ca-es-valencia");
        assert_eq!(l("sr-Latn-RS-x-private").as_str(), "sr-latn-rs");
        assert_eq!(l("zh-Hant-TW-somethinglong").as_str(), "zh-hant-tw");
        for bad in ["x-none", "i-klingon", "und", "zxx", "mul", "e", "-", "", "1x", "näh"] {
            assert_eq!(PackLocale::new(bad), None, "{bad}");
        }
        assert_eq!(names(l("nl")), ["nl"]);
        assert_eq!(names(l("fr-FR")), ["fr_FR", "fr-FR", "fr"]);
        assert_eq!(names(l("sr-Latn")), ["sr_Latn", "sr-Latn"]);
        assert_eq!(names(l("sr-Latn-RS")), ["sr_Latn_RS", "sr-Latn-RS", "sr_Latn", "sr-Latn"]);
        assert_eq!(names(l("sr-Cyrl-RS")), ["sr_Cyrl_RS", "sr-Cyrl-RS", "sr_Cyrl", "sr-Cyrl", "sr_RS", "sr-RS", "sr"]);
        assert_eq!(names(l("bs-Latn-BA")), ["bs_Latn_BA", "bs-Latn-BA", "bs_Latn", "bs-Latn", "bs_BA", "bs-BA", "bs"]);
        assert_eq!(names(l("sr")), ["sr", "sr_Cyrl", "sr-Cyrl"]);
        // A four-character subtag that isn't letters is no script.
        assert_eq!(names(l("de-1996")), ["de_1996", "de-1996", "de"]);
        assert_eq!(hyphenation_locales(l("fr-CA")), ["fr-ca", "fr"]);
        assert_eq!(hyphenation_locales(l("sr-Latn")), ["sr-latn"]);
        assert_eq!(hyphenation_locales(l("sr-Cyrl")), ["sr-cyrl", "sr"]);
    }
}
