//! RFC 2047 encoded-words: non-ASCII text in header fields (`=?UTF-8?B?…?=`).

use crate::charset;
use crate::transfer::{base64, base64_decode};

/// Most text bytes in one encoded-word: 45 bytes are 60 base64 characters, 72 with the wrapping.
pub const WORD_BYTES: usize = 45;

/// Longest character set name read.
const MAX_CHARSET: usize = 40;

/// RFC 2047 encoded-words to text.
///
/// Reads the `B` and `Q` encodings in every character set of the WHATWG Encoding Standard
/// ([`charset::decode`]); a set it doesn't name is read as UTF-8, with U+FFFD for what doesn't fit.
/// White space between
/// two encoded-words is dropped, as the RFC says. Words in a row that share a character set are
/// decoded together, so a character split across two words still comes out whole. Anything that
/// isn't a well-formed encoded-word is left as it is.
pub fn decode_words(s: &str) -> String {
    if !s.contains("=?") {
        return s.to_string();
    }
    fn flush(out: &mut String, pending: &mut Option<(String, Vec<u8>)>) {
        if let Some((charset, bytes)) = pending.take() {
            out.push_str(&charset::decode(&bytes, &charset));
        }
    }
    let mut out = String::with_capacity(s.len());
    let mut pending: Option<(String, Vec<u8>)> = None;
    let mut rest = s;
    while let Some(i) = rest.find("=?") {
        let before = rest.get(..i).unwrap_or("");
        let tail = rest.get(i..).unwrap_or("");
        match word(tail) {
            Some((charset, bytes, used)) => {
                if pending.is_none() || !before.chars().all(|c| matches!(c, ' ' | '\t' | '\r' | '\n')) {
                    flush(&mut out, &mut pending);
                    out.push_str(before);
                }
                match &mut pending {
                    Some((current, buffer)) if *current == charset => buffer.extend_from_slice(&bytes),
                    _ => {
                        flush(&mut out, &mut pending);
                        pending = Some((charset, bytes));
                    }
                }
                rest = tail.get(used..).unwrap_or("");
            }
            None => {
                flush(&mut out, &mut pending);
                out.push_str(before);
                out.push_str("=?");
                rest = tail.get(2..).unwrap_or("");
            }
        }
    }
    flush(&mut out, &mut pending);
    out.push_str(rest);
    out
}

/// One encoded-word at the start of `s`: its character set (lower case), its bytes, and its length.
fn word(s: &str) -> Option<(String, Vec<u8>, usize)> {
    let body = s.strip_prefix("=?")?;
    let charset_len = body.bytes().take(MAX_CHARSET + 1).position(|b| b == b'?')?;
    let charset = body.get(..charset_len)?;
    if charset.is_empty() || charset.bytes().any(|b| b.is_ascii_whitespace() || b.is_ascii_control()) {
        return None;
    }
    let after = body.get(charset_len + 1..)?.as_bytes();
    let encoding = after.first()?.to_ascii_uppercase();
    if after.get(1) != Some(&b'?') {
        return None;
    }
    let text = after.get(2..)?;
    let text_len = text.iter().position(|b| *b == b'?')?;
    if text.get(text_len + 1) != Some(&b'=') {
        return None;
    }
    let text = text.get(..text_len)?;
    // An encoded-word is one word: white space inside means this is something else.
    if text.iter().any(u8::is_ascii_whitespace) {
        return None;
    }
    let bytes = match encoding {
        b'B' => base64_decode(text),
        b'Q' => q_decode(text),
        _ => return None,
    };
    // `charset*language` (RFC 2231): the language is not needed.
    let charset = charset.split('*').next().unwrap_or(charset).to_ascii_lowercase();
    Some((charset, bytes, 2 + charset_len + 3 + text_len + 2))
}

