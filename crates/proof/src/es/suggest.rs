//! Spanish suggestions. The ranking is the shared dictionary-driven one
//! ([`crate::affix_suggest`]: case, MAP — the Spanish dictionaries map each vowel to
//! its accented forms and `n` to `ñ`, so `tambien` → `también` comes first —, REP, missing spaces,
//! KEY, TRY edits and a stem scan); every table comes from the variety's installed dictionary.
//! In Uruguayan Spanish a `vos` form typed without its accent is offered with it first
//! (`veni` → `vení`, `tenes` → `tenés`; [`super::voseo`]).

use crate::ProofLang;
use crate::affix::{AffixDict, Casing, capitalize, casing};
use crate::affix_suggest::{MAX_LETTERS, Tuning};

/// Spelling suggestions for a word in `lang`'s Spanish, best first, at most `max`, in the word's
/// case.
pub fn suggest(word: &str, max: usize, lang: ProofLang) -> Vec<String> {
    suggest_in(&super::dictionary(lang), word, max, lang)
}

/// [`suggest`] against a given dictionary.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize, lang: ProofLang) -> Vec<String> {
    let mut out = Vec::new();
    let word = word.trim();
    if lang == ProofLang::EsUy
        && !d.is_empty()
        && word.chars().count() <= MAX_LETTERS
        && let Some(v) = super::voseo::accented(&word.to_lowercase(), |inf| d.check(inf))
    {
        out.push(match casing(word) {
            Casing::Upper => v.to_uppercase(),
            Casing::Initial => capitalize(&v),
            _ => v,
        });
    }
    for s in crate::affix_suggest::suggest_in(d, word, max, &Tuning::DEFAULT) {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out.truncate(max);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uruguayan_vos_forms_get_their_accent_back_first() {
        crate::es::test_dict::install();
        for (word, want) in [("veni", "vení"), ("tenes", "tenés"), ("Veni", "Vení"), ("TENES", "TENÉS"), ("queres", "querés")] {
            assert_eq!(suggest(word, 4, ProofLang::EsUy).first().map(String::as_str), Some(want), "{word}");
            assert!(!suggest(word, 8, ProofLang::Es419).contains(&want.into()), "{word}: no vos forms outside Uruguay");
        }
    }
}
