//! The Devanagari pieces of proofing, apart from any one language so that every language written
//! in Devanagari (Hindi now) can share them: the script's letters and digits, the two spellings of
//! its nukta letters, the spelling check against a Hunspell dictionary and the generic checks
//! (repeated words, spacing) with each language's messages.

use crate::affix::AffixDict;
use crate::{Issue, IssueKind, ProofLang, words};

/// Is `c` in the Devanagari block (U+0900..=U+097F)?
pub fn is_devanagari_char(c: char) -> bool {
    ('\u{0900}'..='\u{097F}').contains(&c)
}

/// Is `c` a Devanagari digit (`०`..=`९`, U+0966..=U+096F)?
pub fn is_devanagari_digit(c: char) -> bool {
    ('\u{0966}'..='\u{096F}').contains(&c)
}

/// Is `c` part of a Devanagari word: a letter, vowel sign, virama, nukta or other mark of the
/// block? Digits, the danda (`।`, `॥`) and the abbreviation sign (`॰`) are not.
pub fn is_devanagari_letter(c: char) -> bool {
    is_devanagari_char(c) && !is_devanagari_digit(c) && !matches!(c, '\u{0964}' | '\u{0965}' | '\u{0970}')
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

/// Is `word` spelled correctly against `d`, the dictionary of a language written in Devanagari?
/// Numbers (ASCII or Devanagari digits), single letters, words with digits (when `opts` ignores
/// them) and words in other scripts are accepted, and so is every word when `d` is empty.
pub(crate) fn is_correct(d: &AffixDict, word: &str, opts: &crate::Options) -> bool {
    if d.is_empty() {
        return true;
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && !is_devanagari_letter(c) && !matches!(c, '\'' | '’' | '-'));
    let w = w.trim_matches(['\'', '’', '-']);
    let letter_count = w.chars().filter(|&c| is_devanagari_letter(c) || c.is_alphabetic()).count();
    if letter_count <= 1 || (opts.ignore_numbers && w.chars().any(char::is_numeric)) {
        return true;
    }
    // A word in another script (or with no Devanagari letters) is not marked by this dictionary.
    if !w.chars().any(is_devanagari_letter) {
        return true;
    }
    if w.contains('-') {
        // The whole word first, as Hunspell does: a hyphenated entry or user word.
        return known(d, w) || w.split('-').all(|p| p.chars().count() <= 1 || !p.chars().any(is_devanagari_letter) || known(d, p));
    }
    known(d, w)
}

/// Is one word (no hyphens) in the dictionary or the user dictionary, with its nukta letters
/// spelled either way ([`nukta_variant`])?
fn known(d: &AffixDict, w: &str) -> bool {
    let listed = |w: &str| d.check(w) || crate::in_user_dictionary(&w.to_lowercase());
    listed(w) || nukta_variant(w).is_some_and(|v| listed(&v))
}

/// A language's messages and choices for [`check`].
pub(crate) struct Rules {
    /// The language the issues are in.
    pub(crate) lang: ProofLang,
    /// A repeated word's message, before `: “word”`.
    pub(crate) repeated_word: &'static str,
    /// A space before punctuation.
    pub(crate) space_before_punctuation: &'static str,
    /// Two or more spaces.
    pub(crate) double_space: &'static str,
    /// Is a word written twice in a row (`के के`) a likely mistake? Languages that double words
    /// on purpose (`धीरे धीरे`, `घर घर`) flag only the words that never are.
    pub(crate) flag_repeat: fn(&str) -> bool,
}

/// Repeated words and spacing problems in `text`, with `rules`' messages. A space before `,`,
/// `;`, `:`, `!`, `?` or `.` is flagged; one before the danda (`।`, `॥`) is not, since text
/// sets it both ways.
pub(crate) fn check(text: &str, rules: &Rules) -> Vec<Issue> {
    let mut issues = Vec::new();
    repeated_words(text, &words(text), rules, &mut issues);
    spaces(text, rules, &mut issues);
    issues.sort_by_key(|i| (i.start, i.end));
    issues
}

fn issue(rules: &Rules, start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: rules.lang }
}

/// An ASCII or Devanagari digit.
fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || is_devanagari_digit(c)
}

fn repeated_words(text: &str, ws: &[(usize, usize)], rules: &Rules, v: &mut Vec<Issue>) {
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if !gap.is_empty()
            && gap.chars().all(char::is_whitespace)
            && w0.chars().all(|c| c.is_alphabetic() || is_devanagari_letter(c))
            && w0 == w1
            && (rules.flag_repeat)(w0)
        {
            v.push(issue(rules, *a0, *b1, format!("{}: “{w1}”", rules.repeated_word), vec![w0.to_string()]));
        }
    }
}

fn spaces(text: &str, rules: &Rules, v: &mut Vec<Issue>) {
    for (i, c) in text.char_indices() {
        if c != ' ' {
            continue;
        }
        let rest = text.get(i + 1..).unwrap_or("");
        let mut it = rest.chars();
        let Some(p) = it.next() else { continue };
        let after = it.next();
        // A ratio/score keeps the spaces in digit-space-colon-space-digit (ASCII or Devanagari digits).
        let ratio = p == ':'
            && text.get(..i).and_then(|before| before.chars().next_back()).is_some_and(is_digit)
            && after == Some(' ')
            && it.next().is_some_and(is_digit);
        let flag = !ratio
            && match p {
                ',' => true,
                ';' => !matches!(after, Some(')' | '(' | '-' | 'P' | 'D')),
                '.' => !matches!(after, Some('.')) && !after.is_some_and(is_digit),
                ':' | '!' | '?' => after.is_none_or(|a| a.is_whitespace() || matches!(a, '"' | '”' | '’' | '!' | '?')),
                _ => false,
            };
        if flag {
            v.push(issue(rules, i, i + 1 + p.len_utf8(), rules.space_before_punctuation.into(), vec![p.to_string()]));
        }
    }
    for run in crate::space_runs(text) {
        v.push(issue(rules, run.start, run.end, rules.double_space.into(), vec![" ".into()]));
    }
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
