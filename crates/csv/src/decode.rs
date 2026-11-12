//! Bytes of a text file to a string: byte order marks, UTF-8, and a guess between Windows-1252
//! and Windows-1256 for everything else.

/// Windows-1252 code points for bytes 0x80–0x9F (others map to Latin-1).
const CP1252: [u32; 32] = [
    0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019,
    0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
];

/// Windows-1256 code points for bytes 0x80–0xFF (unicode.org `CP1256.TXT`).
const CP1256: [u32; 128] = [
    0x20AC, 0x067E, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0679, 0x2039, 0x0152, 0x0686, 0x0698, 0x0688, 0x06AF, 0x2018,
    0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x06A9, 0x2122, 0x0691, 0x203A, 0x0153, 0x200C, 0x200D, 0x06BA, 0x00A0, 0x060C, 0x00A2, 0x00A3,
    0x00A4, 0x00A5, 0x00A6, 0x00A7, 0x00A8, 0x00A9, 0x06BE, 0x00AB, 0x00AC, 0x00AD, 0x00AE, 0x00AF, 0x00B0, 0x00B1, 0x00B2, 0x00B3, 0x00B4, 0x00B5,
    0x00B6, 0x00B7, 0x00B8, 0x00B9, 0x061B, 0x00BB, 0x00BC, 0x00BD, 0x00BE, 0x061F, 0x06C1, 0x0621, 0x0622, 0x0623, 0x0624, 0x0625, 0x0626, 0x0627,
    0x0628, 0x0629, 0x062A, 0x062B, 0x062C, 0x062D, 0x062E, 0x062F, 0x0630, 0x0631, 0x0632, 0x0633, 0x0634, 0x0635, 0x0636, 0x00D7, 0x0637, 0x0638,
    0x0639, 0x063A, 0x0640, 0x0641, 0x0642, 0x0643, 0x00E0, 0x0644, 0x00E2, 0x0645, 0x0646, 0x0647, 0x0648, 0x00E7, 0x00E8, 0x00E9, 0x00EA, 0x00EB,
    0x0649, 0x064A, 0x00EE, 0x00EF, 0x064B, 0x064C, 0x064D, 0x064E, 0x00F4, 0x064F, 0x0650, 0x00F7, 0x0651, 0x00F9, 0x0652, 0x00FB, 0x00FC, 0x200E,
    0x200F, 0x06D2,
];

/// Evidence that bytes which aren't UTF-8 are Windows-1256 Arabic rather than Windows-1252.
#[derive(Default)]
struct Cp1256Evidence {
    /// High bytes that decode to different characters in the two code pages, apart from invisible
    /// marks and ordinals after a digit.
    differing: usize,
    /// Of those, Arabic letters and Arabic punctuation (، ؛ ؟) in Windows-1256.
    arabic: usize,
    /// Arabic bytes next to an ASCII letter: Western accents sit inside Latin words, Arabic doesn't.
    arabic_touching_latin: usize,
    /// Accented Latin bytes the two code pages share, next to another high byte: Hebrew
    /// Windows-1255 letters land there, Arabic and Western text rarely do.
    shared_letters_in_runs: usize,
}

impl Cp1256Evidence {
    fn of(bytes: &[u8]) -> Self {
        let mut e = Self::default();
        for (i, &b) in bytes.iter().enumerate() {
            if b < 0x80 {
                continue;
            }
            let before = i.checked_sub(1).and_then(|j| bytes.get(j)).copied();
            let after = bytes.get(i + 1).copied();
            let next_to = |pred: fn(u8) -> bool| before.is_some_and(pred) || after.is_some_and(pred);
            if same_in_cp1252_and_cp1256(b) {
                if is_shared_latin_letter(b) && next_to(|x| x >= 0x80) {
                    e.shared_letters_in_runs += 1;
                }
                continue;
            }
            if is_invisible_mark(b) || is_ordinal_after_digit(b, before) {
                continue;
            }
            e.differing += 1;
            if is_cp1256_arabic(b) {
                e.arabic += 1;
                if next_to(|x| x.is_ascii_alphabetic()) {
                    e.arabic_touching_latin += 1;
                }
            }
        }
        e
    }

    /// Thresholds chosen against month, day and language names from 95 glibc locales (Western in
    /// Windows-1252, Arabic, Persian and Urdu in 1256, Hebrew and Yiddish in 1255) plus CSV samples.
    fn is_arabic(&self) -> bool {
        self.arabic >= 2
            && self.arabic * 10 >= self.differing * 9
            && self.arabic_touching_latin * 10 <= self.arabic * 3
            && self.shared_letters_in_runs * 20 <= self.arabic
    }
}

fn same_in_cp1252_and_cp1256(b: u8) -> bool {
    matches!(
        b,
        0x80 | 0x82..=0x89 | 0x8B..=0x8C | 0x91..=0x97 | 0x99 | 0x9B..=0x9C | 0xA0 | 0xA2..=0xA9 | 0xAB..=0xB9 | 0xBB..=0xBE | 0xD7 | 0xE0 | 0xE2 | 0xE7..=0xEB
            | 0xEE..=0xEF | 0xF4 | 0xF7 | 0xF9 | 0xFB..=0xFC
    )
}

