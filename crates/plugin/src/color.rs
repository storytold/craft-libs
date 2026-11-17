//! Hex colours as plug-ins write them (panel colour fields, widget fills): `#` optional, then 1, 2,
//! 3, 4, 6 or 8 hex digits (`F` is `#FFFFFF`, `AB` is `#ABABAB`, `ABC` is `#AABBCC`, `ABCD` is
//! `#AABBCCDD`). Normalised to `#RRGGBB`, or `#RRGGBBAA` when not opaque, in upper case.

/// Longest text read as a colour (`#RRGGBBAA`); longer input is refused before it is looked at.
pub const MAX_HEX_COLOUR: usize = 9;

/// `s` as a normalised hex colour (`#RRGGBB` or `#RRGGBBAA`), or `None` when it isn't one.
pub fn normalize_hex(s: &str) -> Option<String> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() > 8 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let d: Vec<u8> = s.chars().filter_map(|c| c.to_digit(16)).filter_map(|d| u8::try_from(d).ok()).collect();
    let byte = |hi: u8, lo: u8| hi * 16 + lo;
    let (r, g, b, a) = match d.as_slice() {
        [x] => (byte(*x, *x), byte(*x, *x), byte(*x, *x), 255),
        [x, y] => (byte(*x, *y), byte(*x, *y), byte(*x, *y), 255),
        [r, g, b] => (byte(*r, *r), byte(*g, *g), byte(*b, *b), 255),
        [r, g, b, a] => (byte(*r, *r), byte(*g, *g), byte(*b, *b), byte(*a, *a)),
        [r1, r2, g1, g2, b1, b2] => (byte(*r1, *r2), byte(*g1, *g2), byte(*b1, *b2), 255),
        [r1, r2, g1, g2, b1, b2, a1, a2] => (byte(*r1, *r2), byte(*g1, *g2), byte(*b1, *b2), byte(*a1, *a2)),
        _ => return None,
    };
    Some(if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{r:02X}{g:02X}{b:02X}{a:02X}") })
}

#[cfg(test)]
mod tests {
    use super::normalize_hex;

    #[test]
    fn reads_every_hex_form() {
        for (input, want) in [
            ("F", "#FFFFFF"),
            ("#ab", "#ABABAB"),
            ("abc", "#AABBCC"),
            ("#abcd", "#AABBCCDD"),
            (" #ff5a5f ", "#FF5A5F"),
            ("#FF5A5FFF", "#FF5A5F"),
            ("#00000080", "#00000080"),
        ] {
            assert_eq!(normalize_hex(input).as_deref(), Some(want), "{input}");
        }
    }

    #[test]
    fn refuses_what_isnt_hex() {
        for input in ["", "#", "#12345", "#1234567", "#123456789", "red", "#GG0000", "#１２３", "+12"] {
            assert_eq!(normalize_hex(input), None, "{input}");
        }
    }
}
