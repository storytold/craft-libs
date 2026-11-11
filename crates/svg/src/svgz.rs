//! From the bytes of a file to SVG text, and back to SVGZ.
//!
//! An SVGZ file is a gzip-compressed SVG. A megabyte of it can hold a gigabyte of white space,
//! and the XML reader that gets the result allocates more again, so [`inflate`] unpacks within a
//! byte limit the caller sets. [`decode`] reads the encodings SVG files come in. [`text`] does
//! both.

use std::borrow::Cow;
use std::io::Read as _;

use crate::sniff::is_svgz;

/// Why the bytes of a file are not SVG text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The file, or what it unpacks to, is larger than the limit (bytes).
    #[error("the SVG is larger than the {0} byte limit")]
    TooLarge(usize),
    /// The gzip stream is damaged.
    #[error("the compressed SVG is damaged: {0}")]
    Corrupt(String),
    /// The gzip stream holds gzip data again. A reader that unpacks what it is handed would
    /// unpack that second layer with no limit, so it is refused here.
    #[error("the compressed SVG holds another compressed file, not a document")]
    Nested,
    /// The text is not valid in its encoding.
    #[error("the SVG is not UTF-8, UTF-16 or ISO 8859-1")]
    Encoding,
}

/// The plain bytes of an SVG file: `bytes` itself, or what a gzip-compressed file unpacks to.
/// More than `limit` bytes, compressed or not, is [`Error::TooLarge`].
///
/// gzip members one after another are one file (RFC 1952 §2.2); bytes after the last member
/// that are not a member are ignored, as gzip tools do.
pub fn inflate(bytes: &[u8], limit: usize) -> Result<Cow<'_, [u8]>, Error> {
    if !is_svgz(bytes) {
        return if bytes.len() > limit { Err(Error::TooLarge(limit)) } else { Ok(Cow::Borrowed(bytes)) };
    }
    let mut out = Vec::new();
    let mut rest = bytes;
    let mut first = true;
    while is_svgz(rest) {
        let kept = out.len();
        let room = u64::try_from(limit.saturating_sub(kept)).unwrap_or(u64::MAX).saturating_add(1);
        let mut member = flate2::bufread::GzDecoder::new(rest);
        let read = member.by_ref().take(room).read_to_end(&mut out);
        if out.len() > limit {
            return Err(Error::TooLarge(limit));
        }
        match read {
            Ok(_) => {}
            Err(e) if first => return Err(Error::Corrupt(e.to_string())),
            // Trailing bytes that only start like a member: not part of the file.
            Err(_) => {
                out.truncate(kept);
                break;
            }
        }
        let after = member.into_inner();
        // Every member read takes bytes; one that takes none would never end.
        if after.len() >= rest.len() {
            break;
        }
        rest = after;
        first = false;
    }
    if is_svgz(&out) {
        return Err(Error::Nested);
    }
    Ok(Cow::Owned(out))
}

/// SVG text as SVGZ (gzip).
pub fn compress(svg: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut gz = flate2::write::GzEncoder::new(Vec::with_capacity(svg.len() / 4), flate2::Compression::default());
    // Writing to a Vec cannot fail.
    let _ = gz.write_all(svg);
    gz.finish().unwrap_or_default()
}

/// SVG text from the bytes of an SVG or SVGZ file: [`inflate`] within `limit`, then [`decode`].
pub fn text(bytes: &[u8], limit: usize) -> Result<Cow<'_, str>, Error> {
    match inflate(bytes, limit)? {
        Cow::Borrowed(plain) => decode(plain),
        Cow::Owned(plain) => decode(&plain).map(|t| Cow::Owned(t.into_owned())),
    }
}

/// SVG text from plain (not compressed) bytes: UTF-8, with or without a byte order mark; UTF-16
/// after a byte order mark; ISO 8859-1 when the XML declaration names it. Any other declared
/// encoding is read as UTF-8. The byte order mark is not part of the text.
pub fn decode(bytes: &[u8]) -> Result<Cow<'_, str>, Error> {
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        let (units, odd) = rest.as_chunks::<2>();
        if !odd.is_empty() {
            return Err(Error::Encoding);
        }
        char::decode_utf16(units.iter().map(|u| unit(*u))).collect::<Result<String, _>>().map(Cow::Owned).map_err(|_| Error::Encoding)
    };
    let utf8 = |rest| std::str::from_utf8(rest).map(Cow::Borrowed).map_err(|_| Error::Encoding);
    match bytes {
        [0xef, 0xbb, 0xbf, rest @ ..] => utf8(rest),
        [0xfe, 0xff, rest @ ..] => utf16(rest, u16::from_be_bytes),
        [0xff, 0xfe, rest @ ..] => utf16(rest, u16::from_le_bytes),
        _ if declared_latin1(bytes) => Ok(Cow::Owned(bytes.iter().map(|b| char::from(*b)).collect())),
        _ => utf8(bytes),
    }
}

