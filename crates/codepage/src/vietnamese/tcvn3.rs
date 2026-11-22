//! TCVN3 / ABC (TCVN 5712:1993) legacy Vietnamese 8-bit encoding tables and transcoding.
//!
//! Standardized by the Directorate for Standards, Metrology and Quality (STAMEQ),
//! Ministry of Science, Technology and Environment of Vietnam (1993). No third-party
//! implementation source is copied.
//! All decoded outputs deterministically emit standard precomposed Unicode NFC (TCVN 6909:2001).
//! Pass-through inputs that are not decoded (for example, already-Unicode strings passed to
//! [`transcode_tcvn3`]) are returned as `Cow::Borrowed` without forcing NFC normalization.

use std::borrow::Cow;

use super::detect::is_already_unicode_vietnamese;
use super::{DecodeError, DecodeMode, LegacyEncoding};

/// Windows-1252 punctuation and typographic symbols for the range `0x80..=0x9F`.
///
/// Preserves curved quotes, dashes, ellipses, bullets, and symbols commonly found
/// in legacy Word/CAD documents created on Windows.
pub const WINDOWS_1252_PUNCTUATION: [Option<char>; 32] = [
    /* 0x80 */ Some('\u{20AC}'), // EURO SIGN (€)
    /* 0x81 */ None,
    /* 0x82 */ Some('\u{201A}'), // SINGLE LOW-9 QUOTATION MARK (‚)
    /* 0x83 */ Some('\u{0192}'), // LATIN SMALL LETTER F WITH HOOK (ƒ)
    /* 0x84 */ Some('\u{201E}'), // DOUBLE LOW-9 QUOTATION MARK („)
    /* 0x85 */ Some('\u{2026}'), // HORIZONTAL ELLIPSIS (…)
    /* 0x86 */ Some('\u{2020}'), // DAGGER (†)
    /* 0x87 */ Some('\u{2021}'), // DOUBLE DAGGER (‡)
    /* 0x88 */ Some('\u{02C6}'), // MODIFIER LETTER CIRCUMFLEX ACCENT (ˆ)
    /* 0x89 */ Some('\u{2030}'), // PER MILLE SIGN (‰)
    /* 0x8A */ Some('\u{0160}'), // LATIN CAPITAL LETTER S WITH CARON (Š)
    /* 0x8B */ Some('\u{2039}'), // SINGLE LEFT-POINTING ANGLE QUOTATION MARK (‹)
    /* 0x8C */ Some('\u{0152}'), // LATIN CAPITAL LIGATURE OE (Œ)
    /* 0x8D */ None,
    /* 0x8E */ Some('\u{017D}'), // LATIN CAPITAL LETTER Z WITH CARON (Ž)
    /* 0x8F */ None,
    /* 0x90 */ None,
    /* 0x91 */ Some('\u{2018}'), // LEFT SINGLE QUOTATION MARK (‘)
    /* 0x92 */ Some('\u{2019}'), // RIGHT SINGLE QUOTATION MARK (’)
    /* 0x93 */ Some('\u{201C}'), // LEFT DOUBLE QUOTATION MARK (“)
    /* 0x94 */ Some('\u{201D}'), // RIGHT DOUBLE QUOTATION MARK (”)
    /* 0x95 */ Some('\u{2022}'), // BULLET (•)
    /* 0x96 */ Some('\u{2013}'), // EN DASH (–)
    /* 0x97 */ Some('\u{2014}'), // EM DASH (—)
    /* 0x98 */ Some('\u{02DC}'), // SMALL TILDE (˜)
    /* 0x99 */ Some('\u{2122}'), // TRADE MARK SIGN (™)
    /* 0x9A */ Some('\u{0161}'), // LATIN SMALL LETTER S WITH CARON (š)
    /* 0x9B */ Some('\u{203A}'), // SINGLE RIGHT-POINTING ANGLE QUOTATION MARK (›)
    /* 0x9C */ Some('\u{0153}'), // LATIN SMALL LIGATURE OE (œ)
    /* 0x9D */ None,
    /* 0x9E */ Some('\u{017E}'), // LATIN SMALL LETTER Z WITH CARON (ž)
    /* 0x9F */ Some('\u{0178}'), // LATIN CAPITAL LETTER Y WITH DIAERESIS (Ÿ)
];

/// Build a 128-entry TCVN3 high-byte table (`0x80..=0xFF`) sharing [`WINDOWS_1252_PUNCTUATION`]
/// for `0x80..=0x9F` and applying the TCVN3 character table for `0xA0..=0xFF`.
const fn build_tcvn3_table(upper_half: [Option<char>; 96]) -> [Option<char>; 128] {
    let mut table = [None; 128];
    let mut i = 0;
    while i < 32 {
        table[i] = WINDOWS_1252_PUNCTUATION[i];
        i += 1;
    }
    let mut j = 0;
    while j < 96 {
        table[32 + j] = upper_half[j];
        j += 1;
    }
    table
}

