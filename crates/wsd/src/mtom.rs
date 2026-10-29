//! MTOM/XOP packaging and a bounded reader for multipart replies.
//!
//! A SOAP message can carry a binary payload as a separate MIME part referenced by `xop:Include`
//! (`multipart/related`, first part the envelope). [`package`] builds such a message, [`root_part`]
//! reads the envelope back out of a multipart reply, and [`base64`] is the inline alternative. The
//! service crates decide which element holds the payload.

use crate::envelope::new_uuid;
use crate::error::{Error, Result};
use crate::ns;
use crate::xml;

const CRLF: &[u8] = b"\r\n";
const MAX_PARTS: usize = 16;

/// An `xop:Include` element referring to the part with this content id.
pub fn xop_include_ref(cid: &str) -> String {
    format!("<xop:Include xmlns:xop=\"{}\" href=\"cid:{}\"/>", ns::XOP, xml::esc(cid))
}

/// A multipart body and the HTTP `Content-Type` that goes with it.
pub struct Package {
    pub content_type: String,
    pub body: Vec<u8>,
}

/// Wrap the envelope text (which must hold [`xop_include_ref`] for `cid`) and the document into a package.
pub fn package(envelope: &str, cid: &str, document: &[u8]) -> Package {
    let boundary = unique_boundary(document);
    let root_id = format!("root.{}@craft-wsd", new_uuid());
    let mut body = Vec::with_capacity(document.len() + envelope.len() + 512);
    let mut push = |b: &[u8]| body.extend_from_slice(b);
    push(format!("--{boundary}\r\n").as_bytes());
    push(b"Content-Type: application/xop+xml; charset=UTF-8; type=\"application/soap+xml\"\r\n");
    push(b"Content-Transfer-Encoding: binary\r\n");
    push(format!("Content-ID: <{root_id}>\r\n\r\n").as_bytes());
    push(envelope.as_bytes());
    push(CRLF);
    push(format!("--{boundary}\r\n").as_bytes());
    push(b"Content-Type: application/octet-stream\r\nContent-Transfer-Encoding: binary\r\n");
    push(format!("Content-ID: <{cid}>\r\n\r\n").as_bytes());
    push(document);
    push(CRLF);
    push(format!("--{boundary}--\r\n").as_bytes());
    let content_type = format!(
        "multipart/related; type=\"application/xop+xml\"; start=\"<{root_id}>\"; start-info=\"application/soap+xml\"; boundary=\"{boundary}\""
    );
    Package { content_type, body }
}

/// A random multipart boundary that does not occur in `document`.
fn unique_boundary(document: &[u8]) -> String {
    for _ in 0..8 {
        let b = format!("craft-wsd-{}", new_uuid());
        if find(document, b.as_bytes(), 0).is_none() {
            return b;
        }
    }
    // Eight random 122-bit collisions with the document: stop pretending and use the counter-heavy form.
    format!("craft-wsd-{}-{}", new_uuid(), new_uuid())
}

/// Plain base64 (RFC 4648, padded).
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let at = |shift: u32| T.get(((n >> shift) & 63) as usize).copied().map_or('?', char::from);
        out.push(at(18));
        out.push(at(12));
        out.push(if chunk.len() > 1 { at(6) } else { '=' });
        out.push(if chunk.len() > 2 { at(0) } else { '=' });
    }
    out
}

/// The first position of `needle` in `hay` at or after `from`; an empty needle is never found.
fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let first = *needle.first()?;
    let mut i = from;
    while let Some(rel) = hay.get(i..)?.iter().position(|&b| b == first) {
        let at = i + rel;
        if hay.get(at..at + needle.len()) == Some(needle) {
            return Some(at);
        }
        i = at + 1;
    }
    None
}

/// The value of a parameter in a media-type header (`boundary`, `start`, ...), unquoted.
pub fn param(content_type: &str, name: &str) -> Option<String> {
    for piece in split_params(content_type).into_iter().skip(1) {
        let Some((k, v)) = piece.split_once('=') else { continue };
        if k.trim().eq_ignore_ascii_case(name) {
            let v = v.trim();
            let v = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(v);
            return Some(v.to_owned());
        }
    }
    None
}

/// Split on `;` outside double quotes.
fn split_params(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted) = (0usize, false);
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                out.push(s.get(start..i).unwrap_or(""));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(s.get(start..).unwrap_or(""));
    out
}

/// Is this header a `multipart/related` media type?
pub fn is_multipart(content_type: &str) -> bool {
    content_type.trim_start().get(..17).is_some_and(|h| h.eq_ignore_ascii_case("multipart/related"))
}

/// One part of a multipart reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part<'a> {
    /// `Content-ID`, without the angle brackets.
    pub content_id: Option<String>,
    pub content_type: Option<String>,
    pub data: &'a [u8],
}

