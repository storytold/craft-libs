#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! The bottom layer of reading and writing HTML, shared by everything that reads a page, a
//! mail body or a clipboard fragment.
//!
//! - [`Lexer`] is a tolerant tokenizer: text, start tags and end tags, borrowed from the input.
//!   It reads anything: a `<` that starts no tag is text, a tag or comment that never ends runs
//!   to the end of the input. [`Options`] names the elements to drop whole (`script`), the
//!   elements whose content is text (`textarea`), and the caps ([`Limits`]).
//! - [`unescape`], [`unescape_attr`] and [`decode_entity`] decode character references;
//!   [`escape`] and [`escape_dq`] write them.
//! - [`declared_charset`] finds the character set a page names in its first bytes.
//!
//! There is no tree builder, no CSS and no rendering here: what the tags mean is the caller's.
//!
//! Input is hostile. Nothing here panics on any string, and the work is linear in the input.
//!
//! ```
//! use craft_html::{Lexer, Options, Token, unescape};
//!
//! let opts = Options { drop: &["script"], ..Options::default() };
//! let mut text = String::new();
//! let mut links = Vec::new();
//! for token in Lexer::with(r#"Tom &amp; <a href="?a=1&amp;b=2">Jerry</a><script>x<y</script>"#, opts) {
//!     match token {
//!         Token::Text(t) => text.push_str(&unescape(t)),
//!         Token::Start(tag) if tag.name == "a" => links.extend(tag.attr("href")),
//!         _ => {}
//!     }
//! }
//! assert_eq!((text.as_str(), links.as_slice()), ("Tom & Jerry", &["?a=1&b=2".to_string()][..]));
//! ```

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

#[cfg(feature = "flow")]
pub mod doc;
mod entity;
mod lex;

pub use entity::{decode_entity, escape, escape_dq, reference, unescape, unescape_attr};
pub use lex::{Lexer, Limits, Options, Tag, Token};

/// How many bytes of a page [`declared_charset`] looks at.
pub const CHARSET_SNIFF_BYTES: usize = 2048;

/// The character set a page names in its first [`CHARSET_SNIFF_BYTES`] bytes: a
/// `<meta charset=…>`, a `content="text/html; charset=…"`, or an XML declaration's `encoding=…`.
/// Lower case, as written otherwise: the caller maps the label to a decoder. `None` when the
/// page names none. A byte order mark is the caller's to check first.
pub fn declared_charset(bytes: &[u8]) -> Option<String> {
    let head = String::from_utf8_lossy(bytes.get(..bytes.len().min(CHARSET_SNIFF_BYTES)).unwrap_or(bytes)).to_ascii_lowercase();
    let (i, n) = head.find("charset").map(|i| (i, 7)).or_else(|| head.find("encoding").map(|i| (i, 8)))?;
    let after = head.get(i + n..)?.trim_start().strip_prefix('=')?;
    let trimmed = after.trim_start().trim_start_matches(['"', '\'']);
    let end = trimmed.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ';' | '>' | '/')).unwrap_or(trimmed.len());
    let label = trimmed.get(..end)?.trim();
    (!label.is_empty()).then(|| label.to_string())
}

#[cfg(test)]
mod tests {
    use super::declared_charset;

    #[test]
    fn the_declared_charset_is_found_in_each_form() {
        assert_eq!(declared_charset(b"<meta charset=\"UTF-8\">").as_deref(), Some("utf-8"));
        assert_eq!(declared_charset(b"<META http-equiv=Content-Type content='text/html; charset=windows-1251'>").as_deref(), Some("windows-1251"));
        assert_eq!(declared_charset(b"<?xml version=\"1.0\" encoding = 'Shift_JIS'?>").as_deref(), Some("shift_jis"));
        assert_eq!(declared_charset(b"<meta charset=koi8-r/>").as_deref(), Some("koi8-r"));
        for none in [&b"<p>no charset here</p>"[..], b"charset", b"charset=", b"charset=''", b"", b"\xff\xfe\x00"] {
            assert_eq!(declared_charset(none), None);
        }
        // Only the head is looked at.
        let mut late = vec![b' '; super::CHARSET_SNIFF_BYTES];
        late.extend_from_slice(b"<meta charset=utf-8>");
        assert_eq!(declared_charset(&late), None);
    }
}