/// TCVN3 Lower Table (.VnTime) for high bytes `0x80..=0xFF`.
///
/// Contains the 7 base uppercase characters in slots `0xA1..=0xA7` (Ă, Â, Ê, Ô, Ơ, Ư, Đ),
/// base lowercase characters `0xA8..=0xAE`, and all accented lowercase vowels.
pub const TCVN3_LOWER_MAP: [Option<char>; 128] = build_tcvn3_table([
    /* 0xA0 */ Some('\u{00A0}'), // NO-BREAK SPACE
    /* 0xA1 */ Some('Ă'), // Ă base upper in lower font
    /* 0xA2 */ Some('Â'), // Â base upper in lower font
    /* 0xA3 */ Some('Ê'), // Ê base upper in lower font
    /* 0xA4 */ Some('Ô'), // Ô base upper in lower font
    /* 0xA5 */ Some('Ơ'), // Ơ base upper in lower font
    /* 0xA6 */ Some('Ư'), // Ư base upper in lower font
    /* 0xA7 */ Some('Đ'), // Đ base upper in lower font
    /* 0xA8 */ Some('ă'), // ă base lower
    /* 0xA9 */ Some('â'), // â base lower
    /* 0xAA */ Some('ê'), // ê base lower
    /* 0xAB */ Some('ô'), // ô base lower
    /* 0xAC */ Some('ơ'), // ơ base lower
    /* 0xAD */ Some('ư'), // ư base lower
    /* 0xAE */ Some('đ'), // đ base lower
    /* 0xAF */ Some('¯'), // macron
    /* 0xB0 */ Some('°'), // degree
    /* 0xB1 */ Some('±'), // plus-minus
    /* 0xB2 */ Some('²'), // superscript 2
    /* 0xB3 */ Some('³'), // superscript 3
    /* 0xB4 */ Some('´'), // acute accent
    /* 0xB5 */ Some('à'), // a huyền
    /* 0xB6 */ Some('ả'), // a hỏi
    /* 0xB7 */ Some('ã'), // a ngã
    /* 0xB8 */ Some('á'), // a sắc
    /* 0xB9 */ Some('ạ'), // a nặng
    /* 0xBA */ Some('º'), // masculine ordinal
    /* 0xBB */ Some('ằ'), // ă huyền
    /* 0xBC */ Some('ẳ'), // ă hỏi
    /* 0xBD */ Some('ẵ'), // ă ngã
    /* 0xBE */ Some('ắ'), // ă sắc
    /* 0xBF */ Some('¿'), // inverted question mark
    /* 0xC0 */ Some('À'), // A grave
    /* 0xC1 */ Some('Á'), // A acute
    /* 0xC2 */ Some('Â'), // A circumflex
    /* 0xC3 */ Some('Ã'), // A tilde
    /* 0xC4 */ Some('Ä'), // A diaeresis
    /* 0xC5 */ Some('Å'), // A ring
    /* 0xC6 */ Some('ặ'), // ă nặng
    /* 0xC7 */ Some('ầ'), // â huyền
    /* 0xC8 */ Some('ẩ'), // â hỏi
    /* 0xC9 */ Some('ẫ'), // â ngã
    /* 0xCA */ Some('ấ'), // â sắc
    /* 0xCB */ Some('ậ'), // â nặng
    /* 0xCC */ Some('è'), // e huyền
    /* 0xCD */ None,
    /* 0xCE */ Some('ẻ'), // e hỏi
    /* 0xCF */ Some('ẽ'), // e ngã
    /* 0xD0 */ Some('é'), // e sắc
    /* 0xD1 */ Some('ẹ'), // e nặng
    /* 0xD2 */ Some('ề'), // ê huyền
    /* 0xD3 */ Some('ể'), // ê hỏi
    /* 0xD4 */ Some('ễ'), // ê ngã
    /* 0xD5 */ Some('ế'), // ê sắc
    /* 0xD6 */ Some('ệ'), // ê nặng
    /* 0xD7 */ Some('ì'), // i huyền
    /* 0xD8 */ Some('ỉ'), // i hỏi
    /* 0xD9 */ None,
    /* 0xDA */ None,
    /* 0xDB */ None,
    /* 0xDC */ Some('ĩ'), // i ngã
    /* 0xDD */ Some('í'), // i sắc
    /* 0xDE */ Some('ị'), // i nặng
    /* 0xDF */ Some('ò'), // o huyền
    /* 0xE0 */ None,
    /* 0xE1 */ Some('ỏ'), // o hỏi
    /* 0xE2 */ Some('õ'), // o ngã
    /* 0xE3 */ Some('ó'), // o sắc
    /* 0xE4 */ Some('ọ'), // o nặng
    /* 0xE5 */ Some('ồ'), // ô huyền
    /* 0xE6 */ Some('ổ'), // ô hỏi
    /* 0xE7 */ Some('ỗ'), // ô ngã
    /* 0xE8 */ Some('ố'), // ô sắc
    /* 0xE9 */ Some('ộ'), // ô nặng
    /* 0xEA */ Some('ờ'), // ơ huyền
    /* 0xEB */ Some('ở'), // ơ hỏi
    /* 0xEC */ Some('ỡ'), // ơ ngã
    /* 0xED */ Some('ớ'), // ơ sắc
    /* 0xEE */ Some('ợ'), // ơ nặng
    /* 0xEF */ Some('ù'), // u huyền
    /* 0xF0 */ None,
    /* 0xF1 */ Some('ủ'), // u hỏi
    /* 0xF2 */ Some('ũ'), // u ngã
    /* 0xF3 */ Some('ú'), // u sắc
    /* 0xF4 */ Some('ụ'), // u nặng
    /* 0xF5 */ Some('ừ'), // ư huyền
    /* 0xF6 */ Some('ử'), // ư hỏi
    /* 0xF7 */ Some('ữ'), // ư ngã
    /* 0xF8 */ Some('ứ'), // ư sắc
    /* 0xF9 */ Some('ự'), // ư nặng
    /* 0xFA */ Some('ỳ'), // y huyền
    /* 0xFB */ Some('ỷ'), // y hỏi
    /* 0xFC */ Some('ỹ'), // y ngã
    /* 0xFD */ Some('ý'), // y sắc
    /* 0xFE */ Some('ỵ'), // y nặng
    /* 0xFF */ None,
]);

