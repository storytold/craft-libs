//! Bytes to text, for the character sets messages still arrive in.

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};

/// Text from bytes in a named character set.
///
/// Knows every character set of the WHATWG Encoding Standard by any of its names (UTF-8, the
/// Windows and ISO-8859 sets, KOI8, Shift_JIS, EUC-JP, ISO-2022-JP, GBK, Big5, EUC-KR, UTF-16…),
/// as a browser does. ISO-8859-1 is read as Windows-1252, its superset, because text that says
/// Latin-1 very often holds curly quotes and euro signs (the standard does the same). A missing
/// name, or US-ASCII with bytes above 0x7F, is guessed with [`guess`]. A name the standard doesn't
/// know, or one it reads as "replacement" (ISO-2022-KR, HZ…), is read as UTF-8, with U+FFFD for
/// what doesn't fit.
pub fn decode(bytes: &[u8], label: &str) -> String {
    let label = label.trim().trim_matches(['"', '\'']).trim().to_ascii_lowercase();
    if matches!(label.as_str(), "" | "us-ascii" | "ascii" | "ansi_x3.4-1968" | "iso646-us") {
        return guess(bytes);
    }
    let encoding = encoding(&label).unwrap_or(UTF_8);
    // ASCII reads the same in every set that keeps it; UTF-16 and ISO-2022-JP don't.
    if bytes.is_ascii() && encoding.is_ascii_compatible() {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    if encoding == UTF_16LE || encoding == UTF_16BE {
        // "UTF-16" names no byte order (it is read as little-endian): a byte order mark says which.
        return encoding.decode(bytes).0.into_owned();
    }
    // Elsewhere a byte order mark is text like any other (the reader drops one that opens a body).
    encoding.decode_without_bom_handling(bytes).0.into_owned()
}

/// The character set a name means: the standard's own names, and a few more that mail uses.
/// None for a name it doesn't know and for its "replacement" sets.
fn encoding(label: &str) -> Option<&'static Encoding> {
    let label = match label {
        "latin-1" => "latin1",
        "latin9" | "latin-9" => "iso-8859-15",
        "latin-2" => "latin2",
        "cp-1250" => "windows-1250",
        "cp-1252" => "windows-1252",
        other => other,
    };
    Encoding::for_label_no_replacement(label.as_bytes())
}

/// Does [`decode`] know the character set `label` names? An unknown one is read as UTF-8; a
/// caller with a better fallback (detection, a container's own default) asks first.
pub fn is_known(label: &str) -> bool {
    encoding(&label.trim().trim_matches(['"', '\'']).trim().to_ascii_lowercase()).is_some()
}

/// Text from bytes that don't say what they are: UTF-8 when they are valid UTF-8, else Windows-1252.
pub fn guess(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => WINDOWS_1252.decode_without_bom_handling(bytes).0.into_owned(),
    }
}

