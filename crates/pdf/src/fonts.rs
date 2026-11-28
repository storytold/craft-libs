//! PDF single-byte encodings (ISO 32000-2 Annex D), WinAnsi/Unicode CMap encoding helpers,
//! and Standard-14 font metrics.

pub mod encodings;
mod std14;
mod std14_metrics;
#[cfg(feature = "cos")]
pub mod wire;

pub use std14::{Match as Std14Match, Std14};

/// The WinAnsiEncoding (ISO 32000-2 Annex D) code for `c`, if the encoding has one.
///
/// Codes 0x80–0x9F come from the same table the text extractor decodes with, so encoding and
/// decoding agree (Š š Ž ž Œ œ Ÿ ƒ † ‡ ˆ ˜ ‰ ‹ › as well as quotes, dashes, € and ™).
pub fn win_ansi_byte(c: char) -> Option<u8> {
    match c {
        '\u{20}'..='\u{7e}' | '\u{a0}'..='\u{ff}' => u8::try_from(u32::from(c)).ok(),
        _ => {
            let high = encodings::WIN_ANSI.get(0x80..0xa0)?;
            let i = high.iter().position(|&u| u != 0 && u == u32::from(c))?;
            u8::try_from(0x80 + i).ok()
        }
    }
}

/// Encode text in WinAnsiEncoding (ISO 32000-2 Annex D); tabs become spaces and unmappable
/// characters become `?`.
pub fn win_ansi(s: &str) -> Vec<u8> {
    s.chars().map(|c| if c == '\t' { b' ' } else { win_ansi_byte(c).unwrap_or(b'?') }).collect()
}

/// The byte used by Unicode text plans, from the shared WinAnsi table (tabs become spaces).
pub fn win_ansi_char(c: char) -> Option<u8> {
    if c == '\t' { Some(b' ') } else { win_ansi_byte(c) }
}

/// Whether [`win_ansi`] keeps `c` rather than writing `?` (a tab becomes a space).
pub fn win_ansi_encodable(c: char) -> bool {
    c == '\t' || win_ansi_byte(c).is_some()
}

/// The first character of `s` that [`win_ansi`] can't encode and would write as `?`.
pub fn first_non_win_ansi(s: &str) -> Option<char> {
    s.chars().find(|c| !win_ansi_encodable(*c))
}

/// Whether every character can be represented without substitution in WinAnsi.
pub fn fits_win_ansi(s: &str) -> bool {
    s.chars().all(win_ansi_encodable)
}

/// Latin letters outside WinAnsiEncoding that Romanian and Polish text uses, as (character, code,
/// glyph name, base letter), for a standard-14 font whose `/Encoding` is WinAnsiEncoding with a
/// `/Differences` array naming these glyphs at these codes.
///
/// The codes are control codes WinAnsiEncoding leaves undefined (0x01-0x08, 0x0B, 0x0E-0x1F), so
/// no WinAnsi character is displaced. 0x09, 0x0A, 0x0C and 0x0D are not used: viewers treat tab
/// and line feed as white space when drawing, a literal string turns an unescaped carriage return
/// into a line feed, and form feed separates pages in extracted text. ț and Ț are named `uni021B`
/// and `uni021A` because the glyph list maps `tcommaaccent` and `Tcommaaccent` to the cedilla
/// letters U+0163 and U+0162. In the standard-14 metrics each glyph has its base letter's advance.
///
/// The standard-14 fonts are not embedded, so these letters are drawn only where the viewer's
/// substitute font has the glyph; text extraction reads the names back without depending on it.
/// ISO 19005 (PDF/A) requires embedded fonts, so output using this table cannot meet PDF/A.
pub const LATIN_EXTRA: &[(char, u8, &str, char)] = &[
    ('ă', 0x01, "abreve", 'a'),
    ('Ă', 0x02, "Abreve", 'A'),
    ('ș', 0x03, "scommaaccent", 's'),
    ('Ș', 0x04, "Scommaaccent", 'S'),
    ('ț', 0x05, "uni021B", 't'),
    ('Ț', 0x06, "uni021A", 'T'),
    ('ş', 0x07, "scedilla", 's'),
    ('Ş', 0x08, "Scedilla", 'S'),
    ('ţ', 0x0b, "tcedilla", 't'),
    ('Ţ', 0x0e, "Tcedilla", 'T'),
    ('ą', 0x0f, "aogonek", 'a'),
    ('Ą', 0x10, "Aogonek", 'A'),
    ('ę', 0x11, "eogonek", 'e'),
    ('Ę', 0x12, "Eogonek", 'E'),
    ('ł', 0x13, "lslash", 'l'),
    ('Ł', 0x14, "Lslash", 'L'),
    ('ś', 0x15, "sacute", 's'),
    ('Ś', 0x16, "Sacute", 'S'),
    ('ż', 0x17, "zdotaccent", 'z'),
    ('Ż', 0x18, "Zdotaccent", 'Z'),
    ('ź', 0x19, "zacute", 'z'),
    ('Ź', 0x1a, "Zacute", 'Z'),
    ('ć', 0x1b, "cacute", 'c'),
    ('Ć', 0x1c, "Cacute", 'C'),
    ('ń', 0x1d, "nacute", 'n'),
    ('Ń', 0x1e, "Nacute", 'N'),
];