/// TCVN3 Upper Table (.VnTimeH) for high bytes `0x80..=0xFF`.
///
/// Maps all lowercase vowel slots to their uppercase Unicode equivalents for all-caps fonts.
pub const TCVN3_UPPER_MAP: [Option<char>; 128] = build_tcvn3_table([
    /* 0xA0 */ Some('\u{00A0}'), // NO-BREAK SPACE
    /* 0xA1 */ Some('Ă'), // Upper of Ă base upper in lower font
    /* 0xA2 */ Some('Â'), // Upper of Â base upper in lower font
    /* 0xA3 */ Some('Ê'), // Upper of Ê base upper in lower font
    /* 0xA4 */ Some('Ô'), // Upper of Ô base upper in lower font
    /* 0xA5 */ Some('Ơ'), // Upper of Ơ base upper in lower font
    /* 0xA6 */ Some('Ư'), // Upper of Ư base upper in lower font
    /* 0xA7 */ Some('Đ'), // Upper of Đ base upper in lower font
    /* 0xA8 */ Some('Ă'), // Upper of ă base lower
    /* 0xA9 */ Some('Â'), // Upper of â base lower
    /* 0xAA */ Some('Ê'), // Upper of ê base lower
    /* 0xAB */ Some('Ô'), // Upper of ô base lower
    /* 0xAC */ Some('Ơ'), // Upper of ơ base lower
    /* 0xAD */ Some('Ư'), // Upper of ư base lower
    /* 0xAE */ Some('Đ'), // Upper of đ base lower
    /* 0xAF */ Some('¯'), // Upper of macron
    /* 0xB0 */ Some('°'), // Upper of degree
    /* 0xB1 */ Some('±'), // Upper of plus-minus
    /* 0xB2 */ Some('²'), // Upper of superscript 2
    /* 0xB3 */ Some('³'), // Upper of superscript 3
    /* 0xB4 */ Some('´'), // Upper of acute accent
    /* 0xB5 */ Some('À'), // Upper of a huyền
    /* 0xB6 */ Some('Ả'), // Upper of a hỏi
    /* 0xB7 */ Some('Ã'), // Upper of a ngã
    /* 0xB8 */ Some('Á'), // Upper of a sắc
    /* 0xB9 */ Some('Ạ'), // Upper of a nặng
    /* 0xBA */ Some('º'), // Upper of masculine ordinal
    /* 0xBB */ Some('Ằ'), // Upper of ă huyền
    /* 0xBC */ Some('Ẳ'), // Upper of ă hỏi
    /* 0xBD */ Some('Ẵ'), // Upper of ă ngã
    /* 0xBE */ Some('Ắ'), // Upper of ă sắc
    /* 0xBF */ Some('¿'), // Upper of inverted question mark
    /* 0xC0 */ Some('À'), // Upper of A grave
    /* 0xC1 */ Some('Á'), // Upper of A acute
    /* 0xC2 */ Some('Â'), // Upper of A circumflex
    /* 0xC3 */ Some('Ã'), // Upper of A tilde
    /* 0xC4 */ Some('Ä'), // Upper of A diaeresis
    /* 0xC5 */ Some('Å'), // Upper of A ring
    /* 0xC6 */ Some('Ặ'), // Upper of ă nặng
    /* 0xC7 */ Some('Ầ'), // Upper of â huyền
    /* 0xC8 */ Some('Ẩ'), // Upper of â hỏi
    /* 0xC9 */ Some('Ẫ'), // Upper of â ngã
    /* 0xCA */ Some('Ấ'), // Upper of â sắc
    /* 0xCB */ Some('Ậ'), // Upper of â nặng
    /* 0xCC */ Some('È'), // Upper of e huyền
    /* 0xCD */ None,
    /* 0xCE */ Some('Ẻ'), // Upper of e hỏi
    /* 0xCF */ Some('Ẽ'), // Upper of e ngã
    /* 0xD0 */ Some('É'), // Upper of e sắc
    /* 0xD1 */ Some('Ẹ'), // Upper of e nặng
    /* 0xD2 */ Some('Ề'), // Upper of ê huyền
    /* 0xD3 */ Some('Ể'), // Upper of ê hỏi
    /* 0xD4 */ Some('Ễ'), // Upper of ê ngã
    /* 0xD5 */ Some('Ế'), // Upper of ê sắc
    /* 0xD6 */ Some('Ệ'), // Upper of ê nặng
    /* 0xD7 */ Some('Ì'), // Upper of i huyền
    /* 0xD8 */ Some('Ỉ'), // Upper of i hỏi
    /* 0xD9 */ None,
    /* 0xDA */ None,
    /* 0xDB */ None,
    /* 0xDC */ Some('Ĩ'), // Upper of i ngã
    /* 0xDD */ Some('Í'), // Upper of i sắc
    /* 0xDE */ Some('Ị'), // Upper of i nặng
    /* 0xDF */ Some('Ò'), // Upper of o huyền
    /* 0xE0 */ None,
    /* 0xE1 */ Some('Ỏ'), // Upper of o hỏi
    /* 0xE2 */ Some('Õ'), // Upper of o ngã
    /* 0xE3 */ Some('Ó'), // Upper of o sắc
    /* 0xE4 */ Some('Ọ'), // Upper of o nặng
    /* 0xE5 */ Some('Ồ'), // Upper of ô huyền
    /* 0xE6 */ Some('Ổ'), // Upper of ô hỏi
    /* 0xE7 */ Some('Ỗ'), // Upper of ô ngã
    /* 0xE8 */ Some('Ố'), // Upper of ô sắc
    /* 0xE9 */ Some('Ộ'), // Upper of ô nặng
    /* 0xEA */ Some('Ờ'), // Upper of ơ huyền
    /* 0xEB */ Some('Ở'), // Upper of ơ hỏi
    /* 0xEC */ Some('Ỡ'), // Upper of ơ ngã
    /* 0xED */ Some('Ớ'), // Upper of ơ sắc
    /* 0xEE */ Some('Ợ'), // Upper of ơ nặng
    /* 0xEF */ Some('Ù'), // Upper of u huyền
    /* 0xF0 */ None,
    /* 0xF1 */ Some('Ủ'), // Upper of u hỏi
    /* 0xF2 */ Some('Ũ'), // Upper of u ngã
    /* 0xF3 */ Some('Ú'), // Upper of u sắc
    /* 0xF4 */ Some('Ụ'), // Upper of u nặng
    /* 0xF5 */ Some('Ừ'), // Upper of ư huyền
    /* 0xF6 */ Some('Ử'), // Upper of ư hỏi
    /* 0xF7 */ Some('Ữ'), // Upper of ư ngã
    /* 0xF8 */ Some('Ứ'), // Upper of ư sắc
    /* 0xF9 */ Some('Ự'), // Upper of ư nặng
    /* 0xFA */ Some('Ỳ'), // Upper of y huyền
    /* 0xFB */ Some('Ỷ'), // Upper of y hỏi
    /* 0xFC */ Some('Ỹ'), // Upper of y ngã
    /* 0xFD */ Some('Ý'), // Upper of y sắc
    /* 0xFE */ Some('Ỵ'), // Upper of y nặng
    /* 0xFF */ None,
]);

