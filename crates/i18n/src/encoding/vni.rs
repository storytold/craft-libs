//! VNI-Windows legacy Vietnamese encoding tables and transcoding (2-byte composite & single-byte).
//!
//! Based on de facto industry conventions established by VNI Software Company (c. 1991–1995),
//! cross-verified with UniKey core engine (\nconv\ / \ni.tab\) by Phạm Kim Long and
//! CPAN \Encode::VN::VNI\.
//! All conversions deterministically emit standard precomposed Unicode NFC (TCVN 6909:2001).

use std::borrow::Cow;

use super::detect::is_already_unicode_vietnamese;

/// Transcode a VNI Windows string to Unicode NFC.
pub fn transcode_vni(s: &str) -> Cow<'_, str> {
    if s.is_ascii() || is_already_unicode_vietnamese(s) {
        return Cow::Borrowed(s);
    }

    let mut out = String::with_capacity(s.len());
    let mut changed = false;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if let Some(&next) = chars.peek()
            && let Some(comp) = match_vni_composite(c, next)
        {
            out.push_str(comp);
            chars.next();
            changed = true;
            continue;
        }
        if let Some(single) = match_vni_single(c) {
            out.push(single);
            changed = true;
        } else {
            out.push(c);
        }
    }

    if changed { Cow::Owned(out) } else { Cow::Borrowed(s) }
}

