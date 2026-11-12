//! Text decoding for DXF files.
//!
//! R2007 (AC1021) and later DXF files are UTF-8. Older ones store text in the code page named by
//! the `$DWGCODEPAGE` header variable (`ANSI_1252`, `ANSI_932`, `DOS850`, …); characters outside
//! it are written as `\U+XXXX` (any character) or `\M+nXXXX` (a double-byte character of an East
//! Asian code page) escapes, which [`crate::parse`] decodes after the bytes.
//!
//! The Windows, ISO and East Asian code pages are decoded with `encoding_rs`; the DOS pages with
//! `craft-codepage`'s tables (the upper halves of the IBM PC code pages; the lower halves are
//! ASCII); Korean Johab (`ANSI_1361`) by its definition in KS X 1001 annex 3. Decoding never
//! fails: a byte sequence that is invalid in the code page (a truncated double-byte character, an
//! unassigned byte) becomes U+FFFD and the bytes after it decode normally, so line ends and the
//! group structure survive.

use std::borrow::Cow;

use encoding_rs::Encoding;

use crate::Version;
use craft_codepage::dos;

/// How the bytes of a code page become text.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Decoder {
    Utf8,
    Encoding(&'static Encoding),
    /// Single-byte DOS code page: ASCII, then these 128 characters for bytes 0x80–0xFF.
    Table(&'static [char; 128]),
    /// DOS 864: like [`Decoder::Table`], but byte 0x25 is U+066A ARABIC PERCENT SIGN, not `%`.
    Dos864,
    /// Korean Johab (code page 1361).
    Johab,
}

/// What byte 0x25 of DOS 864 stands for.
const DOS864_PERCENT: char = '\u{066A}';

impl Decoder {
    /// True when the ASCII character `c` is not what its byte means in this code page, so a
    /// writer has to escape it.
    pub(crate) fn shadows_ascii(self, c: char) -> bool {
        matches!(self, Decoder::Dos864) && c == '%'
    }
}

/// The decoder for a `$DWGCODEPAGE` value (case-insensitive), or `None` when CADCraft doesn't
/// know the code page.
pub(crate) fn decoder(code_page: &str) -> Option<Decoder> {
    use encoding_rs as e;
    let name = code_page.trim().to_ascii_uppercase().replace(['-', ' '], "_");
    let enc = match name.as_str() {
        "UTF8" | "UTF_8" | "UNICODE" => return Some(Decoder::Utf8),
        "ASCII" | "ANSI_1252" | "ISO8859_1" | "ISO_8859_1" => e::WINDOWS_1252,
        "ANSI_874" => e::WINDOWS_874,
        "ANSI_932" | "DOS932" | "SHIFT_JIS" => e::SHIFT_JIS,
        "ANSI_936" | "GB2312" | "GBK" => e::GBK,
        "ANSI_949" | "KSC5601" | "KOREAN" => e::EUC_KR,
        "ANSI_950" | "BIG5" => e::BIG5,
        "ANSI_1250" => e::WINDOWS_1250,
        "ANSI_1251" => e::WINDOWS_1251,
        "ANSI_1253" => e::WINDOWS_1253,
        "ANSI_1254" | "ISO8859_9" | "ISO_8859_9" => e::WINDOWS_1254,
        "ANSI_1255" => e::WINDOWS_1255,
        "ANSI_1256" => e::WINDOWS_1256,
        "ANSI_1257" => e::WINDOWS_1257,
        "ANSI_1258" => e::WINDOWS_1258,
        "ISO8859_2" | "ISO_8859_2" => e::ISO_8859_2,
        "ISO8859_3" | "ISO_8859_3" => e::ISO_8859_3,
        "ISO8859_4" | "ISO_8859_4" => e::ISO_8859_4,
        "ISO8859_5" | "ISO_8859_5" => e::ISO_8859_5,
        "ISO8859_6" | "ISO_8859_6" => e::ISO_8859_6,
        "ISO8859_7" | "ISO_8859_7" => e::ISO_8859_7,
        "ISO8859_8" | "ISO_8859_8" => e::ISO_8859_8,
        "ISO8859_15" | "ISO_8859_15" => e::ISO_8859_15,
        "DOS866" => e::IBM866,
        "MACINTOSH" | "MAC_ROMAN" => e::MACINTOSH,
        "DOS437" => return Some(Decoder::Table(&dos::DOS437)),
        "DOS850" => return Some(Decoder::Table(&dos::DOS850)),
        "DOS852" => return Some(Decoder::Table(&dos::DOS852)),
        "DOS720" => return Some(Decoder::Table(&dos::DOS720)),
        "DOS737" => return Some(Decoder::Table(&dos::DOS737)),
        "DOS775" => return Some(Decoder::Table(&dos::DOS775)),
        "DOS855" => return Some(Decoder::Table(&dos::DOS855)),
        "DOS857" => return Some(Decoder::Table(&dos::DOS857)),
        "DOS860" => return Some(Decoder::Table(&dos::DOS860)),
        "DOS861" => return Some(Decoder::Table(&dos::DOS861)),
        "DOS863" => return Some(Decoder::Table(&dos::DOS863)),
        "DOS864" => return Some(Decoder::Dos864),
        "DOS865" => return Some(Decoder::Table(&dos::DOS865)),
        "DOS869" => return Some(Decoder::Table(&dos::DOS869)),
        "ANSI_1361" | "JOHAB" => return Some(Decoder::Johab),
        _ => return None,
    };
    Some(Decoder::Encoding(enc))
}

/// Decode `bytes` from the code page `code_page` names (a `$DWGCODEPAGE` value such as
/// `ANSI_1251`), or `None` when CADCraft doesn't know that code page. Invalid sequences become
/// U+FFFD; escapes (`\U+XXXX`, `\M+nXXXX`) are left as they are.
pub fn decode_code_page(bytes: &[u8], code_page: &str) -> Option<String> {
    decoder(code_page).map(|d| decode(bytes, d).into_owned())
}

/// The code page a pre-2007 file's text is written in, or `None` for pure ASCII with escapes:
/// when CADCraft doesn't know `code_page`, and for `ASCII` (the reader takes such files as
/// Windows-1252, but a writer must keep them ASCII) and `UTF8`/`UNICODE`, which no pre-2007
/// reader expects.
pub(crate) fn encoder(code_page: &str) -> Option<Decoder> {
    if code_page.trim().eq_ignore_ascii_case("ASCII") {
        return None;
    }
    decoder(code_page).filter(|d| !matches!(d, Decoder::Utf8))
}

/// Append the bytes of the non-ASCII character `c` in the code page `d`, and return true, when
/// the code page has it; otherwise append nothing and return false (the caller escapes it).
pub(crate) fn encode_char(c: char, d: Decoder, out: &mut Vec<u8>) -> bool {
    let mut buf = [0; 4];
    match d {
        // Not a pre-2007 code page ([`encoder`] never gives it): escaped.
        // Johab is read only: a writer escapes every character.
        Decoder::Utf8 | Decoder::Johab => return false,
        Decoder::Encoding(e) => {
            let (bytes, _, unmappable) = e.encode(c.encode_utf8(&mut buf));
            // A code page byte that is ASCII would read back as another character, and a NUL,
            // CR or LF would break the group, so only all-high-bit sequences or proper double-byte
            // characters are kept (their trail byte may be ASCII, as in Shift-JIS).
            if unmappable || bytes.is_empty() || bytes.first().is_none_or(|b| *b < 0x80) || bytes.iter().any(|b| matches!(b, 0 | b'\r' | b'\n')) {
                return false;
            }
            out.extend_from_slice(&bytes);
        }
        // U+FFFD marks unassigned bytes in the tables; it must be escaped, not written as one.
        Decoder::Table(_) | Decoder::Dos864 if c == '\u{FFFD}' => return false,
        Decoder::Table(t) => {
            let Some(i) = t.iter().position(|&x| x == c).and_then(|i| u8::try_from(i).ok()) else { return false };
            out.push(0x80 | i);
        }
        Decoder::Dos864 => {
            if c == DOS864_PERCENT {
                out.push(b'%');
            } else {
                let Some(i) = dos::DOS864.iter().position(|&x| x == c).and_then(|i| u8::try_from(i).ok()) else { return false };
                out.push(0x80 | i);
            }
        }
    }
    true
}

/// Decode bytes with a decoder. UTF-8 that isn't valid falls back to Windows-1252.
pub(crate) fn decode(bytes: &[u8], d: Decoder) -> Cow<'_, str> {
    match d {
        Decoder::Utf8 => match std::str::from_utf8(bytes) {
            Ok(s) => Cow::Borrowed(s),
            Err(_) => encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes).0,
        },
        Decoder::Encoding(e) => e.decode_without_bom_handling(bytes).0,
        Decoder::Johab => match std::str::from_utf8(bytes) {
            Ok(s) if s.is_ascii() => Cow::Borrowed(s),
            _ => Cow::Owned(decode_johab(bytes)),
        },
        Decoder::Dos864 => {
            if bytes.is_ascii() && !bytes.contains(&b'%') {
                return Cow::Borrowed(std::str::from_utf8(bytes).unwrap_or_default());
            }
            Cow::Owned(
                bytes
                    .iter()
                    .map(|&b| match b {
                        b'%' => DOS864_PERCENT,
                        _ if b.is_ascii() => char::from(b),
                        _ => dos::DOS864.get(usize::from(b & 0x7F)).copied().unwrap_or('\u{FFFD}'),
                    })
                    .collect(),
            )
        }
        Decoder::Table(t) => match std::str::from_utf8(bytes) {
            Ok(s) if s.is_ascii() => Cow::Borrowed(s),
            _ => Cow::Owned(
                bytes
                    .iter()
                    .map(|&b| if b.is_ascii() { char::from(b) } else { t.get(usize::from(b & 0x7F)).copied().unwrap_or('\u{FFFD}') })
                    .collect(),
            ),
        },
    }
}

