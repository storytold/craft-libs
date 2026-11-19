//! Content lines (RFC 5545 §3.1–3.2): unfolding on the way in, folding on the way out, and the
//! `NAME;PARAM=value;PARAM="quoted":value` shape in between.

/// Longest logical line kept, in bytes; the rest of a longer line is dropped.
pub const MAX_LINE: usize = 1 << 20;
/// Most parameters read from one line, and most values of one parameter.
const MAX_PARAMS: usize = 64;
/// Octets per physical line when writing.
const FOLD_AT: usize = 75;

/// The longest prefix of `s` of at most `max` bytes that ends on a char boundary.
pub fn prefix(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("")
}

/// The logical (unfolded) lines of a text. Lines end at CRLF, LF or CR; a line that starts with a
/// space or a tab continues the one before.
pub struct Lines<'a> {
    rest: &'a str,
}

impl<'a> Lines<'a> {
    pub fn new(text: &'a str) -> Lines<'a> {
        Lines { rest: text.strip_prefix('\u{feff}').unwrap_or(text) }
    }

    /// The next physical line, without its line break.
    fn physical(&mut self) -> Option<&'a str> {
        if self.rest.is_empty() {
            return None;
        }
        let bytes = self.rest.as_bytes();
        let end = bytes.iter().position(|b| matches!(b, b'\r' | b'\n')).unwrap_or(bytes.len());
        let skip = match (bytes.get(end), bytes.get(end.saturating_add(1))) {
            (Some(b'\r'), Some(b'\n')) => 2,
            (Some(_), _) => 1,
            (None, _) => 0,
        };
        let line = self.rest.get(..end).unwrap_or("");
        self.rest = self.rest.get(end.saturating_add(skip)..).unwrap_or("");
        Some(line)
    }
}

impl Iterator for Lines<'_> {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        let mut line = String::new();
        push_capped(&mut line, self.physical()?);
        while self.rest.starts_with([' ', '\t']) {
            let Some(more) = self.physical() else { break };
            push_capped(&mut line, more.get(1..).unwrap_or(""));
        }
        Some(line)
    }
}

fn push_capped(line: &mut String, more: &str) {
    line.push_str(prefix(more, MAX_LINE.saturating_sub(line.len())));
}

/// One property parameter: `CN="Ada Lovelace"`, `ROLE=OPT-PARTICIPANT`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    /// Upper-cased.
    pub name: String,
    pub values: Vec<String>,
}

/// One property: `ATTENDEE;CN=Ada:mailto:ada@example.org`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentLine {
    /// Upper-cased.
    pub name: String,
    pub params: Vec<Param>,
    /// As written: still escaped.
    pub value: String,
}

impl ContentLine {
    /// Parse an unfolded line; `None` for a line without a name or without a `:`.
    pub fn parse(line: &str) -> Option<ContentLine> {
        // Every delimiter is ASCII, so the byte positions found here are char boundaries.
        let bytes = line.as_bytes();
        let stop = |from: usize, at: &[u8]| {
            bytes.get(from..).and_then(|b| b.iter().position(|c| at.contains(c))).map_or(bytes.len(), |i| from.saturating_add(i))
        };
        let mut pos = stop(0, b";:");
        let name = line.get(..pos)?.trim();
        if name.is_empty() {
            return None;
        }
        let mut params = Vec::new();
        while bytes.get(pos) == Some(&b';') {
            let start = pos.saturating_add(1);
            pos = stop(start, b"=;:");
            let param = line.get(start..pos)?.trim().to_ascii_uppercase();
            let mut values = Vec::new();
            while matches!(bytes.get(pos), Some(b'=' | b',')) {
                let from = pos.saturating_add(1);
                let value = if bytes.get(from) == Some(&b'"') {
                    let open = from.saturating_add(1);
                    let close = stop(open, b"\"");
                    // A quote that never closes: the line has no value.
                    bytes.get(close)?;
                    pos = stop(close, b",;:");
                    line.get(open..close)?
                } else {
                    pos = stop(from, b",;:");
                    line.get(from..pos)?
                };
                if values.len() < MAX_PARAMS {
                    values.push(caret_decode(value));
                }
            }
            if !param.is_empty() && params.len() < MAX_PARAMS {
                params.push(Param { name: param, values });
            }
        }
        if bytes.get(pos) != Some(&b':') {
            return None;
        }
        Some(ContentLine { name: name.to_ascii_uppercase(), params, value: line.get(pos.saturating_add(1)..)?.to_string() })
    }