/// Encode text in WinAnsiEncoding plus [`LATIN_EXTRA`]: those letters become their codes and the
/// rest is encoded as [`win_ansi`] encodes it (tabs become spaces, unmappable characters `?`).
pub fn win_ansi_extra(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| match LATIN_EXTRA.iter().find(|e| e.0 == c) {
            Some(e) => e.1,
            None if c == '\t' => b' ',
            None => win_ansi_byte(c).unwrap_or(b'?'),
        })
        .collect()
}

/// The base letter a [`LATIN_EXTRA`] character is measured as, if `c` is one.
pub fn extra_base_letter(c: char) -> Option<char> {
    LATIN_EXTRA.iter().find(|e| e.0 == c).map(|e| e.3)
}

/// The character a [`LATIN_EXTRA`] glyph name stands for, so a reader of a `/Differences` array
/// written from that table decodes it (`abreve` is ă, `uni021B` is ț).
pub fn latin_extra_glyph(name: &str) -> Option<char> {
    LATIN_EXTRA.iter().find(|e| e.2 == name).map(|e| e.0)
}

/// The first character of `s` that [`win_ansi_extra`] can't encode and would write as `?`.
pub fn first_non_win_ansi_extra(s: &str) -> Option<char> {
    s.chars().find(|c| !win_ansi_encodable(*c) && extra_base_letter(*c).is_none())
}

/// The character encoding of a predefined Unicode CMap (`UniJIS-UTF16-H`, `UniGB-UCS2-H`, …;
/// ISO 32000-2 §9.7.5.2), which maps Unicode text straight to codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnicodeCMap {
    Ucs2,
    Utf16,
    Utf8,
    Utf32,
}

impl UnicodeCMap {
    /// The encoding of the predefined CMap `name`, if it is a horizontal Unicode one. Vertical
    /// (`-V`) CMaps are refused: field appearances are laid out horizontally, so their glyphs
    /// would be drawn stacked down the field instead of across it.
    pub fn from_name(name: &[u8]) -> Option<Self> {
        let name = std::str::from_utf8(name).ok()?;
        if !name.starts_with("Uni") || !name.ends_with("-H") {
            return None;
        }
        name.split('-').find_map(|part| match part {
            "UCS2" => Some(Self::Ucs2),
            "UTF16" => Some(Self::Utf16),
            "UTF8" => Some(Self::Utf8),
            "UTF32" => Some(Self::Utf32),
            _ => None,
        })
    }

    /// The first character of `s` this encoding can't hold, which [`Self::encode`] would write as
    /// `?`: a character beyond U+FFFF in UCS-2 (`𠀋`). UTF-16, UTF-8 and UTF-32 hold every one.
    pub fn first_unencodable(self, s: &str) -> Option<char> {
        match self {
            Self::Ucs2 => s.chars().find(|c| u16::try_from(u32::from(*c)).is_err()),
            Self::Utf16 | Self::Utf8 | Self::Utf32 => None,
        }
    }