/// The value of the header variable `name` (group `code` after it) of an ASCII DXF file: looked
/// for in the HEADER section only (the scan stops at the first `ENDSEC`). Works on the raw bytes,
/// before they are decoded.
fn ascii_header_value<'a>(bytes: &'a [u8], name: &[u8], code: &[u8]) -> Option<&'a [u8]> {
    let mut lines = bytes.split(|&b| b == b'\n' || b == b'\r').map(<[u8]>::trim_ascii).filter(|l| !l.is_empty());
    while let Some(line) = lines.next() {
        if line == b"ENDSEC" {
            return None;
        }
        if line.eq_ignore_ascii_case(name) {
            return match (lines.next(), lines.next()) {
                (Some(c), Some(value)) if c == code => Some(value),
                _ => None,
            };
        }
    }
    None
}

/// Decode Johab text: ASCII, double-byte Hangul (lead bytes 0x84–0xD3), symbols (0xD9–0xDE) and
/// Hanja (0xE0–0xF9). An unassigned code in that area is one U+FFFD for its two bytes; any other
/// byte that can't start a character is one U+FFFD and the next byte starts afresh.
fn decode_johab(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b.is_ascii() {
            out.push(char::from(b));
            i += 1;
            continue;
        }
        let trail = bytes.get(i + 1).copied();
        match trail.and_then(|t| johab_char(b, t)) {
            Some(c) => {
                out.push(c);
                i += 2;
            }
            None => {
                out.push('\u{FFFD}');
                i += if trail.is_some_and(|t| johab_lead(b) && johab_trail(b, t)) { 2 } else { 1 };
            }
        }
    }
    out
}

