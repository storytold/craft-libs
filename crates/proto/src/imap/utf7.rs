// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Modified UTF-7, the encoding of mailbox names (RFC 3501 §5.1.3).
//!
//! Printable ASCII stands for itself, except `&`, which is written `&-`. Everything else is
//! UTF-16 in base64 (with `,` in place of `/`, and no padding) between `&` and `-`.

const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";

fn digit(six_bits: u32) -> char {
    DIGITS.get((six_bits & 0x3F) as usize).map_or('A', |b| char::from(*b))
}

/// A mailbox name as people read it. Text that isn't valid modified UTF-7 is kept as it is.
pub fn decode_utf7(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    // Every round takes at least the `&` off `rest`.
    while let Some(at) = rest.find('&') {
        out.push_str(rest.get(..at).unwrap_or(""));
        let after = rest.get(at + 1..).unwrap_or("");
        rest = match after.find('-') {
            Some(0) => {
                out.push('&');
                after.get(1..).unwrap_or("")
            }
            Some(end) => match shifted(after.get(..end).unwrap_or("")) {
                Some(text) => {
                    out.push_str(&text);
                    after.get(end + 1..).unwrap_or("")
                }
                // Not base64 after all: an ampersand like any other character.
                None => {
                    out.push('&');
                    after
                }
            },
            None => {
                out.push('&');
                after
            }
        };
    }
    out.push_str(rest);
    out
}

/// The text of one base64 run, or `None` when it isn't one.
fn shifted(encoded: &str) -> Option<String> {
    let mut units = Vec::with_capacity(encoded.len() * 3 / 8 + 1);
    let (mut bits, mut count) = (0u32, 0u32);
    for b in encoded.bytes() {
        let value = DIGITS.iter().position(|d| *d == b)? as u32;
        bits = (bits << 6) | value;
        count += 6;
        if count >= 16 {
            count -= 16;
            units.push((bits >> count) as u16);
            bits &= (1 << count) - 1;
        }
    }
    // What is left over is padding: less than one digit's worth.
    if count >= 6 || units.is_empty() {
        return None;
    }
    String::from_utf16(&units).ok()
}

/// A mailbox name as a server wants it.
pub fn encode_utf7(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending: Vec<u16> = Vec::new();
    for c in name.chars() {
        if matches!(c, ' '..='~') {
            flush(&mut pending, &mut out);
            out.push(c);
            if c == '&' {
                out.push('-');
            }
        } else {
            let mut units = [0u16; 2];
            pending.extend_from_slice(c.encode_utf16(&mut units));
        }
    }
    flush(&mut pending, &mut out);
    out
}

/// Write the characters waiting to be encoded as one base64 run.
fn flush(pending: &mut Vec<u16>, out: &mut String) {
    if pending.is_empty() {
        return;
    }
    out.push('&');
    let (mut bits, mut count) = (0u32, 0u32);
    for unit in pending.drain(..) {
        bits = (bits << 16) | u32::from(unit);
        count += 16;
        while count >= 6 {
            count -= 6;
            out.push(digit(bits >> count));
        }
        bits &= (1 << count) - 1;
    }
    if count > 0 {
        out.push(digit(bits << (6 - count)));
    }
    out.push('-');
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn the_examples_of_the_rfc() {
        // RFC 3501 §5.1.3.
        assert_eq!(decode_utf7("~peter/mail/&U,BTFw-/&ZeVnLIqe-"), "~peter/mail/台北/日本語");
        assert_eq!(encode_utf7("~peter/mail/台北/日本語"), "~peter/mail/&U,BTFw-/&ZeVnLIqe-");
        assert_eq!(decode_utf7("&Jjo-!"), "☺!");
        assert_eq!(encode_utf7("☺!"), "&Jjo-!");
    }

    #[test]
    fn ordinary_names() {
        assert_eq!(encode_utf7("INBOX"), "INBOX");
        assert_eq!(encode_utf7("Tom & Jerry"), "Tom &- Jerry");
        assert_eq!(decode_utf7("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(encode_utf7("Entwürfe"), "Entw&APw-rfe");
        assert_eq!(decode_utf7("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(encode_utf7("Gelöschte Elemente"), "Gel&APY-schte Elemente");
        // Outside the basic plane: a surrogate pair.
        assert_eq!(decode_utf7(&encode_utf7("📬 Post")), "📬 Post");
        assert_eq!(encode_utf7("📬"), "&2D3c7A-");
        // Control characters are encoded too.
        assert_eq!(encode_utf7("a\tb"), "a&AAk-b");
        assert_eq!(encode_utf7(""), "");
    }

    #[test]
    fn junk_is_kept_as_it_is() {
        for junk in ["&", "a&b", "&&&", "&!!!-", "&A-", "&AAAAA-x", "R&D", "&2D0-", "&-&", "é&é-"] {
            let decoded = decode_utf7(junk);
            assert!(!decoded.is_empty(), "{junk}");
        }
        assert_eq!(decode_utf7("R&D"), "R&D");
        assert_eq!(decode_utf7("&!!!-"), "&!!!-");
        // A lone surrogate is not text.
        assert_eq!(decode_utf7("&2D0-"), "&2D0-");
        assert_eq!(decode_utf7("AT&T-Mobile"), "AT&T-Mobile");
    }

    proptest! {
        #[test]
        fn any_name_round_trips(name in "\\PC{0,40}") {
            let encoded = encode_utf7(&name);
            prop_assert!(encoded.bytes().all(|b| (0x20..=0x7E).contains(&b)), "{encoded}");
            prop_assert_eq!(decode_utf7(&encoded), name);
        }

        #[test]
        fn any_text_round_trips_with_control_characters(name in proptest::collection::vec(any::<char>(), 0..40)) {
            let name: String = name.into_iter().collect();
            prop_assert_eq!(decode_utf7(&encode_utf7(&name)), name);
        }

        #[test]
        fn decoding_never_panics(name in "[&A-Za-z0-9+,\\-é ]{0,60}") {
            let _ = decode_utf7(&name);
        }
    }
}
