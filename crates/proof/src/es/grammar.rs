//! Spanish grammar checks with Spanish messages: the generic repeated-word and spacing checks,
//! and the opening question and exclamation marks Spanish writes before a question or an
//! exclamation (`¿Vienes?`, `¡Qué bien!`): a closing `?` or `!` whose sentence has no opening mark
//! is flagged, with the opening mark offered at the sentence's start, and so is a space after an
//! opening mark (`¿ Vienes?`). Either opening mark may pair with either closing one (`¡Qué
//! dices?`), and `(?)` and `(!)`, which mark doubt or surprise, stand alone. No other
//! Spanish-specific grammar, comma or sentence-capital rules are inferred.

use unicode_normalization::UnicodeNormalization;

use crate::{Issue, IssueKind, ProofLang, words};

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>, lang: ProofLang) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang }
}

/// Grammar issues in a paragraph of Spanish text; `lang` is the variety (they explain the same in
/// every one).
pub fn check(text: &str, lang: ProofLang) -> Vec<Issue> {
    let mut issues = Vec::new();
    repeated_words(text, &words(text), lang, &mut issues);
    spaces(text, lang, &mut issues);
    opening_marks(text, lang, &mut issues);
    issues.sort_by_key(|i| (i.start, i.end));
    issues
}

fn repeated_words(text: &str, ws: &[(usize, usize)], lang: ProofLang, v: &mut Vec<Issue>) {
    // Letters and the combining marks of a decomposed word (`tambie\u{301}n`); a composed and a
    // decomposed spelling of one word are the same word.
    let letters = |w: &str| w.chars().all(|c| c.is_alphabetic() || unicode_normalization::char::is_combining_mark(c));
    let folded = |w: &str| w.nfc().collect::<String>().to_lowercase();
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if !gap.is_empty() && gap.chars().all(char::is_whitespace) && letters(w0) && folded(w0) == folded(w1) {
            v.push(issue(*a0, *b1, format!("Palabra repetida: «{w1}»"), vec![w0.to_string()], lang));
        }
    }
}

fn spaces(text: &str, lang: ProofLang, v: &mut Vec<Issue>) {
    for (i, c) in text.char_indices() {
        // A space after an opening mark: `¿ Vienes?`.
        if matches!(c, '¿' | '¡')
            && text.get(i + c.len_utf8()..).is_some_and(|r| r.starts_with(' ') && r.trim_start().chars().next().is_some_and(char::is_alphanumeric))
        {
            v.push(issue(i, i + c.len_utf8() + 1, "Espacio después de un signo de apertura".into(), vec![c.to_string()], lang));
            continue;
        }
        if c != ' ' {
            continue;
        }
        let rest = text.get(i + 1..).unwrap_or("");
        let mut it = rest.chars();
        let Some(p) = it.next() else { continue };
        let after = it.next();
        // A ratio/score keeps the spaces in digit-space-colon-space-digit.
        let ratio = p == ':'
            && text.get(..i).and_then(|before| before.chars().next_back()).is_some_and(|c| c.is_ascii_digit())
            && after == Some(' ')
            && it.next().is_some_and(|c| c.is_ascii_digit());
        let flag = !ratio
            && match p {
                ',' => true,
                // `;)` and `;-)` are emoticons.
                ';' => !matches!(after, Some(')' | '(' | '-' | 'P' | 'D')),
                // ` ...` is an ellipsis, ` .5` a number.
                '.' => !matches!(after, Some('.')) && !after.is_some_and(|a| a.is_ascii_digit()),
                // ` :)`, ` :-)`, ` :D` and similar are emoticons; ` ?!` is still a stray space.
                ':' | '!' | '?' => after.is_none_or(|a| a.is_whitespace() || matches!(a, '"' | '”' | '’' | '»' | '!' | '?')),
                _ => false,
            };
        if flag {
            v.push(issue(i, i + 1 + p.len_utf8(), "Espacio antes de un signo de puntuación".into(), vec![p.to_string()], lang));
        }
    }
    for run in crate::space_runs(text) {
        v.push(issue(run.start, run.end, "Espacio doble".into(), vec![" ".into()], lang));
    }
}

/// Does the character `b` at byte `j` end what comes before it, so a question or exclamation after
/// it starts there? Sentence ends, clause breaks, line breaks and the start of a quotation or
/// parenthesis do, but not punctuation inside a word (`3.5`, `10:30`) or inside an address
/// (`example.com`, `https:`), except a mark ending an address (`…/? Vienes mañana?`).
fn ends_clause(text: &str, addresses: &[(usize, usize)], j: usize, b: char) -> bool {
    let after = j + b.len_utf8();
    if let Some((_, end)) = address_at(addresses, j) {
        return matches!(b, '?' | '!' | '.') && end == after;
    }
    let inside_word = matches!(b, '.' | ':' | ';') && text.get(after..).and_then(|r| r.chars().next()).is_some_and(char::is_alphanumeric);
    !inside_word && matches!(b, '.' | '?' | '!' | '…' | ':' | ';' | '\n' | '\r' | '\t' | '«' | '“' | '"' | '(' | '[' | '—')
}