fn johab_lead(b: u8) -> bool {
    matches!(b, 0x84..=0xD3 | 0xD8..=0xDE | 0xE0..=0xF9)
}

fn johab_trail(lead: u8, t: u8) -> bool {
    if lead <= 0xD3 { matches!(t, 0x41..=0x7E | 0x81..=0xFE) } else { matches!(t, 0x31..=0x7E | 0x91..=0xFE) }
}

/// The character of the Johab code `lead trail`, if it has one.
pub(crate) fn johab_char(lead: u8, trail: u8) -> Option<char> {
    if !johab_lead(lead) || !johab_trail(lead, trail) {
        return None;
    }
    if lead <= 0xD3 {
        return johab_hangul((u16::from(lead) << 8) | u16::from(trail));
    }
    // Symbols and Hanja: each lead byte holds two KS X 1001 rows of 94 cells, the first in
    // trail bytes 0x31–0x7E and 0x91–0xA0, the second in 0xA1–0xFE (0xD8 is user-defined).
    let first = match lead {
        0xD9..=0xDE => (lead - 0xD9) * 2 + 0x21,
        0xE0..=0xF9 => (lead - 0xE0) * 2 + 0x4A,
        _ => return None,
    };
    let (row, col) = match trail {
        0x31..=0x7E => (first, trail - 0x31 + 0x21),
        0x91..=0xA0 => (first, trail - 0x91 + 0x6F),
        _ => (first + 1, trail.saturating_sub(0xA1) + 0x21),
    };
    let euc = [row | 0x80, col | 0x80];
    let text = encoding_rs::EUC_KR.decode_without_bom_handling_and_without_replacement(&euc)?;
    let mut it = text.chars();
    it.next().filter(|_| it.next().is_none())
}