/// The value of the first MIME header `name` (case-insensitive) in a part's header block, trimmed.
fn header_value(headers: &[u8], name: &str) -> Option<String> {
    String::from_utf8_lossy(headers).lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
    })
}

/// A `Content-ID` without surrounding whitespace and angle brackets.
fn bare_id(s: &str) -> String {
    s.trim().trim_matches(|c| c == '<' || c == '>').to_owned()
}

/// Every part of a `multipart/related` body, in order. Bounded: at most 16 parts, and
/// each part's data is a slice of `body`.
pub fn parts<'a>(content_type: &str, body: &'a [u8]) -> Result<Vec<Part<'a>>> {
    let boundary = param(content_type, "boundary").filter(|b| !b.is_empty() && b.len() <= 200);
    let Some(boundary) = boundary else { return Err(Error::Protocol("multipart reply without a boundary".into())) };
    let delim = format!("--{boundary}");
    let mut out: Vec<Part<'a>> = Vec::new();
    let mut at = find(body, delim.as_bytes(), 0);
    while let Some(pos) = at {
        let after = pos + delim.len();
        if body.get(after..after + 2) == Some(b"--") {
            break;
        }
        // The part starts after the delimiter line.
        let line_end = find(body, CRLF, after).ok_or_else(|| Error::Protocol("truncated multipart reply".into()))?;
        let part_start = line_end + 2;
        let next = find(body, format!("\r\n{delim}").as_bytes(), part_start).ok_or_else(|| Error::Protocol("unterminated multipart part".into()))?;
        let whole = body.get(part_start..next).unwrap_or(&[]);
        // A part without headers starts with the blank line that ends the (empty) header block.
        let (headers, data) = match find(whole, b"\r\n\r\n", 0) {
            _ if whole.starts_with(CRLF) => (&[][..], whole.get(2..).unwrap_or(&[])),
            Some(h) => (whole.get(..h).unwrap_or(&[]), whole.get(h + 4..).unwrap_or(&[])),
            None => (&[][..], whole),
        };
        out.push(Part {
            content_id: header_value(headers, "content-id").map(|v| bare_id(&v)),
            content_type: header_value(headers, "content-type"),
            data,
        });
        if out.len() > MAX_PARTS {
            return Err(Error::TooLarge("the number of multipart parts", MAX_PARTS));
        }
        at = Some(next + 2);
    }
    Ok(out)
}

/// The root part (the `start` content id, else the first part) of a multipart reply.
pub fn root_of<'p, 'a>(content_type: &str, parts: &'p [Part<'a>]) -> Result<&'p Part<'a>> {
    if let Some(start) = param(content_type, "start") {
        let want = bare_id(&start).to_ascii_lowercase();
        if let Some(hit) = parts.iter().find(|p| p.content_id.as_ref().is_some_and(|id| id.to_ascii_lowercase() == want)) {
            return Ok(hit);
        }
    }
    parts.first().ok_or_else(|| Error::Protocol("multipart reply without parts".into()))
}

/// The root part's bytes. Bounded as [`parts`].
pub fn root_part<'a>(content_type: &str, body: &'a [u8]) -> Result<&'a [u8]> {
    let all = parts(content_type, body)?;
    root_of(content_type, &all).map(|p| p.data)
}

/// The data of the part an `xop:Include` `href` (`cid:...`, percent-escapes decoded) refers to.
pub fn resolve_href<'a>(parts: &[Part<'a>], href: &str) -> Option<&'a [u8]> {
    let cid = percent_decode(href.strip_prefix("cid:")?);
    parts.iter().find(|p| p.content_id.as_deref() == Some(cid.as_str())).map(|p| p.data)
}

