//! Windows-1258 (CP1258) legacy Vietnamese 8-bit encoding tables and pure decoding primitive.
//!
//! Standardized by Microsoft for Vietnamese typography on Windows 95/NT.
//! Cross-verified with official Unicode Consortium mapping specification `CP1258.TXT`.
//!
//! # Separation of Concerns
//! `decode_windows1258` is a pure decoding primitive: it maps 8-bit bytes strictly to
//! Unicode codepoints (including base vowels and the 5 combining diacritical marks:
//! `U+0300`, `U+0301`, `U+0309`, `U+0303`, `U+0323`) without performing implicit Unicode Normalization
//! Form C (NFC) composition. Canonical normalization is deferred to [`super::normalize_cp1258_nfc`].

use std::borrow::Cow;

use super::{DecodeError, LegacyEncoding};

/// Map a Windows-1258 high byte (`0x80..=0xFF`) using the shared [`crate::CP1258`] table.
///
/// Contains exactly 9 undefined byte positions according to Microsoft / Unicode Consortium `CP1258.TXT`:
/// `0x81, 0x8A, 0x8D, 0x8E, 0x8F, 0x90, 0x9A, 0x9D, 0x9E` (marked as `0xFFFD` in `crate::CP1258`).
#[inline]
pub(super) fn cp1258_high_byte(byte: u8) -> Option<char> {
    let code = crate::CP1258.get(usize::from(byte.wrapping_sub(0x80))).copied()?;
    if code == 0xFFFD { None } else { char::from_u32(u32::from(code)) }
}

/// Pure decoding of a Windows-1258 byte slice into Unicode.
///
/// Returns `Cow::Borrowed` with zero allocations if the input slice is pure ASCII.
/// Returns `DecodeError::UnmappedByte` if an undefined byte in CP1258 is encountered.
///
/// # Invariant
/// This function strictly preserves decomposed combining marks and does NOT normalize to NFC.
pub fn decode_windows1258(input: &[u8]) -> Result<Cow<'_, str>, DecodeError> {
    if input.is_ascii() {
        if let Ok(s) = std::str::from_utf8(input) {
            return Ok(Cow::Borrowed(s));
        }
        return Err(DecodeError::InvalidUtf8 { offset: 0 });
    }

    let mut out = String::with_capacity(input.len());
    for (offset, &byte) in input.iter().enumerate() {
        if byte < 0x80 {
            out.push(byte as char);
        } else {
            match cp1258_high_byte(byte) {
                Some(c) => out.push(c),
                None => {
                    return Err(DecodeError::UnmappedByte { byte, offset, encoding: LegacyEncoding::Windows1258 });
                }
            }
        }
    }

    Ok(Cow::Owned(out))
}

