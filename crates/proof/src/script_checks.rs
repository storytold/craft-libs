//! Spelling against a Hunspell dictionary and the generic checks (repeated words, spacing) for a
//! language written in one script, with that language's messages. A language gives its
//! [`Script`] (which characters are its letters and digits) and its [`Rules`]; Gujarati and Hindi
//! share this one copy.

use crate::affix::AffixDict;
use crate::{Issue, IssueKind, ProofLang, words};

/// What the checks need to know about a script.
pub(crate) struct Script {
    /// Is `c` of the script, as far as a word goes: kept when a word is trimmed, and a word with
    /// none is in another script (and never marked).
    pub(crate) in_words: fn(char) -> bool,
    /// Is `c` one of the script's letters or marks (its digits and punctuation are not)? Words
    /// of one letter are accepted, and only words of letters are repeated words.
    pub(crate) is_letter: fn(char) -> bool,
    /// Is `c` one of the script's digits? (ASCII digits always are digits.)
    pub(crate) is_digit: fn(char) -> bool,
    /// Another spelling of a word the dictionary may list instead (Devanagari's nukta letters,
    /// [`crate::devanagari::nukta_variant`]).
    pub(crate) variant: Option<fn(&str) -> Option<String>>,
}

/// Is `word` spelled correctly against `d`, the dictionary of a language written in `script`?
/// Numbers, single letters, words with digits (when `opts` ignores them) and words in other
/// scripts are accepted, and so is every word when `d` is empty.
pub(crate) fn is_correct(script: &Script, d: &AffixDict, word: &str, opts: &crate::Options) -> bool {
    if d.is_empty() {
        return true;
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && !(script.in_words)(c) && !matches!(c, '\'' | '’' | '-'));
    let w = w.trim_matches(['\'', '’', '-']);
    let letter_count = w.chars().filter(|&c| (script.is_letter)(c) || c.is_alphabetic()).count();
    if letter_count <= 1 || (opts.ignore_numbers && w.chars().any(char::is_numeric)) {
        return true;
    }
    // A word in another script (or with no character of this one) is not marked by this dictionary.
    if !w.chars().any(script.in_words) {
        return true;
    }
    if w.contains('-') {
        // The whole word first, as Hunspell does: a hyphenated entry or user word.
        return known(script, d, w) || w.split('-').all(|p| p.chars().count() <= 1 || !p.chars().any(script.in_words) || known(script, d, p));
    }
    known(script, d, w)
}

/// Is one word (no hyphens) in the dictionary or the user dictionary, spelled as it is or as the
/// script's [`Script::variant`]?
fn known(script: &Script, d: &AffixDict, w: &str) -> bool {
    let listed = |w: &str| d.check(w) || crate::in_user_dictionary(&w.to_lowercase());
    listed(w) || script.variant.and_then(|v| v(w)).is_some_and(|v| listed(&v))
}

/// A language's script, messages and choices for [`check`].
pub(crate) struct Rules {
    /// The language the issues are in.
    pub(crate) lang: ProofLang,
    /// Its script.
    pub(crate) script: &'static Script,
    /// A repeated word's message, before `: “word”`.
    pub(crate) repeated_word: &'static str,
    /// A space before punctuation.
    pub(crate) space_before_punctuation: &'static str,
    /// Two or more spaces.
    pub(crate) double_space: &'static str,
    /// Is a word written twice in a row a likely mistake? A language that doubles words on
    /// purpose answers no for those words.
    pub(crate) flag_repeat: fn(&str) -> bool,
}

/// Repeated words and spacing problems in `text`, with `rules`' messages. A space before `,`,
/// `;`, `:`, `!`, `?` or `.` is flagged, except in a ratio or score (`3 : 1`, in ASCII or the
/// script's digits) and before a decimal point; a space before any other mark (the danda `।`
/// among them) is not.
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

fn repeated_words(text: &str, ws: &[(usize, usize)], rules: &Rules, v: &mut Vec<Issue>) {
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if !gap.is_empty()
            && gap.chars().all(char::is_whitespace)
            && w0.chars().all(|c| c.is_alphabetic() || (rules.script.is_letter)(c))
            && w0 == w1
            && (rules.flag_repeat)(w0)
        {
            v.push(issue(rules, *a0, *b1, format!("{}: “{w1}”", rules.repeated_word), vec![w0.to_string()]));
        }
    }
}

fn spaces(text: &str, rules: &Rules, v: &mut Vec<Issue>) {
    let is_digit = |c: char| c.is_ascii_digit() || (rules.script.is_digit)(c);
    for (i, c) in text.char_indices() {
        if c != ' ' {
            continue;
        }
        let rest = text.get(i + 1..).unwrap_or("");
        let mut it = rest.chars();
        let Some(p) = it.next() else { continue };
        let after = it.next();
        // A ratio/score keeps the spaces in digit-space-colon-space-digit (ASCII or the script's digits).
        let ratio = p == ':'
            && text.get(..i).and_then(|before| before.chars().next_back()).is_some_and(is_digit)
            && after == Some(' ')
            && it.next().is_some_and(is_digit);
        let flag = !ratio
            && match p {
                ',' => true,
                ';' => !matches!(after, Some(')' | '(' | '-' | 'P' | 'D')),
                '.' => !matches!(after, Some('.')) && !after.is_some_and(|a| a.is_ascii_digit()),
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
