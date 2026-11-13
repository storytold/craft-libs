//! VNI-Windows legacy Vietnamese encoding tables and transcoding (2-byte composite & single-byte).
//!
//! Based on de facto industry conventions established by VNI Software Company (c. 1991–1995),
//! cross-verified with UniKey core engine (`vnconv` / `vni.tab`) by Phạm Kim Long and
//! CPAN `Encode::VN::VNI`.
//! All conversions deterministically emit standard precomposed Unicode NFC (TCVN 6909:2001).

use std::borrow::Cow;

use super::detect::is_already_unicode_vietnamese;
use super::{DecodeError, DecodeMode, LegacyEncoding};

/// VNI-Windows single byte lookup table for high bytes `0x80..=0xFF`.
pub const VNI_SINGLE_MAP: [Option<char>; 128] = [
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
    /* 0xA0 */ Some(' '), // NO-BREAK SPACE
    /* 0xA1 */ None,
    /* 0xA2 */ None,
    /* 0xA3 */ None,
    /* 0xA4 */ None,
    /* 0xA5 */ None,
    /* 0xA6 */ None,
    /* 0xA7 */ None,
    /* 0xA8 */ None,
    /* 0xA9 */ None,
    /* 0xAA */ None,
    /* 0xAB */ None,
    /* 0xAC */ None,
    /* 0xAD */ None,
    /* 0xAE */ None,
    /* 0xAF */ Some('¯'), // macron
    /* 0xB0 */ Some('°'), // degree
    /* 0xB1 */ Some('±'), // plus-minus
    /* 0xB2 */ Some('²'), // superscript 2
    /* 0xB3 */ Some('³'), // superscript 3
    /* 0xB4 */ Some('´'), // acute accent
    /* 0xB5 */ Some('µ'), // micro
    /* 0xB6 */ Some('¶'), // pilcrow
    /* 0xB7 */ Some('·'), // middle dot
    /* 0xB8 */ Some('¸'), // cedilla
    /* 0xB9 */ Some('¹'), // superscript 1
    /* 0xBA */ Some('º'), // masculine ordinal
    /* 0xBB */ Some('»'), // right double angle
    /* 0xBC */ Some('¼'), // fraction 1/4
    /* 0xBD */ Some('½'), // fraction 1/2
    /* 0xBE */ Some('¾'), // fraction 3/4
    /* 0xBF */ Some('¿'), // inverted question mark
    /* 0xC0 */ None,
    /* 0xC1 */ None,
    /* 0xC2 */ None,
    /* 0xC3 */ None,
    /* 0xC4 */ None,
    /* 0xC5 */ None,
    /* 0xC6 */ Some('Ỉ'), // VNI capital Ỉ
    /* 0xC7 */ None,
    /* 0xC8 */ None,
    /* 0xC9 */ None,
    /* 0xCA */ None,
    /* 0xCB */ None,
    /* 0xCC */ None,
    /* 0xCD */ None,
    /* 0xCE */ Some('Ỵ'), // VNI capital Ỵ
    /* 0xCF */ None,
    /* 0xD0 */ None,
    /* 0xD1 */ Some('Đ'), // VNI capital Đ
    /* 0xD2 */ Some('Ị'), // VNI capital Ị
    /* 0xD3 */ Some('Ĩ'), // VNI capital Ĩ
    /* 0xD4 */ Some('Ơ'), // VNI capital Ơ
    /* 0xD5 */ None,
    /* 0xD6 */ Some('Ư'), // VNI capital Ư
    /* 0xD7 */ Some('×'), // multiplication sign
    /* 0xD8 */ None,
    /* 0xD9 */ None,
    /* 0xDA */ None,
    /* 0xDB */ None,
    /* 0xDC */ None,
    /* 0xDD */ None,
    /* 0xDE */ None,
    /* 0xDF */ None,
    /* 0xE0 */ None,
    /* 0xE1 */ None,
    /* 0xE2 */ None,
    /* 0xE3 */ None,
    /* 0xE4 */ None,
    /* 0xE5 */ None,
    /* 0xE6 */ Some('ỉ'), // VNI small ỉ
    /* 0xE7 */ None,
    /* 0xE8 */ None,
    /* 0xE9 */ None,
    /* 0xEA */ None,
    /* 0xEB */ None,
    /* 0xEC */ None,
    /* 0xED */ None,
    /* 0xEE */ Some('ỵ'), // VNI small ỵ
    /* 0xEF */ None,
    /* 0xF0 */ None,
    /* 0xF1 */ Some('đ'), // VNI small đ
    /* 0xF2 */ Some('ị'), // VNI small ị
    /* 0xF3 */ Some('ĩ'), // VNI small ĩ
    /* 0xF4 */ Some('ơ'), // VNI small ơ
    /* 0xF5 */ None,
    /* 0xF6 */ Some('ư'), // VNI small ư
    /* 0xF7 */ Some('÷'), // division sign
    /* 0xF8 */ None,
    /* 0xF9 */ None,
    /* 0xFA */ None,
    /* 0xFB */ None,
    /* 0xFC */ None,
    /* 0xFD */ None,
    /* 0xFE */ None,
    /* 0xFF */ None,
];

