//! Reading numbers people type or paste, in any digits and with their locale's separators.

use std::fmt;

use crate::Format;

/// Why [`Format::parse`] rejected a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberError {
    /// No digits: empty, or only spaces and a sign.
    Empty,
    /// A character that isn't a digit, sign, decimal mark or grouping mark.
    Unexpected(char),
    /// Digits from more than one set (`1٢`).
    MixedDigits,
    /// A decimal or grouping mark where the locale puts none (`1.234,5` in English, `1,2.5` in
    /// German).
    Separator,
    /// Too large for an `f64`.
    TooLarge,
}

impl fmt::Display for NumberError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NumberError::Empty => write!(f, "no digits"),
            NumberError::Unexpected(c) => write!(f, "unexpected character {c:?}"),
            NumberError::MixedDigits => write!(f, "digits from more than one script"),
            NumberError::Separator => write!(f, "misplaced or ambiguous decimal or grouping mark"),
            NumberError::TooLarge => write!(f, "number too large"),
        }
    }
}

impl std::error::Error for NumberError {}

/// The marks a [`Format`] writes, as [`Format::parse`] reads them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Marks {
    /// `.` is the decimal mark and `,` grouping; else the other way round. For a format in
    /// Arabic-Indic digits, as the locale writes Latin ones.
    pub(crate) point: bool,
    /// The format's own decimal and grouping marks when they are neither `.` nor `,` (`٫`, `'`, a
    /// mark the system sets).
    decimal: Option<char>,
    group: Option<char>,
}

impl Default for Marks {
    fn default() -> Self {
        Marks { point: true, decimal: None, group: None }
    }
}

impl Marks {
    pub(crate) fn new(decimal: &str, group: &str, point: bool) -> Marks {
        let own = |s: &str| {
            let mut chars = s.chars();
            chars.next().filter(|c| !matches!(c, '.' | ',') && chars.next().is_none())
        };
        Marks { point, decimal: own(decimal), group: own(group) }
    }
}

impl Format {
    /// The number in `text`: what this format writes, and what people type or paste.
    ///
    /// - Digits: any Unicode decimal digits (ASCII, Arabic-Indic, Persian, Devanagari, Bengali,
    ///   Thai, full-width, …), all from one set.
    /// - Decimal mark: the format's, `٫` in every format, and the locale's `.` or `,` when Latin
    ///   digits are typed into an Arabic-Indic format. A lone `.` in a comma locale is the decimal
    ///   mark when it can't be grouping (`1.5`, `0.250`, `1234.5`, `.5`), but `1.234` and
    ///   `1.234.567` stay grouped; the same holds for `,` in a point locale.
    /// - Grouping: the format's, the other of `.` and `,`, spaces (NBSP, narrow NBSP, thin), `'`,
    ///   `’` and `٬`, one kind per number, in groups of three or the Indian 2-2-3 (`12,34,567`).
    ///   Never in the fraction.
    /// - Sign: a leading `-`, `−` (U+2212), full-width `－` or `+`.
    /// - Bidi marks and isolates (LRM, RLM, ALM, LRI…PDI) around or inside are ignored, so a
    ///   number formatted for an RTL interface reads back.
    ///
    /// Percent signs, units and the space before a unit (`50 %`, `12 px`) are not read; strip them
    /// first. Anything else is an error rather than a guess.
    pub fn parse(&self, text: &str) -> Result<f64, NumberError> {
        let marks = self.marks;
        let text: String = text.chars().filter(|c| !is_bidi_mark(*c)).collect();
        let mut rest = text.trim();
        let negative = match rest.chars().next() {
            Some(c @ ('-' | '−' | '－' | '+' | '＋')) => {
                rest = rest.get(c.len_utf8()..).unwrap_or("");
                matches!(c, '-' | '−' | '－')
            }
            _ => false,
        };
        let mut tokens = Vec::with_capacity(rest.len());
        let mut zero = None;
        for c in rest.chars() {
            let token = match c {
                c if Some(c) == marks.decimal => Token::Decimal,
                c if Some(c) == marks.group => Token::Group(c),
                '.' | '．' => Token::Point,
                ',' | '，' => Token::Comma,
                '٫' => Token::Decimal,
                '٬' => Token::Group('٬'),
                '\'' | '’' => Token::Group('\''),
                ' ' | '\u{a0}' | '\u{202f}' | '\u{2009}' => Token::Group(' '),
                _ => {
                    let (z, d) = digit(c).ok_or(NumberError::Unexpected(c))?;
                    if *zero.get_or_insert(z) != z {
                        return Err(NumberError::MixedDigits);
                    }
                    Token::Digit(d)
                }
            };
            tokens.push(token);
        }
        if zero.is_none() {
            return Err(if tokens.is_empty() { NumberError::Empty } else { NumberError::Separator });
        }
        let point = if lone_decimal(&tokens, marks.point) { !marks.point } else { marks.point };
        let digits = read(&tokens, point).ok_or(NumberError::Separator)?;
        let value: f64 = digits.parse().map_err(|_| NumberError::Separator)?;
        if !value.is_finite() {
            return Err(NumberError::TooLarge);
        }
        Ok(if negative { -value } else { value })
    }
}

