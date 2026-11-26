//! Dictionaries handed over as bytes, for hosts that have no folder to put files in (the web, where
//! a dictionary is fetched) or that keep their dictionaries elsewhere.
//!
//! [`register_dictionary`] takes a language tag and the bytes of a Hunspell `.aff`/`.dic` pair, for
//! any language, the Englishes included. The pair behaves as `<tag>.aff` and `<tag>.dic` in a folder
//! that comes after the dictionaries folders ([`crate::set_dictionaries_folder`], the host's own:
//! [`crate::set_host_folders`]) and before the installed Language packs, the system's dictionaries
//! and the embedded English. It survives a reload of the folders or the packs, and stays until it is
//! replaced or [`unregister_dictionary`] removes it.
//!
//! A registered pair is **charged to the memory budget** ([`crate::pack_lang`]): its estimate (six
//! times the bytes of the two files, which covers the text kept and the parsed dictionary) counts
//! against the budget with the dictionaries loaded from packs, and those give way to it. A pair is
//! **refused, with the reason**, when its tag names no language, a file is over
//! [`MAX_FILE_BYTES`], it can't be decoded or holds no words, the registered pairs would pass the
//! budget, or [`MAX_REGISTERED`] languages are registered already.
//!
//! The pair is parsed on first use, as a file is: one that decodes but doesn't parse is registered,
//! reported by [`crate::dictionary_load_error`], and its language falls back to the next source.

use std::path::PathBuf;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::lang::{PackLocale, ProofLang};

/// Largest `.aff` or `.dic` accepted, in bytes: the cap of the dictionary files read from disk.
pub const MAX_FILE_BYTES: usize = crate::hunspell::MAX_FILE as usize;

/// Most languages registered at once.
pub const MAX_REGISTERED: usize = 64;

/// Why [`register_dictionary`] refused a pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterError {
    /// The tag names no language: `x-none`, `und`, `zxx`, an empty or a malformed tag.
    NoLanguage,
    /// A file is longer than the cap ([`MAX_FILE_BYTES`]).
    TooLarge {
        /// `aff` or `dic`.
        file: &'static str,
        /// The file's length.
        bytes: usize,
        /// The cap.
        limit: usize,
    },
    /// The pair can't be decoded (an encoding or bytes that aren't valid) or holds no words.
    Unreadable(String),
    /// The registered pairs, this one included, would take more memory than the budget allows.
    OverBudget {
        /// The estimated memory of every registered pair with this one, in bytes.
        needed: u64,
        /// The budget in force, in bytes.
        budget: u64,
    },
    /// [`MAX_REGISTERED`] other languages are registered already.
    TooMany {
        /// The cap.
        limit: usize,
    },
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegisterError::NoLanguage => f.write_str("the tag names no language, so there is no dictionary to register it for"),
            RegisterError::TooLarge { file, bytes, limit } => {
                write!(f, "the .{file} file is too large: {bytes} bytes, at most {limit}")
            }
            RegisterError::Unreadable(reason) => f.write_str(reason),
            RegisterError::OverBudget { needed, budget } => {
                write!(f, "the registered dictionaries would take about {needed} bytes of memory, over the budget of {budget}")
            }
            RegisterError::TooMany { limit } => write!(f, "{limit} languages are registered already, the most there can be"),
        }
    }
}

impl std::error::Error for RegisterError {}

/// A registered pair, decoded to UTF-8 text.
#[derive(Clone)]
pub(crate) struct Pair {
    pub(crate) aff: Arc<str>,
    pub(crate) dic: Arc<str>,
    /// What the pair is called in load errors (there is no file).
    pub(crate) label: PathBuf,
}

struct Entry {
    key: PackLocale,
    pair: Pair,
    /// Estimated memory, charged to the budget.
    bytes: u64,
}

static REGISTRY: RwLock<Vec<Entry>> = RwLock::new(Vec::new());

fn read() -> RwLockReadGuard<'static, Vec<Entry>> {
    REGISTRY.read().unwrap_or_else(PoisonError::into_inner)
}

fn write() -> RwLockWriteGuard<'static, Vec<Entry>> {
    REGISTRY.write().unwrap_or_else(PoisonError::into_inner)
}

/// The key `tag` is registered under: the locale, except that an English is registered for the
/// English `tag` reads as (`en` is `en-US`, `en-NZ` is `en-GB`), so it is the one its checks look for.
fn key_of(tag: &str) -> Option<PackLocale> {
    if tag.trim().is_empty() {
        return None;
    }
    match ProofLang::from_tag(Some(tag)) {
        ProofLang::Other => None,
        lang if lang.is_english() => PackLocale::new(crate::english::registered_name(lang)),
        _ => PackLocale::new(tag),
    }
}

/// Whether a decoded `.dic` holds a word: a header number, then a line, or a line that isn't one.
fn has_words(dic: &str) -> bool {
    let mut lines = dic.lines().map(str::trim).filter(|l| !l.is_empty());
    match lines.next() {
        None => false,
        Some(first) if first.bytes().all(|b| b.is_ascii_digit()) => lines.next().is_some(),
        Some(_) => true,
    }
}

