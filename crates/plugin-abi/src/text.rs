//! Plugin text rules. Any string a plugin writes and the host shows to the user goes
//! through here: menu labels, plugin names, notices, veto reasons, panel text, log lines.
//!
//! Two threats. A control character can forge a log line or move the terminal cursor. A bidi or
//! zero-width character can make "Acme Tools" look like another name, or hide text.

/// Longest notice or veto reason the host shows, in characters.
pub const MAX_NOTICE_CHARS: usize = 500;
/// Longest log line the host writes, in characters.
pub const MAX_LOG_CHARS: usize = 1024;

/// True for a character the host never shows from a plugin: C0 and C1 controls, DEL, bidi
/// controls (U+061C, U+200E, U+200F, U+202A to U+202E, U+2066 to U+2069), zero-width characters
/// (U+200B to U+200D, U+2060, U+FEFF), and the line and paragraph separators (U+2028, U+2029).
/// Newline and tab are controls too. Use [`has_unsafe_multiline`] to keep them.
pub fn is_unsafe_char(c: char) -> bool {
    c.is_control() || is_hidden_char(c)
}

/// The non-control part of [`is_unsafe_char`]: bidi, zero-width, and separator characters.
pub fn is_hidden_char(c: char) -> bool {
    matches!(c,
        '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// True when `s` has a character [`is_unsafe_char`] names. For single-line text such as a name,
/// a label or a menu entry.
pub fn has_unsafe(s: &str) -> bool {
    s.chars().any(is_unsafe_char)
}

/// True when `s` has a control other than newline and tab, or a hidden character. For multi-line
/// text such as a description or a panel text box.
pub fn has_unsafe_multiline(s: &str) -> bool {
    s.chars().any(|c| c != '\n' && c != '\t' && is_unsafe_char(c))
}

/// One safe line: hidden characters removed, every control (newline and tab included) turned into
/// one space, at most `max_chars` characters, and `...` at the end when text was cut.
pub fn clean_line(s: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_chars.saturating_add(3)));
    let mut count = 0usize;
    for c in s.chars() {
        let c = if is_hidden_char(c) {
            continue;
        } else if c.is_control() {
            ' '
        } else {
            c
        };
        if count == max_chars {
            out.push_str("...");
            return out;
        }
        out.push(c);
        count += 1;
    }
    out
}

/// A notice or veto reason as the user sees it: one safe line of at most [`MAX_NOTICE_CHARS`].
pub fn clean_notice(s: &str) -> String {
    clean_line(s, MAX_NOTICE_CHARS)
}

/// A log line as the host writes it: one safe line of at most [`MAX_LOG_CHARS`].
pub fn clean_log(s: &str) -> String {
    clean_line(s, MAX_LOG_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_and_bidi_are_found() {
        for bad in ["a\u{1b}[2J", "a\nb", "\u{202E}gpj.exe", "a\u{200B}b", "x\u{2066}y", "\u{7f}", "\u{85}", "a\u{2028}b", "\u{FEFF}a"] {
            assert!(has_unsafe(bad), "{bad:?}");
        }
        assert!(!has_unsafe("Acme Tools 2.0, \u{e9}\u{4e2d}\u{6587} \u{1f600}"));
        assert!(!has_unsafe_multiline("two\nlines\tand tab"));
        assert!(has_unsafe_multiline("two\r\nlines"));
        assert!(has_unsafe_multiline("a\u{202E}b"));
    }

    #[test]
    fn clean_line_makes_one_bounded_safe_line() {
        assert_eq!(clean_line("ok\n[INFO] fake\u{1b}[31m", 100), "ok [INFO] fake [31m");
        assert_eq!(clean_line("a\u{202E}b\u{200B}c", 10), "abc");
        assert_eq!(clean_line("abcdef", 3), "abc...");
        assert_eq!(clean_line("abc", 3), "abc");
        // Counts characters, never splits one.
        assert_eq!(clean_line("\u{e9}\u{e9}\u{e9}\u{e9}", 2), "\u{e9}\u{e9}...");
        assert_eq!(clean_notice(&"x".repeat(MAX_NOTICE_CHARS * 10)).chars().count(), MAX_NOTICE_CHARS + 3);
    }
}