/// A Johab Hangul code (bit 15 set, then 5 bits each for the initial, medial and final jamo):
/// a precomposed syllable, or a lone jamo (as a compatibility jamo) when the others are fillers.
fn johab_hangul(code: u16) -> Option<char> {
    let (i, m, f) = ((code >> 10) & 0x1F, (code >> 5) & 0x1F, code & 0x1F);
    // Initial: 1 filler, 2–20 the 19 consonants. Medial: 2 filler, then the 21 vowels in four
    // runs (3–7, 10–15, 18–23, 26–29). Final: 1 none, 2–17 and 19–29 the 27 final consonants.
    let lead = match i {
        1 => None,
        2..=20 => Some(i - 2),
        _ => return None,
    };
    let vowel = match m {
        2 => None,
        3..=7 => Some(m - 3),
        10..=15 => Some(m - 5),
        18..=23 => Some(m - 7),
        26..=29 => Some(m - 9),
        _ => return None,
    };
    let tail = match f {
        1 => 0,
        2..=17 => f - 1,
        19..=29 => f - 2,
        _ => return None,
    };
    // The compatibility jamo (offsets from U+3131) of the initial and of the final consonants.
    const LEADS: [u16; 19] = [0, 1, 3, 6, 7, 8, 16, 17, 18, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29];
    const TAILS: [u16; 27] = [0, 1, 2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 19, 20, 21, 22, 23, 25, 26, 27, 28, 29];
    let cp = match (lead, vowel, tail) {
        (Some(l), Some(v), t) => 0xAC00 + (u32::from(l) * 21 + u32::from(v)) * 28 + u32::from(t),
        (None, None, 0) => 0x3164,
        (Some(l), None, 0) => 0x3131 + u32::from(*LEADS.get(usize::from(l))?),
        (None, Some(v), 0) => 0x314F + u32::from(v),
        (None, None, t) => 0x3131 + u32::from(*TAILS.get(usize::from(t.saturating_sub(1)))?),
        _ => return None,
    };
    char::from_u32(cp)
}

/// The `$DWGCODEPAGE` value of an ASCII DXF file.
fn ascii_code_page(bytes: &[u8]) -> Option<&[u8]> {
    ascii_header_value(bytes, b"$DWGCODEPAGE", b"3")
}

impl Decoder {
    /// True for the multi-byte East Asian code pages (Shift-JIS, GBK, EUC-KR/Wansung, Big5, Johab).
    /// Their short double-byte strings are often also valid UTF-8, so UTF-8 is only trusted for
    /// a whole file there (see [`decode_ascii_file`] and [`BinaryText`]).
    fn is_east_asian(self) -> bool {
        use encoding_rs as e;
        matches!(self, Decoder::Johab) || matches!(self, Decoder::Encoding(x) if x == e::SHIFT_JIS || x == e::GBK || x == e::EUC_KR || x == e::BIG5)
    }
}