/// Match a single Latin-1 char in TCVN3 lower mapping table.
pub(crate) fn match_tcvn3_single(c: char) -> Option<char> {
    let code = c as u32;
    if (0x80..=0xFF).contains(&code) { TCVN3_LOWER_MAP[(code - 0x80) as usize] } else { None }
}

/// Check whether a raw byte slice (ignoring AutoCAD `%%` control escapes) contains uppercase ASCII letters
/// and no lowercase ASCII letters.
pub(crate) fn is_tcvn3_bytes_all_caps(bytes: &[u8]) -> bool {
    let mut has_upper = false;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && bytes.get(i + 1) == Some(&b'%') {
            // Why: In AutoCAD DXF/MTEXT, `%%%` is an escaped literal `%`
            // (so `A%%%%%d` = `A` + `%%%` + `%%d` degree symbol, keeping `d`
            // inside a control code rather than exposing it as lowercase text).
            if bytes.get(i + 2) == Some(&b'%') {
                i += 3;
                continue;
            }
            if let Some(&code) = bytes.get(i + 2) {
                // Documented AutoCAD single-character inline control codes.
                if matches!(code, b'c' | b'C' | b'd' | b'D' | b'p' | b'P' | b'u' | b'U' | b'o' | b'O' | b'k' | b'K') {
                    i += 3;
                    continue;
                }
                // AutoCAD 3-digit character code (`%%nnn`).
                if code.is_ascii_digit() && bytes.get(i + 3).is_some_and(u8::is_ascii_digit) && bytes.get(i + 4).is_some_and(u8::is_ascii_digit) {
                    i += 5;
                    continue;
                }
            }
            // Why: Unknown `%%x` sequences are not control codes; skip only `%%`
            // so `x` remains visible to the lowercase check.
            i += 2;
            continue;
        }
        let b = bytes[i];
        if b.is_ascii_lowercase() {
            return false;
        }
        if b.is_ascii_uppercase() {
            has_upper = true;
        }
        i += 1;
    }
    has_upper
}

