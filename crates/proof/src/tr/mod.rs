//! Turkish proofing.
//!
//! - **Spelling** against the Hunspell `tr_TR` dictionary installed on the system
//!   ([`dictionary`], found by [`crate::hunspell`]: `tr_TR.aff`/`tr_TR.dic` from the OS, a
//!   package manager or LibreOffice; it is not bundled). Without it Turkish text gets no spelling
//!   marks. Capitalized stems are proper nouns (`Ankara`); foreign names inflected with an apostrophe
//!   are accepted (`WordCraft'ta`), and the user dictionary applies as in English.
//! - **Suggestions** tuned for Turkish ([`mod@suggest`]): ASCII-diacritic restoration (`turkce` -> `Türkçe`),
//!   common confusions (`yalnış` -> `yanlış`), detached particles (`geldimi` -> `geldi mi`), and typos.
//! - **Grammar** ([`grammar`]): high-precision rules (reduplications, comma around conjunctions,
//!   sentence start with abbreviations).
//! - **Hyphenation** ([`hyphen`]): syllable division by Turkish Language Association (TDK) rules.

pub mod dict;
pub mod grammar;
pub mod hyphen;
pub mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::Arc;

use crate::slot::DictSlot;

pub use dict::TrDict;
pub use suggest::suggest;

/// File names of a Turkish Hunspell dictionary.
const NAMES: &[&str] = &["tr_TR", "tr-TR", "tr"];

/// The Turkish dictionary, found on first use (see [`DictSlot`]).
pub(crate) static DICT: DictSlot<TrDict> = DictSlot::new(NAMES, "Turkish");

/// The Turkish dictionary: the installed Hunspell `tr_TR` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available.
pub fn dictionary() -> Arc<TrDict> {
    DICT.get()
}

/// Read a Turkish Hunspell pair from disk: size-capped and decoded per the `.aff`'s `SET` line,
/// like the other languages' dictionaries (#3661).
pub(crate) fn load(aff: &std::path::Path, dic: &std::path::Path) -> Result<TrDict, String> {
    let (aff_text, dic_text) = crate::hunspell::decode(&crate::hunspell::read_capped(aff)?, &crate::hunspell::read_capped(dic)?)?;
    TrDict::from_hunspell(&aff_text, &dic_text)
}

/// Use this Turkish dictionary from now on (Hunspell `.aff` and `.dic` text). On error the
/// current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    DICT.install(aff, dic)
}

/// Is a Turkish dictionary available?
pub fn has_dictionary() -> bool {
    DICT.has_dictionary()
}

/// Is `word` spelled correctly in Turkish?
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] with the File › Options › Proofing choices: with Ignore words that contain
/// numbers off, words with digits are checked against the dictionary too.
pub fn is_correct_with_options(word: &str, opts: &crate::Options) -> bool {
    check(&dictionary(), word, opts)
}

/// [`is_correct`] against a given dictionary.
pub fn is_correct_with(d: &TrDict, word: &str) -> bool {
    check(d, word, &crate::Options::default())
}

/// [`is_correct_with_options`] against a given dictionary.
fn check(d: &TrDict, word: &str, opts: &crate::Options) -> bool {
    if d.is_empty() {
        return true; // No Turkish dictionary installed: no spelling marks
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
    let w = w.trim_matches(['\'', '’']);
    // One letter, no letters, digits, capitals (Ignore words in UPPERCASE) or the user dictionary.
    if crate::accepted_by_options(w, opts) || crate::in_user_dictionary(&suggest::turkish_to_lower(w)) {
        return true;
    }

    // Direct lookup or lowercase lookup
    if d.contains(w) {
        return true;
    }
    let lower = suggest::turkish_to_lower(w);
    if d.contains(&lower) {
        return true;
    }

    // Handle apostrophe for proper nouns (e.g., `Ankara'da`, `WordCraft'ta`)
    if let Some((stem, _suffix)) = w.split_once(['\'', '’'])
        && (d.contains(stem) || d.contains(&suggest::turkish_to_lower(stem)))
    {
        return true;
    }

    // Compound words separated by hyphens (e.g., `afyon-karahisar`)
    if w.contains('-') && w.split('-').all(|part| part.is_empty() || check(d, part, opts)) {
        return true;
    }

    false
}
