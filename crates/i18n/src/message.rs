//! Matching a filled-in English message against `@msg` and `@msgplural` templates.

use std::cmp::Reverse;

/// How deep placeholder values that are messages themselves are translated. The status bar's line
/// for an EPS file that can't be opened has five levels: the open error, the loader's error, the
/// reader's error, the PostScript error and its meaning.
pub(crate) const MESSAGE_DEPTH: usize = 5;

/// Messages longer than this are shown as they are (template matching backtracks).
pub(crate) const MAX_MESSAGE_LEN: usize = 2000;

/// Stands in for the value `message_keeping` shows as it is while the text around it is translated.
pub(crate) const KEPT_VALUE: char = '\u{FFFC}';

/// What a [`crate::I18n::message_with`] hook sees of the matched template.
#[derive(Clone, Copy, Debug)]
pub struct MessageMatch<'a> {
    /// The English template that matched.
    pub english: &'a str,
    /// The captured values, as (key, value) pairs.
    pub captures: &'a [(&'a str, &'a str)],
}

/// A piece of a message template: literal text or a `{name}` placeholder.
#[derive(Debug)]
pub(crate) enum Piece {
    Text(String),
    Hole(String),
}

/// A message template (`Couldn't open {name}: {e}`) and its translation: one form for `@msg`, the
/// plural forms for `@msgplural`.
#[derive(Debug)]
pub(crate) struct Template {
    pieces: Vec<Piece>,
    /// Bytes of literal text: a template with more of it is the more specific match.
    literal: usize,
    /// The English text the template was parsed from.
    pub english: String,
    pub forms: Vec<String>,
    /// A `@msgplural` form: its `{n}` matches only a count.
    counted: bool,
}

impl Template {
    /// `None` for a template that can't be matched reliably: no literal text, or two placeholders
    /// in a row. `{{` and `}}` are a literal `{` and `}`, as in a Rust format string, and so is a
    /// `{` with no `}` after it.
    pub fn parse(source: &str, forms: Vec<String>, counted: bool) -> Option<Template> {
        let mut pieces = Vec::new();
        let mut text = String::new();
        let mut rest = source;
        while let Some(a) = rest.find(['{', '}']) {
            text.push_str(rest.get(..a)?);
            let after = rest.get(a + 1..)?;
            let brace = if rest.get(a..)?.starts_with('{') { '{' } else { '}' };
            if let Some(escaped) = after.strip_prefix(brace) {
                text.push(brace);
                rest = escaped;
                continue;
            }
            if brace == '}' {
                text.push('}');
                rest = after;
                continue;
            }
            let Some(b) = after.find('}') else {
                text.push('{');
                rest = after;
                continue;
            };
            if !text.is_empty() {
                pieces.push(Piece::Text(std::mem::take(&mut text)));
            } else if matches!(pieces.last(), Some(Piece::Hole(_))) {
                return None;
            }
            pieces.push(Piece::Hole(after.get(..b)?.to_string()));
            rest = after.get(b + 1..)?;
        }
        text.push_str(rest);
        if !text.is_empty() {
            pieces.push(Piece::Text(text));
        }
        let literal = pieces.iter().map(|p| if let Piece::Text(t) = p { t.len() } else { 0 }).sum();
        (literal > 0).then_some(Template { pieces, literal, english: source.to_string(), forms, counted })
    }

    /// The message text a template without placeholders stands for (`{{` read as `{`): such a row
    /// is an exact one.
    pub fn exact_key(&self) -> Option<String> {
        match self.pieces.as_slice() {
            [Piece::Text(text)] => Some(text.clone()),
            _ => None,
        }
    }