/// Match composite uppercase sequences in `.VnTime` (e.g. 'A' + 0xB5 -> 'À', '¢' + 0xCA -> 'Ấ').
pub(crate) fn match_tcvn3_composite(c1: char, c2: char) -> Option<&'static str> {
    match (c1, c2) {
        ('A', 'µ') => Some("À"),
        ('A', '¸') => Some("Á"),
        ('A', '·') => Some("Ã"),
        ('A', '¹') => Some("Ạ"),
        ('A', '¶') => Some("Ả"),
        ('E', 'Ì') => Some("È"),
        ('E', 'Ð') => Some("É"),
        ('E', 'Ñ') => Some("Ẹ"),
        ('E', 'Î') => Some("Ẻ"),
        ('E', 'Ï') => Some("Ẽ"),
        ('I', '×') => Some("Ì"),
        ('I', 'Ý') => Some("Í"),
        ('I', 'Ü') => Some("Ĩ"),
        ('I', 'Ø') => Some("Ỉ"),
        ('I', 'Þ') => Some("Ị"),
        ('O', 'ß') => Some("Ò"),
        ('O', 'ã') => Some("Ó"),
        ('O', 'â') => Some("Õ"),
        ('O', 'ä') => Some("Ọ"),
        ('O', 'á') => Some("Ỏ"),
        ('U', 'ï') => Some("Ù"),
        ('U', 'ó') => Some("Ú"),
        ('U', 'ò') => Some("Ũ"),
        ('U', 'ô') => Some("Ụ"),
        ('U', 'ñ') => Some("Ủ"),
        ('Y', 'ý') => Some("Ý"),
        ('Y', 'ú') => Some("Ỳ"),
        ('Y', 'þ') => Some("Ỵ"),
        ('Y', 'û') => Some("Ỷ"),
        ('Y', 'ü') => Some("Ỹ"),
        ('¢', 'Ê') => Some("Ấ"),
        ('¢', 'Ç') => Some("Ầ"),
        ('¢', 'È') => Some("Ẩ"),
        ('¢', 'É') => Some("Ẫ"),
        ('¢', 'Ë') => Some("Ậ"),
        ('¡', '¾') => Some("Ắ"),
        ('¡', '»') => Some("Ằ"),
        ('¡', '¼') => Some("Ẳ"),
        ('¡', '½') => Some("Ẵ"),
        ('¡', 'Æ') => Some("Ặ"),
        ('£', 'Õ') => Some("Ế"),
        ('£', 'Ò') => Some("Ề"),
        ('£', 'Ó') => Some("Ể"),
        ('£', 'Ô') => Some("Ễ"),
        ('£', 'Ö') => Some("Ệ"),
        ('¤', 'è') => Some("Ố"),
        ('¤', 'å') => Some("Ồ"),
        ('¤', 'æ') => Some("Ổ"),
        ('¤', 'ç') => Some("Ỗ"),
        ('¤', 'é') => Some("Ộ"),
        ('¥', 'í') => Some("Ớ"),
        ('¥', 'ê') => Some("Ờ"),
        ('¥', 'ë') => Some("Ở"),
        ('¥', 'ì') => Some("Ỡ"),
        ('¥', 'î') => Some("Ợ"),
        ('¦', 'ø') => Some("Ứ"),
        ('¦', 'õ') => Some("Ừ"),
        ('¦', 'ö') => Some("Ử"),
        ('¦', '÷') => Some("Ữ"),
        ('¦', 'ù') => Some("Ự"),
        _ => None,
    }
}

/// Pure decoding of a TCVN3 byte slice into Unicode.
///
/// - If `is_upper` is true, the uppercase table (`TCVN3_UPPER_MAP` / `.VnTimeH`) is applied.
/// - If `is_upper` is false, high bytes are decoded strictly via `TCVN3_LOWER_MAP` without
///   inferring all-caps from surrounding ASCII letters.
/// - If `mode` is `DecodeMode::Strict`, CP1252 punctuation bytes (`0x80..=0x9F`) return `DecodeError::UnmappedByte`.
/// - If `mode` is `DecodeMode::Practical` (default), CP1252 punctuation and typographic symbols are preserved.
pub fn decode_tcvn3(input: &[u8], is_upper: bool, mode: DecodeMode) -> Result<Cow<'_, str>, DecodeError> {
    decode_tcvn3_inner(input, is_upper, false, mode)
}