/// Match a single Latin-1 char in VNI single mapping table.
pub(crate) fn match_vni_single(c: char) -> Option<char> {
    let code = c as u32;
    if (0x80..=0xFF).contains(&code) { VNI_SINGLE_MAP[(code - 0x80) as usize] } else { None }
}

/// Match a 2-byte composite sequence in VNI-Windows (e.g. 'a' + 0xF8 -> 'à', 'a' + 0xF9 -> 'á').
pub(crate) fn match_vni_composite(c1: char, c2: char) -> Option<char> {
    match (c1, c2) {
        ('a', 'ø') => Some('à'),
        ('a', 'ù') => Some('á'),
        ('a', 'ú') => Some('ả'),
        ('a', 'û') => Some('ả'),
        ('a', 'õ') => Some('ã'),
        ('a', 'ï') => Some('ạ'),
        ('ă', 'ø') => Some('ằ'),
        ('ă', 'ù') => Some('ắ'),
        ('ă', 'ú') => Some('ẳ'),
        ('ă', 'û') => Some('ẳ'),
        ('ă', 'õ') => Some('ẵ'),
        ('ă', 'ï') => Some('ặ'),
        ('â', 'ø') => Some('ầ'),
        ('â', 'ù') => Some('ấ'),
        ('â', 'ú') => Some('ẩ'),
        ('â', 'û') => Some('ẩ'),
        ('â', 'õ') => Some('ẫ'),
        ('â', 'ï') => Some('ậ'),
        ('e', 'ø') => Some('è'),
        ('e', 'ù') => Some('é'),
        ('e', 'ú') => Some('ẻ'),
        ('e', 'û') => Some('ẻ'),
        ('e', 'õ') => Some('ẽ'),
        ('e', 'ï') => Some('ẹ'),
        ('ê', 'ø') => Some('ề'),
        ('ê', 'ù') => Some('ế'),
        ('ê', 'ú') => Some('ể'),
        ('ê', 'û') => Some('ể'),
        ('ê', 'õ') => Some('ễ'),
        ('ê', 'ï') => Some('ệ'),
        ('i', 'ø') => Some('ì'),
        ('i', 'ù') => Some('í'),
        ('i', 'ú') => Some('ỉ'),
        ('i', 'û') => Some('ỉ'),
        ('i', 'õ') => Some('ĩ'),
        ('i', 'ï') => Some('ị'),
        ('o', 'ø') => Some('ò'),
        ('o', 'ù') => Some('ó'),
        ('o', 'ú') => Some('ỏ'),
        ('o', 'û') => Some('ỏ'),
        ('o', 'õ') => Some('õ'),
        ('o', 'ï') => Some('ọ'),
        ('ơ', 'ø') => Some('ờ'),
        ('ơ', 'ù') => Some('ớ'),
        ('ơ', 'ú') => Some('ở'),
        ('ơ', 'û') => Some('ở'),
        ('ơ', 'õ') => Some('ỡ'),
        ('ơ', 'ï') => Some('ợ'),
        ('u', 'ø') => Some('ù'),
        ('u', 'ù') => Some('ú'),
        ('u', 'ú') => Some('ủ'),
        ('u', 'û') => Some('ủ'),
        ('u', 'õ') => Some('ũ'),
        ('u', 'ï') => Some('ụ'),
        ('ư', 'ø') => Some('ừ'),
        ('ư', 'ù') => Some('ứ'),
        ('ư', 'ú') => Some('ử'),
        ('ư', 'û') => Some('ử'),
        ('ư', 'õ') => Some('ữ'),
        ('ư', 'ï') => Some('ự'),
        ('y', 'ø') => Some('ỳ'),
        ('y', 'ù') => Some('ý'),
        ('y', 'ú') => Some('ỷ'),
        ('y', 'û') => Some('ỷ'),
        ('y', 'õ') => Some('ỹ'),
        ('y', 'ï') => Some('ỵ'),
        ('A', 'Ø') => Some('À'),
        ('A', 'ø') => Some('À'),
        ('A', 'Ù') => Some('Á'),
        ('A', 'ù') => Some('Á'),
        ('A', 'Ú') => Some('Ả'),
        ('A', 'Û') => Some('Ả'),
        ('A', 'ú') => Some('Ả'),
        ('A', 'û') => Some('Ả'),
        ('A', 'Õ') => Some('Ã'),
        ('A', 'õ') => Some('Ã'),
        ('A', 'Ï') => Some('Ạ'),
        ('A', 'ï') => Some('Ạ'),
        ('Ă', 'Ø') => Some('Ằ'),
        ('Ă', 'ø') => Some('Ằ'),
        ('Ă', 'Ù') => Some('Ắ'),
        ('Ă', 'ù') => Some('Ắ'),
        ('Ă', 'Ú') => Some('Ẳ'),
        ('Ă', 'Û') => Some('Ẳ'),
        ('Ă', 'ú') => Some('Ẳ'),
        ('Ă', 'û') => Some('Ẳ'),
        ('Ă', 'Õ') => Some('Ẵ'),
        ('Ă', 'õ') => Some('Ẵ'),
        ('Ă', 'Ï') => Some('Ặ'),
        ('Ă', 'ï') => Some('Ặ'),
        ('Â', 'Ø') => Some('Ầ'),
        ('Â', 'ø') => Some('Ầ'),
        ('Â', 'Ù') => Some('Ấ'),
        ('Â', 'ù') => Some('Ấ'),
        ('Â', 'Ú') => Some('Ẩ'),
        ('Â', 'Û') => Some('Ẩ'),
        ('Â', 'ú') => Some('Ẩ'),
        ('Â', 'û') => Some('Ẩ'),
        ('Â', 'Õ') => Some('Ẫ'),
        ('Â', 'õ') => Some('Ẫ'),
        ('Â', 'Ï') => Some('Ậ'),
        ('Â', 'ï') => Some('Ậ'),
        ('E', 'Ø') => Some('È'),
        ('E', 'ø') => Some('È'),
        ('E', 'Ù') => Some('É'),
        ('E', 'ù') => Some('É'),
        ('E', 'Ú') => Some('Ẻ'),
        ('E', 'Û') => Some('Ẻ'),
        ('E', 'ú') => Some('Ẻ'),
        ('E', 'û') => Some('Ẻ'),
        ('E', 'Õ') => Some('Ẽ'),
        ('E', 'õ') => Some('Ẽ'),
        ('E', 'Ï') => Some('Ẹ'),
        ('E', 'ï') => Some('Ẹ'),
        ('Ê', 'Ø') => Some('Ề'),
        ('Ê', 'ø') => Some('Ề'),
        ('Ê', 'Ù') => Some('Ế'),
        ('Ê', 'ù') => Some('Ế'),
        ('Ê', 'Ú') => Some('Ể'),
        ('Ê', 'Û') => Some('Ể'),
        ('Ê', 'ú') => Some('Ể'),
        ('Ê', 'û') => Some('Ể'),
        ('Ê', 'Õ') => Some('Ễ'),
        ('Ê', 'õ') => Some('Ễ'),
        ('Ê', 'Ï') => Some('Ệ'),
        ('Ê', 'ï') => Some('Ệ'),
        ('I', 'Ø') => Some('Ì'),
        ('I', 'ø') => Some('Ì'),
        ('I', 'Ù') => Some('Í'),
        ('I', 'ù') => Some('Í'),
        ('I', 'Ú') => Some('Ỉ'),
        ('I', 'Û') => Some('Ỉ'),
        ('I', 'ú') => Some('Ỉ'),
        ('I', 'û') => Some('Ỉ'),
        ('I', 'Õ') => Some('Ĩ'),
        ('I', 'õ') => Some('Ĩ'),
        ('I', 'Ï') => Some('Ị'),
        ('I', 'ï') => Some('Ị'),
        ('O', 'Ø') => Some('Ò'),
        ('O', 'ø') => Some('Ò'),
        ('O', 'Ù') => Some('Ó'),
        ('O', 'ù') => Some('Ó'),
        ('O', 'Ú') => Some('Ỏ'),
        ('O', 'Û') => Some('Ỏ'),
        ('O', 'ú') => Some('Ỏ'),
        ('O', 'û') => Some('Ỏ'),
        ('O', 'Õ') => Some('Õ'),
        ('O', 'õ') => Some('Õ'),
        ('O', 'Ï') => Some('Ọ'),
        ('O', 'ï') => Some('Ọ'),
        ('Ơ', 'Ø') => Some('Ờ'),
        ('Ơ', 'ø') => Some('Ờ'),
        ('Ơ', 'Ù') => Some('Ớ'),
        ('Ơ', 'ù') => Some('Ớ'),
        ('Ơ', 'Ú') => Some('Ở'),
        ('Ơ', 'Û') => Some('Ở'),
        ('Ơ', 'ú') => Some('Ở'),
        ('Ơ', 'û') => Some('Ở'),
        ('Ơ', 'Õ') => Some('Ỡ'),
        ('Ơ', 'õ') => Some('Ỡ'),
        ('Ơ', 'Ï') => Some('Ợ'),
        ('Ơ', 'ï') => Some('Ợ'),
        ('U', 'Ø') => Some('Ù'),
        ('U', 'ø') => Some('Ù'),
        ('U', 'Ù') => Some('Ú'),
        ('U', 'ù') => Some('Ú'),
        ('U', 'Ú') => Some('Ủ'),
        ('U', 'Û') => Some('Ủ'),
        ('U', 'ú') => Some('Ủ'),
        ('U', 'û') => Some('Ủ'),
        ('U', 'Õ') => Some('Ũ'),
        ('U', 'õ') => Some('Ũ'),
        ('U', 'Ï') => Some('Ụ'),
        ('U', 'ï') => Some('Ụ'),
        ('Ư', 'Ø') => Some('Ừ'),
        ('Ư', 'ø') => Some('Ừ'),
        ('Ư', 'Ù') => Some('Ứ'),
        ('Ư', 'ù') => Some('Ứ'),
        ('Ư', 'Ú') => Some('Ử'),
        ('Ư', 'Û') => Some('Ử'),
        ('Ư', 'ú') => Some('Ử'),
        ('Ư', 'û') => Some('Ử'),
        ('Ư', 'Õ') => Some('Ữ'),
        ('Ư', 'õ') => Some('Ữ'),
        ('Ư', 'Ï') => Some('Ự'),
        ('Ư', 'ï') => Some('Ự'),
        ('Y', 'Ø') => Some('Ỳ'),
        ('Y', 'ø') => Some('Ỳ'),
        ('Y', 'Ù') => Some('Ý'),
        ('Y', 'ù') => Some('Ý'),
        ('Y', 'Ú') => Some('Ỷ'),
        ('Y', 'Û') => Some('Ỷ'),
        ('Y', 'ú') => Some('Ỷ'),
        ('Y', 'û') => Some('Ỷ'),
        ('Y', 'Õ') => Some('Ỹ'),
        ('Y', 'õ') => Some('Ỹ'),
        ('Y', 'Ï') => Some('Ỵ'),
        ('Y', 'ï') => Some('Ỵ'),
        ('ô', 'ø') => Some('ờ'),
        ('ô', 'ù') => Some('ớ'),
        ('ô', 'ú') => Some('ở'),
        ('ô', 'û') => Some('ở'),
        ('ô', 'õ') => Some('ỡ'),
        ('ô', 'ï') => Some('ợ'),
        ('ö', 'ø') => Some('ừ'),
        ('ö', 'ù') => Some('ứ'),
        ('ö', 'ú') => Some('ử'),
        ('ö', 'û') => Some('ử'),
        ('ö', 'õ') => Some('ữ'),
        ('ö', 'ï') => Some('ự'),
        ('Ô', 'Ø') => Some('Ờ'),
        ('Ô', 'ø') => Some('Ờ'),
        ('Ô', 'Ù') => Some('Ớ'),
        ('Ô', 'ù') => Some('Ớ'),
        ('Ô', 'Ú') => Some('Ở'),
        ('Ô', 'Û') => Some('Ở'),
        ('Ô', 'ú') => Some('Ở'),
        ('Ô', 'û') => Some('Ở'),
        ('Ô', 'Õ') => Some('Ỡ'),
        ('Ô', 'õ') => Some('Ỡ'),
        ('Ô', 'Ï') => Some('Ợ'),
        ('Ô', 'ï') => Some('Ợ'),
        ('Ö', 'Ø') => Some('Ừ'),
        ('Ö', 'ø') => Some('Ừ'),
        ('Ö', 'Ù') => Some('Ứ'),
        ('Ö', 'ù') => Some('Ứ'),
        ('Ö', 'Ú') => Some('Ử'),
        ('Ö', 'Û') => Some('Ử'),
        ('Ö', 'ú') => Some('Ử'),
        ('Ö', 'û') => Some('Ử'),
        ('Ö', 'Õ') => Some('Ữ'),
        ('Ö', 'õ') => Some('Ữ'),
        ('Ö', 'Ï') => Some('Ự'),
        ('Ö', 'ï') => Some('Ự'),
        ('a', 'â') => Some('â'),
        ('a', 'ê') => Some('ă'),
        ('a', 'á') => Some('ấ'),
        ('a', 'à') => Some('ầ'),
        ('a', 'å') => Some('ẩ'),
        ('a', 'ã') => Some('ẫ'),
        ('a', 'ä') => Some('ậ'),
        ('a', 'é') => Some('ắ'),
        ('a', 'è') => Some('ằ'),
        ('a', 'ü') => Some('ẵ'),
        ('a', 'ë') => Some('ặ'),
        ('A', 'Â') => Some('Â'),
        ('A', 'â') => Some('Â'),
        ('A', 'Ê') => Some('Ă'),
        ('A', 'ê') => Some('Ă'),
        ('A', 'Á') => Some('Ấ'),
        ('A', 'á') => Some('Ấ'),
        ('A', 'À') => Some('Ầ'),
        ('A', 'à') => Some('Ầ'),
        ('A', 'Å') => Some('Ẩ'),
        ('A', 'å') => Some('Ẩ'),
        ('A', 'Ã') => Some('Ẫ'),
        ('A', 'ã') => Some('Ẫ'),
        ('A', 'Ä') => Some('Ậ'),
        ('A', 'ä') => Some('Ậ'),
        ('A', 'É') => Some('Ắ'),
        ('A', 'é') => Some('Ắ'),
        ('A', 'È') => Some('Ằ'),
        ('A', 'è') => Some('Ằ'),
        ('A', 'Ü') => Some('Ẵ'),
        ('A', 'ü') => Some('Ẵ'),
        ('A', 'Ë') => Some('Ặ'),
        ('A', 'ë') => Some('Ặ'),
        ('e', 'â') => Some('ê'),
        ('e', 'á') => Some('ế'),
        ('e', 'à') => Some('ề'),
        ('e', 'å') => Some('ể'),
        ('e', 'ã') => Some('ễ'),
        ('e', 'ä') => Some('ệ'),
        ('E', 'Â') => Some('Ê'),
        ('E', 'â') => Some('Ê'),
        ('E', 'Á') => Some('Ế'),
        ('E', 'á') => Some('Ế'),
        ('E', 'À') => Some('Ề'),
        ('E', 'à') => Some('Ề'),
        ('E', 'Å') => Some('Ể'),
        ('E', 'å') => Some('Ể'),
        ('E', 'Ã') => Some('Ễ'),
        ('E', 'ã') => Some('Ễ'),
        ('E', 'Ä') => Some('Ệ'),
        ('E', 'ä') => Some('Ệ'),
        ('o', 'â') => Some('ô'),
        ('o', 'á') => Some('ố'),
        ('o', 'à') => Some('ồ'),
        ('o', 'å') => Some('ổ'),
        ('o', 'ã') => Some('ỗ'),
        ('o', 'ä') => Some('ộ'),
        ('O', 'Â') => Some('Ô'),
        ('O', 'â') => Some('Ô'),
        ('O', 'Á') => Some('Ố'),
        ('O', 'á') => Some('Ố'),
        ('O', 'À') => Some('Ồ'),
        ('O', 'à') => Some('Ồ'),
        ('O', 'Å') => Some('Ổ'),
        ('O', 'å') => Some('Ổ'),
        ('O', 'Ã') => Some('Ỗ'),
        ('O', 'ã') => Some('Ỗ'),
        ('O', 'Ä') => Some('Ộ'),
        ('O', 'ä') => Some('Ộ'),
        _ => None,
    }
}

