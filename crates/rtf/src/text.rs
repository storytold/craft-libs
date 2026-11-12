//! The text of an RTF file: `\uN` with its fallback characters, and 8-bit text in a code page.

use crate::codepage::{code_page_byte, multibyte_encoding_name};

fn multibyte(code_page: u16) -> Option<&'static encoding_rs::Encoding> {
    encoding_rs::Encoding::for_label(multibyte_encoding_name(code_page)?.as_bytes())
}

/// Decodes the text tokens of one RTF file into `String`s the caller supplies.
///
/// The caller says which code page applies to each piece of 8-bit text (the current font's,
/// else the default font's, else the document's) and how many fallback characters follow a
/// `\uN` (`\ucN`, 1 when not set).
///
/// - `\uN` is one UTF-16 code unit. A high surrogate pairs only with a low surrogate in the
///   very next `\u`; every surrogate that does not pair reads as U+FFFD, as in Word.
/// - After a `\uN`, the next `\ucN` characters are its fallback and are skipped: a text byte,
///   a `\'hh`, a control word and a control symbol each count as one ([`TextDecoder::skip`]).
///   The skipping ends with the group ([`TextDecoder::group`]).
/// - A single-byte code page maps each byte. A multibyte one (932, 936, 949, 950) streams
///   through one decoder, so a lead byte and its trail byte decode together even when a
///   `\'hh`, a text run or a formatting word falls between them, and a run that happens to
///   start `EF BB BF` is not taken for a UTF-8 byte order mark. A lead byte without its trail
///   byte reads as U+FFFD.
#[derive(Default)]
pub struct TextDecoder {
    skip: usize,
    high: Option<u16>,
    mb: Option<encoding_rs::Decoder>,
    mb_cp: u16,
}

impl TextDecoder {
    pub fn new() -> TextDecoder {
        TextDecoder::default()
    }

    /// A held high surrogate that nothing paired with.
    fn flush_high(&mut self, out: &mut String) {
        if self.high.take().is_some() {
            out.push('\u{FFFD}');
        }
    }

    /// `\uN` with `uc` fallback characters to skip after it.
    pub fn unicode(&mut self, param: i32, uc: usize, out: &mut String) {
        self.finish_multibyte(out);
        let u = i64::from(param).rem_euclid(65_536) as u16;
        if (0xD800..0xDC00).contains(&u) {
            self.flush_high(out);
            self.high = Some(u);
        } else {
            match self.high.take() {
                Some(h) if (0xDC00..0xE000).contains(&u) => out.push(char::decode_utf16([h, u]).next().and_then(Result::ok).unwrap_or('\u{FFFD}')),
                Some(_) => {
                    out.push('\u{FFFD}');
                    out.push(char::from_u32(u32::from(u)).unwrap_or('\u{FFFD}'));
                }
                None => out.push(char::from_u32(u32::from(u)).unwrap_or('\u{FFFD}')),
            }
        }
        self.skip = uc;
    }

    /// Is the control word or symbol just read a fallback character of the last `\uN`? Then it
    /// is used up and the caller ignores it.
    pub fn skip(&mut self) -> bool {
        let skipped = self.skip > 0;
        self.skip = self.skip.saturating_sub(1);
        skipped
    }

    /// A character a control word or symbol stands for (`\emdash`, `\~`).
    pub fn char(&mut self, c: char, out: &mut String) {
        self.finish_multibyte(out);
        self.flush_high(out);
        out.push(c);
    }

    /// `\'hh`: one byte in `code_page`.
    pub fn byte(&mut self, b: u8, code_page: u16, out: &mut String) {
        if !self.skip() {
            self.decode(&[b], code_page, out);
        }
    }

    /// A run of text bytes in `code_page`.
    pub fn bytes(&mut self, chunk: &[u8], code_page: u16, out: &mut String) {
        let skip = self.skip.min(chunk.len());
        self.skip -= skip;
        let chunk = chunk.get(skip..).unwrap_or_default();
        if !chunk.is_empty() {
            self.decode(chunk, code_page, out);
        }
    }

    fn decode(&mut self, chunk: &[u8], cp: u16, out: &mut String) {
        self.flush_high(out);
        if self.mb.is_none() && chunk.is_ascii() {
            // ASCII is itself in every code page here; with no lead byte pending, as it is.
            out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
            return;
        }
        let Some(encoding) = multibyte(cp) else {
            self.finish_multibyte(out);
            out.extend(chunk.iter().map(|b| code_page_byte(cp, *b)));
            return;
        };
        // A code page switch ends the previous run before the new decoder starts.
        if self.mb_cp != cp {
            self.finish_multibyte(out);
        }
        self.mb_cp = cp;
        let decoder = self.mb.get_or_insert_with(|| encoding.new_decoder_without_bom_handling());
        let mut rest = chunk;
        loop {
            out.reserve(rest.len().saturating_mul(3).saturating_add(16));
            let (result, read, _) = decoder.decode_to_string(rest, out, false);
            rest = rest.get(read..).unwrap_or_default();
            if !matches!(result, encoding_rs::CoderResult::OutputFull) {
                break;
            }
        }
    }

    /// Ends the current multibyte run: a lead byte still waiting for its trail byte becomes
    /// U+FFFD.
    fn finish_multibyte(&mut self, out: &mut String) {
        if let Some(mut decoder) = self.mb.take() {
            out.reserve(16);
            let _ = decoder.decode_to_string(&[], out, true);
        }
    }