pub(super) fn decode_tcvn3_practical_facade(input: &[u8]) -> Result<Cow<'_, str>, DecodeError> {
    decode_tcvn3_inner(input, false, is_tcvn3_bytes_all_caps(input), DecodeMode::Practical)
}

fn decode_tcvn3_inner(input: &[u8], is_upper: bool, inferred_all_caps: bool, mode: DecodeMode) -> Result<Cow<'_, str>, DecodeError> {
    let enc = if is_upper { LegacyEncoding::Tcvn3H } else { LegacyEncoding::Tcvn3 };

    if input.is_ascii() {
        if !is_upper {
            if let Ok(s) = std::str::from_utf8(input) {
                return Ok(Cow::Borrowed(s));
            }
            return Err(DecodeError::InvalidUtf8 { offset: 0 });
        }
        if let Ok(s) = std::str::from_utf8(input) {
            if s.chars().all(|c| !c.is_ascii_lowercase()) {
                return Ok(Cow::Borrowed(s));
            }
            return Ok(Cow::Owned(s.to_uppercase()));
        }
        return Err(DecodeError::InvalidUtf8 { offset: 0 });
    }

    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let len = input.len();
    let all_caps = !is_upper && inferred_all_caps;

    while i < len {
        let byte = input[i];

        // 2-byte composite sequences in .VnTime
        if i + 1 < len {
            let next_byte = input[i + 1];
            if let Some(comp) = match_tcvn3_composite(byte as char, next_byte as char) {
                if is_upper || all_caps {
                    out.extend(comp.chars().flat_map(|c| c.to_uppercase()));
                } else {
                    out.push_str(comp);
                }
                i += 2;
                continue;
            }
        }

        // Single byte mapping
        if byte < 0x80 {
            if is_upper && byte.is_ascii_lowercase() {
                out.push(byte.to_ascii_uppercase() as char);
            } else {
                out.push(byte as char);
            }
        } else {
            let table_idx = (byte - 0x80) as usize;
            let mapped_char = if is_upper { TCVN3_UPPER_MAP[table_idx] } else { TCVN3_LOWER_MAP[table_idx] };

            match mapped_char {
                Some(c) => {
                    if mode == DecodeMode::Strict && (0x80..=0x9F).contains(&byte) {
                        return Err(DecodeError::UnmappedByte { byte, offset: i, encoding: enc });
                    }
                    if all_caps && byte >= 0xA1 {
                        out.extend(c.to_uppercase());
                    } else {
                        out.push(c);
                    }
                }
                None => {
                    return Err(DecodeError::UnmappedByte { byte, offset: i, encoding: enc });
                }
            }
        }
        i += 1;
    }

    Ok(Cow::Owned(out))
}

/// Returns `true` for one of the 40 TCVN3 Latin-1 slots in `0xA8..=0xFE` whose Unicode
/// Latin-1 equivalents are NOT valid Vietnamese letters (`«`, `¬`, `×`, `Ø`, `Ð`, `Þ`,
/// `ß`, `Ç`, `Ü`, `þ`, etc.).
///
/// `©` (`0xA9`, TCVN3 `â`) is deliberately excluded: it is a copyright sign in both
/// repertoires, so standalone occurrences such as `"TÒA ÁN © 2026"` are Unicode text.
const fn is_unambiguous_tcvn3_latin1_marker(c: char) -> bool {
    matches!(
        c as u32,
        0xA8
            | 0xAA..=0xAE
            | 0xB5..=0xB9
            | 0xBB..=0xBE
            | 0xC6..=0xC7
            | 0xCB
            | 0xCE..=0xD1
            | 0xD6..=0xD8
            | 0xDC
            | 0xDE..=0xDF
            | 0xE4..=0xE7
            | 0xEB
            | 0xEE..=0xF1
            | 0xF6..=0xF8
            | 0xFB..=0xFC
            | 0xFE
    )
}

/// Returns `true` when `c` is a letter in the TCVN3 tables.
#[inline]
fn is_tcvn3_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || match_tcvn3_single(c).is_some_and(char::is_alphabetic)
}

/// Returns `true` when `c` is a vowel in the Latin/Vietnamese repertoire.
#[inline]
const fn is_unicode_vowel(c: char) -> bool {
    matches!(
        c,
        'a' | 'e'
            | 'i'
            | 'o'
            | 'u'
            | 'y'
            | 'A'
            | 'E'
            | 'I'
            | 'O'
            | 'U'
            | 'Y'
            | 'à'
            | 'á'
            | 'â'
            | 'ã'
            | 'è'
            | 'é'
            | 'ê'
            | 'ì'
            | 'í'
            | 'ò'
            | 'ó'
            | 'ô'
            | 'õ'
            | 'ù'
            | 'ú'
            | 'ý'
            | 'À'
            | 'Á'
            | 'Â'
            | 'Ã'
            | 'È'
            | 'É'
            | 'Ê'
            | 'Ì'
            | 'Í'
            | 'Ò'
            | 'Ó'
            | 'Ô'
            | 'Õ'
            | 'Ù'
            | 'Ú'
            | 'Ý'
    )
}

