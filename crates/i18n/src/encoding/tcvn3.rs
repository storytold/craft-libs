//! TCVN3 / ABC (TCVN 5712:1993) legacy Vietnamese 8-bit encoding tables and transcoding.
//!
//! Standardized by the Directorate for Standards, Metrology and Quality (STAMEQ),
//! Ministry of Science, Technology and Environment of Vietnam (1993).
//! Cross-verified with GNU C Library (`glibc`) official charmap specification `TCVN5712-1`.
//! All conversions deterministically emit standard precomposed Unicode NFC (TCVN 6909:2001).

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

/// TCVN3 Lower Table (.VnTime) for high bytes `0x80..=0xFF`.
///
/// Contains the 7 base uppercase characters in slots `0xA1..=0xA7` (Ă, Â, Ê, Ô, Ơ, Ư, Đ),
/// base lowercase characters `0xA8..=0xAE`, and all accented lowercase vowels.
pub const TCVN3_LOWER_MAP: [Option<char>; 128] = [
    /* 0x80 */ Some('\u{20AC}'), // CP1252 EURO SIGN (€)
    /* 0x81 */ None,
    /* 0x82 */ Some('\u{201A}'), // CP1252 SINGLE LOW-9 QUOTATION MARK (‚)
    /* 0x83 */ Some('\u{0192}'), // CP1252 LATIN SMALL LETTER F WITH HOOK (ƒ)
    /* 0x84 */ Some('\u{201E}'), // CP1252 DOUBLE LOW-9 QUOTATION MARK („)
    /* 0x85 */ Some('\u{2026}'), // CP1252 HORIZONTAL ELLIPSIS (…)
    /* 0x86 */ Some('\u{2020}'), // CP1252 DAGGER (†)
    /* 0x87 */ Some('\u{2021}'), // CP1252 DOUBLE DAGGER (‡)
    /* 0x88 */ Some('\u{02C6}'), // CP1252 MODIFIER LETTER CIRCUMFLEX ACCENT (ˆ)
    /* 0x89 */ Some('\u{2030}'), // CP1252 PER MILLE SIGN (‰)
    /* 0x8A */ Some('\u{0160}'), // CP1252 LATIN CAPITAL LETTER S WITH CARON (Š)
    /* 0x8B */ Some('\u{2039}'), // CP1252 SINGLE LEFT-POINTING ANGLE QUOTATION MARK (‹)
    /* 0x8C */ Some('\u{0152}'), // CP1252 LATIN CAPITAL LIGATURE OE (Œ)
    /* 0x8D */ None,
    /* 0x8E */ Some('\u{017D}'), // CP1252 LATIN CAPITAL LETTER Z WITH CARON (Ž)
    /* 0x8F */ None,
    /* 0x90 */ None,
    /* 0x91 */ Some('\u{2018}'), // CP1252 LEFT SINGLE QUOTATION MARK (‘)
    /* 0x92 */ Some('\u{2019}'), // CP1252 RIGHT SINGLE QUOTATION MARK (’)
    /* 0x93 */ Some('\u{201C}'), // CP1252 LEFT DOUBLE QUOTATION MARK (“)
    /* 0x94 */ Some('\u{201D}'), // CP1252 RIGHT DOUBLE QUOTATION MARK (”)
    /* 0x95 */ Some('\u{2022}'), // CP1252 BULLET (•)
    /* 0x96 */ Some('\u{2013}'), // CP1252 EN DASH (–)
    /* 0x97 */ Some('\u{2014}'), // CP1252 EM DASH (—)
    /* 0x98 */ Some('\u{02DC}'), // CP1252 SMALL TILDE (˜)
    /* 0x99 */ Some('\u{2122}'), // CP1252 TRADE MARK SIGN (™)
    /* 0x9A */ Some('\u{0161}'), // CP1252 LATIN SMALL LETTER S WITH CARON (š)
    /* 0x9B */ Some('\u{203A}'), // CP1252 SINGLE RIGHT-POINTING ANGLE QUOTATION MARK (›)
    /* 0x9C */ Some('\u{0153}'), // CP1252 LATIN SMALL LIGATURE OE (œ)
    /* 0x9D */ None,
    /* 0x9E */ Some('\u{017E}'), // CP1252 LATIN SMALL LETTER Z WITH CARON (ž)
    /* 0x9F */ Some('\u{0178}'), // CP1252 LATIN CAPITAL LETTER Y WITH DIAERESIS (Ÿ)
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
];

