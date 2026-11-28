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
#[cfg(feature = "standalone")]
pub mod standalone;

pub use entity::{decode_entity, escape, escape_dq, reference, unescape, unescape_attr};
pub use lex::{Lexer, Limits, Options, Tag, Token};

/// How many bytes of a page [`declared_charset`] looks at.
pub const CHARSET_SNIFF_BYTES: usize = 2048;

/// The character set a page names in its first [`CHARSET_SNIFF_BYTES`] bytes. Looks for
/// `<meta charset=…>` and `<meta http-equiv="content-type" content="text/html; charset=…">`,
/// with the leading XML declaration's `encoding` as a fallback. Returns the label in lower case.
/// The caller checks the byte order mark first and maps the label to a decoder.
pub fn declared_charset(bytes: &[u8]) -> Option<String> {
    let head = String::from_utf8_lossy(bytes.get(..bytes.len().min(CHARSET_SNIFF_BYTES)).unwrap_or(bytes));
    let options = Options {
        // The byte limit already bounds storage, including a meta tag with many attributes.
        limits: Limits { max_attrs: CHARSET_SNIFF_BYTES, ..Limits::DEFAULT },
        drop: &["script", "style", "title", "textarea", "xmp", "iframe", "noembed", "noframes"],
        ..Options::default()
    };
    for token in Lexer::with(&head, options) {
        let Token::Start(tag) = token else { continue };
        if tag.name == "plaintext" {
            break;
        }
        if tag.name == "meta"
            && let Some(label) = meta_charset(&tag)
        {
            return Some(label);
        }
    }
    xml_charset(&head)
}

fn meta_charset(tag: &Tag<'_>) -> Option<String> {
    if let Some(label) = tag.raw_attr("charset").and_then(charset_label) {
        return Some(label);
    }
    if !tag.raw_attr("http-equiv")?.trim().eq_ignore_ascii_case("content-type") {
        return None;
    }
    content_charset(tag.raw_attr("content")?)
}

/// HTML extracts the legacy charset from the pragma value even without a separating semicolon.
fn content_charset(content: &str) -> Option<String> {
    let content = content.to_ascii_lowercase();
    let value = content
        .match_indices("charset")
        .find_map(|(start, name)| content.get(start + name.len()..)?.trim_ascii_start().strip_prefix('='))?
        .trim_ascii_start();
    let label = match value.chars().next()? {
        quote @ ('"' | '\'') => value.strip_prefix(quote)?.split_once(quote)?.0.trim_ascii(),
        _ => value.split(|c: char| c.is_ascii_whitespace() || c == ';').next()?,
    };
    (!label.is_empty()).then(|| label.to_string())
}

fn xml_charset(head: &str) -> Option<String> {
    let head = head.strip_prefix('\u{feff}').unwrap_or(head).trim_start();
    let declaration = head.strip_prefix("<?xml")?;
    if !declaration.starts_with(|c: char| c.is_ascii_whitespace()) {
        return None;
    }
    let (attrs, _) = declaration.split_once("?>")?;
    // The HTML lexer skips processing instructions. Reuse its attribute parsing on a start tag.
    let tag = format!("<xml{attrs}>");
    let Token::Start(tag) = Lexer::new(&tag).next()? else { return None };
    tag.raw_attr("encoding").and_then(charset_label)
}

fn charset_label(value: &str) -> Option<String> {
    let label = value.trim_start().trim_start_matches(['"', '\'']);
    let label = label.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ';' | '>' | '/')).next()?;
    (!label.is_empty()).then(|| label.to_ascii_lowercase())
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

    #[test]
    fn a_legacy_content_charset_needs_no_semicolon() {
        let html = br#"<meta http-equiv=content-type content="text/html charset=windows-1251">"#;
        assert_eq!(declared_charset(html).as_deref(), Some("windows-1251"));
        let html = br#"<meta http-equiv=content-type content='text/html charset note CHARSET = " Shift_JIS "'>"#;
        assert_eq!(declared_charset(html).as_deref(), Some("shift_jis"));
        let html = br#"<meta http-equiv=content-type content="text/html; charset='windows-1251">"#;
        assert_eq!(declared_charset(html), None);
        let html = br#"<meta name=description content="text/html charset=windows-1251"><meta charset=utf-8>"#;
        assert_eq!(declared_charset(html).as_deref(), Some("utf-8"));
    }

    #[test]
    fn charset_decoys_do_not_hide_a_later_meta_declaration() {
        for decoy in [
            r#"<!-- charset=windows-1251 <meta charset=windows-1251> -->"#,
            r#"<p>charset=windows-1251</p>"#,
            r#"<script>const tag = "<meta charset=windows-1251>";</script>"#,
            r#"<style>/* <meta charset=windows-1251> */</style>"#,
            r#"<title><meta charset=windows-1251></title>"#,
            r#"<textarea><meta charset=windows-1251></textarea>"#,
            r#"<xmp><meta charset=windows-1251></xmp>"#,
            r#"<p title="<meta charset=windows-1251>">text</p>"#,
            r#"<metadata charset=windows-1251></metadata>"#,
            r#"<meta data-charset=windows-1251>"#,
            r#"<meta name=description content="text/html; charset=windows-1251">"#,
            r#"<meta http-equiv=refresh content="text/html; charset=windows-1251">"#,
            r#"<?xml-stylesheet encoding='windows-1251'?>"#,
        ] {
            let html = format!("{decoy}<meta charset=UTF-8>");
            assert_eq!(declared_charset(html.as_bytes()).as_deref(), Some("utf-8"), "{decoy}");
            assert_eq!(declared_charset(decoy.as_bytes()), None, "{decoy}");
        }
        assert_eq!(declared_charset(b"<plaintext><meta charset=windows-1251>"), None);
    }

    #[test]
    fn html_meta_takes_precedence_over_the_leading_xml_encoding() {
        let html = b"<?xml version='1.0' encoding='windows-1251'?><meta charset='UTF-8'>";
        assert_eq!(declared_charset(html).as_deref(), Some("utf-8"));
        let html = b"<?xml version='1.0'?><meta name=viewport /><meta content='text/html; Charset = GBK' http-equiv='content-type' />";
        assert_eq!(declared_charset(html).as_deref(), Some("gbk"));
        assert_eq!(declared_charset(b"<p></p><?xml version='1.0' encoding='windows-1251'?>"), None);
    }

    #[test]
    fn a_charset_decoy_with_a_real_declaration_beyond_the_probe_window_is_ignored() {
        let mut html = b"<!-- charset=windows-1251 -->".to_vec();
        html.resize(super::CHARSET_SNIFF_BYTES, b' ');
        html.extend_from_slice(b"<meta charset=utf-8>");
        assert_eq!(declared_charset(&html), None);
    }

    #[test]
    fn a_meta_declaration_within_the_probe_window_can_follow_many_attributes() {
        let mut html = String::from("<meta");
        for number in 0..70 {
            html.push_str(&format!(" data-probe-{number}=x"));
        }
        html.push_str(" charset=utf-8>");
        assert!(html.len() < super::CHARSET_SNIFF_BYTES);
        assert_eq!(declared_charset(html.as_bytes()).as_deref(), Some("utf-8"));
    }
}
