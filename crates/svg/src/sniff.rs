//! Recognising SVG data by its first bytes, for open, place and paste routes.

use quick_xml::{Error, Reader, errors::SyntaxError, events::Event, name::QName};

/// How much of the data [`is_svg`] reads.
const HEAD: usize = 4096;

/// Does `bytes` start like gzip data? An SVGZ file is a gzip-compressed SVG; whether what it
/// holds is an SVG is known only after unpacking it ([`crate::svgz::inflate`]).
pub fn is_svgz(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1f, 0x8b])
}

/// Does `bytes` look like an SVG document? The first element must have the local name `svg`.
/// It may follow a byte order mark, whitespace, processing instructions, a doctype or comments.
/// Only the first 4096 bytes are read. Plain UTF-8 text only: SVGZ is recognised
/// by [`is_svgz`], and UTF-16 after [`crate::svgz::decode`].
pub fn is_svg(bytes: &[u8]) -> bool {
    let head = bytes.get(..bytes.len().min(HEAD)).unwrap_or(bytes);
    let text = String::from_utf8_lossy(head);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if !text.starts_with('<') {
        return false;
    }
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag) | Event::Empty(tag)) => return tag.local_name().as_ref() == b"svg",
            Ok(Event::Decl(_) | Event::PI(_) | Event::Comment(_) | Event::DocType(_)) => {}
            Err(Error::Syntax(
                SyntaxError::UnclosedTag | SyntaxError::UnclosedSingleQuotedAttributeValue | SyntaxError::UnclosedDoubleQuotedAttributeValue,
            )) => {
                // The prefix can end before the opening tag's closing bracket.
                let name = text
                    .get(reader.error_position() as usize..)
                    .and_then(|tag| tag.strip_prefix('<'))
                    .and_then(|tag| tag.split([' ', '\t', '\r', '\n', '/', '>']).next());
                return name.is_some_and(|name| QName(name.as_bytes()).local_name().as_ref() == b"svg");
            }
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_element_can_identify_an_svg() {
        assert!(!is_svg(b"<!-- mentions <svg/> -->"));
        assert!(!is_svg(b"<?xml version='1.0'?><html><svg/></html>"));
        assert!(!is_svg(b"<!DOCTYPE html><html><svg/></html>"));
        assert!(!is_svg(b"<svg-icon/>"));
        assert!(is_svg(b"<!DOCTYPE svg [<!ENTITY color 'red'>]><svg/>"));
        assert!(is_svg(b"<svg:svg xmlns:svg='http://www.w3.org/2000/svg'/>"));
        assert!(is_svg(b"<svg"));
    }

    #[test]
    fn an_svg_is_recognised_after_its_prolog() {
        assert!(is_svg(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"));
        assert!(is_svg(b"\xef\xbb\xbf \n<svg/>"));
        assert!(is_svg(b"<?xml version=\"1.0\"?>\n<!-- made by hand -->\n<svg/>"));
        assert!(is_svg(b"<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"x.dtd\"><svg/>"));
        assert!(is_svg(b"<!-- a comment first --><svg/>"));
        assert!(is_svg(b"<?xml-stylesheet href=\"a.css\"?><svg/>"));
    }

    #[test]
    fn other_data_is_not_an_svg() {
        assert!(!is_svg(b""));
        assert!(!is_svg(b"\x89PNG\r\n\x1a\n"));
        assert!(!is_svg(b"<html><body><svg/></body></html>"), "an SVG inside another document");
        assert!(!is_svg(b"<?xml version=\"1.0\"?><vector/>"));
        assert!(!is_svg(b"text that mentions <svg"));
        assert!(!is_svg(&[0x1f, 0x8b, 8, 0]), "SVGZ is not plain SVG");
    }

    #[test]
    fn only_the_head_is_read() {
        let mut late = format!("<?xml version=\"1.0\"?><!--{}-->", " ".repeat(HEAD)).into_bytes();
        late.extend_from_slice(b"<svg/>");
        assert!(!is_svg(&late));
        for quote in ['\'', '"'] {
            let clipped_tag = format!("<!-- prefix --><svg width={quote}{}{quote}/>", "1".repeat(HEAD));
            assert!(is_svg(clipped_tag.as_bytes()));
        }
        // A multi-byte character cut at the end of the head is not a reason to refuse.
        let mut cut = b"<svg>".to_vec();
        cut.extend("é".repeat(HEAD).as_bytes());
        assert!(is_svg(&cut));
    }

    #[test]
    fn gzip_data_is_svgz() {
        assert!(is_svgz(&[0x1f, 0x8b, 8]));
        assert!(!is_svgz(b"<svg/>") && !is_svgz(&[0x1f]) && !is_svgz(b""));
    }
}