/// Would a pair of `bytes` estimated memory for `key` (replacing the one registered for it, if any)
/// keep the registered pairs within the budget?
fn fits(registry: &[Entry], key: PackLocale, bytes: u64) -> Result<(), RegisterError> {
    let others: u64 = registry.iter().filter(|e| e.key != key).map(|e| e.bytes).fold(0, u64::saturating_add);
    let needed = others.saturating_add(bytes);
    let budget = crate::pack_lang::budget();
    if needed > budget { Err(RegisterError::OverBudget { needed, budget }) } else { Ok(()) }
}

/// The estimated memory of a pair of `len` bytes of file.
fn estimate(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX).saturating_mul(crate::pack_lang::BYTES_PER_FILE_BYTE).max(1)
}

/// Register the Hunspell pair `aff` and `dic` (bytes, decoded by the `.aff`'s `SET` line) as the
/// dictionary of the language `tag`, replacing the pair registered for it before. See the module
/// documentation for where it ranks, what it costs and when it is refused.
///
/// The key is the language of the tag: `de` serves `de-DE`, `de-AT` and every German region that has
/// no pair of its own, as a file `de.dic` would; an English is registered under its own tag (`en-US`
/// and `en` are the same, `en-GB`, `en-NZ` and the other British-spelling Englishes are the
/// British English's, `en-CA` and `en-AU` their own).
///
/// Every dictionary loaded is read again afterwards ([`crate::dictionary_generation`] changes), so
/// register at start, or when a language is first needed.
///
/// # Errors
///
/// A [`RegisterError`] that says why the pair was not registered; nothing changes then.
pub fn register_dictionary(tag: &str, aff: &[u8], dic: &[u8]) -> Result<(), RegisterError> {
    let key = key_of(tag).ok_or(RegisterError::NoLanguage)?;
    for (file, bytes) in [("aff", aff.len()), ("dic", dic.len())] {
        if bytes > MAX_FILE_BYTES {
            return Err(RegisterError::TooLarge { file, bytes, limit: MAX_FILE_BYTES });
        }
    }
    // What can't fit the budget is refused before it is decoded: decoding copies the file.
    let bytes = estimate(aff.len().saturating_add(dic.len()));
    fits(&read(), key, bytes)?;
    let (aff_text, dic_text) = crate::hunspell::decode(aff, dic).map_err(RegisterError::Unreadable)?;
    if !has_words(&dic_text) {
        return Err(RegisterError::Unreadable("the dictionary holds no words".into()));
    }
    let pair = Pair { aff: aff_text.into(), dic: dic_text.into(), label: PathBuf::from(format!("registered dictionary {key}")) };
    {
        let mut registry = write();
        let at = registry.iter().position(|e| e.key == key);
        if at.is_none() && registry.len() >= MAX_REGISTERED {
            return Err(RegisterError::TooMany { limit: MAX_REGISTERED });
        }
        fits(&registry, key, bytes)?; // again, now that the registry is held
        let entry = Entry { key, pair, bytes };
        match at.and_then(|i| registry.get_mut(i)) {
            Some(slot) => *slot = entry,
            None => registry.push(entry),
        }
    }
    crate::reload_dictionaries();
    Ok(())
}

/// Remove the pair registered for `tag` (read as [`register_dictionary`] reads it); true when there
/// was one. The language goes back to its next source and text is checked again.
pub fn unregister_dictionary(tag: &str) -> bool {
    let Some(key) = key_of(tag) else { return false };
    let removed = {
        let mut registry = write();
        let before = registry.len();
        registry.retain(|e| e.key != key);
        registry.len() != before
    };
    if removed {
        crate::reload_dictionaries();
    }
    removed
}

/// The languages with a registered pair, as lower-case locales (`en-us`, `qr`), sorted.
pub fn registered_dictionaries() -> Vec<String> {
    let mut keys: Vec<String> = read().iter().map(|e| e.key.to_string()).collect();
    keys.sort();
    keys
}

/// The estimated memory of the registered pairs, which counts against the budget.
pub(crate) fn bytes() -> u64 {
    read().iter().map(|e| e.bytes).fold(0, u64::saturating_add)
}

/// The registered pair a file called any of `names` would be (`pl_PL`, `pl-PL`, `pl`), the first
/// name that has one winning.
pub(crate) fn find(names: &[&str]) -> Option<Pair> {
    let registry = read();
    if registry.is_empty() {
        return None;
    }
    names.iter().find_map(|n| {
        let key = PackLocale::new(n)?;
        registry.iter().find(|e| e.key == key).map(|e| e.pair.clone())
    })
}

/// Where a dictionary for `names` is found.
pub(crate) enum Located {
    /// A pair of files: its `.aff` and `.dic`.
    File(PathBuf, PathBuf),
    /// A registered pair.
    Registered(Pair),
    /// None.
    Missing,
}

/// Where the dictionary for `names` is: in the dictionaries folders first, then a registered
/// pair, then an installed pack or the system's folders ([`crate::hunspell::candidates`]'s order).
pub(crate) fn locate(names: &[&str]) -> Located {
    let (folders, rest) = crate::hunspell::candidate_tiers(names);
    if let Some((aff, dic)) = folders.into_iter().next() {
        return Located::File(aff, dic);
    }
    if let Some(pair) = find(names) {
        return Located::Registered(pair);
    }
    rest.into_iter().next().map_or(Located::Missing, |(aff, dic)| Located::File(aff, dic))
}

#[cfg(test)]
mod tests;