/// TCVN3 Upper Table (.VnTimeH) for high bytes `0x80..=0xFF`.
///
/// Maps all lowercase vowel slots to their uppercase Unicode equivalents for all-caps fonts.
pub const TCVN3_UPPER_MAP: [Option<char>; 128] = [
    /* 0x80 */ Some('\u{20AC}'), // CP1252 EURO SIGN (€)
    /* 0x81 */ None,
    /* 0x82 */ Some('\u{201A}'), // CP1252 SINGLE LOW-9 QUOTATION MARK (‚)
    /* 0x83 */ Some('\u{0192}'), // CP1252 LATIN SMALL LETTER F WITH HOOK (ƒ)
    /* 0x84 */ Some('\u{201E}'), // CP1252 DOUBLE LOW-9 QUOTATION MARK („)
    /* 0x85 */ Some('\u{2026}'), // CP1252 HORIZONTAL ELLIPSIS (…)
    /* 0x86 */ Some('\u{2020}'), // CP1252 DAGGER (†)
    /* 0x87 */ Some('\u{2021}'), // CP1252 DOUBLE DAGGER (‡)
    /* 0x88 */ Some('\u{02C6}'), // CP1252 MODIFIER LETTER CIRCUMFLEX ACCENT (ˆ)
    /* 0x89 */ Some('\u{2030}'), // CP1252 PER MILLE SIGN (‰)
    /* 0x8A */ Some('\u{0160}'), // CP1252 LATIN CAPITAL LETTER S WITH CARON (Š)
    /* 0x8B */ Some('\u{2039}'), // CP1252 SINGLE LEFT-POINTING ANGLE QUOTATION MARK (‹)
    /* 0x8C */ Some('\u{0152}'), // CP1252 LATIN CAPITAL LIGATURE OE (Œ)
    /* 0x8D */ None,
    /* 0x8E */ Some('\u{017D}'), // CP1252 LATIN CAPITAL LETTER Z WITH CARON (Ž)
    /* 0x8F */ None,
    /* 0x90 */ None,
    /* 0x91 */ Some('\u{2018}'), // CP1252 LEFT SINGLE QUOTATION MARK (‘)
    /* 0x92 */ Some('\u{2019}'), // CP1252 RIGHT SINGLE QUOTATION MARK (’)
    /* 0x93 */ Some('\u{201C}'), // CP1252 LEFT DOUBLE QUOTATION MARK (“)
    /* 0x94 */ Some('\u{201D}'), // CP1252 RIGHT DOUBLE QUOTATION MARK (”)
    /* 0x95 */ Some('\u{2022}'), // CP1252 BULLET (•)
    /* 0x96 */ Some('\u{2013}'), // CP1252 EN DASH (–)
    /* 0x97 */ Some('\u{2014}'), // CP1252 EM DASH (—)
    /* 0x98 */ Some('\u{02DC}'), // CP1252 SMALL TILDE (˜)
    /* 0x99 */ Some('\u{2122}'), // CP1252 TRADE MARK SIGN (™)
    /* 0x9A */ Some('\u{0161}'), // CP1252 LATIN SMALL LETTER S WITH CARON (š)
    /* 0x9B */ Some('\u{203A}'), // CP1252 SINGLE RIGHT-POINTING ANGLE QUOTATION MARK (›)
    /* 0x9C */ Some('\u{0153}'), // CP1252 LATIN SMALL LIGATURE OE (œ)
    /* 0x9D */ None,
    /* 0x9E */ Some('\u{017E}'), // CP1252 LATIN SMALL LETTER Z WITH CARON (ž)
    /* 0x9F */ Some('\u{0178}'), // CP1252 LATIN CAPITAL LETTER Y WITH DIAERESIS (Ÿ)
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
];