/// ZWNJ, ZWJ, LRM and RLM: common inside Persian words and around Arabic cells, but no evidence
/// either way.
fn is_invisible_mark(b: u8) -> bool {
    matches!(b, 0x9D | 0x9E | 0xFD | 0xFE)
}

/// `º` and `ª` after a digit (`1º`, `2ª`) are Western ordinals, though Windows-1256 reads the bytes
/// as `؛` and `ھ`; either way they are no evidence.
fn is_ordinal_after_digit(b: u8, before: Option<u8>) -> bool {
    matches!(b, 0xAA | 0xBA) && before.is_some_and(|x| x.is_ascii_digit())
}

fn is_shared_latin_letter(b: u8) -> bool {
    matches!(b, 0xE0 | 0xE2 | 0xE7..=0xEB | 0xEE..=0xEF | 0xF4 | 0xF9 | 0xFB..=0xFC)
}

fn is_cp1256_arabic(b: u8) -> bool {
    matches!(
        b,
        0x81 | 0x8A | 0x8D..=0x90 | 0x98 | 0x9A | 0x9F | 0xA1 | 0xAA | 0xBA | 0xBF..=0xD6 | 0xD8..=0xDB | 0xDD..=0xDF | 0xE1 | 0xE3..=0xE6 | 0xEC..=0xED | 0xFF
    )
}

