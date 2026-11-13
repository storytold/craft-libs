//! Recognising SVG data by its first bytes, for open, place and paste routes.

/// How much of the data [`is_svg`] reads.
const HEAD: usize = 4096;

/// Does `bytes` start like gzip data? An SVGZ file is a gzip-compressed SVG; whether what it
/// holds is an SVG is known only after unpacking it ([`crate::svgz::inflate`]).
pub fn is_svgz(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1f, 0x8b])
}

/// Does `bytes` look like an SVG document? Its first element is `<svg`, after nothing but a byte
/// order mark, white space, an XML declaration or other processing instructions, a document type
/// and comments. Only the first 4096 bytes are read. Plain UTF-8 text only: SVGZ is recognised
/// by [`is_svgz`], and UTF-16 after [`crate::svgz::decode`].
pub fn is_svg(bytes: &[u8]) -> bool {
    let head = bytes.get(..bytes.len().min(HEAD)).unwrap_or(bytes);
    let text = String::from_utf8_lossy(head);
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with("<svg") {
        return true;
    }
    (t.starts_with("<?") || t.starts_with("<!DOCTYPE") || t.starts_with("<!--")) && t.contains("<svg")
}

#[cfg(test)]
mod tests {
    use super::*;

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