/// The number in `text` as `locale` (a language tag, `de-CH`, `ar_EG.UTF-8`) writes it:
/// [`Format::parse`] for [`Format::from_tag`]. Build the [`Format`] once to read many numbers. A
/// locale CLDR doesn't know reads as CLDR's root does (`1,234.5`).
pub fn parse_number(text: &str, locale: &str) -> Result<f64, NumberError> {
    Format::from_tag(locale).parse(text)
}

#[derive(Clone, Copy, PartialEq)]
enum Token {
    Digit(u8),
    Point,
    Comma,
    /// A decimal mark in every reading: `٫`, or the format's own.
    Decimal,
    /// A grouping mark in every reading, by kind (space, apostrophe, `٬`, the format's own).
    Group(char),
}

/// Whether the locale's grouping mark of `.` and `,` (`.` when `point` is false) is the decimal
/// mark here because it can't be grouping: it is the only mark of the two, there is one of it, and
/// the digits before it are none, start with 0 or are more than three, or the digits after it
/// aren't three.
fn lone_decimal(tokens: &[Token], point: bool) -> bool {
    let (group, decimal) = if point { (Token::Comma, Token::Point) } else { (Token::Point, Token::Comma) };
    if tokens.contains(&decimal) || tokens.contains(&Token::Decimal) || tokens.iter().filter(|t| **t == group).count() != 1 {
        return false;
    }
    let Some(at) = tokens.iter().position(|t| *t == group) else { return false };
    let (int, frac) = (tokens.get(..at).unwrap_or_default(), tokens.get(at + 1..).unwrap_or_default());
    int.is_empty() || int.first() == Some(&Token::Digit(0)) || int.len() > 3 || frac.len() != 3
}

/// The ASCII number `tokens` spell when the decimal mark is `.` (`point`) or `,`; `None` when the
/// marks don't fit that reading.
fn read(tokens: &[Token], point: bool) -> Option<String> {
    let tokens: Vec<Token> = tokens
        .iter()
        .map(|t| match t {
            Token::Point if point => Token::Decimal,
            Token::Point => Token::Group('.'),
            Token::Comma if point => Token::Group(','),
            Token::Comma => Token::Decimal,
            t => *t,
        })
        .collect();
    let (int, frac) = match tokens.iter().position(|t| *t == Token::Decimal) {
        Some(i) => (tokens.get(..i)?, tokens.get(i + 1..)?),
        None => (&tokens[..], &[][..]),
    };
    let ascii =
        |part: &[Token]| -> Option<String> { part.iter().map(|t| if let Token::Digit(d) = t { Some(char::from(b'0' + d)) } else { None }).collect() };
    let frac = ascii(frac)?;
    let mut group = None;
    let mut groups = Vec::new();
    for chunk in int.split(|t| matches!(t, Token::Group(_))) {
        groups.push(ascii(chunk)?);
    }
    for t in int {
        if let Token::Group(g) = t
            && *group.get_or_insert(*g) != *g
        {
            return None;
        }
    }
    if let [first, middle @ .., last] = groups.as_slice()
        && group.is_some()
    {
        let width = middle.first().map_or(3, String::len);
        let first_max = if width == 2 { 2 } else { 3 };
        let fits = (1..=first_max).contains(&first.len())
            && !first.starts_with('0')
            && last.len() == 3
            && middle.iter().all(|g| g.len() == width)
            && (2..=3).contains(&width);
        if !fits {
            return None;
        }
    }
    let int = groups.concat();
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    Some(format!("{}.{}", if int.is_empty() { "0" } else { &int }, if frac.is_empty() { "0" } else { &frac }))
}