/// Line endings as `\n`: CRLF and a lone CR both become LF.
pub fn unix_lines(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_and_ascii() {
        assert_eq!(decode("héllo €".as_bytes(), "UTF-8"), "héllo €");
        assert_eq!(decode("héllo".as_bytes(), "utf8"), "héllo");
        assert_eq!(decode(b"plain", "x-unknown"), "plain");
        assert_eq!(decode(b"a\xFFb", "utf-8"), "a\u{fffd}b");
        assert_eq!(decode(b"", "utf-8"), "");
    }

    #[test]
    fn latin1_is_read_as_windows_1252() {
        assert_eq!(decode(b"caf\xE9 \xA4", "ISO-8859-1"), "café ¤");
        assert_eq!(decode(b"\x93hi\x94 \x80 \x85 \x99", "iso-8859-1"), "“hi” € … ™");
        assert_eq!(decode(b"\x93hi\x94 \x80", "\"Windows-1252\""), "“hi” €");
        assert_eq!(decode(b"\x81\x8D\x8F\x90\x9D", "cp1252"), "\u{81}\u{8d}\u{8f}\u{90}\u{9d}");
        // Every byte gives a character.
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&all, "windows-1252").chars().count(), 256);
    }

    #[test]
    fn latin9_has_the_euro() {
        assert_eq!(decode(b"\xA4 \xBC\xBD \xE9 \xA6\xA8\xB4\xB8\xBE", "ISO-8859-15"), "€ Œœ é ŠšŽžŸ");
        assert_eq!(decode(b"\x80", "latin9"), "\u{80}");
    }

    #[test]
    fn unlabelled_bytes_are_guessed() {
        assert_eq!(decode("naïve".as_bytes(), ""), "naïve");
        assert_eq!(decode(b"na\xEFve", ""), "naïve");
        assert_eq!(decode(b"na\xEFve", "us-ascii"), "naïve");
        assert_eq!(guess(b"\xE2\x82\xAC"), "€");
        assert_eq!(guess(b"\xE2\x82"), "\u{e2}\u{201a}");
    }

    #[test]
    fn known_and_unknown_names() {
        for known in ["utf-8", " GBK ", "\"Shift_JIS\"", "latin-9", "cp-1250", "windows-1251", "koi8-r", "utf-16le"] {
            assert!(is_known(known), "{known}");
        }
        for unknown in ["", "x-unknown", "iso-2022-kr", "hz-gb-2312", "utf-99"] {
            assert!(!is_known(unknown), "{unknown}");
        }
    }

    #[test]
    fn line_endings() {
        assert_eq!(unix_lines("a\r\nb\rc\nd\r\n"), "a\nb\nc\nd\n");
        assert_eq!(unix_lines("\r\r\n\r"), "\n\n\n");
        assert_eq!(unix_lines("none"), "none");
    }

    #[test]
    fn unknown_charsets_fall_back_to_lossy_utf8() {
        assert_eq!(decode("grüße".as_bytes(), "x-unknown"), "grüße");
        assert_eq!(decode(b"\xC0\xC1", "x-unknown"), "\u{fffd}\u{fffd}");
        // The standard's "replacement" sets are read the same way.
        assert_eq!(decode("grüße".as_bytes(), "iso-2022-kr"), "grüße");
        assert_eq!(decode(b"a\xFFb", "hz-gb-2312"), "a\u{fffd}b");
    }

    #[test]
    fn central_european_sets() {
        // #178: windows-1250 and iso-8859-2 were read as UTF-8, every letter past ASCII a box.
        assert_eq!(decode(b"Gvo\x9E\xF0e we\x92ve \x96 \x8E\x9A\xE8\xE6", "windows-1250"), "Gvožđe we’ve – Žščć");
        assert_eq!(decode(b"Gvo\x9E\xF0e", "cp1250"), "Gvožđe");
        assert_eq!(decode(b"Gvo\xBE\xF0e \xAE\xB9\xE8\xE6", "ISO-8859-2"), "Gvožđe Žščć");
        assert_eq!(decode(b"\xBE", "latin2"), "ž");
        let all: Vec<u8> = (0..=255).collect();
        for label in ["windows-1250", "iso-8859-2"] {
            let text = decode(&all, label);
            assert_eq!(text.chars().count(), 256, "{label}");
            assert!(!text.contains('\u{fffd}'), "{label}");
        }
    }

    #[test]
    fn other_sets_of_the_standard() {
        assert_eq!(decode(b"\xD0\xD2\xC9\xD7\xC5\xD4", "koi8-r"), "привет");
        assert_eq!(decode(b"\x93\xFA\x96\x7B", "Shift_JIS"), "日本");
        // ISO-2022-JP is seven-bit, and still not ASCII.
        assert_eq!(decode(b"\x1B$BF|K\\\x1B(B", "iso-2022-jp"), "日本");
        assert_eq!(decode(b"h\0i\0", "utf-16le"), "hi");
        assert_eq!(decode(b"\xFE\xFF\0h\0i", "utf-16"), "hi");
        assert_eq!(decode(b"\xA4", "latin9"), "€");
    }

    proptest::proptest! {
        #[test]
        fn any_bytes_in_any_set_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..256), label in "\\PC{0,24}") {
            let _ = (decode(&bytes, &label), guess(&bytes));
            for label in ["", "utf-16", "utf-16be", "iso-2022-jp", "shift_jis", "gb18030", "big5", "euc-kr", "windows-1250", "iso-8859-2"] {
                let _ = decode(&bytes, label);
            }
        }
    }
}