    /// Encode `s`; tabs become spaces, and characters UCS-2 can't hold become `?` (check them
    /// first with [`Self::first_unencodable`]: callers should refuse such text rather than write `?`).
    pub fn encode(self, s: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(s.len().saturating_mul(2));
        let mut units = [0u16; 2];
        let mut buf = [0u8; 4];
        for c in s.chars() {
            let c = if c == '\t' { ' ' } else { c };
            match self {
                Self::Ucs2 => {
                    let unit = u16::try_from(u32::from(c)).unwrap_or(u16::from(b'?'));
                    out.extend_from_slice(&unit.to_be_bytes());
                }
                Self::Utf16 => {
                    for unit in c.encode_utf16(&mut units) {
                        out.extend_from_slice(&unit.to_be_bytes());
                    }
                }
                Self::Utf8 => out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes()),
                Self::Utf32 => out.extend_from_slice(&u32::from(c).to_be_bytes()),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_alias_codes_read_as_the_glyph_they_draw() {
        assert_eq!(encodings::WIN_ANSI.get(0xAD), Some(&u32::from('-')));
        assert_eq!(encodings::WIN_ANSI.get(0x2D), Some(&u32::from('-')));
        assert_eq!(encodings::WIN_ANSI.get(0xA0), Some(&u32::from(' ')));
        assert_eq!(encodings::WIN_ANSI.get(0x20), Some(&u32::from(' ')));
        assert_eq!(encodings::MAC_ROMAN.get(0xCA), Some(&u32::from(' ')));
        assert_eq!(encodings::MAC_ROMAN.get(0x20), Some(&u32::from(' ')));
        // MacRoman has no soft hyphen; 0xAD there is `notequal`, and keeps it.
        assert_eq!(encodings::MAC_ROMAN.get(0xAD), Some(&0x2260));
        // Writing is unaffected: '-' and ' ' take the canonical low codes, and a caller holding a
        // literal U+00AD or U+00A0 still writes the high byte, which is the one that draws them.
        assert_eq!(win_ansi_byte('-'), Some(0x2D));
        assert_eq!(win_ansi_byte(' '), Some(0x20));
        assert_eq!(win_ansi_byte('\u{00ad}'), Some(0xAD));
        assert_eq!(win_ansi_byte('\u{00a0}'), Some(0xA0));
        assert_eq!(win_ansi("a-b c"), b"a-b c");
    }

    #[test]
    fn win_ansi_covers_the_0x80_to_0x9f_glyphs() {
        const ALIASES: &[u8] = &[0xA0, 0xAD];
        for (code, &u) in encodings::WIN_ANSI.iter().enumerate() {
            let Some(c) = char::from_u32(u).filter(|_| u != 0) else { continue };
            if ALIASES.contains(&(code as u8)) {
                let canonical = win_ansi_byte(c).expect("an aliased glyph still encodes");
                assert!(canonical < code as u8, "{c:?}: the canonical code is the lower one");
                assert_eq!(encodings::WIN_ANSI.get(usize::from(canonical)), Some(&u), "both codes mean the same character");
                continue;
            }
            assert_eq!(win_ansi_byte(c), Some(code as u8), "{c:?} (U+{u:04X})");
            assert_eq!(win_ansi(&c.to_string()), [code as u8]);
        }
        assert_eq!(
            win_ansi("Šta? žaba, œuvre — „ok“ ‰ ƒ † ‡ ˆ ˜ ‹›Ÿ"),
            b"\x8ata? \x9eaba, \x9cuvre \x97 \x84ok\x93 \x89 \x83 \x86 \x87 \x88 \x98 \x8b\x9b\x9f"
        );
        assert_eq!(win_ansi("čć\u{81}\u{8d}\t"), b"???? ");
        assert_eq!(win_ansi_byte('\0'), None);
    }

    /// A reader that knows WinAnsiEncoding and the `/Differences` names gets back the text
    /// written with [`win_ansi_extra`].
    fn decode_extra(bytes: &[u8]) -> String {
        bytes
            .iter()
            .map(|&b| match LATIN_EXTRA.iter().find(|e| e.1 == b) {
                Some(e) => latin_extra_glyph(e.2).unwrap_or('?'),
                None => encodings::WIN_ANSI.get(usize::from(b)).and_then(|&u| char::from_u32(u)).unwrap_or('?'),
            })
            .collect()
    }

    #[test]
    fn latin_extra_text_round_trips_through_its_differences_names() {
        for text in ["Declarație de impozit pe venit, școală, ăĂșȘțȚşŞţŢ", "Zażółć gęślą jaźń, ŁÓDŹ, ĄĘŚŻĆŃ"] {
            let bytes = win_ansi_extra(text);
            assert!(!bytes.contains(&b'?'), "{text}");
            assert_eq!(decode_extra(&bytes), text);
            assert_eq!(first_non_win_ansi_extra(text), None);
        }
        // Plain WinAnsi text is written exactly as `win_ansi` writes it.
        let plain = "Šta? žaba, œuvre — âîé€\t";
        assert_eq!(win_ansi_extra(plain), win_ansi(plain));
        // What neither table holds is still reported and written as `?`.
        assert_eq!(first_non_win_ansi_extra("Obligații Ω"), Some('Ω'));
        assert_eq!(win_ansi_extra("Ω"), b"?");
    }

    #[test]
    fn latin_extra_codes_displace_nothing_and_are_never_white_space() {
        let mut seen = std::collections::HashSet::new();
        for &(c, code, name, base) in LATIN_EXTRA {
            assert!(seen.insert(code), "{c:?}: code {code:#04x} used twice");
            assert_eq!(encodings::WIN_ANSI.get(usize::from(code)), Some(&0), "{c:?}: code {code:#04x} is defined in WinAnsi");
            assert!(!char::from(code).is_ascii_whitespace(), "{c:?}: code {code:#04x} is white space");
            assert!(win_ansi_byte(c).is_none(), "{c:?} is already WinAnsi");
            assert!(win_ansi_byte(base).is_some(), "{c:?}: base letter {base:?} must be WinAnsi");
            assert_eq!(latin_extra_glyph(name), Some(c));
        }
    }

    #[test]
    fn unicode_cmaps_encode_cjk_text() {
        assert_eq!(UnicodeCMap::from_name(b"UniJIS-UTF16-H"), Some(UnicodeCMap::Utf16));
        assert_eq!(UnicodeCMap::from_name(b"UniGB-UCS2-V"), None);
        assert_eq!(UnicodeCMap::from_name(b"UniJIS-UCS2-HW-H"), Some(UnicodeCMap::Ucs2));
        assert_eq!(UnicodeCMap::from_name(b"UniKS-UTF8-H"), Some(UnicodeCMap::Utf8));
        assert_eq!(UnicodeCMap::from_name(b"UniJIS2004-UTF32-H"), Some(UnicodeCMap::Utf32));
        assert_eq!(UnicodeCMap::from_name(b"Identity-H"), None);
        assert_eq!(UnicodeCMap::from_name(b"90ms-RKSJ-H"), None);
        assert_eq!(UnicodeCMap::from_name(b"\xff\xfe"), None);
        assert_eq!(UnicodeCMap::Utf16.encode("令A"), [0x4e, 0xe4, 0x00, 0x41]);
        assert_eq!(UnicodeCMap::Utf16.encode("𠀋"), [0xd8, 0x40, 0xdc, 0x0b]);
        assert_eq!(UnicodeCMap::Ucs2.encode("𠀋\t"), [0x00, b'?', 0x00, b' ']);
        assert_eq!(UnicodeCMap::Ucs2.first_unencodable("令和𠀋"), Some('𠀋'));
        assert_eq!(UnicodeCMap::Ucs2.first_unencodable("令和"), None);
        for cmap in [UnicodeCMap::Utf16, UnicodeCMap::Utf8, UnicodeCMap::Utf32] {
            assert_eq!(cmap.first_unencodable("令和𠀋"), None, "{cmap:?}");
        }
        assert_eq!(UnicodeCMap::Utf8.encode("令"), "令".as_bytes());
        assert_eq!(UnicodeCMap::Utf32.encode("令"), [0, 0, 0x4e, 0xe4]);
    }
}