/// The one of the `addresses` ([`address_ranges`]) that byte `j` is inside, if any. The ranges come
/// in text order and never overlap, so this is a binary search, O(log m) for m addresses: a paragraph
/// of thousands of addresses costs O(n log m) when every character before a question mark asks
/// (#3562 review), not the O(n m) of a scan per character.
fn address_at(addresses: &[(usize, usize)], j: usize) -> Option<(usize, usize)> {
    let after_start = addresses.partition_point(|&(s, _)| s <= j);
    let (s, e) = *addresses.get(after_start.checked_sub(1)?)?;
    (s <= j && j < e).then_some((s, e))
}

/// The byte ranges of the whitespace-delimited tokens of `text` that are web or e-mail addresses
/// (`https://example.com/?`, `www.example.com`, `ana@example.com`), as spelling skips them.
fn address_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    // A final space ends the last token.
    for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        if !c.is_whitespace() {
            start = start.or(Some(i));
        } else if let Some(s) = start.take()
            && text.get(s..i).is_some_and(crate::is_address)
        {
            out.push((s, i));
        }
    }
    out
}

/// A closing `?` or `!` with no opening `¿` or `¡` before it. Openers are counted left to right, like
/// brackets, and each closing mark takes one back (either opener pairs with either closer:
/// `¡Qué dices?`), so what sits between them doesn't matter: quotes, parentheses, dashes, an
/// ellipsis or an abbreviation's full stop (`¿Viene el Sr. García?`, `¿Leíste «Rayuela»?`) all belong
/// to the question. Only a closing mark with nothing open is flagged, and then the sentence it
/// closes starts after the last clause break ([`ends_clause`]). Marks in web and e-mail addresses
/// aren't sentence punctuation, and neither is punctuation inside an address or a word
/// (`example.com`, `3.5`, `10:30`) a sentence's start.
fn opening_marks(text: &str, lang: ProofLang, v: &mut Vec<Issue>) {
    let addresses = address_ranges(text);
    // Openers not closed yet: a run of them (`¡¡`) counts once, like a run of closers (`!!`).
    let mut open = 0usize;
    for (i, c) in text.char_indices() {
        let before = text.get(..i).unwrap_or("");
        let prev = before.chars().next_back();
        let (opener, message) = match c {
            '¿' | '¡' => {
                if !matches!(prev, Some('¿' | '¡')) {
                    open += 1;
                }
                continue;
            }
            '?' => ('¿', "Falta el signo de interrogación de apertura (¿)"),
            '!' => ('¡', "Falta el signo de exclamación de apertura (¡)"),
            _ => continue,
        };
        let next = text.get(i + 1..).and_then(|r| r.chars().next());
        // `?!` and `!!!` are handled at their first mark; `(?)` stands alone; `?q=`, `a!b` and a
        // mark ending an address (`https://example.com/?`) aren't sentence ends.
        if matches!(prev, Some('?' | '!'))
            || (prev == Some('(') && next == Some(')'))
            || next.is_some_and(char::is_alphanumeric)
            || address_at(&addresses, i).is_some()
        {
            continue;
        }
        if open > 0 {
            open -= 1;
            continue;
        }
        let from = before.char_indices().rev().find(|&(j, b)| matches!(b, '¿' | '¡') || ends_clause(text, &addresses, j, b));
        let start = from.map_or(0, |(j, b)| j + b.len_utf8());
        let Some(sentence) = text.get(start..i) else { continue };
        let lead = sentence.len() - sentence.trim_start().len();
        let start = start + lead;
        let Some(sentence) = text.get(start..i + 1) else { continue };
        if !sentence.chars().any(char::is_alphabetic) {
            continue;
        }
        v.push(issue(start, i + 1, message.into(), vec![format!("{opener}{sentence}")], lang));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The missing-opening-mark issues of `text`: the flagged text and the suggestions offered.
    fn missing(text: &str) -> Vec<(String, Vec<String>)> {
        check(text, ProofLang::Es)
            .into_iter()
            .filter(|i| i.message.starts_with("Falta"))
            .map(|i| (text[i.start..i.end].to_string(), i.suggestions))
            .collect()
    }

    /// A closing mark with nothing open is still flagged, after a sentence or a clause that closed
    /// an earlier question, and with the sentence it closes offered with its opening mark.
    #[test]
    fn closing_marks_with_nothing_open_are_still_flagged() {
        for (text, flagged, offered) in [
            ("Vienes mañana?", "Vienes mañana?", "¿Vienes mañana?"),
            ("Llegó. Vienes?", "Vienes?", "¿Vienes?"),
            ("Le preguntó: vienes?", "vienes?", "¿vienes?"),
            ("Dijo «vienes?»", "vienes?", "¿vienes?"),
            ("Qué bien!", "Qué bien!", "¡Qué bien!"),
            ("¿Vienes? No sé qué hacer?", "No sé qué hacer?", "¿No sé qué hacer?"),
            ("¿Viene el Sr. García? Llegó tarde!", "Llegó tarde!", "¡Llegó tarde!"),
            ("¡¡Hola!! Qué bien!", "Qué bien!", "¡Qué bien!"),
            ("¿Qué?! No sé qué decir!", "No sé qué decir!", "¡No sé qué decir!"),
        ] {
            let found = missing(text);
            assert_eq!(found.len(), 1, "{text:?}: {found:?}");
            assert_eq!((found[0].0.as_str(), found[0].1.first().map(String::as_str)), (flagged, Some(offered)), "{text:?}");
        }
    }
}