/// Match a single Latin-1 char in TCVN3 lower mapping table.
pub(crate) fn match_tcvn3_single(c: char) -> Option<char> {
    let code = c as u32;
    if (0x80..=0xFF).contains(&code) { TCVN3_LOWER_MAP[(code - 0x80) as usize] } else { None }
}

/// Check whether an ASCII string or %-escaped CAD control represents uppercase TCVN3 text.
pub(crate) fn is_tcvn3_all_caps(s: &str) -> bool {
    let mut has_upper = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' && chars.peek() == Some(&'%') {
            chars.next();
            if let Some(&next) = chars.peek() {
                if next.is_ascii_alphabetic() {
                    chars.next();
                    continue;
                }
                if next.is_ascii_digit() {
                    for _ in 0..3 {
                        if let Some(&d) = chars.peek() {
                            if d.is_ascii_digit() {
                                chars.next();
                            } else {
                                break;
                            }
                        }
                    }
                    continue;
                }
            }
            continue;
        }
        if c.is_ascii_lowercase() {
            return false;
        }
        if c.is_ascii_uppercase() {
            has_upper = true;
        }
    }
    has_upper
}

/// Check whether a raw byte slice contains only uppercase ASCII letters and no lowercase ASCII letters.
pub(crate) fn is_tcvn3_bytes_all_caps(bytes: &[u8]) -> bool {
    let mut has_upper = false;
    for &b in bytes {
        if b.is_ascii_lowercase() {
            return false;
        }
        if b.is_ascii_uppercase() {
            has_upper = true;
        }
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
/// - If `mode` is `DecodeMode::Strict`, CP1252 punctuation bytes (`0x80..=0x9F`) return `DecodeError::UnmappedByte`.
/// - If `mode` is `DecodeMode::Practical` (default), CP1252 punctuation and typographic symbols are preserved.
pub fn decode_tcvn3(input: &[u8], is_upper: bool, mode: DecodeMode) -> Result<Cow<'_, str>, DecodeError> {
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
    let all_caps = if !is_upper { is_tcvn3_bytes_all_caps(input) } else { false };

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
                    if all_caps {
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

/// Transcode a TCVN3 (ABC) standard string to Unicode NFC.
pub fn transcode_tcvn3(s: &str) -> Cow<'_, str> {
    if s.is_ascii() || is_already_unicode_vietnamese(s) {
        return Cow::Borrowed(s);
    }

    let all_caps = is_tcvn3_all_caps(s);
    let mut bytes = Vec::with_capacity(s.len());
    for c in s.chars() {
        let code = c as u32;
        if code > 0xFF {
            return Cow::Borrowed(s);
        }
        bytes.push(code as u8);
    }

    match decode_tcvn3(&bytes, all_caps, DecodeMode::Practical) {
        Ok(decoded) => {
            let nfc = super::normalize_nfc(&decoded);
            if nfc.as_ref() == s { Cow::Borrowed(s) } else { Cow::Owned(nfc.into_owned()) }
        }
        Err(_) => Cow::Borrowed(s),
    }
}

/// Transcode a TCVN3H all-uppercase string to Unicode NFC uppercase.
pub fn transcode_tcvn3_h(s: &str) -> Cow<'_, str> {
    if s.is_ascii() {
        let upper = s.to_uppercase();
        if upper == s { Cow::Borrowed(s) } else { Cow::Owned(upper) }
    } else {
        let mut bytes = Vec::with_capacity(s.len());
        for c in s.chars() {
            let code = c as u32;
            if code > 0xFF {
                return Cow::Borrowed(s);
            }
            bytes.push(code as u8);
        }

        match decode_tcvn3(&bytes, true, DecodeMode::Practical) {
            Ok(decoded) => {
                let nfc = super::normalize_nfc(&decoded);
                let upper = nfc.to_uppercase();
                if upper == s { Cow::Borrowed(s) } else { Cow::Owned(upper) }
            }
            Err(_) => Cow::Borrowed(s),
        }
    }
}
