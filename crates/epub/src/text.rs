//! Small values of the package document: language tags, dates, identifiers.

/// Whether `tag` is a well-formed BCP 47 language tag, as far as its shape goes: at most 63
/// characters, a primary subtag of 2 to 8 letters, then subtags of 1 to 8 letters or digits, with
/// at least one subtag after a single-character one.
pub fn valid_language_tag(tag: &str) -> bool {
    if tag.len() > 63 {
        return false;
    }
    let mut parts = tag.split('-');
    let Some(primary) = parts.next() else { return false };
    if !(2..=8).contains(&primary.len()) || !primary.bytes().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    let mut parts = parts.peekable();
    while let Some(part) = parts.next() {
        if part.is_empty() || part.len() > 8 || !part.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return false;
        }
        if part.len() == 1 && parts.peek().is_none() {
            return false;
        }
    }
    true
}

/// `s` as the `CCYY-MM-DDThh:mm:ssZ` form `dcterms:modified` requires, when it starts with a
/// date and time of that shape whose fields are in range. Fractions and zone offsets after the
/// seconds are cut, not converted.
pub fn w3cdtf(s: &str) -> Option<String> {
    let b = s.as_bytes().get(..19)?;
    let shape = b"dddd-dd-ddTdd:dd:dd";
    if !b.iter().zip(shape).all(|(c, p)| if *p == b'd' { c.is_ascii_digit() } else { c == p }) {
        return None;
    }
    let number = |range: std::ops::Range<usize>| s.get(range).and_then(|v| v.parse::<u32>().ok());
    let (month, day) = (number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    Some(format!("{}Z", s.get(..19)?))
}

/// A `urn:uuid:` identifier derived from `seed`: the same seed gives the same identifier, so a
/// book keeps its identifier from one export to the next.
pub fn urn_uuid(seed: &str) -> String {
    // Two FNV-1a 64-bit hashes (different offsets) as 128 bits, shaped as a version-5-style UUID.
    let fnv = |offset: u64| seed.bytes().fold(offset, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3));
    let (a, b) = (fnv(0xcbf2_9ce4_8422_2325), fnv(0x8422_2325_cbf2_9ce4));
    let hi = (a & 0xffff_ffff_ffff_0fff) | 0x5000;
    let lo = (b & 0x3fff_ffff_ffff_ffff) | 0x8000_0000_0000_0000;
    format!("urn:uuid:{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", hi >> 32, (hi >> 16) & 0xffff, hi & 0xffff, lo >> 48, lo & 0xffff_ffff_ffff)
}

/// The usual heading of a table of contents in `language` (a BCP 47 tag; `_` is read as `-`),
/// English for languages without one here.
pub fn contents_title(language: &str) -> &'static str {
    let lower = language.replace('_', "-").to_ascii_lowercase();
    match lower.split('-').next().unwrap_or("") {
        "ar" => "المحتويات",
        "bg" => "Съдържание",
        "cs" => "Obsah",
        "da" => "Indhold",
        "de" => "Inhalt",
        "es" => "Índice",
        "fi" => "Sisällys",
        "fr" => "Table des matières",
        "he" => "תוכן העניינים",
        "hr" => "Sadržaj",
        "hu" => "Tartalomjegyzék",
        "it" => "Indice",
        "ja" => "目次",
        "ko" => "목차",
        "nb" | "no" => "Innhold",
        "nl" => "Inhoud",
        "pl" => "Spis treści",
        "pt" if lower.contains("-br") => "Sumário",
        "pt" => "Índice",
        "ro" => "Cuprins",
        "ru" => "Содержание",
        "sr" if lower.contains("latn") => "Sadržaj",
        "sr" => "Садржај",
        "sv" => "Innehåll",
        "tr" => "İçindekiler",
        "uk" => "Зміст",
        "vi" => "Mục lục",
        "zh" if lower.contains("hant") || lower.contains("-tw") || lower.contains("-hk") => "目錄",
        "zh" => "目录",
        _ => "Contents",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_tags() {
        for ok in ["en", "en-GB", "zh-Hant-TW", "de-CH-1996", "en-x-private", "sr-Latn"] {
            assert!(valid_language_tag(ok), "{ok}");
        }
        for bad in ["", "e", "en_GB", "en-", "en--GB", "en-x", "123", "abcdefghi", "en-abcdefghi", "en\"><x", &"en-".repeat(30)] {
            assert!(!valid_language_tag(bad), "{bad}");
        }
    }

    #[test]
    fn dates() {
        assert_eq!(w3cdtf("2026-10-13T09:30:00Z").as_deref(), Some("2026-10-13T09:30:00Z"));
        assert_eq!(w3cdtf("2026-10-13T09:30:00.123+02:00").as_deref(), Some("2026-10-13T09:30:00Z"));
        for bad in
            ["", "2026-10-13", "2026-13-13T09:30:00Z", "2026-10-32T09:30:00Z", "2026-10-13T24:30:00Z", "2026-10-13 09:30:00", "12026-10-13T09:30:00Z"]
        {
            assert_eq!(w3cdtf(bad), None, "{bad}");
        }
    }

    #[test]
    fn identifiers_are_stable_and_distinct() {
        let a = urn_uuid("Book\n1");
        assert_eq!(a, urn_uuid("Book\n1"));
        assert_ne!(a, urn_uuid("Book\n2"));
        assert_eq!(a.len(), "urn:uuid:".len() + 36);
        assert_eq!(a.as_bytes().get(23), Some(&b'5'), "{a}");
    }

    #[test]
    fn contents_titles() {
        assert_eq!(contents_title("nl-NL"), "Inhoud");
        assert_eq!(contents_title("pt_BR"), "Sumário");
        assert_eq!(contents_title("pt"), "Índice");
        assert_eq!(contents_title("zh-Hant"), "目錄");
        assert_eq!(contents_title("zh"), "目录");
        assert_eq!(contents_title("sr-Latn-RS"), "Sadržaj");
        assert_eq!(contents_title("xx"), "Contents");
    }
}