/// Standalone spacing accents for VNI diacritical bytes that cannot combine with a preceding base vowel.
pub(crate) fn vni_standalone_accent(byte: u8) -> Option<char> {
    match byte {
        0xF8 | 0xD8 => Some('`'),                      // Grave accent ` (U+0060)
        0xF9 | 0xD9 => Some('\u{00B4}'),               // Acute accent ´ (U+00B4)
        0xF5 | 0xD5 => Some('\u{02DC}'),               // Small tilde ˜ (U+02DC)
        0xFA | 0xDA | 0xFB | 0xDB => Some('\u{02C9}'), // Modifier letter macron (U+02C9)
        0xEF | 0xCF => Some('.'),                      // Heavy dot / underdot fallback
        0xE0 | 0xC0 => Some('`'),
        0xE1 | 0xC1 => Some('\u{00B4}'),
        0xE2 | 0xC2 => Some('^'),
        0xE3 | 0xC3 => Some('\u{02DC}'),
        0xE4 | 0xC4 => Some('.'),
        0xE5 | 0xC5 => Some('\u{02C9}'),
        0xEA | 0xCA => Some('^'),
        0xE9 | 0xC9 => Some('\u{00B4}'),
        0xE8 | 0xC8 => Some('`'),
        0xEB | 0xCB => Some('.'),
        0xFC | 0xDC => Some('\u{02DC}'),
        _ => None,
    }
}