/// Decode one line of an ASCII DXF file: valid UTF-8 is read as UTF-8 (R2007+ files, pure-ASCII
/// text, and older files from writers that ignored the code page); anything else comes from the
/// code page `d`, so one stray byte spoils one line, not the whole file. Binary strings don't use
/// this rule: a pre-R2007 binary file has no way to escape to UTF-8, so its strings are read
/// from the code page (see [`BinaryText::string`]).
pub(crate) fn decode_text(bytes: &[u8], d: Decoder) -> Cow<'_, str> {
    match std::str::from_utf8(bytes) {
        Ok(s) => Cow::Borrowed(s),
        Err(_) => decode(bytes, d),
    }
}

/// Decode a whole ASCII DXF file. A file that is valid UTF-8 throughout is UTF-8. Otherwise, for
/// a single-byte code page, each line is decoded on its own (see [`decode_text`]): lines that
/// aren't valid UTF-8 come from the file's `$DWGCODEPAGE`, or from Windows-1252 (the usual
/// `ANSI_1252`) when the header has none or names a code page CADCraft doesn't know, which is
/// logged as a warning. For a multi-byte East Asian code page the whole file is decoded from
/// that code page, because a short double-byte string (two half-width katakana, many kanji
/// pairs) can be valid UTF-8 by accident and must not be misread line by line.
pub(crate) fn decode_ascii_file(bytes: &[u8]) -> Cow<'_, str> {
    if let Ok(s) = std::str::from_utf8(bytes) {
        if !s.contains('%') || !is_pre_2007_dos864(bytes) {
            return Cow::Borrowed(s);
        }
        return decode(bytes, Decoder::Dos864);
    }
    let d = match ascii_code_page(bytes) {
        Some(name) => {
            let name = String::from_utf8_lossy(name);
            decoder(&name).unwrap_or_else(|| {
                log::warn!("DXF: unknown code page {}; reading the text as ANSI_1252", loggable(&name));
                Decoder::Utf8
            })
        }
        None => Decoder::Utf8,
    };
    if d.is_east_asian() {
        return decode(bytes, d);
    }
    // Split after each line terminator (CR, LF or CRLF are all ASCII, so they never occur inside
    // a multi-byte character of the supported code pages) and decode each line on its own.
    let mut out = String::with_capacity(bytes.len());
    for line in bytes.split_inclusive(|&b| b == b'\n' || b == b'\r') {
        out.push_str(&decode_text(line, d));
    }
    Cow::Owned(out)
}

/// True for an ASCII-only file whose header names `DOS864` and is not R2007+ (which is UTF-8).
fn is_pre_2007_dos864(bytes: &[u8]) -> bool {
    bytes.is_ascii()
        && ascii_code_page(bytes).is_some_and(|n| n.eq_ignore_ascii_case(b"DOS864"))
        && !ascii_header_value(bytes, b"$ACADVER", b"1").is_some_and(|v| Version::for_acadver(&String::from_utf8_lossy(v)).is_some_and(Version::utf8))
}

/// The text state of a binary DXF file, learnt from the header as the tags go by: `$ACADVER`
/// (R2007+ strings are UTF-8) and `$DWGCODEPAGE` (the code page of older files). Both come
/// first in the HEADER section and are ASCII, so every later string is decoded right.
#[derive(Default)]
pub(crate) struct BinaryText {
    /// The header variable whose value comes next (after a group 9).
    var: Option<HeaderVar>,
    utf8: bool,
    code_page: Option<Decoder>,
    /// Decode every string of an East Asian code page file from the code page, never as UTF-8
    /// (the second pass of [`BinaryText::needs_code_page_pass`]).
    code_page_only: bool,
    /// Non-ASCII strings seen under an East Asian code page: valid UTF-8 / not valid UTF-8.
    ea_utf8: bool,
    ea_other: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum HeaderVar {
    AcadVer,
    CodePage,
}

impl BinaryText {
    /// The state for the second pass over a file whose strings are a mix of valid UTF-8 and
    /// not: every string of an East Asian code page is decoded from that code page.
    pub(crate) fn code_page_pass() -> Self {
        Self { code_page_only: true, ..Self::default() }
    }

