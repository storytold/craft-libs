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
