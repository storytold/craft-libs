//! Bytes that don't say what they are: is this text at all, and in which encoding?
//!
//! [`crate::charset::decode`] reads bytes whose character set is named. This module reads the
//! ones that name none: a `.txt` file, a file with an unknown extension, a page with no
//! `charset`. Needs the `detect` feature (the `chardetng` detector).

use encoding_rs::Encoding;

/// Most bytes looked at to guess a legacy encoding.
pub const SNIFF_BYTES: usize = 1 << 20;

/// Most bytes [`is_text`] looks at.
pub const IS_TEXT_BYTES: usize = 64 << 10;

fn utf16(b: &[u8], le: bool) -> String {
    let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| if le { u16::from_le_bytes(*c) } else { u16::from_be_bytes(*c) }).collect();
    String::from_utf16_lossy(&units)
}

/// UTF-16 without a BOM, told by its zero bytes in the first 512: `Some(true)` little-endian,
/// `Some(false)` big-endian.
fn unmarked_utf16(b: &[u8]) -> Option<bool> {
    if b.len() < 4 || !b.len().is_multiple_of(2) {
        return None;
    }
    let sample = b.get(..b.len().min(512))?;
    let even_zero = sample.iter().step_by(2).filter(|x| **x == 0).count();
    let odd_zero = sample.iter().skip(1).step_by(2).filter(|x| **x == 0).count();
    let half = sample.len() / 2;
    if odd_zero * 10 >= half * 4 && even_zero * 10 < half {
        return Some(true);
    }
    if even_zero * 10 >= half * 4 && odd_zero * 10 < half {
        return Some(false);
    }
    None
}

/// Does `b` look like plain text (a playlist, a log, a config file), rather than a binary
/// file?
///
/// Text has a BOM, or reads as UTF-16 by its zero bytes, or has no NUL byte and next to no
/// other control characters in its first [`IS_TEXT_BYTES`]. PDFs are refused by their
/// signature (they start as text). An empty file is text.
pub fn is_text(b: &[u8]) -> bool {
    if b.starts_with(&[0xEF, 0xBB, 0xBF]) || b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) {
        return true;
    }
    if b.starts_with(b"%PDF-") {
        return false;
    }
    let sample = b.get(..b.len().min(IS_TEXT_BYTES)).unwrap_or(b);
    // Tab, line feed, form feed, carriage return, DOS end-of-file (^Z) and escape appear in text;
    // other controls in more than 1 of 100 characters mean binary.
    let odd = |c: u32| c == 0 || (c < 0x20 && !matches!(c, 0x09 | 0x0A | 0x0C | 0x0D | 0x1A | 0x1B)) || c == 0x7F;
    if let Some(le) = unmarked_utf16(sample) {
        let units = sample.as_chunks::<2>().0;
        let n = units.iter().filter(|c| odd(u32::from(if le { u16::from_le_bytes(**c) } else { u16::from_be_bytes(**c) }))).count();
        return n.saturating_mul(100) <= units.len();
    }
    if sample.contains(&0) {
        return false;
    }
    sample.iter().filter(|c| odd(u32::from(**c))).count().saturating_mul(100) <= sample.len()
}

/// Text from bytes that name no encoding: a BOM first (UTF-8, UTF-16 LE or BE; it is dropped),
/// then UTF-16 without a BOM (by its zero bytes), then UTF-8 when the bytes are valid UTF-8, and
/// the legacy encoding they look like ([`legacy`]) as the last resort.
pub fn decode(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = b.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, true);
    }
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, false);
    }
    if let Some(le) = unmarked_utf16(b) {
        return utf16(b, le);
    }
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => legacy(b),
    }
}

/// The name of the legacy encoding bytes that aren't UTF-8 look like (`chardetng`, the
/// detector Firefox uses for unlabelled pages), as the WHATWG Encoding Standard names it:
/// `GBK` or `gb18030`, `Big5`, `Shift_JIS` or `EUC-JP`, `EUC-KR`, or a single-byte Windows or ISO
/// set; `windows-1252` when nothing fits better. The first [`SNIFF_BYTES`] are looked at.
pub fn legacy_label(b: &[u8]) -> &'static str {
    guess(b).name()
}

fn guess(b: &[u8]) -> &'static Encoding {
    let mut det = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    det.feed(b.get(..SNIFF_BYTES).unwrap_or(b), b.len() <= SNIFF_BYTES);
    det.guess(None, chardetng::Utf8Detection::Deny)
}