pub(crate) fn match_vni_composite(c1: char, c2: char) -> Option<&'static str> {
    match (c1, c2) {
        ('A', 'Ø') => Some("À"),
        ('A', 'Ù') => Some("Á"),
        ('A', 'Â') => Some("Â"),
        ('A', 'Õ') => Some("Ã"),
        ('A', 'Ï') => Some("Ạ"),
        ('A', 'Û') => Some("Ả"),
        ('A', 'Á') => Some("Ấ"),
        ('A', 'À') => Some("Ầ"),
        ('A', 'Å') => Some("Ẩ"),
        ('A', 'Ã') => Some("Ẫ"),
        ('A', 'Ä') => Some("Ậ"),
        ('A', 'Ê') => Some("Ă"),
        ('A', 'É') => Some("Ắ"),
        ('A', 'È') => Some("Ằ"),
        ('A', 'Ú') => Some("Ẳ"),
        ('A', 'Ü') => Some("Ẵ"),
        ('A', 'Ë') => Some("Ặ"),
        ('E', 'Ø') => Some("È"),
        ('E', 'Ù') => Some("É"),
        ('E', 'Â') => Some("Ê"),
        ('E', 'Õ') => Some("Ẽ"),
        ('E', 'Ï') => Some("Ẹ"),
        ('E', 'Û') => Some("Ẻ"),
        ('E', 'Á') => Some("Ế"),
        ('E', 'À') => Some("Ề"),
        ('E', 'Å') => Some("Ể"),
        ('E', 'Ã') => Some("Ễ"),
        ('E', 'Ä') => Some("Ệ"),
        ('I', 'Ø') => Some("Ì"),
        ('I', 'Ù') => Some("Í"),
        ('I', 'Û') => Some("Ỉ"),
        ('I', 'Õ') => Some("Ĩ"),
        ('I', 'Ï') => Some("Ị"),
        ('O', 'Ø') => Some("Ò"),
        ('O', 'Ù') => Some("Ó"),
        ('O', 'Â') => Some("Ô"),
        ('O', 'Õ') => Some("Õ"),
        ('O', 'Ï') => Some("Ọ"),
        ('O', 'Û') => Some("Ỏ"),
        ('O', 'Á') => Some("Ố"),
        ('O', 'À') => Some("Ồ"),
        ('O', 'Å') => Some("Ổ"),
        ('O', 'Ã') => Some("Ỗ"),
        ('O', 'Ä') => Some("Ộ"),
        ('Ô', 'Ù') => Some("Ớ"),
        ('Ô', 'Ø') => Some("Ờ"),
        ('Ô', 'Û') => Some("Ở"),
        ('Ô', 'Õ') => Some("Ỡ"),
        ('Ô', 'Ï') => Some("Ợ"),
        ('U', 'Ø') => Some("Ù"),
        ('U', 'Ù') => Some("Ú"),
        ('U', 'Õ') => Some("Ũ"),
        ('U', 'Ï') => Some("Ụ"),
        ('U', 'Û') => Some("Ủ"),
        ('Ö', 'Ù') => Some("Ứ"),
        ('Ö', 'Ø') => Some("Ừ"),
        ('Ö', 'Û') => Some("Ử"),
        ('Ö', 'Õ') => Some("Ữ"),
        ('Ö', 'Ï') => Some("Ự"),
        ('Y', 'Ø') => Some("Ỳ"),
        ('Y', 'Ù') => Some("Ý"),
        ('Y', 'Õ') => Some("Ỹ"),
        ('Y', 'Û') => Some("Ỷ"),
        ('Y', 'Ï') => Some("Ỵ"),
        ('a', 'ø') => Some("à"),
        ('a', 'ù') => Some("á"),
        ('a', 'â') => Some("â"),
        ('a', 'õ') => Some("ã"),
        ('a', 'ï') => Some("ạ"),
        ('a', 'û') => Some("ả"),
        ('a', 'á') => Some("ấ"),
        ('a', 'à') => Some("ầ"),
        ('a', 'å') => Some("ẩ"),
        ('a', 'ã') => Some("ẫ"),
        ('a', 'ä') => Some("ậ"),
        ('a', 'ê') => Some("ă"),
        ('a', 'é') => Some("ắ"),
        ('a', 'è') => Some("ằ"),
        ('a', 'ú') => Some("ẳ"),
        ('a', 'ü') => Some("ẵ"),
        ('a', 'ë') => Some("ặ"),
        ('e', 'ø') => Some("è"),
        ('e', 'ù') => Some("é"),
        ('e', 'â') => Some("ê"),
        ('e', 'õ') => Some("ẽ"),
        ('e', 'ï') => Some("ẹ"),
        ('e', 'û') => Some("ẻ"),
        ('e', 'á') => Some("ế"),
        ('e', 'à') => Some("ề"),
        ('e', 'å') => Some("ể"),
        ('e', 'ã') => Some("ễ"),
        ('e', 'ä') => Some("ệ"),
        ('i', 'ø') => Some("ì"),
        ('i', 'ù') => Some("í"),
        ('i', 'û') => Some("ỉ"),
        ('i', 'õ') => Some("ĩ"),
        ('i', 'ï') => Some("ị"),
        ('o', 'ø') => Some("ò"),
        ('o', 'ù') => Some("ó"),
        ('o', 'â') => Some("ô"),
        ('o', 'õ') => Some("õ"),
        ('o', 'ï') => Some("ọ"),
        ('o', 'û') => Some("ỏ"),
        ('o', 'á') => Some("ố"),
        ('o', 'à') => Some("ồ"),
        ('o', 'å') => Some("ổ"),
        ('o', 'ã') => Some("ỗ"),
        ('o', 'ä') => Some("ộ"),
        ('ô', 'ù') => Some("ớ"),
        ('ô', 'ø') => Some("ờ"),
        ('ô', 'û') => Some("ở"),
        ('ô', 'õ') => Some("ỡ"),
        ('ô', 'ï') => Some("ợ"),
        ('u', 'ø') => Some("ù"),
        ('u', 'ù') => Some("ú"),
        ('u', 'õ') => Some("ũ"),
        ('u', 'ï') => Some("ụ"),
        ('u', 'û') => Some("ủ"),
        ('ö', 'ù') => Some("ứ"),
        ('ö', 'ø') => Some("ừ"),
        ('ö', 'û') => Some("ử"),
        ('ö', 'õ') => Some("ữ"),
        ('ö', 'ï') => Some("ự"),
        ('y', 'ø') => Some("ỳ"),
        ('y', 'ù') => Some("ý"),
        ('y', 'õ') => Some("ỹ"),
        ('y', 'û') => Some("ỷ"),
        ('y', 'ï') => Some("ỵ"),
        _ => None,
    }
}

pub(crate) fn match_vni_single(c: char) -> Option<char> {
    match c {
        'Ñ' => Some('Đ'),
        'ñ' => Some('đ'),
        'Ô' => Some('Ơ'),
        'ô' => Some('ơ'),
        'Ö' => Some('Ư'),
        'ö' => Some('ư'),
        'Æ' => Some('Ỉ'),
        'æ' => Some('ỉ'),
        'Ò' => Some('Ị'),
        'ò' => Some('ị'),
        'Î' => Some('Ỵ'),
        'î' => Some('ỵ'),
        'Ó' => Some('Ĩ'),
        'ó' => Some('ĩ'),
        _ => None,
    }
}