/// Does the XML declaration at the start of `bytes` name ISO 8859-1?
fn declared_latin1(bytes: &[u8]) -> bool {
    let head = bytes.get(..bytes.len().min(200)).unwrap_or_default();
    let Some(decl) = head.strip_prefix(b"<?xml").and_then(|d| d.split(|b| *b == b'>').next()) else { return false };
    let decl = String::from_utf8_lossy(decl).to_ascii_lowercase();
    let Some(at) = decl.find("encoding") else { return false };
    let value = decl.get(at + "encoding".len()..).unwrap_or("").trim_start().trim_start_matches('=').trim_start();
    let name = value.trim_start_matches(['"', '\'']).split(['"', '\'', ' ', '?']).next().unwrap_or("");
    matches!(name, "iso-8859-1" | "iso8859-1" | "iso_8859-1" | "latin1" | "latin-1" | "l1")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\"><title>caf\u{e9} \u{1f600}</title></svg>";
    const MB: usize = 1 << 20;

    #[test]
    fn plain_bytes_pass_through_within_the_limit() {
        assert!(matches!(inflate(SVG.as_bytes(), MB), Ok(Cow::Borrowed(b)) if b == SVG.as_bytes()));
        assert_eq!(inflate(SVG.as_bytes(), SVG.len()).map(|b| b.len()), Ok(SVG.len()));
        assert_eq!(inflate(SVG.as_bytes(), SVG.len() - 1), Err(Error::TooLarge(SVG.len() - 1)));
    }

    #[test]
    fn svgz_unpacks_to_what_was_compressed() {
        let z = compress(SVG.as_bytes());
        assert!(is_svgz(&z));
        assert_eq!(inflate(&z, MB).map(Cow::into_owned), Ok(SVG.as_bytes().to_vec()));
        assert_eq!(text(&z, MB).as_deref(), Ok(SVG));
        assert_eq!(text(SVG.as_bytes(), MB).as_deref(), Ok(SVG));
    }

    #[test]
    fn a_file_that_unpacks_past_the_limit_is_refused_without_unpacking_it_all() {
        // 64 MiB of spaces is a few dozen kilobytes of gzip.
        let bomb = compress(&vec![b' '; 64 * MB]);
        assert!(bomb.len() < MB);
        assert_eq!(inflate(&bomb, MB), Err(Error::TooLarge(MB)));
        // Exactly at the limit is allowed; one byte less is not.
        let z = compress(SVG.as_bytes());
        assert!(inflate(&z, SVG.len()).is_ok());
        assert_eq!(inflate(&z, SVG.len() - 1), Err(Error::TooLarge(SVG.len() - 1)));
    }

    #[test]
    fn members_are_joined_and_trailing_bytes_ignored() {
        let (a, b) = SVG.split_at(20);
        let mut two = compress(a.as_bytes());
        two.extend(compress(b.as_bytes()));
        assert_eq!(text(&two, MB).as_deref(), Ok(SVG));
        // The limit counts every member.
        assert_eq!(inflate(&two, SVG.len() - 1), Err(Error::TooLarge(SVG.len() - 1)));
        let mut padded = compress(SVG.as_bytes());
        padded.extend_from_slice(&[0; 512]);
        assert_eq!(text(&padded, MB).as_deref(), Ok(SVG));
        // Bytes that start like a member and are not one.
        let mut junk = compress(SVG.as_bytes());
        junk.extend_from_slice(&[0x1f, 0x8b, 0xff, 0xff, 0xff]);
        assert_eq!(text(&junk, MB).as_deref(), Ok(SVG));
    }

    #[test]
    fn damaged_and_nested_streams_are_errors() {
        let z = compress(SVG.as_bytes());
        assert!(matches!(inflate(z.get(..z.len() / 2).unwrap_or_default(), MB), Err(Error::Corrupt(_))));
        assert!(matches!(inflate(&[0x1f, 0x8b, 0xff], MB), Err(Error::Corrupt(_))));
        assert_eq!(inflate(&compress(&z), MB), Err(Error::Nested));
    }

    #[test]
    fn utf8_utf16_and_latin1_are_decoded() {
        assert_eq!(decode(SVG.as_bytes()).as_deref(), Ok(SVG));
        assert!(matches!(decode(SVG.as_bytes()), Ok(Cow::Borrowed(_))));
        let bom: Vec<u8> = [0xef, 0xbb, 0xbf].into_iter().chain(SVG.bytes()).collect();
        assert_eq!(decode(&bom).as_deref(), Ok(SVG));
        let be: Vec<u8> = [0xfe, 0xff].into_iter().chain(SVG.encode_utf16().flat_map(u16::to_be_bytes)).collect();
        let le: Vec<u8> = [0xff, 0xfe].into_iter().chain(SVG.encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(decode(&be).as_deref(), Ok(SVG));
        assert_eq!(decode(&le).as_deref(), Ok(SVG));
        assert_eq!(text(&compress(&be), MB).as_deref(), Ok(SVG));
        let latin1 = b"<?xml version='1.0' encoding=\"ISO-8859-1\"?><svg><title>caf\xe9</title></svg>";
        assert_eq!(decode(latin1).as_deref(), Ok("<?xml version='1.0' encoding=\"ISO-8859-1\"?><svg><title>caf\u{e9}</title></svg>"));
    }

    #[test]
    fn text_that_is_not_valid_in_its_encoding_is_refused() {
        assert_eq!(decode(&[0xfe, 0xff, 0xd8]), Err(Error::Encoding), "odd UTF-16");
        assert_eq!(decode(&[0xfe, 0xff, 0xd8, 0x00]), Err(Error::Encoding), "a lone surrogate");
        assert_eq!(decode(b"<svg>\xe9</svg>"), Err(Error::Encoding), "ISO 8859-1 that is not declared");
        assert_eq!(decode(b"<?xml version='1.0' encoding='windows-1252'?><svg>\xe9</svg>"), Err(Error::Encoding));
    }
}