/// The first code point of each run of ten Unicode decimal digits (general category Nd), Unicode
/// 16.0.
const ZEROS: [u32; 76] = [
    0x30, 0x660, 0x6F0, 0x7C0, 0x966, 0x9E6, 0xA66, 0xAE6, 0xB66, 0xBE6, 0xC66, 0xCE6, 0xD66, 0xDE6, 0xE50, 0xED0, 0xF20, 0x1040, 0x1090, 0x17E0,
    0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90, 0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0, 0xFF10, 0x104A0,
    0x10D30, 0x10D40, 0x11066, 0x110F0, 0x11136, 0x111D0, 0x112F0, 0x11450, 0x114D0, 0x11650, 0x116C0, 0x116D0, 0x116DA, 0x11730, 0x118E0, 0x11950,
    0x11BF0, 0x11C50, 0x11D50, 0x11DA0, 0x11F50, 0x16130, 0x16A60, 0x16AC0, 0x16B50, 0x16D70, 0x1CCF0, 0x1D7CE, 0x1D7D8, 0x1D7E2, 0x1D7EC, 0x1D7F6,
    0x1E140, 0x1E2F0, 0x1E4F0, 0x1E5F1, 0x1E950, 0x1FBF0,
];

/// `c`'s digit set (its zero) and value.
fn digit(c: char) -> Option<(u32, u8)> {
    let c = u32::from(c);
    let zero = match ZEROS.binary_search(&c) {
        Ok(i) => ZEROS.get(i),
        Err(i) => ZEROS.get(i.checked_sub(1)?),
    }?;
    let value = c - zero;
    u8::try_from(value).ok().filter(|v| *v < 10).map(|v| (*zero, v))
}

