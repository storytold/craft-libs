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
