//! What the proofing of a language written in Devanagari (Hindi now) needs to know about the
//! script, for the shared checks ([`crate::script_checks`]): its letters (as the akshara rules
//! classify them, [`crate::indic::Script::Devanagari`]), its digits and the two spellings of its
//! nukta letters.

use crate::indic;
use crate::script_checks::Script;

/// Devanagari for the shared checks.
pub(crate) const SCRIPT: Script =
    Script { in_words: is_letter, is_letter, is_digit: is_devanagari_digit, variant: Some(nukta_variant as fn(&str) -> Option<String>) };

fn is_letter(c: char) -> bool {
    indic::Script::Devanagari.is_letter(c)
}

/// Is `c` a Devanagari digit (`०`..=`९`, U+0966..=U+096F)?
fn is_devanagari_digit(c: char) -> bool {
    ('\u{0966}'..='\u{096F}').contains(&c)
}

/// The consonants with a precomposed nukta letter (U+0958..=U+095F): the base consonant, then
/// the precomposed letter.
const NUKTA_LETTERS: [(char, char); 8] = [
    ('\u{0915}', '\u{0958}'),
    ('\u{0916}', '\u{0959}'),
    ('\u{0917}', '\u{095A}'),
    ('\u{091C}', '\u{095B}'),
    ('\u{0921}', '\u{095C}'),
    ('\u{0922}', '\u{095D}'),
    ('\u{092B}', '\u{095E}'),
    ('\u{092F}', '\u{095F}'),
];

/// The nukta sign (U+093C).
const NUKTA: char = '\u{093C}';

/// `w` with its nukta letters spelled the other way, if it has any: precomposed ones (`क़`,
/// U+0958) as consonant and nukta (`क` + `़`), else consonant-and-nukta pairs precomposed.
/// Composition (NFC) always gives the pair, since the precomposed letters are excluded from it,
/// but a dictionary may list either spelling.
pub(crate) fn nukta_variant(w: &str) -> Option<String> {
    let precomposed = |c: char| NUKTA_LETTERS.iter().find(|&&(_, p)| p == c).map(|&(base, _)| base);
    if w.chars().any(|c| precomposed(c).is_some()) {
        let mut out = String::with_capacity(w.len() + 4);
        for c in w.chars() {
            match precomposed(c) {
                Some(base) => {
                    out.push(base);
                    out.push(NUKTA);
                }
                None => out.push(c),
            }
        }
        return Some(out);
    }
    let mut out = String::with_capacity(w.len());
    let mut changed = false;
    let mut chars = w.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(&(_, p)) = NUKTA_LETTERS.iter().find(|&&(base, _)| base == c)
            && chars.peek() == Some(&NUKTA)
        {
            chars.next();
            out.push(p);
            changed = true;
        } else {
            out.push(c);
        }
    }
    changed.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nukta_letters_have_two_spellings() {
        // ज़मीन precomposed and as ज + nukta; सड़क the same.
        assert_eq!(nukta_variant("\u{095B}मीन").as_deref(), Some("ज\u{093C}मीन"));
        assert_eq!(nukta_variant("ज\u{093C}मीन").as_deref(), Some("\u{095B}मीन"));
        assert_eq!(nukta_variant("सड\u{093C}क").as_deref(), Some("स\u{095C}क"));
        // A nukta on a consonant with no precomposed letter, and no nukta at all: one spelling.
        assert_eq!(nukta_variant("स\u{093C}"), None);
        assert_eq!(nukta_variant("किताब"), None);
    }
}
