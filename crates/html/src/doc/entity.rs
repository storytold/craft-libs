//! Character references (from the crate's core) and decoding a page's bytes to text.

/// The text of an entity body (`amp`, `#38`, `#x26`).
pub fn decode_entity(name: &str) -> Option<String> {
    crate::decode_entity(name).map(String::from)
}

pub use crate::unescape;

/// Decode HTML bytes: BOM, then a `charset` in the first 2 KB, else UTF-8 with a legacy fallback.
pub fn decode(b: &[u8]) -> String {
    decode_as(b, None)
}

/// [`decode`] in the `charset` its container states (a MIME part's, a content type's), when it
/// states one. Single-byte Windows code pages (`windows-1250`…, `iso-8859-1`, `us-ascii` as 1252)
/// are decoded as such, other labels by their WHATWG decoders.
pub fn decode_as(b: &[u8], charset: Option<&str>) -> String {
    if b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) || b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return craft_mime::detect::decode(b);
    }
    let named = charset.map(str::to_ascii_lowercase).or_else(|| crate::declared_charset(b));
    let code_page = match named.as_deref().map(str::trim) {
        Some("iso-8859-1" | "latin1" | "us-ascii") => Some(1252),
        Some(cs) => cs
            .strip_prefix("windows-")
            .or_else(|| cs.strip_prefix("cp"))
            .and_then(|n| n.parse::<u16>().ok())
            .filter(|cp| craft_codepage::SINGLE_BYTE_CODE_PAGES.contains(cp)),
        None => None,
    };
    // Other labels (ISO 8859, KOI8, Shift_JIS, GBK, Big5, EUC-KR…) by their WHATWG decoders; a
    // UTF-16 one without a BOM means UTF-8, as in browsers.
    let other = named
        .as_deref()
        .and_then(|l| encoding_rs::Encoding::for_label(l.trim().as_bytes()))
        .filter(|e| ![encoding_rs::UTF_8, encoding_rs::UTF_16LE, encoding_rs::UTF_16BE].contains(e));
    match (code_page, other) {
        (Some(cp), _) => b.iter().map(|&c| craft_codepage::windows_byte(cp, c)).collect(),
        (None, Some(enc)) => enc.decode_without_bom_handling(b).0.into_owned(),
        (None, None) => match std::str::from_utf8(b) {
            Ok(s) => s.to_string(),
            Err(_) => craft_mime::detect::decode(b),
        },
    }
}
