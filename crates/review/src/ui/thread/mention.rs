//! `@mention` parsing, matching, the completer and the rendering runs.

use std::ops::Range;

use craft_ui::primitives::{Completer, Completion};

/// The mention list's rows.
const MENTION_ROWS: usize = 6;

/// The longest `@query` the picker follows.
const MENTION_QUERY_CHARS: usize = 40;

/// The `@query` being typed just before `cursor` (chars) in `text`: its byte range from the `@`
/// to the cursor. `None` unless the `@` starts the text or follows a non-alphanumeric character,
/// and the query stays on one line, is at most 40 chars and doesn't end in whitespace.
pub fn mention_query(text: &str, cursor: usize) -> Option<(usize, usize)> {
    let end = text.char_indices().nth(cursor).map_or(text.len(), |(byte, _)| byte);
    let before = text.get(..end)?;
    let at = before.rfind('@')?;
    let query = before.get(at + 1..)?;
    if query.contains('\n') || query.chars().count() > MENTION_QUERY_CHARS || query.ends_with(char::is_whitespace) {
        return None;
    }
    let starts = before.get(..at)?.chars().next_back().is_none_or(|c| !c.is_alphanumeric());
    starts.then_some((at, end))
}

/// The people (not `me`) whose name, or a word of it, starts with `query` in any case, in list
/// order, at most six (the list's rows).
pub fn mention_matches(people: &[String], query: &str, me: &str) -> Vec<String> {
    let query = query.to_lowercase();
    people
        .iter()
        .filter(|name| name.as_str() != me)
        .filter(|name| {
            let lower = name.to_lowercase();
            lower.starts_with(&query) || lower.split_whitespace().any(|word| word.starts_with(&query))
        })
        .take(MENTION_ROWS)
        .cloned()
        .collect()
}

/// Replace the `@query` at byte `range` with `@name ` and return the cursor after it (chars); an
/// invalid range changes nothing and returns the text's length in chars.
pub fn insert_mention(text: &mut String, range: (usize, usize), name: &str) -> usize {
    let (start, end) = range;
    if start > end || end > text.len() || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return text.chars().count();
    }
    let inserted = format!("@{name} ");
    text.replace_range(start..end, &inserted);
    text.get(..start).map_or(0, |before| before.chars().count()) + inserted.chars().count()
}

/// Completes `@name` for the people a thread's reply box offers.
pub(crate) struct MentionCompleter {
    pub(super) people: Vec<String>,
    pub(super) me: String,
}

impl Completer for MentionCompleter {
    fn complete(&self, text: &str, caret: usize, out: &mut Vec<Completion>) -> Option<Range<usize>> {
        let cursor = text.get(..caret)?.chars().count();
        let range = mention_query(text, cursor)?;
        let names = mention_matches(&self.people, text.get(range.0 + 1..range.1).unwrap_or(""), &self.me);
        out.extend(names.iter().map(|name| Completion::new(name.clone(), format!("@{name} "))));
        (!names.is_empty()).then_some(range.0..range.1)
    }
}

/// Where each `@name` of `names` stands in `text`: an `@` at the start or after a
/// non-alphanumeric character, then a whole name.
fn mention_ranges(text: &str, names: &[&str]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find('@') {
        let at = from + offset;
        let starts = text[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let rest = &text[at + 1..];
        let name = names
            .iter()
            .filter(|name| !name.is_empty() && rest.starts_with(**name) && rest[name.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric()))
            .max_by_key(|name| name.len());
        match name {
            Some(name) if starts => {
                ranges.push(at..at + 1 + name.len());
                from = at + 1 + name.len();
            }
            _ => from = at + 1,
        }
    }
    ranges
}

/// The words of `text` as runs: a mention in the accent, the rest plain.
pub(super) fn mention_runs(text: &str, names: &[&str]) -> Vec<(String, bool)> {
    let mut runs = Vec::new();
    let mut at = 0;
    for range in mention_ranges(text, names) {
        if let Some(plain) = text.get(at..range.start) {
            runs.push((plain.to_owned(), false));
        }
        runs.push((text.get(range.clone()).unwrap_or_default().to_owned(), true));
        at = range.end;
    }
    runs.push((text.get(at..).unwrap_or_default().to_owned(), false));
    runs
}

/// The runs of a message split at its line breaks, one list of runs per line; an empty line keeps one blank run so it
/// takes a line's height.
pub(super) fn message_lines(runs: Vec<(String, bool)>) -> Vec<Vec<(String, bool)>> {
    let mut lines = vec![Vec::new()];
    for (run, mention) in runs {
        for (index, part) in run.split('\n').enumerate() {
            if index > 0 {
                lines.push(Vec::new());
            }
            if !part.is_empty() {
                lines.last_mut().into_iter().for_each(|line| line.push((part.to_owned(), mention)));
            }
        }
    }
    for line in &mut lines {
        if line.is_empty() {
            line.push((" ".to_owned(), false));
        }
    }
    lines
}