/// Transcode a string whose byte characters represent Windows-1258 to Unicode NFC.
pub fn transcode_windows1258<'a>(s: &'a str) -> Cow<'a, str> {
    // Keep only the ASCII and non-Latin-1 (> U+00FF) fast paths here: an explicit
    // Windows-1258 transcoder must still decode 0xFE ('þ' -> '₫') even when bytes like
    // 0xC1 ('Á') happen to form a valid Unicode Vietnamese substring.
    if s.is_ascii() || s.chars().any(|ch| ch > '\u{00FF}') {
        return Cow::Borrowed(s);
    }

    let mut bytes = Vec::with_capacity(s.len());
    for c in s.chars() {
        let code = c as u32;
        if code > 0xFF {
            return Cow::Borrowed(s);
        }
        bytes.push(code as u8);
    }

    match decode_windows1258(&bytes) {
        Ok(decoded) => {
            let nfc = super::apply_cp1258_nfc_cow(decoded);
            if nfc.as_ref() == s { Cow::Borrowed(s) } else { Cow::Owned(nfc.into_owned()) }
        }
        Err(_) => Cow::Borrowed(s),
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    /// Verifies that `CP1258` contains exactly 9 undefined positions matching CP1258.TXT.
    #[test]
    fn windows1258_table_has_exact_undefined_count() {
        let undefined_count = (0x80u8..=0xFF).filter(|&b| cp1258_high_byte(b).is_none()).count();
        assert_eq!(undefined_count, 9, "CP1258 must have exactly 9 undefined byte positions");

        let expected_undefined_bytes = [0x81, 0x8A, 0x8D, 0x8E, 0x8F, 0x90, 0x9A, 0x9D, 0x9E];
        for b in expected_undefined_bytes {
            assert!(cp1258_high_byte(b).is_none(), "Byte 0x{b:02X} must be undefined (None)");
        }
    }

    /// Verifies that all 9 undefined CP1258 bytes return DecodeError::UnmappedByte in strict decoding.
    #[test]
    fn windows1258_undefined_bytes_return_error() {
        let undefined_bytes = [0x81, 0x8A, 0x8D, 0x8E, 0x8F, 0x90, 0x9A, 0x9D, 0x9E];
        for b in undefined_bytes {
            let input = [b'A', b, b'B'];
            let res = decode_windows1258(&input);
            assert_eq!(
                res,
                Err(DecodeError::UnmappedByte { byte: b, offset: 1, encoding: LegacyEncoding::Windows1258 }),
                "Undefined byte 0x{b:02X} at offset 1 must return UnmappedByte error"
            );
        }
    }

    /// Verifies that pure ASCII slices take the fast path returning borrowed Cow.
    #[test]
    fn windows1258_ascii_is_borrowed_fast_path() {
        let sample = b"Hello, World! 12345 (pure ASCII)";
        let res = decode_windows1258(sample);
        assert!(matches!(res, Ok(Cow::Borrowed(_))), "Pure ASCII input must return Cow::Borrowed");
        if let Ok(Cow::Borrowed(s)) = res {
            assert_eq!(s, "Hello, World! 12345 (pure ASCII)");
        }
    }

    /// Verifies that the 5 combining diacritical marks in CP1258 are preserved without NFC composition.
    #[test]
    fn windows1258_combining_marks_are_preserved_without_nfc() {
        // Test 5 combining diacritical marks in CP1258:
        // 0xCC: U+0300 (grave)
        // 0xEC: U+0301 (acute)
        // 0xD2: U+0309 (hook above)
        // 0xDE: U+0303 (tilde)
        // 0xF2: U+0323 (dot below)
        let input = b"a\xCC e\xEC o\xD2 u\xDE i\xF2";
        let res = decode_windows1258(input).expect("Valid CP1258 byte sequence");

        // The decoder must strictly output decomposed combining marks, NOT composed NFC
        assert_eq!(res.as_ref(), "a\u{0300} e\u{0301} o\u{0309} u\u{0303} i\u{0323}");
        assert!(res.contains('\u{0300}'));
        assert!(res.contains('\u{0301}'));
        assert!(res.contains('\u{0309}'));
        assert!(res.contains('\u{0303}'));
        assert!(res.contains('\u{0323}'));
    }

    /// Verifies decoding of a real-world CP1258 sentence and subsequent canonical NFC normalization.
    #[test]
    fn windows1258_vietnamese_real_world_sentence_without_nfc() {
        // "Tiếng Việt 100.000₫" in CP1258:
        // T i ê(0xEA) acute(0xEC) n g ' ' V i ê(0xEA) dot(0xF2) t ' ' 1 0 0 . 0 0 0 dong(0xFE)
        let input = b"Ti\xEA\xECng Vi\xEA\xF2t 100.000\xFE";
        let res = decode_windows1258(input).expect("Valid CP1258 Vietnamese text");

        // Assert pure decoded output has combining acute and combining dot
        assert_eq!(res.as_ref(), "Tiê\u{0301}ng Viê\u{0323}t 100.000\u{20AB}");

        // Normalizing via the Windows-1258 NFC pipeline produces the canonical composed form
        let nfc = super::super::normalize_cp1258_nfc(&res);
        assert_eq!(nfc.as_ref(), "Tiếng Việt 100.000₫");
    }

    /// Verifies decoding of unique Vietnamese base letters (Ă, Â, Đ, Ơ, Ư) in CP1258.
    #[test]
    fn windows1258_special_vietnamese_letters() {
        // 0xC3: Ă (U+0102), 0xE3: ă (U+0103)
        // 0xD0: Đ (U+0110), 0xF0: đ (U+0111)
        // 0xD5: Ơ (U+01A0), 0xF5: ơ (U+01A1)
        // 0xDD: Ư (U+01AF), 0xFD: ư (U+01B0)
        let input = b"\xC3\xE3 \xD0\xF0 \xD5\xF5 \xDD\xFD";
        let res = decode_windows1258(input).expect("Valid CP1258 base vowels");
        assert_eq!(res.as_ref(), "Ăă Đđ Ơơ Ưư");
    }

    /// Performs an independent byte-by-byte audit against official CP1258.TXT reference table.
    #[test]
    fn windows1258_full_high_byte_audit() {
        /// Ground-truth reference table derived independently from official Unicode Consortium CP1258.TXT.
        const INDEPENDENT_CP1258_REF: [Option<char>; 128] = [
            /* 0x80 */ Some('\u{20AC}'),
            /* 0x81 */ None,
            /* 0x82 */ Some('\u{201A}'),
            /* 0x83 */ Some('\u{0192}'),
            /* 0x84 */ Some('\u{201E}'),
            /* 0x85 */ Some('\u{2026}'),
            /* 0x86 */ Some('\u{2020}'),
            /* 0x87 */ Some('\u{2021}'),
            /* 0x88 */ Some('\u{02C6}'),
            /* 0x89 */ Some('\u{2030}'),
            /* 0x8A */ None,
            /* 0x8B */ Some('\u{2039}'),
            /* 0x8C */ Some('\u{0152}'),
            /* 0x8D */ None,
            /* 0x8E */ None,
            /* 0x8F */ None,
            /* 0x90 */ None,
            /* 0x91 */ Some('\u{2018}'),
            /* 0x92 */ Some('\u{2019}'),
            /* 0x93 */ Some('\u{201C}'),
            /* 0x94 */ Some('\u{201D}'),
            /* 0x95 */ Some('\u{2022}'),
            /* 0x96 */ Some('\u{2013}'),
            /* 0x97 */ Some('\u{2014}'),
            /* 0x98 */ Some('\u{02DC}'),
            /* 0x99 */ Some('\u{2122}'),
            /* 0x9A */ None,
            /* 0x9B */ Some('\u{203A}'),
            /* 0x9C */ Some('\u{0153}'),
            /* 0x9D */ None,
            /* 0x9E */ None,
            /* 0x9F */ Some('\u{0178}'),
            /* 0xA0 */ Some('\u{00A0}'),
            /* 0xA1 */ Some('\u{00A1}'),
            /* 0xA2 */ Some('\u{00A2}'),
            /* 0xA3 */ Some('\u{00A3}'),
            /* 0xA4 */ Some('\u{00A4}'),
            /* 0xA5 */ Some('\u{00A5}'),
            /* 0xA6 */ Some('\u{00A6}'),
            /* 0xA7 */ Some('\u{00A7}'),
            /* 0xA8 */ Some('\u{00A8}'),
            /* 0xA9 */ Some('\u{00A9}'),
            /* 0xAA */ Some('\u{00AA}'),
            /* 0xAB */ Some('\u{00AB}'),
            /* 0xAC */ Some('\u{00AC}'),
            /* 0xAD */ Some('\u{00AD}'),
            /* 0xAE */ Some('\u{00AE}'),
            /* 0xAF */ Some('\u{00AF}'),
            /* 0xB0 */ Some('\u{00B0}'),
            /* 0xB1 */ Some('\u{00B1}'),
            /* 0xB2 */ Some('\u{00B2}'),
            /* 0xB3 */ Some('\u{00B3}'),
            /* 0xB4 */ Some('\u{00B4}'),
            /* 0xB5 */ Some('\u{00B5}'),
            /* 0xB6 */ Some('\u{00B6}'),
            /* 0xB7 */ Some('\u{00B7}'),
            /* 0xB8 */ Some('\u{00B8}'),
            /* 0xB9 */ Some('\u{00B9}'),
            /* 0xBA */ Some('\u{00BA}'),
            /* 0xBB */ Some('\u{00BB}'),
            /* 0xBC */ Some('\u{00BC}'),
            /* 0xBD */ Some('\u{00BD}'),
            /* 0xBE */ Some('\u{00BE}'),
            /* 0xBF */ Some('\u{00BF}'),
            /* 0xC0 */ Some('\u{00C0}'),
            /* 0xC1 */ Some('\u{00C1}'),
            /* 0xC2 */ Some('\u{00C2}'),
            /* 0xC3 */ Some('\u{0102}'),
            /* 0xC4 */ Some('\u{00C4}'),
            /* 0xC5 */ Some('\u{00C5}'),
            /* 0xC6 */ Some('\u{00C6}'),
            /* 0xC7 */ Some('\u{00C7}'),
            /* 0xC8 */ Some('\u{00C8}'),
            /* 0xC9 */ Some('\u{00C9}'),
            /* 0xCA */ Some('\u{00CA}'),
            /* 0xCB */ Some('\u{00CB}'),
            /* 0xCC */ Some('\u{0300}'),
            /* 0xCD */ Some('\u{00CD}'),
            /* 0xCE */ Some('\u{00CE}'),
            /* 0xCF */ Some('\u{00CF}'),
            /* 0xD0 */ Some('\u{0110}'),
            /* 0xD1 */ Some('\u{00D1}'),
            /* 0xD2 */ Some('\u{0309}'),
            /* 0xD3 */ Some('\u{00D3}'),
            /* 0xD4 */ Some('\u{00D4}'),
            /* 0xD5 */ Some('\u{01A0}'),
            /* 0xD6 */ Some('\u{00D6}'),
            /* 0xD7 */ Some('\u{00D7}'),
            /* 0xD8 */ Some('\u{00D8}'),
            /* 0xD9 */ Some('\u{00D9}'),
            /* 0xDA */ Some('\u{00DA}'),
            /* 0xDB */ Some('\u{00DB}'),
            /* 0xDC */ Some('\u{00DC}'),
            /* 0xDD */ Some('\u{01AF}'),
            /* 0xDE */ Some('\u{0303}'),
            /* 0xDF */ Some('\u{00DF}'),
            /* 0xE0 */ Some('\u{00E0}'),
            /* 0xE1 */ Some('\u{00E1}'),
            /* 0xE2 */ Some('\u{00E2}'),
            /* 0xE3 */ Some('\u{0103}'),
            /* 0xE4 */ Some('\u{00E4}'),
            /* 0xE5 */ Some('\u{00E5}'),
            /* 0xE6 */ Some('\u{00E6}'),
            /* 0xE7 */ Some('\u{00E7}'),
            /* 0xE8 */ Some('\u{00E8}'),
            /* 0xE9 */ Some('\u{00E9}'),
            /* 0xEA */ Some('\u{00EA}'),
            /* 0xEB */ Some('\u{00EB}'),
            /* 0xEC */ Some('\u{0301}'),
            /* 0xED */ Some('\u{00ED}'),
            /* 0xEE */ Some('\u{00EE}'),
            /* 0xEF */ Some('\u{00EF}'),
            /* 0xF0 */ Some('\u{0111}'),
            /* 0xF1 */ Some('\u{00F1}'),
            /* 0xF2 */ Some('\u{0323}'),
            /* 0xF3 */ Some('\u{00F3}'),
            /* 0xF4 */ Some('\u{00F4}'),
            /* 0xF5 */ Some('\u{01A1}'),
            /* 0xF6 */ Some('\u{00F6}'),
            /* 0xF7 */ Some('\u{00F7}'),
            /* 0xF8 */ Some('\u{00F8}'),
            /* 0xF9 */ Some('\u{00F9}'),
            /* 0xFA */ Some('\u{00FA}'),
            /* 0xFB */ Some('\u{00FB}'),
            /* 0xFC */ Some('\u{00FC}'),
            /* 0xFD */ Some('\u{01B0}'),
            /* 0xFE */ Some('\u{20AB}'),
            /* 0xFF */ Some('\u{00FF}'),
        ];

        // Test every byte 0x80..=0xFF against the independent reference table
        for b in 0x80u8..=0xFFu8 {
            let input = [b];
            let res = decode_windows1258(&input);
            let ref_entry = INDEPENDENT_CP1258_REF[(b - 0x80) as usize];
            match ref_entry {
                Some(c) => {
                    let expected_str: String = std::iter::once(c).collect();
                    assert_eq!(res, Ok(Cow::Owned(expected_str)), "Byte 0x{b:02X} must decode to expected CP1258 char");
                }
                None => {
                    assert_eq!(
                        res,
                        Err(DecodeError::UnmappedByte { byte: b, offset: 0, encoding: LegacyEncoding::Windows1258 }),
                        "Byte 0x{b:02X} must return UnmappedByte error"
                    );
                }
            }
        }
    }

    #[test]
    fn explicit_windows1258_transcoder_does_not_skip_mixed_latin1_dong_sign() {
        // Widened b"TO\xC1N 100.000\xFE" contains "OÁN" alongside 0xFE ('þ' -> '₫').
        let widened: String = b"TO\xC1N 100.000\xFE".iter().map(|&b| b as char).collect();
        assert_eq!(transcode_windows1258(&widened), "TOÁN 100.000₫");
    }

    #[test]
    fn cp1258_nfc_composes_non_vietnamese_latin_consonants() {
        let widened_pena: String = b"Pen\xDEa".iter().map(|&b| b as char).collect();
        assert_eq!(transcode_windows1258(&widened_pena), "Peña");

        let opts = super::super::DecodeOptions { force_encoding: Some(LegacyEncoding::Windows1258), ..Default::default() };
        assert_eq!(super::super::decode_text(b"Pen\xDEa", &opts).as_deref(), Ok("Peña"));
        assert_eq!(super::super::decode_text(b"\xC7\xEC", &opts).as_deref(), Ok("Ḉ"));
        assert_eq!(super::super::decode_text(b"c\xEC s\xEC n\xCC", &opts).as_deref(), Ok("ć ś ǹ"));
    }
}