/// Check whether a byte can act as a base character in VNI composite sequences.
#[inline]
pub(crate) fn is_vni_base_candidate(byte: u8) -> bool {
    matches!(
        byte,
        b'a' | b'A'
            | b'e' | b'E'
            | b'i' | b'I'
            | b'o' | b'O'
            | b'u' | b'U'
            | b'y' | b'Y'
            | 0xD4 // Ơ
            | 0xF4 // ơ
            | 0xD6 // Ư
            | 0xF6 // ư
            | 0xCA // Ê
            | 0xEA // ê
            | 0xC2 // Â
            | 0xE2 // â
    )
}

/// Convert a candidate base byte to its char representation for composite matching.
#[inline]
pub(crate) fn byte_to_vni_base(byte: u8) -> char {
    match byte {
        0xD4 => 'Ô', // or Ơ
        0xF4 => 'ô', // or ơ
        0xD6 => 'Ö', // or Ư
        0xF6 => 'ö', // or ư
        0xCA => 'Ê',
        0xEA => 'ê',
        0xC2 => 'Â',
        0xE2 => 'â',
        b => b as char,
    }
}

/// Decode a single VNI byte that was not combined with a neighbor.
pub(crate) fn decode_vni_single_byte(byte: u8, offset: usize, mode: DecodeMode) -> Result<char, DecodeError> {
    if byte < 0x80 {
        return Ok(byte as char);
    }
    let table_idx = (byte - 0x80) as usize;

    if mode == DecodeMode::Strict {
        if (0x80..=0x9F).contains(&byte) {
            return Err(DecodeError::UnmappedByte { byte, offset, encoding: LegacyEncoding::Vni });
        }
        if let Some(ch) = VNI_SINGLE_MAP[table_idx] {
            return Ok(ch);
        }
        return Err(DecodeError::UnmappedByte { byte, offset, encoding: LegacyEncoding::Vni });
    }

    // Practical mode: try single map first, then standalone accent fallback
    if let Some(ch) = VNI_SINGLE_MAP[table_idx] {
        return Ok(ch);
    }
    if let Some(accent) = vni_standalone_accent(byte) {
        return Ok(accent);
    }
    Err(DecodeError::UnmappedByte { byte, offset, encoding: LegacyEncoding::Vni })
}