/// Decode `%XX` escapes; malformed escapes are kept as written and invalid UTF-8 is replaced.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = |x: Option<&u8>| x.and_then(|d| char::from(*d).to_digit(16));
        match (c, hex(b.get(i + 1)), hex(b.get(i + 2))) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_vectors() {
        for (i, o) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(i), o);
        }
    }

    #[test]
    fn the_include_reference_escapes_the_content_id() {
        let inc = xop_include_ref("a\"<b>@x");
        assert!(inc.contains("href=\"cid:a&quot;&lt;b&gt;@x\""), "{inc}");
        assert!(xml::parse(&inc).is_ok());
    }

    #[test]
    fn a_package_is_a_two_part_related_message() {
        let env = format!("<e>{}</e>", xop_include_ref("doc@x"));
        let doc = [0u8, 255, 13, 10, 45, 45, 7];
        let p = package(&env, "doc@x", &doc);
        assert!(is_multipart(&p.content_type));
        let boundary = param(&p.content_type, "boundary").unwrap();
        assert!(p.content_type.contains("application/xop+xml") && p.content_type.contains("start-info"));
        assert!(find(&doc, boundary.as_bytes(), 0).is_none());
        // The root part reads back as the envelope, and the binary part carries the bytes intact.
        assert_eq!(root_part(&p.content_type, &p.body).unwrap(), env.as_bytes());
        assert!(find(&p.body, &doc, 0).is_some());
        assert!(p.body.ends_with(format!("--{boundary}--\r\n").as_bytes()));
    }

    #[test]
    fn the_start_parameter_picks_the_root_part() {
        let ct = "multipart/related; boundary=\"B\"; start=\"<two>\"";
        let body = b"--B\r\nContent-ID: <one>\r\n\r\nfirst\r\n--B\r\nContent-ID: <two>\r\n\r\nsecond\r\n--B--\r\n";
        assert_eq!(root_part(ct, body).unwrap(), b"second");
        let no_start = "multipart/related; boundary=B";
        assert_eq!(root_part(no_start, body).unwrap(), b"first");
    }

    #[test]
    fn hostile_multipart_replies_are_errors_not_panics() {
        let ct = "multipart/related; boundary=B";
        for body in [&b""[..], b"--B", b"--B\r\n", b"--B\r\nx: y\r\n\r\nnever ends", b"--B--", b"\r\n--B\r\n"] {
            assert!(root_part(ct, body).is_err(), "{:?}", String::from_utf8_lossy(body));
        }
        assert!(root_part("multipart/related", b"--B\r\n\r\nx\r\n--B--").is_err());
        assert!(root_part("multipart/related; boundary=\"\"", b"x").is_err());
        let mut many = Vec::new();
        for _ in 0..40 {
            many.extend_from_slice(b"--B\r\n\r\nx\r\n");
        }
        many.extend_from_slice(b"--B--\r\n");
        assert!(matches!(root_part(ct, &many), Err(Error::TooLarge(..))));
    }

    #[test]
    fn params_respect_quotes() {
        assert_eq!(param("multipart/related; type=\"a;b\"; boundary=zz", "boundary").as_deref(), Some("zz"));
        assert_eq!(param("multipart/related; type=\"a;b\"; boundary=zz", "type").as_deref(), Some("a;b"));
        assert_eq!(param("text/xml", "boundary"), None);
        assert_eq!(param("multipart/related; flag; boundary=zz", "boundary").as_deref(), Some("zz"), "a parameter without '=' is skipped");
        assert!(!is_multipart("text/xml") && !is_multipart("") && is_multipart("Multipart/Related; x=1"));
    }

    #[test]
    fn a_part_without_headers_is_all_data() {
        let ct = "multipart/related; boundary=B";
        let body = b"--B\r\n\r\nfirst\r\n\r\nstill first\r\n--B\r\n\r\n\r\n--B--\r\n";
        let all = parts(ct, body).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!((all[0].content_id.as_deref(), all[0].data), (None, &b"first\r\n\r\nstill first"[..]));
        assert_eq!(all[1].data, b"");
    }

    #[test]
    fn find_handles_edges() {
        assert_eq!(find(b"abcabc", b"cab", 0), Some(2));
        assert_eq!(find(b"abc", b"", 0), None);
        assert_eq!(find(b"abc", b"abcd", 0), None);
        assert_eq!(find(b"abc", b"a", 9), None);
    }

    #[test]
    fn parts_expose_ids_types_and_resolve_includes() {
        let env = "<e/>";
        let p = package(env, "img1@x", &[1, 2, 3, 13, 10]);
        let all = parts(&p.content_type, &p.body).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].content_id.as_deref(), Some("img1@x"));
        assert_eq!(all[1].content_type.as_deref(), Some("application/octet-stream"));
        assert_eq!(root_of(&p.content_type, &all).unwrap().data, env.as_bytes());
        assert_eq!(resolve_href(&all, "cid:img1@x"), Some(&[1u8, 2, 3, 13, 10][..]));
        assert_eq!(resolve_href(&all, "cid:img1%40x"), Some(&[1u8, 2, 3, 13, 10][..]), "percent-escapes are decoded");
        assert_eq!(resolve_href(&all, "cid:other"), None);
        assert_eq!(resolve_href(&all, "http://x/img1@x"), None);
        assert_eq!(percent_decode("a%2"), "a%2");
        assert!(parts("multipart/related", b"x").is_err());
        let many: Vec<u8> = (0..40).flat_map(|_| b"--b\r\n\r\nx\r\n".to_vec()).chain(b"--b--".to_vec()).collect();
        assert!(matches!(parts("multipart/related; boundary=b", &many), Err(Error::TooLarge(..))));
    }
}