    /// The first value of the parameter `name` (upper case).
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|p| p.name == name).and_then(|p| p.values.first()).map(String::as_str)
    }
}

/// Undo RFC 6868: `^n` is a line break, `^'` a double quote, `^^` a caret.
fn caret_decode(s: &str) -> String {
    if !s.contains('^') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        let decoded = match (c, chars.peek()) {
            ('^', Some('n')) => Some('\n'),
            ('^', Some('\'')) => Some('"'),
            ('^', Some('^')) => Some('^'),
            _ => None,
        };
        match decoded {
            Some(d) => {
                out.push(d);
                chars.next();
            }
            None => out.push(c),
        }
    }
    out
}

/// A parameter value ready to write: RFC 6868 escapes for what a parameter can't hold, other
/// control characters dropped, and quotes around it when it needs them.
pub fn param_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len().saturating_add(2));
    for c in s.chars() {
        match c {
            '^' => out.push_str("^^"),
            '"' => out.push_str("^'"),
            '\n' => out.push_str("^n"),
            '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    if out.contains([';', ':', ',']) || out.starts_with(' ') || out.ends_with(' ') { format!("\"{out}\"") } else { out }
}

/// Append `line` to `out` with CRLF, folded so no physical line is longer than 75 octets and no
/// character is split.
pub fn fold(out: &mut String, line: &str) {
    let mut rest = line;
    let mut room = FOLD_AT;
    while rest.len() > room {
        let head = prefix(rest, room);
        if head.is_empty() {
            break;
        }
        out.push_str(head);
        out.push_str("\r\n ");
        rest = rest.get(head.len()..).unwrap_or("");
        // The leading space counts.
        room = FOLD_AT - 1;
    }
    out.push_str(rest);
    out.push_str("\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        Lines::new(text).collect()
    }

    #[test]
    fn unfolds_every_kind_of_line_break() {
        assert_eq!(lines("A:1\r\nB:2\nC:3\rD:4"), ["A:1", "B:2", "C:3", "D:4"]);
        assert_eq!(lines("SUMMARY:Lunch \r\n with Ada\r\n"), ["SUMMARY:Lunch with Ada"]);
        assert_eq!(lines("SUMMARY:Lun\n\tch\n  twice\n"), ["SUMMARY:Lunch twice"]);
        // Only the one space is the fold.
        assert_eq!(lines("X:a\r\n  b"), ["X:a b"]);
        assert_eq!(lines("X:a \r\nY:b"), ["X:a ", "Y:b"]);
        assert!(lines("").is_empty());
    }

    #[test]
    fn a_byte_order_mark_and_blank_lines_are_harmless() {
        assert_eq!(lines("\u{feff}BEGIN:VCALENDAR\r\n\r\nEND:VCALENDAR"), ["BEGIN:VCALENDAR", "", "END:VCALENDAR"]);
        assert_eq!(ContentLine::parse(""), None);
    }

    #[test]
    fn parses_names_parameters_and_values() {
        let l = ContentLine::parse("dtstart;tzid=Example/Riverton;Value=DATE-TIME:20261012T090000").unwrap();
        assert_eq!(l.name, "DTSTART");
        assert_eq!(l.param("TZID"), Some("Example/Riverton"));
        assert_eq!(l.param("VALUE"), Some("DATE-TIME"));
        assert_eq!(l.param("CN"), None);
        assert_eq!(l.value, "20261012T090000");
        let plain = ContentLine::parse("SUMMARY:Budget: phase two; hiring").unwrap();
        assert_eq!((plain.name.as_str(), plain.params.len(), plain.value.as_str()), ("SUMMARY", 0, "Budget: phase two; hiring"));
        assert_eq!(ContentLine::parse("DESCRIPTION:").unwrap().value, "");
    }

    #[test]
    fn quoted_parameters_hide_delimiters() {
        let l = ContentLine::parse("ATTENDEE;CN=\"Okafor, Nadia: lead; planning\";ROLE=REQ-PARTICIPANT:mailto:nadia@example.org").unwrap();
        assert_eq!(l.param("CN"), Some("Okafor, Nadia: lead; planning"));
        assert_eq!(l.param("ROLE"), Some("REQ-PARTICIPANT"));
        assert_eq!(l.value, "mailto:nadia@example.org");
        let several = ContentLine::parse("X-TEST;MEMBER=\"mailto:a@example.org\",\"mailto:b@example.org\",c:v").unwrap();
        assert_eq!(several.params[0].values, ["mailto:a@example.org", "mailto:b@example.org", "c"]);
        // A parameter without a value, and an empty one.
        let bare = ContentLine::parse("X-TEST;FLAG;EMPTY=:v").unwrap();
        assert_eq!(bare.params.len(), 2);
        assert_eq!(bare.param("FLAG"), None);
        assert_eq!(bare.param("EMPTY"), Some(""));
    }

    #[test]
    fn broken_lines_are_refused() {
        for bad in ["no colon here", ":value without a name", ";CN=x:y", "ATTENDEE;CN=\"never closed:mailto:a@example.org", "ATTENDEE;CN=x", "   "] {
            assert_eq!(ContentLine::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn parameter_values_escape_and_quote() {
        assert_eq!(param_value("Ada Lovelace"), "Ada Lovelace");
        assert_eq!(param_value("Lovelace, Ada"), "\"Lovelace, Ada\"");
        assert_eq!(param_value("Ada \"the Countess\" ^1\nLine\r\u{7}"), "Ada ^'the Countess^' ^^1^nLine");
        for name in ["Ada", "Lovelace, Ada", "a\"b^c\nd", "x:y;z", "^n^'^^", "Zoë — 東京"] {
            let l = ContentLine::parse(&format!("ORGANIZER;CN={}:mailto:a@example.org", param_value(name))).unwrap();
            assert_eq!(l.param("CN"), Some(name));
        }
    }

    #[test]
    fn folds_at_75_octets_without_splitting_characters() {
        let line = format!("SUMMARY:{}", "é東🎉x".repeat(40));
        let mut out = String::new();
        fold(&mut out, &line);
        assert!(out.ends_with("\r\n"));
        let physical: Vec<&str> = out.trim_end_matches("\r\n").split("\r\n").collect();
        assert!(physical.len() > 3);
        assert!(physical.iter().all(|l| l.len() <= 75), "{physical:?}");
        assert!(physical.iter().skip(1).all(|l| l.starts_with(' ')));
        assert_eq!(lines(&out), [line]);
        // A short line is left alone, and an exact fit isn't folded.
        let mut short = String::new();
        fold(&mut short, &"x".repeat(75));
        assert_eq!(short, format!("{}\r\n", "x".repeat(75)));
        let mut one_more = String::new();
        fold(&mut one_more, &"x".repeat(76));
        assert_eq!(one_more, format!("{}\r\n x\r\n", "x".repeat(75)));
    }

    #[test]
    fn enormous_lines_are_capped() {
        let text = format!("SUMMARY:{}\r\n {}\r\nUID:after", "é".repeat(MAX_LINE), "x".repeat(1000));
        let got = lines(&text);
        assert_eq!(got.len(), 2);
        assert!(got[0].len() <= MAX_LINE);
        assert_eq!(got[1], "UID:after");
        assert_eq!(prefix("héllo", 2), "h");
        assert_eq!(prefix("héllo", 0), "");
        assert_eq!(prefix("héllo", 99), "héllo");
    }

    proptest::proptest! {
        #[test]
        fn folding_then_unfolding_gives_the_line_back(line in "[^\r\n]{0,400}") {
            let mut out = String::new();
            fold(&mut out, &line);
            proptest::prop_assert!(out.split("\r\n").all(|l| l.len() <= 75));
            // A byte order mark at the very start of a text isn't part of its first line.
            proptest::prop_assert_eq!(Lines::new(&out).collect::<Vec<_>>(), vec![line.strip_prefix('\u{feff}').unwrap_or(&line).to_string()]);
        }

        #[test]
        fn any_line_parses_or_is_refused(line in proptest::prelude::any::<String>()) {
            let _ = ContentLine::parse(&line);
            let _ = Lines::new(&line).count();
        }
    }
}