/// Streaming state machine for VNI-Windows decoding across chunk boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VniDecoder {
    /// Decoding mode: Strict specification vs Practical real-world tolerance.
    mode: DecodeMode,
    /// Pending base vowel byte awaiting a potential diacritical accent byte.
    pending_base_byte: Option<u8>,
    /// Byte stream offset of the pending base vowel.
    pending_base_offset: usize,
    /// Pending VNI "I" / "i" + 0xCF composite awaiting potential trailing "T" / "t" for "IỆ" / "iệ".
    /// Stores `(is_upper, offset)` where `offset` is the absolute stream offset of the base 'I'/'i'.
    pending_iet: Option<(bool, usize)>,
    /// Cumulative stream byte offset across previous chunks.
    stream_offset: usize,
}

impl Default for VniDecoder {
    fn default() -> Self {
        Self::new(DecodeMode::Practical)
    }
}

impl VniDecoder {
    /// Create a new streaming VNI decoder.
    pub const fn new(mode: DecodeMode) -> Self {
        Self { mode, pending_base_byte: None, pending_base_offset: 0, pending_iet: None, stream_offset: 0 }
    }

    /// Cumulative byte offset of the stream processed across prior chunks.
    #[inline]
    pub const fn stream_offset(&self) -> usize {
        self.stream_offset
    }