    /// A group opened or closed: the fallback of a `\uN` ends with its group, and so does a
    /// multibyte run.
    pub fn group(&mut self, out: &mut String) {
        self.finish_multibyte(out);
        self.skip = 0;
    }

    /// The text ends here (a paragraph end, a cell end, the end of the file): nothing waits
    /// for a partner any longer.
    pub fn finish(&mut self, out: &mut String) {
        self.finish_multibyte(out);
        self.flush_high(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(ns: &[i32], uc: usize) -> String {
        let (mut d, mut out) = (TextDecoder::new(), String::new());
        for n in ns {
            d.unicode(*n, uc, &mut out);
        }
        d.finish(&mut out);
        out
    }

    #[test]
    fn surrogates_pair_only_with_the_next_unit() {
        assert_eq!(units(&[-10179, -8704], 0), "\u{1F600}");
        assert_eq!(units(&[0xD83D, 0xDE00], 0), "\u{1F600}", "written unsigned");
        assert_eq!(units(&[8364], 0), "€");
        assert_eq!(units(&[-10179], 0), "\u{FFFD}", "a lone high surrogate at the end");
        assert_eq!(units(&[-8704], 0), "\u{FFFD}", "a lone low surrogate");
        assert_eq!(units(&[-8704, -10179], 0), "\u{FFFD}\u{FFFD}", "low then high");
        assert_eq!(units(&[-10179, -10179, -8704], 0), "\u{FFFD}\u{1F600}", "a high surrogate before a pair");
        assert_eq!(units(&[-10179, 65, -8704], 0), "\u{FFFD}A\u{FFFD}");
        assert_eq!(units(&[65536 + 65, -65536 + 66], 0), "AB", "taken modulo 65536");
    }

    #[test]
    fn a_held_high_surrogate_does_not_pair_across_text() {
        let (mut d, mut out) = (TextDecoder::new(), String::new());
        d.unicode(-10179, 0, &mut out);
        d.bytes(b"x", 1252, &mut out);
        d.unicode(-8704, 0, &mut out);
        d.unicode(-10179, 0, &mut out);
        d.char('\u{2014}', &mut out);
        d.unicode(-10179, 1, &mut out);
        // The fallback and a formatting group do not come between a pair.
        d.bytes(b"?", 1252, &mut out);
        d.group(&mut out);
        d.unicode(-8704, 1, &mut out);
        d.finish(&mut out);
        assert_eq!(out, "\u{FFFD}x\u{FFFD}\u{FFFD}\u{2014}\u{1F600}");
    }

    #[test]
    fn fallback_characters_are_skipped() {
        let (mut d, mut out) = (TextDecoder::new(), String::new());
        d.unicode(8364, 1, &mut out);
        d.bytes(b"? price", 1252, &mut out);
        d.unicode(8220, 2, &mut out);
        d.byte(0x93, 1252, &mut out);
        assert!(d.skip(), "a control word is one fallback character");
        assert!(!d.skip());
        d.unicode(233, 3, &mut out);
        d.bytes(b"e", 1252, &mut out);
        d.group(&mut out);
        d.bytes(b"AB", 1252, &mut out);
        assert_eq!(out, "\u{20AC} price\u{201C}\u{E9}AB", "the skip ends with the group");
    }

    #[test]
    fn single_byte_code_pages() {
        let (mut d, mut out) = (TextDecoder::new(), String::new());
        d.bytes(&[b'C', b'a', b'f', 0xE9, b' '], 1252, &mut out);
        d.byte(0xC4, 1251, &mut out);
        d.byte(0x8E, 10000, &mut out);
        d.byte(0x82, 437, &mut out);
        d.byte(0xE9, 9999, &mut out);
        assert_eq!(out, "Café Дééé");
    }

    #[test]
    fn a_lead_byte_and_its_trail_byte_decode_together() {
        // 日本 in Shift-JIS is 93 FA 96 7B: the first character as two `\'hh`, the second as a
        // `\'hh` lead byte and a raw trail byte (`{`'s neighbour 0x7B is a text byte here).
        let (mut d, mut out) = (TextDecoder::new(), String::new());
        d.byte(0x93, 932, &mut out);
        d.byte(0xFA, 932, &mut out);
        d.byte(0x96, 932, &mut out);
        d.bytes(b"\x7bA", 932, &mut out);
        assert_eq!(out, "日本A");
        // A lead byte left at a group end, a code page switch or the end is U+FFFD.
        for end in 0..3 {
            let (mut d, mut out) = (TextDecoder::new(), String::new());
            d.byte(0x93, 932, &mut out);
            match end {
                0 => d.group(&mut out),
                1 => d.bytes(&[0xE9], 1252, &mut out),
                _ => d.finish(&mut out),
            }
            assert!(out.starts_with('\u{FFFD}'), "{end}: {out:?}");
        }
        // GBK text that starts EF BB BF is not a UTF-8 byte order mark.
        let (mut d, mut out) = (TextDecoder::new(), String::new());
        d.bytes(&[0xEF, 0xBB, 0xBF, 0xC4, 0xE3], 936, &mut out);
        d.finish(&mut out);
        assert_eq!(out.chars().count(), 3, "{out:?}");
        assert!(out.ends_with('\u{FFFD}') && !out.starts_with('\u{FEFF}'), "{out:?}");
        // A long run decodes whole.
        let (mut d, mut out) = (TextDecoder::new(), String::new());
        d.bytes(&[0x93, 0xFA].repeat(10_000), 932, &mut out);
        assert_eq!(out.chars().count(), 10_000);
    }
}