/// The `Q` encoding: `_` is a space, `=XX` a byte.
fn q_decode(text: &[u8]) -> Vec<u8> {
    let hex = |b: Option<&u8>| b.and_then(|b| char::from(*b).to_digit(16)).map(|d| d as u8);
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while let Some(&c) = text.get(i) {
        match (c, hex(text.get(i + 1)), hex(text.get(i + 2))) {
            (b'=', Some(h), Some(l)) => {
                out.push((h << 4) | l);
                i += 3;
            }
            (b'_', ..) => {
                out.push(b' ');
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Whether text can go in a header field as it is: printable ASCII that doesn't look like an encoded-word.
pub fn is_plain(s: &str) -> bool {
    s.bytes().all(|b| (0x20..=0x7E).contains(&b)) && !s.contains("=?")
}

/// Text in a header-safe form.
///
/// Printable ASCII is returned unchanged (unless it holds `=?`, which a reader would take for an
/// encoded-word). Anything else becomes UTF-8 `B` encoded-words of at most 75 characters each,
/// separated by single spaces, so a header can be folded between them. The result never holds a
/// line break; [`decode_words`] gives the text back exactly.
pub fn encode_words(s: &str) -> String {
    if is_plain(s) { s.to_string() } else { encode_b(s, WORD_BYTES) }
}

/// UTF-8 `B` encoded-words whatever the text; the first word holds at most `first` bytes of it
/// (to leave room for a header name), the others [`WORD_BYTES`].
pub fn encode_b(s: &str, first: usize) -> String {
    let mut out = String::with_capacity(s.len().saturating_add(s.len() / 2).saturating_add(16));
    let mut limit = first.clamp(4, WORD_BYTES);
    let mut start = 0;
    let mut end = 0;
    let emit = |chunk: &str, out: &mut String| {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str("=?UTF-8?B?");
        out.push_str(&base64(chunk.as_bytes()));
        out.push_str("?=");
    };
    for (i, c) in s.char_indices() {
        if i + c.len_utf8() - start > limit && i > start {
            emit(s.get(start..i).unwrap_or(""), &mut out);
            start = i;
            limit = WORD_BYTES;
        }
        end = i + c.len_utf8();
    }
    if end > start || s.is_empty() {
        emit(s.get(start..end).unwrap_or(""), &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_b_and_q() {
        assert_eq!(decode_words("=?UTF-8?B?SMOpbGxv?="), "Héllo");
        assert_eq!(decode_words("=?utf-8?q?H=C3=A9llo_world?="), "Héllo world");
        assert_eq!(decode_words("=?ISO-8859-1?Q?caf=E9?="), "café");
        assert_eq!(decode_words("=?iso-8859-15?Q?=A4_5?="), "€ 5");
        assert_eq!(decode_words("=?windows-1252?Q?=93hi=94?="), "“hi”");
        assert_eq!(decode_words("=?US-ASCII?Q?plain?="), "plain");
        assert_eq!(decode_words("=?UTF-8*en?Q?lang?="), "lang");
    }

    #[test]
    fn text_around_words_is_kept() {
        assert_eq!(decode_words("Re: =?UTF-8?Q?caf=C3=A9?= today"), "Re: café today");
        assert_eq!(decode_words("no words here"), "no words here");
        assert_eq!(decode_words(""), "");
        assert_eq!(decode_words("a =?UTF-8?Q?b?= c =?UTF-8?Q?d?= e"), "a b c d e");
        assert_eq!(decode_words("\"=?UTF-8?Q?Ad=C3=A1?=\" <x>"), "\"Adá\" <x>");
    }

    #[test]
    fn space_between_words_is_dropped() {
        assert_eq!(decode_words("=?UTF-8?Q?a?= =?UTF-8?Q?b?="), "ab");
        assert_eq!(decode_words("=?UTF-8?Q?a?=\r\n\t=?UTF-8?Q?b?=  =?ISO-8859-1?Q?=E9?="), "abé");
        assert_eq!(decode_words("=?UTF-8?Q?a?==?UTF-8?Q?b?="), "ab");
        assert_eq!(decode_words("=?UTF-8?Q?a?= x =?UTF-8?Q?b?="), "a x b");
        assert_eq!(decode_words("=?UTF-8?Q?a_?= =?UTF-8?Q?b?="), "a b");
    }

    #[test]
    fn a_character_split_across_words_is_whole() {
        // "é" is C3 A9; one byte in each word.
        assert_eq!(decode_words("=?UTF-8?Q?caf=C3?= =?UTF-8?Q?=A9!?="), "café!");
        assert_eq!(decode_words("=?UTF-8?B?w6nD?= =?utf-8?B?qQ==?="), "éé");
    }

    #[test]
    fn malformed_words_are_left_alone() {
        for s in [
            "=?",
            "=?UTF-8",
            "=?UTF-8?",
            "=?UTF-8?Q",
            "=?UTF-8?Q?",
            "=?UTF-8?Q?abc",
            "=?UTF-8?Q?abc?",
            "=?UTF-8?X?abc?=",
            "=??Q?abc?=",
            "=?UTF-8?Q?a b?=",
            "=? ?Q?a?=",
            "a=?b",
            "=?=?=?=?",
        ] {
            assert_eq!(decode_words(s), s, "{s}");
        }
        assert_eq!(decode_words("=?x?=?UTF-8?Q?ok?="), "=?x?ok");
    }

    #[test]
    fn unknown_charsets_and_bad_bytes_do_not_fail() {
        assert_eq!(decode_words("=?koi8-r?Q?abc?="), "abc");
        assert_eq!(decode_words("=?UTF-8?Q?=FF?="), "\u{fffd}");
        assert_eq!(decode_words("=?UTF-8?B?!!!!?="), "");
        assert_eq!(decode_words("=?UTF-8?Q?=ZZ=4?="), "=ZZ=4");
    }

    #[test]
    fn ascii_is_not_encoded() {
        assert_eq!(encode_words("Lunch on Friday?"), "Lunch on Friday?");
        assert_eq!(encode_words(""), "");
        assert_eq!(encode_words("a = b ? c"), "a = b ? c");
    }

    #[test]
    fn non_ascii_becomes_b_words() {
        assert_eq!(encode_words("Héllo"), "=?UTF-8?B?SMOpbGxv?=");
        assert_eq!(decode_words(&encode_words("naïve café")), "naïve café");
        // Control characters and things that look like encoded-words too.
        assert!(encode_words("tab\there").starts_with("=?UTF-8?B?"));
        assert!(encode_words("=?UTF-8?Q?fake?=").starts_with("=?UTF-8?B?"));
        assert_eq!(decode_words(&encode_words("=?UTF-8?Q?fake?=")), "=?UTF-8?Q?fake?=");
    }

    #[test]
    fn long_text_folds_into_words_of_at_most_75() {
        let text = "Ünïcödé ".repeat(40);
        let encoded = encode_words(&text);
        let words: Vec<&str> = encoded.split(' ').collect();
        assert!(words.len() > 5);
        for w in &words {
            assert!(w.len() <= 75, "{}", w.len());
            assert!(w.starts_with("=?UTF-8?B?") && w.ends_with("?="));
        }
        assert!(!encoded.contains(['\r', '\n']));
        assert_eq!(decode_words(&encoded), text);
        // Four-byte characters are never cut.
        let emoji = "😀".repeat(50);
        assert_eq!(decode_words(&encode_words(&emoji)), emoji);
    }

    #[test]
    fn the_first_word_can_be_shorter() {
        let text = "é".repeat(40);
        let encoded = encode_b(&text, 10);
        let first = encoded.split(' ').next().unwrap();
        assert_eq!(first, "=?UTF-8?B?w6nDqcOpw6nDqQ==?=");
        assert_eq!(decode_words(&encoded), text);
        assert_eq!(decode_words(&encode_b("", 45)), "");
        assert_eq!(decode_words(&encode_b("x", 0)), "x");
    }

    #[test]
    fn hostile_input_is_linear() {
        for s in [
            "=?".repeat(200_000),
            "=?a?Q?".repeat(80_000),
            "=?a?Q?b?= ".repeat(40_000),
            format!("=?{}", "a".repeat(400_000)),
            "=?UTF-8?Q?".to_string() + &"x".repeat(400_000),
        ] {
            let _ = decode_words(&s);
        }
    }

    proptest::proptest! {
        #[test]
        fn any_text_round_trips(s in proptest::prelude::any::<String>()) {
            let encoded = encode_words(&s);
            proptest::prop_assert!(!encoded.contains(['\r', '\n', '\0']) || is_plain(&s));
            proptest::prop_assert_eq!(decode_words(&encoded), s.clone());
            proptest::prop_assert_eq!(decode_words(&encode_b(&s, 7)), s);
        }

        #[test]
        fn any_text_decodes(s in "[=?a-zA-Z0-9_ -]{0,40}") {
            let _ = decode_words(&s);
        }
    }
}