    /// Decode a chunk of bytes, appending the decoded Unicode characters to `out`.
    ///
    /// # Errors
    /// Returns `Err(DecodeError)` if an unmapped byte or malformed sequence is encountered.
    /// If an error is returned, the decoder is considered poisoned/errored and must not be
    /// used for subsequent chunks.
    pub fn decode_chunk(&mut self, chunk: &[u8], out: &mut String) -> Result<(), DecodeError> {
        let mut i = 0;
        let len = chunk.len();

        while i < len {
            // Check if we are waiting for a trailing 'T' / 't' to complete "IỆ" / "iệ" across chunks.
            // In decode_chunk, non-'T' followers are valid VNI composites (resolving to 'Ị'/'ị');
            // the stored offset is retained for finish() in case the stream terminates prematurely at EOF.
            if let Some((is_upper, _)) = self.pending_iet.take() {
                let byte = chunk[i];
                if byte == b'T' || byte == b't' {
                    // Base case determines prefix ("IỆ" vs "iệ"), while follower byte ('T' vs 't')
                    // preserves its own case when processed by the subsequent loop iteration.
                    out.push_str(if is_upper { "IỆ" } else { "iệ" });
                } else {
                    out.push(if is_upper { 'Ị' } else { 'ị' });
                }
                // Do not increment i; process `byte` ('T' or other) in the loop below
            }

            let byte = chunk[i];

            // If we have a pending base byte from the previous chunk
            if let Some(prev_byte) = self.pending_base_byte.take() {
                let mark_char = byte as char;
                let base_char = byte_to_vni_base(prev_byte);

                let is_iet_prefix = (base_char == 'I' || base_char == 'i') && (mark_char == 'Ï' || mark_char == 'ï');
                if is_iet_prefix {
                    if i + 1 < len {
                        if chunk[i + 1] == b'T' || chunk[i + 1] == b't' {
                            out.push_str(if base_char == 'I' { "IỆ" } else { "iệ" });
                            // Consume the mark byte at i, leave chunk[i + 1] ('T') for next iteration
                            i += 1;
                            continue;
                        } else {
                            out.push(if base_char == 'I' { 'Ị' } else { 'ị' });
                            i += 1;
                            continue;
                        }
                    } else {
                        // Mark byte is the last byte in this chunk; defer 'T' check to next chunk
                        self.pending_iet = Some((base_char == 'I', self.pending_base_offset));
                        break;
                    }
                } else if let Some(comp) = match_vni_composite(base_char, mark_char) {
                    out.push(comp);
                    i += 1;
                    continue;
                } else {
                    let ch = decode_vni_single_byte(prev_byte, self.pending_base_offset, self.mode)?;
                    out.push(ch);
                    // Do not increment i, process `byte` in current iteration
                    continue;
                }
            }

            // Lookahead within chunk
            if is_vni_base_candidate(byte) {
                if i + 1 < len {
                    let next_byte = chunk[i + 1];
                    let base_char = byte_to_vni_base(byte);
                    let mark_char = next_byte as char;

                    let is_iet_prefix = (base_char == 'I' || base_char == 'i') && (mark_char == 'Ï' || mark_char == 'ï');
                    if is_iet_prefix {
                        if i + 2 < len {
                            if chunk[i + 2] == b'T' || chunk[i + 2] == b't' {
                                out.push_str(if base_char == 'I' { "IỆ" } else { "iệ" });
                                // Consume base and mark (i and i + 1), leave chunk[i + 2] ('T') for next iteration
                                i += 2;
                                continue;
                            } else {
                                out.push(if base_char == 'I' { 'Ị' } else { 'ị' });
                                i += 2;
                                continue;
                            }
                        } else {
                            // Mark byte is the last byte of the chunk; defer 'T' check
                            self.pending_iet = Some((base_char == 'I', self.stream_offset + i));
                            break;
                        }
                    } else if let Some(comp) = match_vni_composite(base_char, mark_char) {
                        out.push(comp);
                        i += 2;
                        continue;
                    }
                } else {
                    // Last byte of chunk is a base candidate
                    self.pending_base_byte = Some(byte);
                    self.pending_base_offset = self.stream_offset + i;
                    break;
                }
            }

            // Single byte
            let decoded_char = decode_vni_single_byte(byte, self.stream_offset + i, self.mode)?;
            out.push(decoded_char);
            i += 1;
        }

        self.stream_offset += len;
        Ok(())
    }

