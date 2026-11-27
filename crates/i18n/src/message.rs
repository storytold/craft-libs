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
            // A typed placeholder (`{item :noun}`) is its name.
            let inside = after.get(..b)?;
            pieces.push(Piece::Hole(inside.split_whitespace().next().unwrap_or(inside).to_string()));
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

// ---------------------------------------------------------------------------------------------------------------------
// Typed messages: a template of the code and typed values

use std::borrow::Cow;
use std::cell::Cell;

use crate::limits::MAX_OUTPUT_CHARS;
use crate::select::{self, Arg, Env};
use crate::{Lang, Locale, Row, assemble, split};

/// A message's pieces joined (isolating values as [`crate::fmt`] does) and cut to [`MAX_OUTPUT_CHARS`] characters, balanced: a cut
/// never leaves an isolate, embedding or override open for what follows the message to be reordered by, nor does a translation
/// that opens one of its own.
fn finish(pieces: Vec<(Cow<'_, str>, bool)>) -> String {
    crate::bidi::cap_balanced(&assemble(pieces, true, 64), MAX_OUTPUT_CHARS)
}

impl crate::I18n {
    /// A message with typed values, in `lang`: `source` is the English template, `{name}` or `{name :type}` placeholders, and
    /// `args` the values by name. The row is the (`context`, `source`) of the catalogs along `lang`'s chain, else the plain
    /// row of `source` (an empty `context` is the plain row alone); its translation is plain text with `{name}` placeholders,
    /// or a selector message ([`select`]) that chooses words by a number, an ordinal or a noun's gender. Values are written as
    /// [`I18n::format`](crate::I18n::format) writes numbers and as [`I18n::dates`](crate::I18n::dates) writes dates, and
    /// isolated as [`crate::fmt`] does. A message the language cannot show (a value of another type than the function takes,
    /// a noun the language has no row for) shows in English, and so does a message with no row. The output is at most
    /// [`MAX_OUTPUT_CHARS`] characters, and a person's text ([`Arg::Name`]) at most [`MAX_ARG_CHARS`](crate::limits::MAX_ARG_CHARS).
    /// In `@msg` rows, `{{` and `}}` are a literal `{` and `}`.
    pub fn tm(&self, lang: Lang, context: &str, source: &str, args: &[(&str, Arg<'_>)]) -> String {
        let escapes = context == "@msg";
        if let Some(pseudo) = lang.pseudo() {
            return pseudo.show(&self.render_english(source, args, escapes));
        }
        let both = [(context, source), ("", source)];
        let keys = if context.is_empty() { both.get(1..).unwrap_or(&both) } else { &both[..] };
        if let Some((locale, row)) = self.find_row(lang, keys) {
            let shown = match row {
                Row::Text(text) => self.render_text(locale, text, args, escapes),
                Row::Message(message) => self.render_message(locale, message, args, false),
            };
            if let Some(shown) = shown {
                return shown;
            }
        }
        self.render_english(source, args, escapes)
    }

    /// The pieces' environment for `locale`'s rules and nouns, or for English (`None`).
    fn with_env<T>(&self, locale: Option<&Locale>, plain: bool, run: impl FnOnce(&Env<'_>) -> T) -> T {
        let format = self.format();
        #[cfg(feature = "dates")]
        let dates = self.dates();
        let env = Env {
            cardinal: locale.and_then(|l| l.plural.as_ref()).map(|(rules, _)| rules),
            ordinal: locale.and_then(|l| l.ordinal.as_ref()).map(|(rules, _)| rules),
            nouns: locale.map(|l| &l.parsed().nouns),
            format: &format,
            plain,
            #[cfg(feature = "dates")]
            dates: &dates,
        };
        run(&env)
    }

    /// A selector message of `locale` shown for `args`; `None` when it cannot be shown.
    pub(crate) fn render_message(&self, locale: &Locale, message: &select::Message, args: &[(&str, Arg<'_>)], plain: bool) -> Option<String> {
        self.with_env(Some(locale), plain, |env| message.evaluate(env, args).map(finish))
    }

    /// Plain translated text with `{name}` placeholders, filled from `args`; `None` when a value cannot be shown.
    fn render_text(&self, locale: &Locale, template: &str, args: &[(&str, Arg<'_>)], escapes: bool) -> Option<String> {
        self.with_env(Some(locale), false, |env| {
            let failed = Cell::new(false);
            let pieces = split(template, escapes, true, &|name| {
                let (_, arg) = args.iter().find(|(n, _)| *n == name)?;
                let shown = select::shown(arg, env);
                failed.set(failed.get() || shown.is_none());
                shown.map(Cow::Owned)
            });
            (!failed.get()).then(|| finish(pieces))
        })
    }

    /// The English `template` filled from `args`, a noun as its English name.
    fn render_english(&self, template: &str, args: &[(&str, Arg<'_>)], escapes: bool) -> String {
        self.with_env(None, false, |env| {
            let pieces = split(template, escapes, true, &|name| {
                let (_, arg) = args.iter().find(|(n, _)| *n == name)?;
                select::shown(arg, env).map(Cow::Owned)
            });
            finish(pieces)
        })
    }
}
