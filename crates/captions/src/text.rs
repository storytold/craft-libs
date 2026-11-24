//! Caption text helpers shared by the writers.

/// The named entities cue text may carry, as a WebVTT import keeps them.
const ENTITIES: [(&str, char); 8] =
    [("amp;", '&'), ("lt;", '<'), ("gt;", '>'), ("nbsp;", '\u{a0}'), ("quot;", '"'), ("apos;", '\''), ("lrm;", '\u{200e}'), ("rlm;", '\u{200f}')];

/// The entity at the start of `s`, as the character it names and its length in bytes. Cue text
/// keeps a literal `<` or `&` as `&lt;` / `&amp;` (as a WebVTT import keeps its entities) so that
/// its style tags stay tags; plain text and the writers decode them here.
pub fn entity_at(s: &str) -> Option<(char, usize)> {
    let rest = s.strip_prefix('&')?;
    ENTITIES.iter().find(|(name, _)| rest.starts_with(name)).map(|(name, ch)| (*ch, name.len() + 1))
}

/// `s` with its entities decoded and its tags kept: for a format that has tags but no entities.
pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(ch) = rest.chars().next() {
        if let Some((decoded, len)) = entity_at(rest) {
            out.push(decoded);
            rest = rest.get(len..).unwrap_or("");
        } else {
            out.push(ch);
            rest = rest.get(ch.len_utf8()..).unwrap_or("");
        }
    }
    out
}

/// Strip `<…>` tags and decode the common HTML/WebVTT entities.
pub fn plain_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                // A tag runs to the next '>'.  Scan forward once; preserving an unterminated
                // tag as literal text must not rescan every later opening bracket.
                let mut literal = String::new();
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '>' {
                        closed = true;
                        break;
                    }
                    literal.push(next);
                }
                if !closed {
                    out.push('<');
                    out.push_str(&literal);
                }
            }
            '{' if chars.peek() == Some(&'\\') => {
                // SSA-style override blocks some SRT files carry: {\an8}
                let mut literal = String::new();
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '}' {
                        closed = true;
                        break;
                    }
                    literal.push(next);
                }
                if !closed {
                    out.push('{');
                    out.push_str(&literal);
                }
            }
            '&' => {
                let rest: String = std::iter::once('&').chain(chars.clone().take(8)).collect();
                if let Some((ch, len)) = entity_at(&rest) {
                    for _ in 1..len {
                        chars.next();
                    }
                    out.push(ch);
                } else {
                    out.push('&');
                }
            }
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

use crate::TICKS_PER_MS;
use crate::time::{TICKS_PER_SECOND, Tick};

/// How long the last cue of a format that gives only start times lasts: such a file says when a
/// caption appears, and the next one ends it.
pub(crate) const LAST_CUE: Tick = Tick(4 * TICKS_PER_SECOND);

/// `start + LAST_CUE`, not overflowing.
pub(crate) fn last_cue_end(start: Tick) -> Tick {
    Tick(start.0.saturating_add(LAST_CUE.0))
}

/// Milliseconds as a time.
pub(crate) fn ms_to_tick(ms: i64) -> Tick {
    Tick(ms.saturating_mul(TICKS_PER_MS))
}

/// A time in whole milliseconds, rounded to the nearest; negative times read as zero.
pub(crate) fn tick_to_ms(t: Tick) -> i64 {
    let per = i128::from(TICKS_PER_MS);
    ((i128::from(t.0.max(0)) + per / 2) / per) as i64
}

/// A decimal number of at most nine digits (so that times built from it cannot overflow).
pub(crate) fn number(s: &str) -> Option<i64> {
    if s.is_empty() || s.len() > 9 || !s.bytes().all(|b| b.is_ascii_digit()) { None } else { s.parse().ok() }
}

/// Plain text as cue text: a literal `&` or `<` becomes an entity so that it is not read as markup.
pub(crate) fn escape_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            _ => out.push(c),
        }
    }
    out
}

/// The cue's text as plain lines (tags gone, entities decoded, empty lines dropped).
pub(crate) fn plain_lines(text: &str) -> Vec<String> {
    text.lines().map(plain_text).map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain() {
        assert_eq!(plain_text("<i>Hello</i> &amp; <b>bye</b>"), "Hello & bye");
        assert_eq!(plain_text("{\\an8}Top"), "Top");
        assert_eq!(plain_text("a < b"), "a < b");
        assert_eq!(plain_text("<v Bob>Hi</v>"), "Hi");
    }

    #[test]
    fn malformed_caption_markup_is_preserved_without_repeated_suffix_scans() {
        let angle = "<".repeat(10_000);
        let ssa = format!("{{\\{}", "{".repeat(10_000));
        assert_eq!(plain_text(&angle), angle);
        assert_eq!(plain_text(&ssa), ssa);
    }

    #[test]
    fn entities() {
        assert_eq!(entity_at("&lt;b"), Some(('<', 4)));
        assert_eq!(entity_at("&eacute;"), None);
        assert_eq!(entity_at("&"), None);
        assert_eq!(decode_entities("1 &lt; 2 &amp; <i>x</i> &#60; &"), "1 < 2 & <i>x</i> &#60; &");
    }
}
