//! MTEXT inline-escape handling independent of drawing and text layout.
//!
//! Unicode and MIF escapes are decoded to literal characters, with markup-significant
//! characters re-escaped so subsequent MTEXT parsers cannot treat them as commands.

/// Whether an inline command takes an argument terminated by `;`.
pub fn takes_arg(code: char) -> bool {
    matches!(code, 'f' | 'F' | 'H' | 'W' | 'Q' | 'T' | 'A' | 'C' | 'c' | 'p')
}

/// Four hex digits at `chars[at..at + 4]`.
fn hex4(chars: &[char], at: usize) -> Option<u32> {
    let digits = chars.get(at..at.checked_add(4)?)?;
    digits.iter().try_fold(0u32, |acc, c| Some(acc * 16 + c.to_digit(16)?))
}

/// Push a decoded character so that the tokenizers read it as text, never as markup.
fn push_literal(out: &mut String, c: char) {
    if matches!(c, '\\' | '{' | '}') {
        out.push('\\');
    }
    out.push(c);
}

/// Decode `\U+hhhh` (one UTF-16 unit; a surrogate pair gives one character, a lone surrogate
/// U+FFFD) and `\M+nhhhh` (double-byte East Asian characters) to the characters they stand
/// for. Malformed escapes remain unchanged for the caller to interpret.
/// An escaped backslash (`\\U+0041`) is not an escape.
pub fn unescape_codes(s: &str) -> std::borrow::Cow<'_, str> {
    if !s.contains("\\U+") && !s.contains("\\M+") {
        return std::borrow::Cow::Borrowed(s);
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if c != '\\' {
            out.push(c);
            i += 1;
            continue;
        }
        let (n, plus) = (chars.get(i + 1).copied(), chars.get(i + 2).copied());
        if n == Some('U') && plus == Some('+') {
            if let Some(unit) = hex4(&chars, i + 3) {
                i += 7;
                let low = |at: usize| {
                    (chars.get(at) == Some(&'\\') && chars.get(at + 1) == Some(&'U') && chars.get(at + 2) == Some(&'+'))
                        .then(|| hex4(&chars, at + 3))
                        .flatten()
                        .filter(|u| (0xDC00..0xE000).contains(u))
                };
                let ch = if (0xD800..0xDC00).contains(&unit) {
                    match low(i) {
                        Some(lo) => {
                            i += 7;
                            char::from_u32(0x10000 + ((unit - 0xD800) << 10) + (lo - 0xDC00))
                        }
                        None => None,
                    }
                } else {
                    char::from_u32(unit)
                };
                push_literal(&mut out, ch.unwrap_or('\u{fffd}'));
                continue;
            }
        } else if n == Some('M') && plus == Some('+') && chars.get(i + 3).is_some_and(char::is_ascii_digit) && hex4(&chars, i + 4).is_some() {
            push_literal(&mut out, crate::mif_escape_at(&chars, i).unwrap_or('\u{fffd}'));
            i += 8;
            continue;
        }
        // Any other backslash pair stays as it is (and an escaped backslash stays one).
        out.push('\\');
        i += 1;
        if let Some(&next) = chars.get(i) {
            out.push(next);
            i += 1;
        }
    }
    std::borrow::Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_and_mif_escapes_remain_literal_markup() {
        assert_eq!(unescape_codes(r"a\U+0041\U+D83D\U+DE00"), "aA😀");
        assert_eq!(unescape_codes(r"\M+5BCFE"), "件");
        assert_eq!(unescape_codes(r"\U+005C\U+007B\U+007D"), r"\\\{\}");
        assert_eq!(unescape_codes(r"\\U+0041"), r"\\U+0041");
        assert_eq!(unescape_codes(r"\U+12"), r"\U+12");
        assert_eq!(unescape_codes(r"\U+D800"), "�");
    }
    #[test]
    fn parameterized_commands_are_classified_without_relaxing_others() {
        for c in "fFHWQTACcp".chars() {
            assert!(takes_arg(c));
        }
        for c in ['S', 'P', 'Z', '\\'] {
            assert!(!takes_arg(c));
        }
    }
}