    /// True when the file declared a multi-byte East Asian code page and its non-ASCII strings are
    /// not all valid UTF-8 although some are: those can be short double-byte strings that look
    /// like UTF-8 by accident, so the file must be parsed again with [`Self::code_page_pass`]. (A
    /// file whose strings are all valid UTF-8 is UTF-8; one with none is plainly the code page.)
    pub(crate) fn needs_code_page_pass(&self) -> bool {
        !self.code_page_only && self.ea_utf8 && self.ea_other
    }

    /// Decode one string value of group `code` and update the state from it. Strings of R2007+
    /// files, and of files without a code page, are UTF-8 when valid (else Windows-1252); a
    /// pre-R2007 file's strings are in its code page, which a binary DXF cannot escape to, so
    /// they are read from the code page even when their bytes are valid UTF-8 by accident —
    /// except under an East Asian code page, where a file of all-valid-UTF-8 strings is taken
    /// as UTF-8 (see [`Self::needs_code_page_pass`]).
    pub(crate) fn string(&mut self, code: i32, raw: &[u8]) -> String {
        let d = if self.utf8 { Decoder::Utf8 } else { self.code_page.unwrap_or(Decoder::Utf8) };
        let s = if d.is_east_asian() && !self.code_page_only {
            if !raw.is_ascii() {
                if std::str::from_utf8(raw).is_ok() {
                    self.ea_utf8 = true;
                } else {
                    self.ea_other = true;
                }
            }
            decode_text(raw, d).into_owned()
        } else {
            decode(raw, d).into_owned()
        };
        match (code, self.var.take()) {
            (9, _) => {
                self.var = match s.trim() {
                    "$ACADVER" => Some(HeaderVar::AcadVer),
                    "$DWGCODEPAGE" => Some(HeaderVar::CodePage),
                    _ => None,
                }
            }
            (1, Some(HeaderVar::AcadVer)) => self.utf8 = Version::for_acadver(&s).is_some_and(Version::utf8),
            (3, Some(HeaderVar::CodePage)) => {
                self.code_page = decoder(&s);
                if self.code_page.is_none() {
                    log::warn!("DXF: unknown code page {}; reading the text as ANSI_1252", loggable(s.trim()));
                }
            }
            _ => {}
        }
        s
    }
}

/// The character of a `\M+n` escape's two bytes in its double-byte code page `n`: 1 Japanese
/// (Shift-JIS), 2 Traditional Chinese (Big5), 3 Korean (Wansung), 4 Korean (Johab), 5 Simplified
/// Chinese (GB 2312). Other code pages and unassigned codes give `None`.
pub(crate) fn mif_char(n: char, bytes: [u8; 2]) -> Option<char> {
    let enc: &'static Encoding = match n {
        '1' => encoding_rs::SHIFT_JIS,
        '2' => encoding_rs::BIG5,
        '3' => encoding_rs::EUC_KR,
        '4' => return johab_char(bytes[0], bytes[1]),
        '5' => encoding_rs::GBK,
        _ => return None,
    };
    let text = enc.decode_without_bom_handling_and_without_replacement(&bytes)?;
    let mut it = text.chars();
    it.next().filter(|_| it.next().is_none())
}