/// Returns `true` when a single whitespace-delimited token carries an unambiguous
/// TCVN3 Latin-1 widened marker embedded alongside at least one ASCII letter
/// (`kh«ng`, `h¬n`, `c×nh`, `KH«NG`).
///
/// # Why
/// Standalone punctuation or symbols such as `©` (`0xA9` in `"TÒA ÁN © 2026"`) or the
/// middle dot `·` (`0xB7` in `"TÒA NHÀ · A"`) are valid Unicode separators, so they must
/// not bypass the Unicode guard. The middle dot is also TCVN3 `ã`/`Ã`, so it still counts
/// when it sits inside a multi-character token built only from TCVN3 letters (`"®·"` is
/// TCVN3 `"đã"` and has no ASCII letter at all).
fn token_has_contextual_tcvn3_marker(token: &str) -> bool {
    let word = token.trim_matches(|c: char| c.is_ascii_punctuation());
    let mut char_count = 0_usize;
    let mut has_middle_dot = false;
    let mut all_tcvn3_letters = true;
    let mut has_ascii_alpha = false;
    let mut has_marker = false;

    for c in word.chars() {
        char_count += 1;
        if c == '\u{00B7}' {
            has_middle_dot = true;
        }
        if !is_tcvn3_letter(c) {
            all_tcvn3_letters = false;
        }
        if c.is_ascii_alphabetic() {
            has_ascii_alpha = true;
        }
        if is_unambiguous_tcvn3_latin1_marker(c) {
            has_marker = true;
        }
    }

    (has_middle_dot && char_count >= 2 && all_tcvn3_letters) || (has_ascii_alpha && has_marker)
}

/// Checks whether a standalone Latin-1 word under a lowercase `.VnTime` hint
/// violates Vietnamese phonotactics as Unicode while decoding to valid TCVN3:
/// 1. Mid-word uppercase Latin-1 vowel after lowercase ASCII (`"bÊn"`, `"tÝnh"`, `"vÒ"`)
///    unless it opens a Unicode vowel cluster (`"tÔi"` stays, `"nÒn"` decodes to `"nền"`).
/// 2. Word-initial `{È, Ê, Ì, Ò, Ô, Õ, Ý}` closed by `-p/-t/-c/-ch` (`"Ých"`, `"Ýt"`, `"Êt"`, `"Õp"`),
///    which violates Vietnamese checked-syllable tone constraints in Unicode. Acute `É`
///    is excluded because `"Ép"`, `"Ét"`, `"Éc"`, `"Éch"` are valid Vietnamese.
/// 3. Word-final open `'â'` (`0xE2` -> `'õ'`, e.g. `"gâ"`, `"râ"`, `"ngâ"`), since `'â'` cannot end
///    an open syllable in Vietnamese.
fn is_false_unicode_tcvn3_token(token: &str) -> bool {
    let trimmed = token.trim_matches(|c: char| c.is_ascii_punctuation());
    if trimmed.is_empty() {
        return false;
    }

    // Rule 2a: Lowercase ASCII followed immediately by an uppercase Latin-1 vowel
    // (e.g. "bÊn" -> "bến", "tÝnh" -> "tính", "vÒ" -> "về").
    // Why: A mixed-case Unicode word such as `"tÔi"` is genuine text only because the
    // capital vowel opens a valid vowel cluster (`Ôi`); when that same capital is
    // flanked by consonants (`"nÒn"` = TCVN3 `"nền"`), the capital is a `.VnTime`
    // substitution and the token must decode. Lowercase ASCII alone cannot tell the
    // two apart, so the adjacent Unicode vowel does.
    let mut prev_is_ascii_lower = false;
    let mut chars = trimmed.chars().peekable();
    while let Some(c) = chars.next() {
        let is_upper_latin1_vowel = matches!(c as u32, 0xC8..=0xCA | 0xCC | 0xD2..=0xD5 | 0xDD);
        if prev_is_ascii_lower && is_upper_latin1_vowel && !chars.peek().is_some_and(|&next| is_unicode_vowel(next)) {
            return true;
        }
        prev_is_ascii_lower = c.is_ascii_lowercase();
    }

    // Rule 2b: Checked-syllable tone violation on word-initial uppercase Latin-1 vowel.
    // Why: Vietnamese syllables closed by `-p`, `-t`, `-c`, `-ch` can only carry Sắc
    // or Nặng, so `È, Ê, Ì, Ò, Ô, Õ, Ý` cannot take those codas. Acute `É` (`0xC9`) is
    // deliberately absent: `"Ép tôn"` is valid Vietnamese and must stay untouched.
    let mut chars = trimmed.chars();
    if let Some(first) = chars.next() {
        let rest = chars.as_str();
        let is_non_checked_upper_vowel = matches!(first as u32, 0xC8 | 0xCA | 0xCC | 0xD2 | 0xD4..=0xD5 | 0xDD);
        if is_non_checked_upper_vowel
            && !rest.is_empty()
            && rest.chars().all(|c| c.is_ascii_alphabetic())
            && (rest.eq_ignore_ascii_case("p") || rest.eq_ignore_ascii_case("t") || rest.eq_ignore_ascii_case("c") || rest.eq_ignore_ascii_case("ch"))
        {
            return true;
        }
    }

    // Rule 3b: Open syllable ending in 'â' (0xE2 -> 'õ' in TCVN3, e.g. "gâ" -> "gõ", "ngâ" -> "ngõ").
    if let Some(onset) = trimmed.strip_suffix('\u{00E2}')
        && !onset.is_empty()
        && onset.chars().all(|c| c.is_ascii_alphabetic())
    {
        return true;
    }

    false
}

