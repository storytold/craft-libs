//! The user dictionary and the Ignore All list: in-memory words, the custom-dictionary file.

use super::{DICTIONARY_GENERATION, composed};
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{OnceLock, RwLock};

fn user_words() -> &'static RwLock<HashSet<String>> {
    static U: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();
    U.get_or_init(|| RwLock::new(HashSet::new()))
}

/// The key a word has in the user dictionary and the Ignore All list: lower case and composed
/// (NFC), as spelling looks words up (`composed`). A word added, ignored, removed or loaded
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
pub(super) fn read_dictionary_file(path: &std::path::Path) -> Vec<String> {
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