    /// Flush any remaining pending base character or lookahead state to `out`.
    pub fn finish(mut self, out: &mut String) -> Result<(), DecodeError> {
        if let Some((is_upper, offset)) = self.pending_iet.take() {
            if self.mode == DecodeMode::Strict {
                return Err(DecodeError::MalformedSequence { offset, encoding: LegacyEncoding::Vni });
            }
            out.push(if is_upper { 'Ị' } else { 'ị' });
        }
        if let Some(byte) = self.pending_base_byte.take() {
            let ch = decode_vni_single_byte(byte, self.pending_base_offset, self.mode)?;
            out.push(ch);
        }
        Ok(())
    }
}

/// Pure decoding of a VNI-Windows byte slice into Unicode.
pub fn decode_vni(input: &[u8], mode: DecodeMode) -> Result<Cow<'_, str>, DecodeError> {
    if input.is_ascii() {
        if let Ok(s) = std::str::from_utf8(input) {
            return Ok(Cow::Borrowed(s));
        }
        return Err(DecodeError::InvalidUtf8 { offset: 0 });
    }

    let mut out = String::with_capacity(input.len());
    let mut decoder = VniDecoder::new(mode);
    decoder.decode_chunk(input, &mut out)?;
    decoder.finish(&mut out)?;

    Ok(Cow::Owned(out))
}

/// Transcode a VNI Windows string to Unicode NFC.
pub fn transcode_vni(s: &str) -> Cow<'_, str> {
    if s.is_ascii() || is_already_unicode_vietnamese(s) {
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

    match decode_vni(&bytes, DecodeMode::Practical) {
        Ok(decoded) => {
            let nfc = super::normalize_nfc(&decoded);
            if nfc.as_ref() == s { Cow::Borrowed(s) } else { Cow::Owned(nfc.into_owned()) }
        }
        Err(_) => Cow::Borrowed(s),
    }
}