/// Decodes the bytes of a text file with BOM-aware UTF-8 and UTF-16 handling. Without a BOM, valid UTF-8 is kept.
/// Other bytes decode as Windows-1256 when they read as Arabic (see [`Cp1256Evidence`]), else as
/// Windows-1252.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    let utf16 = |b: &[u8], le: bool| -> String {
        let units = b.as_chunks::<2>().0.iter().map(|c| if le { u16::from_le_bytes(*c) } else { u16::from_be_bytes(*c) });
        char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect()
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, false);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let use_windows_1256 = Cp1256Evidence::of(bytes).is_arabic();
            bytes
                .iter()
                .map(|&b| match b {
                    0x80..=0xFF if use_windows_1256 => {
                        char::from_u32(CP1256.get((b - 0x80) as usize).copied().unwrap_or(0xFFFD)).unwrap_or('\u{FFFD}')
                    }
                    0x80..=0x9F => char::from_u32(CP1252.get((b - 0x80) as usize).copied().unwrap_or(0xFFFD)).unwrap_or('\u{FFFD}'),
                    _ => b as char,
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings() {
        let mut b = vec![0xEF, 0xBB, 0xBF];
        b.extend_from_slice("é,x".as_bytes());
        assert_eq!(decode_text(&b), "é,x");
        let mut le = vec![0xFF, 0xFE];
        for u in "ü;1".encode_utf16() {
            le.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode_text(&le), "ü;1");
        let mut be = vec![0xFE, 0xFF];
        for u in "ß".encode_utf16() {
            be.extend_from_slice(&u.to_be_bytes());
        }
        assert_eq!(decode_text(&be), "ß");
        assert_eq!(decode_text(&[0x80, b'a', 0xE9]), "€aé");
        // Bytes that are no text at all still decode.
        assert_eq!(decode_text(&[0xFF; 1000]).chars().count(), 1000);
        assert_eq!(decode_text(&[0xFF, 0xFE, 0x00, 0xD8]), "\u{FFFD}", "a lone surrogate");
    }

    #[test]
    fn windows_1256_table_matches_the_unicode_mapping() {
        let at = |b: u8| CP1256.get(usize::from(b - 0x80)).and_then(|&c| char::from_u32(c));
        let expected = [
            (0x80, '€'),
            (0x81, 'پ'),
            (0x8D, 'چ'),
            (0x98, 'ک'),
            (0x9D, '\u{200C}'),
            (0xA1, '،'),
            (0xBA, '؛'),
            (0xBF, '؟'),
            (0xC7, 'ا'),
            (0xC8, 'ب'),
            (0xDC, 'ـ'),
            (0xE1, 'ل'),
            (0xE3, 'م'),
            (0xE9, 'é'),
            (0xFD, '\u{200E}'),
            (0xFE, '\u{200F}'),
            (0xFF, 'ے'),
        ];
        for (b, c) in expected {
            assert_eq!(at(b), Some(c), "0x{b:02X}");
        }
    }

    #[test]
    fn arabic_csv_in_windows_1256_reads_as_arabic() {
        let bytes = [
            0xC7, 0xE1, 0xC7, 0xD3, 0xE3, b',', 0xC7, 0xE1, 0xDE, 0xED, 0xE3, 0xC9, b'\n', 0xE3, 0xD1, 0xCD, 0xC8, 0xC7, b' ', b'1', b'2', b'3',
            0xA1, b' ', b'4', b'2', 0xBF, b',', 0xC8, 0xED, 0xC7, 0xE4, 0xC7, 0xCA, b' ', b'7', b'\n',
        ];
        assert_eq!(decode_text(&bytes), "الاسم,القيمة\nمرحبا 123، 42؟,بيانات 7\n");
    }

    fn reads_as_arabic(bytes: &[u8]) -> bool {
        decode_text(bytes).chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c))
    }

    #[test]
    fn western_1252_text_stays_latin() {
        let french = [
            b'C', b'r', 0xE8, b'm', b'e', b' ', b'b', b'r', 0xFB, b'l', 0xE9, b'e', b',', b' ', 0xE0, b' ', b'c', 0xF4, b't', 0xE9, b',', b' ', 0xE9,
            b't', 0xE9,
        ];
        let german = [
            b'G', b'r', 0xF6, 0xDF, b'e', b',', b' ', 0xDC, b'b', b'e', b'r', b'm', b'a', 0xDF, b',', b' ', b'F', b'u', 0xDF, b'g', 0xE4, b'n', b'g',
            b'e', b'r',
        ];
        assert_eq!(decode_text(&french), "Crème brûlée, à côté, été");
        assert_eq!(decode_text(&german), "Größe, Übermaß, Fußgänger");
        let portuguese = [
            b'A', 0xC7, 0xC3, b'O', b',', b' ', b'i', b'n', b'f', b'o', b'r', b'm', b'a', 0xE7, 0xE3, b'o', b',', b' ', b's', b'i', b't', b'u', b'a',
            0xE7, 0xE3, b'o', b',', b' ', b'P', 0xE3, b'o',
        ];
        assert_eq!(decode_text(&portuguese), "AÇÃO, informação, situação, Pão");
        let nordic = [
            0xC6, b'r', 0xF8, b',', 0xD8, b's', b't', b',', 0xC5, b's', b'e', b',', 0xDE, 0xF3, b'r', 0xF0, b'u', b'r', b',', b'G', b'u', 0xF0, b'r',
            0xFA, b'n', b',', 0xC4, 0xD6, 0xDC,
        ];
        assert_eq!(decode_text(&nordic), "Ærø,Øst,Åse,Þórður,Guðrún,ÄÖÜ");
        let ordinals = [b'1', 0xBA, b',', b'2', 0xBA, b',', b'3', 0xBA, b'\n', b'1', 0xAA, b',', b'2', 0xAA];
        assert_eq!(decode_text(&ordinals), "1º,2º,3º\n1ª,2ª");
        let inside_words = [b'K', 0xE4, b's', b'e', b',', b'B', 0xE4, b'c', b'k', b'e', b'r'];
        assert_eq!(decode_text(&inside_words), "Käse,Bäcker");
    }

    #[test]
    fn short_arabic_words_read_as_arabic() {
        assert_eq!(decode_text(&[0xDD, 0xED, b',', 0xE3, 0xE4, b',', 0xDA, 0xE4, b'\n']), "في,من,عن\n");
        assert_eq!(decode_text(&[0xE4, 0xDA, 0xE3, b',', 0xE1, 0xC7, b'\n', 0xE1, 0xC7, b',', 0xE4, 0xDA, 0xE3, b'\n']), "نعم,لا\nلا,نعم\n");
        assert_eq!(decode_text(&[0x98, 0xCA, 0xC7, 0xC8, b' ', 0x81, 0xC7, 0xD1, 0xD3, 0xED]), "کتاب پارسي");
        let with_marks = [
            0xDF, 0xCA, 0xC7, 0xC8, 0x9D, 0xE5, 0xC7, b',', 0xCE, 0xC7, 0xE4, 0xE5, 0x9D, 0xE5, 0xC7, b',', 0xE3, 0xCD, 0xE3, 0xCF, 0xFE, b',', 0xDA,
            0xE1, 0xED, 0xFE,
        ];
        assert_eq!(decode_text(&with_marks), "كتاب\u{200C}ها,خانه\u{200C}ها,محمد\u{200F},علي\u{200F}");
    }

    #[test]
    fn hebrew_1255_bytes_are_not_read_as_arabic() {
        let plain = [0xF9, 0xEC, 0xE5, 0xED, b',', 0xF2, 0xE5, 0xEC, 0xED];
        let pointed = [0xF9, 0xC8, 0xD1, 0xEC, 0xE5, 0xC9, 0xED, b' ', 0xF2, 0xE5, 0xC9, 0xEC, 0xC8, 0xED];
        let maqaf = [0xFA, 0xEC, 0xCE, 0xE0, 0xE1, 0xE9, 0xE1, b',', 0xE1, 0xEF, 0xCE, 0xE2, 0xE5, 0xF8, 0xE9, 0xE5, 0xEF];
        let gershayim = [0xF6, 0xE4, 0xD8, 0xEC, b',', 0xE3, 0xD8, 0xF8];
        for bytes in [&plain[..], &pointed, &maqaf, &gershayim] {
            assert!(!reads_as_arabic(bytes), "{:?}", decode_text(bytes));
        }
    }
}