/// Bytes that aren't UTF-8, read in the legacy encoding they look like ([`legacy_label`]).
/// What the encoding has no character for is U+FFFD.
pub fn legacy(b: &[u8]) -> String {
    guess(b).decode_without_bom_handling(b).0.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_told_from_binary() {
        assert!(is_text(b""));
        assert!(is_text(b"#EXTM3U\r\n#EXTINF:123,Artist - Title\r\nC:\\Music\\song.mp3\r\n"));
        assert!(is_text("#EXTM3U\n#EXTINF:-1,Мелодия — ünïcødé\nhttp://x/y.mp3\n".as_bytes()));
        assert!(is_text(&[0x23, 0x45, 0x58, 0x54, 0xE9, 0x0D, 0x0A, 0x1A])); // Windows-1252, DOS ^Z
        assert!(is_text(&[0xEF, 0xBB, 0xBF, b'a']));
        let u16le: Vec<u8> = "#EXTM3U\r\n".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert!(is_text(&u16le));
        assert!(!is_text(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"));
        assert!(!is_text(b"PK\x03\x04\x14\0\x06\0"));
        assert!(!is_text(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n"));
        assert!(!is_text(&[0x7F, b'E', b'L', b'F', 2, 1, 1, 3, 4, 5, 6, 7, 8]));
        // Odd-length bytes with zeros mixed in: binary, not UTF-16.
        assert!(!is_text(&[1, 0, 2, 0, 3]));
    }

    #[test]
    fn only_the_head_decides_whether_it_is_text() {
        let mut late = vec![b'a'; IS_TEXT_BYTES];
        late.extend_from_slice(&[0; 4096]);
        assert!(is_text(&late));
        let mut early = vec![0u8; 4096];
        early.extend_from_slice(&vec![b'a'; IS_TEXT_BYTES]);
        assert!(!is_text(&early));
    }

    #[test]
    fn marked_and_unmarked_unicode() {
        assert_eq!(decode(b"\xEF\xBB\xBFh\xC3\xA9"), "hé");
        let mut u16le = vec![0xFF, 0xFE];
        u16le.extend("héllo".encode_utf16().flat_map(|u| u.to_le_bytes()));
        assert_eq!(decode(&u16le), "héllo");
        let mut u16be = vec![0xFE, 0xFF];
        u16be.extend("héllo".encode_utf16().flat_map(|u| u.to_be_bytes()));
        assert_eq!(decode(&u16be), "héllo");
        // No BOM: told by the zero bytes. An odd byte at the end is dropped.
        let raw: Vec<u8> = "hello world".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        assert_eq!(decode(&raw), "hello world");
        let raw: Vec<u8> = "hello world".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode(&raw), "hello world");
        assert_eq!(decode(b"\xFF\xFEh\0i\0!"), "hi");
        assert_eq!(decode("naïve €".as_bytes()), "naïve €");
        assert_eq!(decode(b""), "");
    }

    #[test]
    fn legacy_bytes_are_read_in_the_encoding_they_look_like() {
        assert_eq!(decode(&[0x63, 0x61, 0x66, 0xE9, 0x80]), "café€");
        assert_eq!(legacy_label(&[0x63, 0x61, 0x66, 0xE9, 0x80]), "windows-1252");
        let (gbk, _, _) = encoding_rs::GBK.encode("这是一个用简体中文写的文本文件，用来测试编码检测。");
        assert_eq!(legacy_label(&gbk), "GBK");
        assert_eq!(decode(&gbk), "这是一个用简体中文写的文本文件，用来测试编码检测。");
        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode("これは日本語で書かれたテキストファイルです。エンコーディングの検出をテストします。");
        assert_eq!(decode(&sjis), "これは日本語で書かれたテキストファイルです。エンコーディングの検出をテストします。");
        let (cyr, _, _) = encoding_rs::WINDOWS_1251.encode("Это текстовый файл на русском языке для проверки определения кодировки.");
        assert_eq!(decode(&cyr), "Это текстовый файл на русском языке для проверки определения кодировки.");
    }

    #[test]
    fn only_the_head_is_sniffed_and_all_of_it_is_decoded() {
        // Past the sniffed megabyte the bytes are still decoded, whatever they are.
        let mut b = vec![b'a'; SNIFF_BYTES];
        b.extend_from_slice(&[0xE9; 3]);
        let text = legacy(&b);
        assert_eq!(text.chars().count(), SNIFF_BYTES + 3);
        assert!(text.ends_with("ééé"));
    }

    proptest::proptest! {
        #[test]
        fn any_bytes_decode_without_panicking(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..600)) {
            let _ = (is_text(&bytes), decode(&bytes), legacy(&bytes), legacy_label(&bytes));
        }
    }
}
