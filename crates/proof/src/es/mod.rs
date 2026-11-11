//! Spanish proofing (#3360, #3553) using the installed Hunspell dictionaries (never bundled;
//! LibreOffice ships them as its `dict-es` extension, Debian and Ubuntu as `hunspell-es`). Each
//! variety reads its own dictionary: Spain ([`ProofLang::Es`]) `es_ES`, Mexico
//! ([`ProofLang::EsMx`]) `es_MX`, Latin America ([`ProofLang::Es419`]) `es_419` and Uruguay
//! ([`ProofLang::EsUy`]) `es_UY`, each falling back to a nearby or generic one. Uruguayan Spanish
//! also accepts the `vos` forms of every verb in the dictionary ([`voseo`]: `tenés`, `vení`,
//! `decime`). Suggestions use the dictionary's own MAP/REP/TRY/KEY tables (the regional
//! dictionaries' REP tables carry their differences, such as `vosotros` forms in Mexico).
//! Grammar is limited to generic repeated-word and spacing checks and the Spanish opening
//! question and exclamation marks, with Spanish messages; hyphenation keeps the English fallback
//! (no Spanish patterns are bundled).

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;
pub mod voseo;

use std::sync::Arc;

use crate::ProofLang;
use crate::affix::AffixDict;
use crate::slot::DictSlot;

pub use suggest::suggest;

/// The dictionary of each variety, found on first use (see [`DictSlot`]).
pub(crate) static SPAIN_DICT: DictSlot<AffixDict> = DictSlot::new(SPAIN, "Spanish (Spain)");
pub(crate) static MEXICO_DICT: DictSlot<AffixDict> = DictSlot::new(MEXICO, "Spanish (Mexico)");
pub(crate) static LATIN_AMERICA_DICT: DictSlot<AffixDict> = DictSlot::new(LATIN_AMERICA, "Spanish (Latin America)");
pub(crate) static URUGUAY_DICT: DictSlot<AffixDict> = DictSlot::new(URUGUAY, "Spanish (Uruguay)");

/// File names of a Spanish Hunspell dictionary for Spain, most specific first (`es_ANY` is the
/// generic one RLA-ES publishes).
const SPAIN: &[&str] = &["es_ES", "es-ES", "es_ANY", "es"];
/// For Mexico: its own, then a Latin American or generic one.
const MEXICO: &[&str] = &["es_MX", "es-MX", "es_419", "es-419", "es_ANY", "es"];
/// For Latin America: its own or a generic one, then the United States' and Mexico's, which
/// LibreOffice ships (it has no `es_419`) and which use Latin American spelling, then
/// Argentina's, so `es-AR` text is checked when that is the only one installed.
const LATIN_AMERICA: &[&str] = &["es_419", "es-419", "es_ANY", "es", "es_US", "es_MX", "es_AR", "es-AR"];
/// For Uruguay: its own, then Argentina's (the same Rioplatense Spanish), then a Latin American
/// or generic one.
const URUGUAY: &[&str] = &["es_UY", "es-UY", "es_AR", "es-AR", "es_419", "es-419", "es_ANY", "es"];

/// `lang`'s variety's dictionary; Spain's for any other language.
fn slot(lang: ProofLang) -> &'static DictSlot<AffixDict> {
    match lang {
        ProofLang::EsMx => &MEXICO_DICT,
        ProofLang::Es419 => &LATIN_AMERICA_DICT,
        ProofLang::EsUy => &URUGUAY_DICT,
        _ => &SPAIN_DICT,
    }
}

/// The Hunspell name each variety's dictionary is known by (`es_ES`, `es_MX`, `es_419`,
/// `es_UY`): the first of its file names, used for the status-bar hint and load errors.
pub fn dictionary_name(lang: ProofLang) -> &'static str {
    slot(lang).name()
}

/// The Spanish dictionary of `lang`'s variety: the installed one (read on first use), one given
/// with [`install_dictionary`], or an empty one when none is available (spelling is off then;
/// see [`has_dictionary`]).
pub fn dictionary(lang: ProofLang) -> Arc<AffixDict> {
    slot(lang).get()
}

/// Use this dictionary for `lang`'s variety from now on (Hunspell `.aff` and `.dic` text,
/// decoded to UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(lang: ProofLang, aff: &str, dic: &str) -> Result<(), String> {
    slot(lang).install(aff, dic)
}

/// Is a dictionary available for `lang`'s variety (spelling is checked)?
pub fn has_dictionary(lang: ProofLang) -> bool {
    slot(lang).has_dictionary()
}

/// Is `word` spelled correctly in `lang`'s Spanish? Numbers, single letters, short words in
/// capitals (acronyms: `ONU`, `CDMX`), words with digits and words in other scripts are accepted,
/// and so is every word when no dictionary is installed for the variety. Hyphenated compounds
/// are checked part by part (`teórico-práctico`, `franco-mexicano`). In Uruguayan Spanish the
/// `vos` forms of the dictionary's verbs are correct too ([`voseo`]).
pub fn is_correct(word: &str, lang: ProofLang) -> bool {
    is_correct_with(&dictionary(lang), word, lang)
}

/// [`is_correct`] with the File › Options › Proofing choices: with Ignore words in UPPERCASE off,
/// short words in capitals are checked against the dictionary too, and with Ignore words that
/// contain numbers off, so are words with digits.
pub fn is_correct_with_options(word: &str, lang: ProofLang, opts: &crate::Options) -> bool {
    check(&dictionary(lang), word, lang, opts)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str, lang: ProofLang) -> bool {
    check(d, word, lang, &crate::Options::default())
}

/// [`is_correct_with_options`] against a given dictionary.
fn check(d: &AffixDict, word: &str, lang: ProofLang, opts: &crate::Options) -> bool {
    let Some(w) = crate::spelling::word_to_check(d, word, opts) else {
        return true;
    };
    if w.contains('-') {
        return known(d, w, lang) || w.split('-').all(|p| p.chars().count() <= 1 || known(d, p, lang));
    }
    known(d, w, lang)
}

/// Is one word in the dictionary or the user dictionary, or (in Uruguayan Spanish) the `vos`
/// form of one of the dictionary's verbs?
fn known(d: &AffixDict, w: &str, lang: ProofLang) -> bool {
    d.check(w) || crate::in_user_dictionary(&w.to_lowercase()) || (lang == ProofLang::EsUy && voseo::is_vos_form(w, |inf| d.check(inf)))
}