    /// The placeholder values if `s` is an instance of this template.
    pub fn captures<'a>(&'a self, s: &'a str) -> Option<Vec<(&'a str, &'a str)>> {
        let mut out = Vec::new();
        match_pieces(&self.pieces, s, self.counted, &mut out, None).then_some(out)
    }

    /// [`Template::captures`] for a template that starts with a literal `[` and a placeholder
    /// whose value is an index of ASCII digits, such as ``[{j}] `{_1}`: {why}``. A JSON array such
    /// as ``["] `x`: b"]`` fits that template's text, but not its index.
    pub fn indexed_captures<'a>(&'a self, s: &'a str) -> Option<Vec<(&'a str, &'a str)>> {
        let [Piece::Text(open), Piece::Hole(index), ..] = self.pieces.as_slice() else { return None };
        if open != "[" {
            return None;
        }
        let captures = self.captures(s)?;
        captures.iter().any(|(k, v)| *k == index.as_str() && v.bytes().all(|b| b.is_ascii_digit())).then_some(captures)
    }
}

/// Add `template` to `templates`, replacing an earlier one with the same English text (an app row
/// over a shared one).
pub(crate) fn insert_template(templates: &mut Vec<Template>, template: Template) {
    templates.retain(|t| !(t.english == template.english && t.counted == template.counted));
    templates.push(template);
}

/// Order templates most specific first: those that start with text before those that start with a
/// placeholder, so `plug-in manifest: {0}` wins over `{_1} is {_2} bytes` for a message it wraps;
/// then the longest text before the first placeholder; then the most literal text, so
/// `Couldn't open {name}: {e}` wins over `{path}: {e}`. The sort is stable: ties keep catalog order.
pub(crate) fn sort_templates(templates: &mut [Template]) {
    templates.sort_by_key(|t| {
        let lead = if let Some(Piece::Text(text)) = t.pieces.first() { text.len() } else { 0 };
        (matches!(t.pieces.first(), Some(Piece::Hole(_))), Reverse(lead), Reverse(t.literal))
    });
}

/// Match `s` against `pieces`, a placeholder taking the shortest non-empty text that lets the rest
/// match (the last one takes what remains). A placeholder between two of the same quote (`'{key}'`,
/// `"{key}"` or `` `{key}` ``) may also take no text, as for a property named by the empty string.
/// With `counted`, `{n}` takes only a count. A placeholder that occurs twice takes the same text
/// both times. `before` is the last character of the text piece before `pieces`.
fn match_pieces<'p, 'a>(pieces: &'p [Piece], s: &'a str, counted: bool, out: &mut Vec<(&'p str, &'a str)>, before: Option<char>) -> bool {
    let Some((first, rest)) = pieces.split_first() else { return s.is_empty() };
    match first {
        Piece::Text(t) => s.strip_prefix(t.as_str()).is_some_and(|tail| match_pieces(rest, tail, counted, out, t.chars().last())),
        Piece::Hole(name) => {
            let earlier = out.iter().find(|(n, _)| *n == name.as_str()).map(|&(_, v)| v);
            let fits = |value: &str| (!(counted && name == "n") || value.parse::<u64>().is_ok()) && earlier.is_none_or(|e| e == value);
            let Some((next, after)) = rest.split_first() else {
                if s.is_empty() || !fits(s) {
                    return false;
                }
                out.push((name.as_str(), s));
                return true;
            };
            let Piece::Text(next) = next else { return false };
            let quoted = before.is_some_and(|q| matches!(q, '\'' | '"' | '`') && next.starts_with(q));
            if quoted
                && fits("")
                && let Some(tail) = s.strip_prefix(next.as_str())
            {
                let mark = out.len();
                out.push((name.as_str(), ""));
                if match_pieces(after, tail, counted, out, next.chars().last()) {
                    return true;
                }
                out.truncate(mark);
            }
            // Every other place the next literal occurs, nearest first.
            let mut from = s.chars().next().map_or(1, char::len_utf8);
            while let Some(at) = s.get(from..).and_then(|tail| tail.find(next.as_str())).map(|i| i + from) {
                let (Some(value), Some(tail)) = (s.get(..at), s.get(at + next.len()..)) else { return false };
                if !fits(value) {
                    from = at + next.chars().next().map_or(1, char::len_utf8);
                    continue;
                }
                let mark = out.len();
                out.push((name.as_str(), value));
                if match_pieces(after, tail, counted, out, next.chars().last()) {
                    return true;
                }
                out.truncate(mark);
                from = at + next.chars().next().map_or(1, char::len_utf8);
            }
            false
        }
    }
}