/// A name from the file for a log line: quoted, with line breaks and other control characters
/// escaped, and cut at 64 characters, so a file can't forge log lines or fill the log.
fn loggable(name: &str) -> String {
    let mut shown: String = name.chars().take(64).collect();
    if name.chars().nth(64).is_some() {
        shown.push('…');
    }
    format!("{shown:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_page_name_is_escaped_and_cut_before_it_is_logged() {
        assert_eq!(loggable("ANSI_1252"), "\"ANSI_1252\"");
        let shown = loggable("x\r\n[ERROR] forged\u{1b}[31m");
        assert!(!shown.contains(['\r', '\n', '\u{1b}']), "{shown}");
        let long = loggable(&"a".repeat(10_000));
        assert!(long.chars().count() <= 70, "{}", long.chars().count());
    }

    #[test]
    fn code_page_names() {
        let d = |bytes: &[u8], cp: &str| decode_code_page(bytes, cp);
        assert_eq!(d(b"\xCF\xF0\xE8\xE2\xE5\xF2", "ANSI_1251").as_deref(), Some("Привет"));
        assert_eq!(d(b"\xCF\xF0\xE8\xE2\xE5\xF2", " ansi_1251 ").as_deref(), Some("Привет"));
        assert_eq!(d(b"\x90\x7D\x96\xCA", "ANSI_932").as_deref(), Some("図面"));
        assert_eq!(d(b"\xCD\xBC\xB2\xE3", "GB2312").as_deref(), Some("图层"));
        assert_eq!(d(b"\xB9\xCF\xBC\x68", "BIG5").as_deref(), Some("圖層"));
        assert_eq!(d(b"\xB5\xB5\xB8\xE9", "ANSI_949").as_deref(), Some("도면"));
        assert_eq!(d(b"\xD3\xF7\xDD\xE4\xE9\xEF", "ANSI_1253").as_deref(), Some("Σχέδιο"));
        assert_eq!(d(b"\xC0\xD2\xBE", "ANSI_874").as_deref(), Some("ภาพ"));
        assert_eq!(d(b"\x80", "ANSI_1252").as_deref(), Some("€"));
        // DOS 437 and 850 agree on letters but differ on 0x9B (¢ / ø).
        assert_eq!(d(b"Gr\x94\xE1e \x9B", "DOS437").as_deref(), Some("Größe ¢"));
        assert_eq!(d(b"Gr\x94\xE1e \x9B", "dos850").as_deref(), Some("Größe ø"));
        assert_eq!(d(b"\xBD\xA2\x88w", "DOS852").as_deref(), Some("Żółw"));
        assert_eq!(d(b"\x91\xAB\xAE\xA9", "DOS866").as_deref(), Some("Слой"));
        assert_eq!(d("é".as_bytes(), "UTF8").as_deref(), Some("é"));
        assert_eq!(d(b"x", "EBCDIC"), None);
        assert_eq!(d(b"x", ""), None);
    }

    #[test]
    fn replacement_char_is_never_encoded_as_an_unassigned_byte() {
        for cp in ["DOS857", "DOS864", "DOS437"] {
            let d = encoder(cp).unwrap();
            let mut out = Vec::new();
            assert!(!encode_char('\u{FFFD}', d, &mut out), "{cp}");
            assert!(out.is_empty(), "{cp}");
        }
    }

    #[test]
    fn dos864_percent_is_the_arabic_percent_sign() {
        // Byte 0x25 is U+066A in CP864 (Python's cp864 agrees); the rest of the low half is ASCII.
        assert_eq!(decode_code_page(b"50%", "DOS864").as_deref(), Some("50\u{066A}"));
        assert_eq!(decode_code_page(b"a%\x80%", "dos864").as_deref(), Some("a\u{066A}\u{00B0}\u{066A}"));
        assert_eq!(decode_code_page(b"50&", "DOS864").as_deref(), Some("50&"));
        // The other DOS pages keep `%`.
        assert_eq!(decode_code_page(b"50%", "DOS437").as_deref(), Some("50%"));
        // Every other low byte of every table is ASCII.
        for cp in [
            "DOS437", "DOS850", "DOS852", "DOS720", "DOS737", "DOS775", "DOS855", "DOS857", "DOS860", "DOS861", "DOS863", "DOS864", "DOS865",
            "DOS869",
        ] {
            for b in 0u8..0x80 {
                if b == 0x25 && cp == "DOS864" {
                    continue;
                }
                assert_eq!(decode_code_page(&[b], cp).as_deref(), Some(char::from(b).to_string().as_str()), "{cp} {b:#x}");
            }
        }
    }

    #[test]
    fn ascii_only_dos864_file_keeps_the_arabic_percent_sign() {
        let file = |ver: &str, cp: &str| format!("0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\n{ver}\n9\n$DWGCODEPAGE\n3\n{cp}\n0\nENDSEC\n1\n50%\n");
        assert!(decode_ascii_file(file("AC1015", "DOS864").as_bytes()).ends_with("50\u{066A}\n"));
        // Other code pages, and R2007+ files (UTF-8), keep `%`.
        assert!(decode_ascii_file(file("AC1015", "DOS437").as_bytes()).ends_with("50%\n"));
        assert!(decode_ascii_file(file("AC1021", "DOS864").as_bytes()).ends_with("50%\n"));
    }

    /// Korean Johab (`ANSI_1361`): Hangul syllables composed from their jamo, lone jamo,
    /// symbols and Hanja through KS X 1001.
    #[test]
    fn johab() {
        let d = |bytes: &[u8]| decode_code_page(bytes, "ANSI_1361");
        // 한글 (U+D55C U+AE00) and 가 (U+AC00).
        assert_eq!(d(b"\xD0\x65\x8B\x69 \x88\x61").as_deref(), Some("한글 가"));
        // Lone initial ㄱ, lone vowel ㅏ, the Hangul filler.
        assert_eq!(d(b"\x88\x41\x84\x61\x84\x41").as_deref(), Some("ㄱㅏ\u{3164}"));
        // The ideographic space (KS X 1001 0x2121) and 伽 (0x4A21, the first Hanja).
        assert_eq!(d(b"\xD9\x31\xE0\x31").as_deref(), Some("\u{3000}伽"));
        assert_eq!(decode_code_page(b"\x88\x61", "johab").as_deref(), Some("가"));
        // A truncated lead byte, a lead byte without a valid trail byte, a user-defined code.
        assert_eq!(d(b"A\x88").as_deref(), Some("A\u{FFFD}"));
        assert_eq!(d(b"\x88\x20B").as_deref(), Some("\u{FFFD} B"));
        assert_eq!(d(b"\xD8\x31C").as_deref(), Some("\u{FFFD}C"));
        // Every pair of bytes decodes to something without panicking.
        for a in 0x80..=0xFFu8 {
            for b in 0..=0xFFu8 {
                assert!(!d(&[a, b, b'\n']).unwrap_or_default().is_empty());
            }
        }
    }

    #[test]
    fn invalid_sequences_become_replacement_characters() {
        // A Shift-JIS lead byte cut off by the line end keeps the line end.
        assert_eq!(decode_code_page(b"\x90\n1\n", "ANSI_932").as_deref(), Some("\u{FFFD}\n1\n"));
        assert_eq!(decode_code_page(b"\x90", "ANSI_932").as_deref(), Some("\u{FFFD}"));
        assert_eq!(decode_code_page(b"\xFF\xFF", "ANSI_936").as_deref(), Some("\u{FFFD}\u{FFFD}"));
    }

    #[test]
    fn header_scan() {
        let head = b"0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n9\n$DWGCODEPAGE\n3\nANSI_1251\r\n0\nENDSEC\n";
        assert_eq!(ascii_code_page(head), Some(&b"ANSI_1251"[..]));
        // Only the HEADER section counts.
        assert_eq!(ascii_code_page(b"0\nSECTION\n2\nHEADER\n0\nENDSEC\n9\n$DWGCODEPAGE\n3\nANSI_1251\n"), None);
        // Wrong group code, or the file ends early.
        assert_eq!(ascii_code_page(b"9\n$DWGCODEPAGE\n1\nANSI_1251\n"), None);
        assert_eq!(ascii_code_page(b"9\n$DWGCODEPAGE\n3"), None);
        let utf8 = |v: &str| Version::for_acadver(v).is_some_and(Version::utf8);
        assert!(utf8("AC1021") && utf8(" AC1032 "));
        assert!(!utf8("AC1018") && !utf8("AC1009") && !utf8("garbage"));
    }

    #[test]
    fn mif_escapes() {
        let one = |s: &str| crate::parse(format!("1\n{s}\n0\nEOF\n").as_bytes()).unwrap()[0].str();
        assert_eq!(one("\\M+5BCFE\\M+1907D\\M+2B9CF\\M+3B5B5!"), "件図圖도!");
        assert_eq!(one("\\M+5BCFE\\U+0021"), "件!");
        // Johab: 한.
        assert_eq!(one("\\M+4D065"), "한");
        // Unknown code pages, bad hex, short escapes and unassigned codes stay literal.
        for s in ["\\M+4D820", "\\M+9BCFE", "\\M+5GGGG", "\\M+5BC", "\\M+", "\\M+50041", "\\M+5FFFF"] {
            assert_eq!(one(s), s);
        }
    }
}
