//! Indonesian proofing using the installed Hunspell `id_ID` dictionary (never bundled; Debian and
//! Ubuntu ship it as `hunspell-id`, LibreOffice as its `dict-id` extension). Suggestions use the
//! dictionary's own MAP/REP/TRY/KEY tables. Grammar is limited to generic repeated-word and
//! spacing checks with Indonesian messages; hyphenation keeps the English fallback (no Indonesian
//! patterns are bundled).

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::Arc;

use crate::affix::AffixDict;
use crate::slot::DictSlot;

pub use suggest::suggest;

/// File names of an Indonesian Hunspell dictionary (`in` is the old ISO 639 code, still used by
/// Java locales).
const NAMES: &[&str] = &["id_ID", "id-ID", "id", "in_ID"];

/// The Indonesian dictionary, found on first use (see [`DictSlot`]).
pub(crate) static DICT: DictSlot<AffixDict> = DictSlot::new(NAMES, "Indonesian");

/// The Indonesian dictionary: the installed Hunspell `id_ID` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Indonesian spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    DICT.get()
}

/// Use this Indonesian dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    DICT.install(aff, dic)
}

/// Is an Indonesian dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    DICT.has_dictionary()
}

/// Clitics PUEBI joins with a hyphen to a capitalized word, an abbreviation or a number
/// (`se-Indonesia`, `KTP-nya`, `Jakarta-lah`); they aren't dictionary words on their own.
const CLITICS: &[&str] = &["se", "nya", "ku", "mu", "lah", "kah", "pun"];

/// Is `word` spelled correctly in Indonesian? Numbers, single letters, short words in capitals
/// (acronyms: `DPR`, `KTP`), words with digits and words in other scripts are accepted, and so
/// is every word when no Indonesian dictionary is installed. A hyphenated word is correct when
/// the dictionary has it whole (`kupu-kupu`, `sayur-mayur`) or has each part (`anak-anak`,
/// `rumah-rumah`); a hyphenated clitic (`KTP-nya`, `se-Indonesia`) counts as known.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] with the File › Options › Proofing choices: with Ignore words in UPPERCASE off,
/// short words in capitals are checked against the dictionary too, and with Ignore words that
/// contain numbers off, so are words with digits.
pub fn is_correct_with_options(word: &str, opts: &crate::Options) -> bool {
    check(&dictionary(), word, opts)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    check(d, word, &crate::Options::default())
}

/// [`is_correct_with_options`] against a given dictionary.
fn check(d: &AffixDict, word: &str, opts: &crate::Options) -> bool {
    let Some(w) = crate::spelling::word_to_check(d, word, opts) else {
        return true;
    };
    if w.contains('-') {
        return known(d, w)
            || w.split('-').all(|p| {
                p.chars().count() <= 1
                    || CLITICS.iter().any(|c| p.eq_ignore_ascii_case(c))
                    || (opts.ignore_uppercase && crate::spelling::capitals_part(p, 6))
                    || known(d, p)
            });
    }
    known(d, w)
}

/// Is one word in the dictionary or the user dictionary?
fn known(d: &AffixDict, w: &str) -> bool {
    d.check(w) || crate::in_user_dictionary(&w.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indonesian_words_affixes_case_and_hyphens() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert_eq!(d.len(), 20);
        assert!(d.rule_count() > 0);
        for w in [
            "rumah",
            "makan",
            "makanan",
            "dimakan",
            "membaca",
            "bacaan",
            "dibaca",
            "Jakarta",
            "Rumah",
            "SEKOLAH",
            "anak-anak",
            "rumah-rumah",
            "kupu-kupu",
            "KTP-nya",
            "se-Indonesia",
            "Jakarta-lah",
            "DPR",
        ] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["rumha", "makanann", "skolah", "jakarta", "kupu", "anak-anka", "mAkan", "bukunya-xyz"] {
            assert!(!is_correct(w), "{w}");
        }
    }
}
