//! TCVN3 / ABC (TCVN 5712:1993) legacy Vietnamese 8-bit encoding tables and transcoding.
//!
//! Standardized by the Directorate for Standards, Metrology and Quality (STAMEQ),
//! Ministry of Science, Technology and Environment of Vietnam (1993).
//! Cross-verified with GNU C Library (\glibc\) official charmap specification \TCVN5712-1\.
//! All conversions deterministically emit standard precomposed Unicode NFC (TCVN 6909:2001).

use std::borrow::Cow;

use super::detect::is_already_unicode_vietnamese;

/// Check whether an ASCII character or %-escaped CAD control represents uppercase TCVN3 text.
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

/// Transcode a TCVN3 (ABC) standard string to Unicode NFC.
pub fn transcode_tcvn3(s: &str) -> Cow<'_, str> {
    if s.is_ascii() || is_already_unicode_vietnamese(s) {
        return Cow::Borrowed(s);
    }

    let all_caps = is_tcvn3_all_caps(s);
    let mut out = String::with_capacity(s.len());
    let mut changed = false;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if let Some(&next) = chars.peek()
            && let Some(comp) = match_tcvn3_composite(c, next)
        {
            out.push_str(comp);
            chars.next();
            changed = true;
            continue;
        }
        if let Some(single) = match_tcvn3_single(c) {
            if all_caps {
                out.extend(single.to_uppercase());
            } else {
                out.push(single);
            }
            changed = true;
        } else {
            out.push(c);
        }
    }

    if changed { Cow::Owned(out) } else { Cow::Borrowed(s) }
}

/// Transcode a TCVN3H all-uppercase string to Unicode NFC uppercase.
pub fn transcode_tcvn3_h(s: &str) -> Cow<'_, str> {
    if s.is_ascii() {
        let upper = s.to_uppercase();
        if upper == s { Cow::Borrowed(s) } else { Cow::Owned(upper) }
    } else {
        let transcoded = transcode_tcvn3(s);
        let upper = transcoded.to_uppercase();
        if upper == s { Cow::Borrowed(s) } else { Cow::Owned(upper) }
    }
}

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

pub(crate) fn match_tcvn3_single(c: char) -> Option<char> {
    match c {
        '¢' => Some('Â'),
        '£' => Some('Ê'),
        '¤' => Some('Ô'),
        '¥' => Some('Ơ'),
        '¦' => Some('Ư'),
        '§' => Some('Đ'),
        '¡' => Some('Ă'),
        'µ' => Some('à'),
        '¸' => Some('á'),
        '¶' => Some('ả'),
        '·' => Some('ã'),
        '¹' => Some('ạ'),
        '¨' => Some('ă'),
        '»' => Some('ằ'),
        '¾' => Some('ắ'),
        '¼' => Some('ẳ'),
        '½' => Some('ẵ'),
        'Æ' => Some('ặ'),
        '©' => Some('â'),
        'Ç' => Some('ầ'),
        'Ê' => Some('ấ'),
        'È' => Some('ẩ'),
        'É' => Some('ẫ'),
        'Ë' => Some('ậ'),
        '®' => Some('đ'),
        'Ì' => Some('è'),
        'Ð' => Some('é'),
        'Î' => Some('ẻ'),
        'Ï' => Some('ẽ'),
        'Ñ' => Some('ẹ'),
        'ª' => Some('ê'),
        'Ò' => Some('ề'),
        'Õ' => Some('ế'),
        'Ó' => Some('ể'),
        'Ô' => Some('ễ'),
        'Ö' => Some('ệ'),
        '×' => Some('ì'),
        'Ý' => Some('í'),
        'Ø' => Some('ỉ'),
        'Ü' => Some('ĩ'),
        'Þ' => Some('ị'),
        'ß' => Some('ò'),
        'ã' => Some('ó'),
        'á' => Some('ỏ'),
        'â' => Some('õ'),
        'ä' => Some('ọ'),
        '«' => Some('ô'),
        'å' => Some('ồ'),
        'è' => Some('ố'),
        'æ' => Some('ổ'),
        'ç' => Some('ỗ'),
        'é' => Some('ộ'),
        '¬' => Some('ơ'),
        'ê' => Some('ờ'),
        'í' => Some('ớ'),
        'ë' => Some('ở'),
        'ì' => Some('ỡ'),
        'î' => Some('ợ'),
        'ï' => Some('ù'),
        'ó' => Some('ú'),
        'ñ' => Some('ủ'),
        'ò' => Some('ũ'),
        'ô' => Some('ụ'),
        '­' => Some('ư'),
        'õ' => Some('ừ'),
        'ø' => Some('ứ'),
        'ö' => Some('ử'),
        '÷' => Some('ữ'),
        'ù' => Some('ự'),
        'ú' => Some('ỳ'),
        'ý' => Some('ý'),
        'û' => Some('ỷ'),
        'ü' => Some('ỹ'),
        'þ' => Some('ỵ'),
        _ => None,
    }
}