/// Marks bidi formatting adds around numbers: LRM, RLM, ALM, embeddings, overrides and isolates.
fn is_bidi_mark(c: char) -> bool {
    matches!(c, '\u{200e}' | '\u{200f}' | '\u{061c}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Digits, Region};

    #[test]
    fn every_digit_set() {
        for zero in ZEROS {
            let d = |n: u32| char::from_u32(zero + n).unwrap();
            let all: String = (0..10).rev().map(d).collect();
            assert_eq!(parse_number(&all, "en"), Ok(9_876_543_210.0), "{zero:X}");
            let grouped = format!("-{}{}{},{}{}{}.{}", d(1), d(2), d(3), d(4), d(5), d(6), d(7));
            assert_eq!(parse_number(&grouped, "en"), Ok(-123_456.7), "{zero:X}");
            assert!((0..10).all(|n| d(n).is_numeric()), "{zero:X}");
        }
        assert_eq!(digit('\u{1D800}'), None);
        assert_eq!(digit('/'), None);
    }

    #[test]
    fn locale_forms() {
        for (text, locale, want) in [
            ("1,234.5", "en", 1234.5),
            ("1.234,5", "de", 1234.5),
            ("1 234,5", "fr", 1234.5),
            ("1\u{202f}234,5", "fr", 1234.5),
            ("1\u{a0}234,5", "ru", 1234.5),
            ("1’234.5", "de-CH", 1234.5),
            ("1'234.5", "de_CH.UTF-8", 1234.5),
            ("1,234", "en", 1234.0),
            ("1,234", "de", 1.234),
            ("1.234", "pt-br", 1234.0),
            ("1,5", "es", 1.5),
            ("1.5", "es-419", 1.5),
            ("١٬٢٣٤٫٥", "ar", 1234.5),
            ("١٢٫٥", "ar", 12.5),
            ("12.5", "ar", 12.5),
            ("12,5", "ar-MA", 12.5),
            ("۱۲٫۵", "fa", 12.5),
            ("−۴۲", "fa", -42.0),
            ("１２３，４５６．７", "ja", 123_456.7),
            ("－５", "ja", -5.0),
            ("१२,३४,५६७.८९", "hi", 1_234_567.89),
            ("১২,৩৪,৫৬৭", "bn", 1_234_567.0),
            ("౧,౨౩,౪౫౬", "te", 123_456.0),
            ("๑,๒๓๔.๕", "th", 1234.5),
            ("1,234,567", "en", 1_234_567.0),
            ("−3,5", "sv", -3.5),
            ("+7", "en", 7.0),
            (".5", "en", 0.5),
            ("5.", "en", 5.0),
            ("  42  ", "en", 42.0),
            ("\u{2066}-1,5\u{2069}", "de", -1.5),
            ("\u{061c}-١٢", "ar", -12.0),
            ("1,234.5", "xx", 1234.5),
            ("12", "xx", 12.0),
            ("1,5", "en-PL", 1.5),
            ("1.234,5", "en-DE", 1234.5),
            ("1.234,5", "lo", 1234.5),
            ("1,234.5", "km", 1234.5),
        ] {
            assert_eq!(parse_number(text, locale), Ok(want), "{text:?} in {locale}");
        }
    }

    #[test]
    fn errors_instead_of_guesses() {
        for (text, locale, want) in [
            ("", "en", NumberError::Empty),
            ("  ", "en", NumberError::Empty),
            ("-", "en", NumberError::Empty),
            (",", "en", NumberError::Separator),
            ("1.2.3", "en", NumberError::Separator),
            ("1,,234", "en", NumberError::Separator),
            ("1,234,", "en", NumberError::Separator),
            ("1.234,5", "en", NumberError::Separator),
            ("1 234'567", "en", NumberError::Separator),
            ("1.234 567", "de", NumberError::Separator),
            ("0.123,4", "de", NumberError::Separator),
            ("1.5,2", "en", NumberError::Separator),
            ("123,45,678", "hi", NumberError::Separator),
            ("12٫3٫4", "ar", NumberError::Separator),
            ("1٫5.2", "ar", NumberError::Separator),
            ("1٢", "ar", NumberError::MixedDigits),
            ("12abc", "en", NumberError::Unexpected('a')),
            ("1e5", "en", NumberError::Unexpected('e')),
            ("--5", "en", NumberError::Unexpected('-')),
            ("5-", "en", NumberError::Unexpected('-')),
            ("- 5", "en", NumberError::Separator),
        ] {
            assert_eq!(parse_number(text, locale), Err(want), "{text:?} in {locale}");
        }
        assert_eq!(parse_number(&"9".repeat(400), "en"), Err(NumberError::TooLarge));
        for unit in ["50 %", "50%", "12 px", "1,5 MB"] {
            assert!(parse_number(unit, "de").is_err(), "{unit}: units are the caller's to strip");
        }
    }

    /// A lone mark that can't be grouping is the decimal mark; one that can stays grouping.
    #[test]
    fn lone_marks() {
        for (text, locale, want) in [
            ("1.5", "de", 1.5),
            ("0.250", "de", 0.25),
            ("1234.5", "de", 1234.5),
            (".5", "de", 0.5),
            ("-12.34", "ru", -12.34),
            ("1.234", "de", 1234.0),
            ("1.234.567", "de", 1_234_567.0),
            ("1,5", "en", 1.5),
            ("0,5", "en", 0.5),
            ("12,34", "en", 12.34),
            ("1234,567", "en", 1234.567),
            ("1,234", "en", 1234.0),
            ("12,345", "en", 12345.0),
        ] {
            assert_eq!(parse_number(text, locale), Ok(want), "{text:?} in {locale}");
        }
    }

    /// What a format writes reads back, in its digits and with its separators, the system's
    /// included.
    #[test]
    fn reads_what_format_writes() {
        let mut formats: Vec<Format> = ["en", "de", "de-CH", "fr", "hi", "ar-EG", "fa", "bn-BD", "ar-MA", "en-PL", "nb", "pt-PT", "sv"]
            .iter()
            .flat_map(|t| [Format::from_tag(t), Format::new(&Region::from_tag(t), Digits::Latin)])
            .collect();
        for (tag, decimal, grouping) in [
            ("en-US", Some(','), None),
            ("en-US", Some('.'), Some(Some('\u{a0}'))),
            ("en-US", None, Some(None)),
            ("en-US", Some('·'), Some(Some('_'))),
            ("de-DE", Some('·'), None),
        ] {
            let mut region = Region::from_tag(tag);
            region.decimal = decimal;
            region.grouping = grouping;
            formats.push(Format::new(&region, Digits::Locale));
        }
        // Windows sets both marks: a decimal comma the user picks arrives beside a grouping comma.
        let reg = |pairs: &'static [(&'static str, &'static str)]| move |n: &str| pairs.iter().find(|(k, _)| *k == n).map(|(_, v)| v.to_string());
        let windows = crate::region::from_windows(reg(&[("LocaleName", "en-US"), ("sDecimal", ","), ("sThousand", ",")]), || None);
        formats.push(Format::new(&windows, Digits::Locale));
        formats.push(Format::new(&Region::from_tag("ar-EG-u-nu-latn"), Digits::Locale));
        formats.push(Format::new(&Region::from_tag("C"), Digits::Locale));
        for f in &formats {
            for value in [0.0, 1.5, -1234.25, 1_234_567.89, 12.0, 1234.0] {
                let text = f.number(value, 2);
                assert_eq!(f.parse(&text), Ok(value), "{text:?} ({f:?})");
            }
            let text = f.integer(1_234_567);
            assert_eq!(f.parse(&text), Ok(1_234_567.0), "{text:?}");
        }
    }
}