/// Returns `true` when an explicit `Tcvn3` / `Tcvn3H` font hint should bypass
/// `is_already_unicode_vietnamese` for a Latin-1 representable string.
pub(super) fn should_bypass_unicode_guard_for_tcvn3(s: &str, is_uppercase_font: bool) -> bool {
    if !s.chars().all(|c| u32::from(c) <= 0xFF) {
        return false;
    }
    s.split_whitespace().any(|tok| token_has_contextual_tcvn3_marker(tok) || (!is_uppercase_font && is_false_unicode_tcvn3_token(tok)))
}

#[inline]
pub(super) fn transcode_tcvn3_unchecked(s: &str) -> Cow<'_, str> {
    if s.is_ascii() || !s.chars().all(|c| u32::from(c) <= 0xFF) {
        return Cow::Borrowed(s);
    }

    let bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
    let all_caps = is_tcvn3_bytes_all_caps(&bytes);
    match decode_tcvn3_inner(&bytes, false, all_caps, DecodeMode::Practical) {
        Ok(decoded) => {
            let nfc = super::apply_vietnamese_nfc_cow(decoded);
            if nfc.as_ref() == s { Cow::Borrowed(s) } else { Cow::Owned(nfc.into_owned()) }
        }
        Err(_) => Cow::Borrowed(s),
    }
}

/// Transcode a TCVN3 (ABC) standard string to Unicode NFC.
pub fn transcode_tcvn3(s: &str) -> Cow<'_, str> {
    if s.is_ascii() || !s.chars().all(|c| u32::from(c) <= 0xFF) {
        return Cow::Borrowed(s);
    }
    // Why: Bypass the Unicode heuristic when a `.VnTime` hint accompanies
    // unambiguous TCVN3 markers (e.g. `"kh«ng Ýt h¬n"` -> `"không ít hơn"`)
    // or narrow ambiguous Latin-1 words like `"Ých"` -> `"ích"`.
    if !should_bypass_unicode_guard_for_tcvn3(s, false) && is_already_unicode_vietnamese(s) {
        return Cow::Borrowed(s);
    }

    transcode_tcvn3_unchecked(s)
}

#[inline]
pub(super) fn transcode_tcvn3_h_unchecked(s: &str) -> Cow<'_, str> {
    if s.is_ascii() {
        if !s.bytes().any(|b| b.is_ascii_lowercase()) {
            return Cow::Borrowed(s);
        }
        return Cow::Owned(s.to_ascii_uppercase());
    }
    // Why: Check Latin-1 representability before allocating `bytes` so strings
    // with non-Latin-1 characters (e.g. `"Plan 🙂"`) stay zero-allocation.
    if !s.chars().all(|c| u32::from(c) <= 0xFF) {
        return Cow::Borrowed(s);
    }

    let bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
    match decode_tcvn3(&bytes, true, DecodeMode::Practical) {
        Ok(decoded) => {
            let nfc = super::apply_vietnamese_nfc_cow(decoded);
            if nfc.as_ref() == s { Cow::Borrowed(s) } else { Cow::Owned(nfc.into_owned()) }
        }
        Err(_) => Cow::Borrowed(s),
    }
}

/// Transcodes a Latin-1 widened string using the uppercase TCVN3H (`.VnTimeH`) mapping.
///
/// Vietnamese letters and ASCII `a..=z` are mapped to uppercase, whereas
/// CP1252 fallback characters in `0x80..=0x9F` (such as `š` or `œ`) preserve
/// their original case. Returns `Cow::Borrowed(s)` without allocating when
/// `s` is already uppercase ASCII, non-Latin-1, or recognized as Unicode
/// Vietnamese without TCVN3 markers.
pub fn transcode_tcvn3_h(s: &str) -> Cow<'_, str> {
    if s.is_ascii() {
        return transcode_tcvn3_h_unchecked(s);
    }
    if !s.chars().all(|c| u32::from(c) <= 0xFF) {
        return Cow::Borrowed(s);
    }
    if !should_bypass_unicode_guard_for_tcvn3(s, true) && is_already_unicode_vietnamese(s) {
        return Cow::Borrowed(s);
    }

    transcode_tcvn3_h_unchecked(s)
}
