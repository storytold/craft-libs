//! Telling German and English apart, so a paragraph whose runs say English but whose words are
//! German (text pasted from an e-mail, a document written with Word's `en-US` on every run) is
//! checked in German instead of being covered in red underlines, and the other way round (as
//! Word's "detect language automatically" does for Latin-script text). [`crate::vi`] does the same
//! for Vietnamese by its letters; German and English share their letters, so this test reads the
//! words.
//!
//! The test is lexical: of a paragraph's words in one of the two languages, those only the
//! tagged language's dictionary knows are counted against those only the other language's
//! dictionary knows. Words both know (`in`, `was`, `Hotel`), words neither knows (names,
//! misspellings, technical terms) and words every check accepts ([`crate::accepted_by_options`]:
//! single letters, numbers, capitals, the user dictionary) say nothing either way. The paragraph
//! is the other language when at least two of its words are only that language's and they
//! outnumber the tagged language's own words more than two to one: an English paragraph quoting
//! one German word (`the Zeitgeist`, `Ich bin ein Berliner`) stays English. Nothing changes
//! without the other language's dictionary (German is a Language pack, or a `de_DE` Hunspell
//! dictionary).

use crate::{Options, ProofLang};
use std::ops::Range;

/// Most words of each language a paragraph is judged by: enough to decide, and a very long
/// paragraph costs no more than a short one.
const SAMPLE: usize = 64;

/// The language a run tagged `lang` may really be written in, if detection covers `lang`.
fn counterpart(lang: ProofLang) -> Option<ProofLang> {
    if lang.is_english() {
        Some(ProofLang::De)
    } else if lang == ProofLang::De {
        Some(ProofLang::En)
    } else {
        None
    }
}

/// Can `lang`'s words be looked up? English always can (its built-in word list); another
/// language only with its dictionary installed, as without one every word counts as correct.
fn has_words(lang: ProofLang) -> bool {
    crate::dictionary_loaded(lang) != Some(false)
}

/// The language of byte `i` in `spans` (sorted, not overlapping), `default` outside them.
fn lang_in(spans: &[(Range<usize>, ProofLang)], default: ProofLang, i: usize) -> ProofLang {
    let k = spans.partition_point(|(r, _)| r.end <= i);
    spans.get(k).filter(|(r, _)| r.start <= i).map_or(default, |s| s.1)
}

/// Words counted for one tagged language: those only it knows, those only its counterpart knows.
#[derive(Default)]
struct Tally {
    seen: usize,
    own: usize,
    other: usize,
}

impl Tally {
    /// Do the counted words read as the counterpart language?
    fn switches(&self) -> bool {
        self.other >= 2 && self.other > self.own.saturating_mul(2)
    }
}

/// Re-label the English and German parts of a paragraph that are written in the other language:
/// `spans` (sorted, not overlapping) and `default` (the language of text outside them) are
/// changed in place. English runs of every variety become [`ProofLang::De`]; German runs become
/// [`ProofLang::En`].
pub(crate) fn relabel(text: &str, spans: &mut [(Range<usize>, ProofLang)], default: &mut ProofLang, opts: &Options) {
    // English runs (any variety) and German runs are judged separately.
    let mut tally = [Tally::default(), Tally::default()];
    for (a, b) in crate::words(text) {
        let lang = lang_in(spans, *default, a);
        let Some(other) = counterpart(lang) else { continue };
        let t = if lang.is_english() { &mut tally[0] } else { &mut tally[1] };
        if t.seen >= SAMPLE {
            continue;
        }
        let Some(w) = text.get(a..b) else { continue };
        if crate::accepted_by_options(w, opts) {
            continue;
        }
        t.seen += 1;
        match (crate::is_correct_in_with(w, lang, opts), crate::is_correct_in_with(w, other, opts)) {
            (true, false) => t.own += 1,
            (false, true) => t.other += 1,
            _ => {}
        }
    }
    let english = tally[0].switches() && has_words(ProofLang::De);
    let german = tally[1].switches() && has_words(ProofLang::En);
    if !english && !german {
        return;
    }
    let swap = |l: ProofLang| match l {
        l if english && l.is_english() => ProofLang::De,
        ProofLang::De if german => ProofLang::En,
        l => l,
    };
    for (_, l) in spans.iter_mut() {
        *l = swap(*l);
    }
    *default = swap(*default);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paragraph_needs_two_words_and_a_clear_majority() {
        let t = |own, other| Tally { seen: own + other, own, other }.switches();
        assert!(t(0, 2) && t(1, 3) && t(3, 7));
        assert!(!t(0, 1), "one word is a quotation");
        assert!(!t(1, 2) && !t(3, 6), "a mixed paragraph keeps its language");
    }

    #[test]
    fn only_english_and_german_are_detected() {
        assert_eq!(counterpart(ProofLang::EnGb), Some(ProofLang::De));
        assert_eq!(counterpart(ProofLang::De), Some(ProofLang::En));
        assert_eq!(counterpart(ProofLang::Pl), None);
        assert_eq!(counterpart(ProofLang::Other), None);
    }

    #[test]
    fn spans_are_looked_up_with_the_default_between_them() {
        let spans = [(0..4, ProofLang::De), (6..9, ProofLang::Pl)];
        assert_eq!(lang_in(&spans, ProofLang::En, 3), ProofLang::De);
        assert_eq!(lang_in(&spans, ProofLang::En, 4), ProofLang::En);
        assert_eq!(lang_in(&spans, ProofLang::En, 6), ProofLang::Pl);
        assert_eq!(lang_in(&spans, ProofLang::En, 20), ProofLang::En);
    }
}
