//! Repeated words as a spelling option ([`Options::flag_repeated`](crate::Options::flag_repeated)):
//! two equal words, ignoring case, with only white space between them, in text of one language that
//! has proofing tools, give an [`IssueKind::Repeated`] issue on the second word. Numbers are not words
//! for it (a word needs a letter).
//!
//! The grammar rules have a repeated-word rule of their own (`repeated-word`,
//! [`crate::rules::REPEATED_WORD`]), tuned to each language's idioms (`tak tak`, `yavaş yavaş`) and
//! covering both words; this one is the plain option of a spelling check, and replaces the rule when
//! both are on.

use crate::{Issue, IssueKind, ProofLang};

/// The no-break space French puts inside quotation marks.
const NBSP: char = '\u{a0}';

/// The repeated-word issues of `text`, in order, each word in the language `lang_at` gives for its
/// first byte.
pub(crate) fn issues(text: &str, lang_at: &dyn Fn(usize) -> ProofLang) -> Vec<Issue> {
    let mut out = Vec::new();
    // Whether a language has proofing tools, asked once for each run of one language.
    let mut proofed: Option<(ProofLang, bool)> = None;
    for pair in crate::words(text).windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(first), Some(second), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if gap.is_empty() || !gap.chars().all(char::is_whitespace) || !second.chars().any(char::is_alphabetic) || !same_word(first, second) {
            continue;
        }
        let lang = lang_at(*a1);
        if lang_at(*a0) != lang {
            continue;
        }
        let has_tools = match proofed {
            Some((l, p)) if l == lang => p,
            _ => {
                let p = lang.is_proofed();
                proofed = Some((lang, p));
                p
            }
        };
        if has_tools {
            out.push(Issue {
                start: *a1,
                end: *b1,
                kind: IssueKind::Repeated,
                message: message(lang, second),
                suggestions: Vec::new(),
                lang,
                rule: None,
            });
        }
    }
    out
}

/// Are `a` and `b` the same word, ignoring case and the way an accent is written (composed or not)?
fn same_word(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || crate::composed(a).to_lowercase() == crate::composed(b).to_lowercase()
}

/// The issue's message for `word` in `lang`, in the language where its grammar rules have one, else in
/// English.
pub(crate) fn message(lang: ProofLang, word: &str) -> String {
    if lang.is_english() {
        return format!("Repeated word: \"{word}\"");
    }
    if lang.is_spanish() {
        return format!("Palabra repetida: «{word}»");
    }
    if lang.pack_locale().is_some_and(|l| l.primary() == "fr") {
        return format!("Mot répété{NBSP}: «{NBSP}{word}{NBSP}»");
    }
    match lang.built_in() {
        ProofLang::Pl => format!("Powtórzone słowo: „{word}”"),
        ProofLang::Pt => format!("Palavra repetida: \"{word}\""),
        ProofLang::It => format!("Parola ripetuta: \"{word}\""),
        ProofLang::Hr => format!("Ponovljena riječ: \"{word}\""),
        ProofLang::Tr => "Yinelenen sözcük".to_string(),
        ProofLang::Ru => format!("Повтор слова: «{word}»"),
        ProofLang::Cs => format!("Opakované slovo: „{word}“"),
        ProofLang::Id => format!("Kata berulang: “{word}”"),
        _ => format!("Repeated word: \"{word}\""),
    }
}

#[cfg(test)]
mod tests;
