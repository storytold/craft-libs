//! Generic repeated-word and spacing checks with Czech messages.
//! No Czech-specific grammar, comma or sentence-capital rules are inferred.

use crate::{Issue, IssueKind, ProofLang, words};

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: ProofLang::Cs }
}

/// Checks `text` for repeated words and spacing problems, with Czech messages.
pub fn check(text: &str) -> Vec<Issue> {
    let mut issues = Vec::new();
    repeated_words(text, &words(text), &mut issues);
    spaces(text, &mut issues);
    issues.sort_by_key(|i| (i.start, i.end));
    issues
}

/// Words Czech repeats on purpose (`ne ne`, `ano ano`, `už už`, `to to`).
const ALLOWED_REPEATS: &[&str] = &["ne", "ano", "už", "to", "jo", "no", "tak", "pryč", "dál", "pomalu", "honem"];

fn repeated_words(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if !gap.is_empty()
            && gap.chars().all(char::is_whitespace)
            && w0.chars().all(char::is_alphabetic)
            && w0.to_lowercase() == w1.to_lowercase()
            && !ALLOWED_REPEATS.contains(&w0.to_lowercase().as_str())
        {
            v.push(issue(*a0, *b1, format!("Opakované slovo: „{w1}“"), vec![w0.to_string()]));
        }
    }
}

fn spaces(text: &str, v: &mut Vec<Issue>) {
    for (i, c) in text.char_indices() {
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
                ':' | '!' | '?' => after.is_none_or(|a| a.is_whitespace() || matches!(a, '"' | '”' | '’' | '!' | '?')),
                _ => false,
            };
        if flag {
            v.push(issue(i, i + 1 + p.len_utf8(), "Mezera před interpunkcí".into(), vec![p.to_string()]));
        }
    }
    for run in crate::space_runs(text) {
        v.push(issue(run.start, run.end, "Dvojitá mezera".into(), vec![" ".into()]));
    }
}
