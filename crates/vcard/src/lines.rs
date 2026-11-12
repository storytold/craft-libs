//! Content lines: unfolding (and quoted-printable soft line breaks) on the way in, folding on the
//! way out, and the `group.NAME;PARAM=value;PARAM:value` shape in between.

/// Longest logical line kept, in bytes; the rest of a longer line is dropped.
pub(crate) const MAX_LINE: usize = 1 << 20;
/// Most parameters read from one line, and most values of one parameter.
const MAX_PARAMS: usize = 64;
/// Octets per physical line when writing.
const FOLD_AT: usize = 75;
/// How far into a line the `:` that ends its parameters is looked for.
const MAX_HEAD: usize = 1024;

/// The longest prefix of `s` of at most `max` bytes that ends on a char boundary.
pub(crate) fn prefix(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("")
}

/// The text of a vCard file from its bytes, each property's value read in the `CHARSET` it names.
///
/// vCard 2.1 lets a value be raw 8-bit bytes in a character set of its own
/// (`FN;CHARSET=WINDOWS-1250:Gvo\xF0\xE6i\xE8`), so a file is read line by line: a value with
/// bytes above 0x7F is decoded with the `CHARSET` of its property (a folded line goes on with the
/// property above it), and without one it is UTF-8 when it is valid UTF-8, else Windows-1252. Names,
/// parameters and values in ASCII (quoted-printable and base64 among them, decoded later with
/// their charset) are kept as they are. A file that opens with a UTF-16 byte order mark is read as
/// UTF-16 throughout.
pub(crate) fn text_from_bytes(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return craft_mime::charset::decode(bytes, "utf-16");
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if bytes.is_ascii() {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut out = String::with_capacity(bytes.len());
    // The charset of the property being read, for the folded lines that go on with it.
    let mut charset = String::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let end = rest.iter().position(|b| matches!(b, b'\r' | b'\n')).unwrap_or(rest.len());
        let skip = match (rest.get(end), rest.get(end.saturating_add(1))) {
            (Some(b'\r'), Some(b'\n')) => 2,
            (Some(_), _) => 1,
            (None, _) => 0,
        };
        let line = rest.get(..end).unwrap_or(&[]);
        let value = if line.starts_with(b" ") || line.starts_with(b"\t") {
            line
        } else if let Some(colon) = head_end(line) {
            let head = line.get(..colon).unwrap_or(&[]);
            charset = charset_of(head);
            out.push_str(&String::from_utf8_lossy(head));
            line.get(colon..).unwrap_or(&[])
        } else {
            charset.clear();
            line
        };
        out.push_str(&value_text(value, &charset));
        // The line break itself, as it was.
        out.push_str(&String::from_utf8_lossy(rest.get(end..end.saturating_add(skip)).unwrap_or(&[])));
        rest = rest.get(end.saturating_add(skip)..).unwrap_or(&[]);
    }
    out
}

/// Where the `:` that ends a line's name and parameters is: outside quotes, in the first kilobyte.
fn head_end(line: &[u8]) -> Option<usize> {
    let mut quoted = false;
    for (i, b) in line.iter().enumerate().take(MAX_HEAD) {
        match b {
            b'"' => quoted = !quoted,
            b':' if !quoted => return Some(i),
            _ => {}
        }
    }
    None
}

/// The `CHARSET` named among a line's parameters; empty when there is none.
fn charset_of(head: &[u8]) -> String {
    let head = String::from_utf8_lossy(head);
    let named = head.split(';').skip(1).find_map(|param| {
        let (name, value) = param.split_once('=')?;
        name.trim().eq_ignore_ascii_case("CHARSET").then(|| value.trim().trim_matches('"').trim().to_string())
    });
    named.unwrap_or_default()
}

/// A value's bytes as text in `charset`, or guessed without one. ASCII stays as it is; a set that
/// ASCII isn't part of (UTF-16, ISO-2022) can't be raw 8-bit bytes on a line, so it is guessed.
fn value_text(value: &[u8], charset: &str) -> String {
    let label = charset.to_ascii_lowercase();
    if value.is_ascii() || label.starts_with("utf-16") || label.contains("2022") {
        return craft_mime::charset::guess(value);
    }
    craft_mime::charset::decode(value, &label)
}

/// A cursor over the lines of a text. Lines end at CRLF, LF or CR.
#[derive(Clone, Copy)]
pub(crate) struct Lines<'a> {
    rest: &'a str,
}

impl<'a> Lines<'a> {
    pub(crate) fn new(text: &'a str) -> Lines<'a> {
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

    /// The next logical line, or `None` at the end of the text.
    ///
    /// A line that starts with a space or a tab continues the one before. vCard 3.0 and 4.0 drop
    /// that first character; vCard 2.1 (`v21`) keeps it, since there a line is folded where it has
    /// white space anyway. A quoted-printable value that ends in `=` goes on with the next line,
    /// whatever that starts with.
    pub(crate) fn next_line(&mut self, v21: bool) -> Option<String> {
        let mut line = String::new();
        let mut last = self.physical()?;
        // Whether `line` holds all that was read; a line past the cap is still read to its end.
        let mut whole = push_capped(&mut line, last);
        let mut quoted_printable = None;
        loop {
            if quoted_printable.is_none() {
                quoted_printable = says_quoted_printable(&line);
            }
            let mut ahead = *self;
            let Some(next) = ahead.physical() else { break };
            if quoted_printable == Some(true) && last.trim_end().ends_with('=') && !starts_or_ends_a_card(next) {
                if whole {
                    line.truncate(line.trim_end().len());
                    line.pop();
                }
                whole &= push_capped(&mut line, next);
            } else if next.starts_with([' ', '\t']) {
                whole &= push_capped(&mut line, if v21 { next } else { next.get(1..).unwrap_or("") });
            } else {
                break;
            }
            last = next;
            *self = ahead;
        }
        Some(line)
    }
}

/// Append as much of `more` as the cap leaves room for; whether that was all of it.
fn push_capped(line: &mut String, more: &str) -> bool {
    let piece = prefix(more, MAX_LINE.saturating_sub(line.len()));
    line.push_str(piece);
    piece.len() == more.len()
}

/// Whether the parameters of `line` name the quoted-printable encoding; `None` while the `:` that
/// ends them hasn't been seen yet.
fn says_quoted_printable(line: &str) -> Option<bool> {
    let head = prefix(line, MAX_HEAD);
    match head.find(':') {
        Some(colon) => Some(head.get(..colon).unwrap_or("").to_ascii_uppercase().contains("QUOTED-PRINTABLE")),
        None if line.len() > MAX_HEAD => Some(false),
        None => None,
    }
}

/// A soft line break never swallows the line that ends the card.
fn starts_or_ends_a_card(line: &str) -> bool {
    let line = line.trim_start();
    let is = |word: &str| line.get(..word.len()).is_some_and(|start| start.eq_ignore_ascii_case(word));
    is("END:") || is("BEGIN:")
}

/// One property parameter: `TYPE=WORK,VOICE`, `ENCODING=QUOTED-PRINTABLE`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Param {
    /// Upper-cased.
    pub name: String,
    pub values: Vec<String>,
}

/// One property: `item1.TEL;TYPE=CELL:+1 555 0100`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContentLine {
    /// The group before the name (`item1`), lower-cased; empty when there is none.
    pub group: String,
    /// Upper-cased, without the group.
    pub name: String,
    pub params: Vec<Param>,
    /// As written: still encoded and escaped.
    pub value: String,
}

impl ContentLine {
    /// Parse an unfolded line; `None` for a line without a name or without a `:`.
    ///
    /// A parameter without a name (vCard 2.1: `TEL;WORK;VOICE:`) is a `TYPE`, or an `ENCODING` when
    /// it names one.
    pub(crate) fn parse(line: &str) -> Option<ContentLine> {
        // Every delimiter is ASCII, so the byte positions found here are char boundaries.
        let bytes = line.as_bytes();
        let stop = |from: usize, at: &[u8]| {
            bytes.get(from..).and_then(|b| b.iter().position(|c| at.contains(c))).map_or(bytes.len(), |i| from.saturating_add(i))
        };
        let mut pos = stop(0, b";:");
        let full = line.get(..pos)?.trim();
        let (group, name) = full.rsplit_once('.').unwrap_or(("", full));
        if name.is_empty() {
            return None;
        }
        let mut params = Vec::new();
        while bytes.get(pos) == Some(&b';') {
            let start = pos.saturating_add(1);
            pos = stop(start, b"=;:");
            let written = line.get(start..pos)?.trim().to_ascii_uppercase();
            let param = if bytes.get(pos) == Some(&b'=') {
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
                        values.push(value.trim().to_string());
                    }
                }
                Param { name: written, values }
            } else if matches!(written.as_str(), "QUOTED-PRINTABLE" | "BASE64" | "B" | "7BIT" | "8BIT") {
                Param { name: "ENCODING".into(), values: vec![written] }
            } else {
                Param { name: "TYPE".into(), values: vec![written] }
            };
            if !param.name.is_empty() && param.values.iter().any(|v| !v.is_empty()) && params.len() < MAX_PARAMS {
                params.push(param);
            }
        }
        if bytes.get(pos) != Some(&b':') {
            return None;
        }
        Some(ContentLine {
            group: group.trim().to_ascii_lowercase(),
            name: name.trim().to_ascii_uppercase(),
            params,
            value: line.get(pos.saturating_add(1)..)?.to_string(),
        })
    }

    /// The first value of the parameter `name` (upper case).
    pub(crate) fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|p| p.name == name).and_then(|p| p.values.first()).map(String::as_str)
    }

    /// Every `TYPE`, upper-cased: `TYPE=work,voice`, `TYPE=work;TYPE=voice`, `TYPE="work,voice"`
    /// and the bare `WORK;VOICE` all give `WORK` and `VOICE`.
    pub(crate) fn types(&self) -> Vec<String> {
        let values = self.params.iter().filter(|p| p.name == "TYPE").flat_map(|p| p.values.iter());
        values.flat_map(|v| v.split(',')).map(|v| v.trim().to_ascii_uppercase()).filter(|v| !v.is_empty()).take(MAX_PARAMS).collect()
    }
}

/// Append `line` to `out` with CRLF, folded so no physical line is longer than 75 octets and no
/// character is split.
pub(crate) fn fold(out: &mut String, line: &str) {
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

    fn lines(text: &str, v21: bool) -> Vec<String> {
        let mut cursor = Lines::new(text);
        std::iter::from_fn(|| cursor.next_line(v21)).collect()
    }

    #[test]
    fn unfolds_every_kind_of_line_break() {
        assert_eq!(lines("A:1\r\nB:2\nC:3\rD:4", false), ["A:1", "B:2", "C:3", "D:4"]);
        assert_eq!(lines("NOTE:Lunch \r\n with Ada\r\n", false), ["NOTE:Lunch with Ada"]);
        assert_eq!(lines("NOTE:Lun\n\tch\n  twice\n", false), ["NOTE:Lunch twice"]);
        assert_eq!(lines("\u{feff}BEGIN:VCARD\r\n\r\nEND:VCARD", false), ["BEGIN:VCARD", "", "END:VCARD"]);
        assert!(lines("", false).is_empty());
    }

    #[test]
    fn version_2_1_keeps_the_white_space_of_a_fold() {
        let text = "NOTE:This is a long\r\n description\r\n\tin three pieces\r\nFN:x";
        assert_eq!(lines(text, true), ["NOTE:This is a long description\tin three pieces", "FN:x"]);
        assert_eq!(lines(text, false), ["NOTE:This is a longdescriptionin three pieces", "FN:x"]);
    }

    #[test]
    fn quoted_printable_soft_breaks_join_lines() {
        let text = "NOTE;ENCODING=QUOTED-PRINTABLE:first=0D=0A=\r\nsecond =\r\n third= \r\nfourth\r\nFN:x\r\n";
        assert_eq!(lines(text, true), ["NOTE;ENCODING=QUOTED-PRINTABLE:first=0D=0Asecond  thirdfourth", "FN:x"]);
        // The bare 2.1 spelling, in lower case.
        assert_eq!(lines("note;quoted-printable:a=\nb\nFN:x", false), ["note;quoted-printable:ab", "FN:x"]);
        // Without the encoding an `=` at the end is just an `=`.
        assert_eq!(lines("NOTE:1+1=\r\nFN:x", false), ["NOTE:1+1=", "FN:x"]);
        // The parameters may themselves be folded before the `:` comes.
        assert_eq!(lines("NOTE;CHARSET=UTF-8;\r\n ENCODING=QUOTED-PRINTABLE:a=\r\nb", false), ["NOTE;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:ab"]);
        // A soft break at the very end of a card doesn't eat the END line, and one at the end of the text is harmless.
        assert_eq!(lines("NOTE;QUOTED-PRINTABLE:a=\r\nEND:VCARD\r\n", false), ["NOTE;QUOTED-PRINTABLE:a=", "END:VCARD"]);
        assert_eq!(lines("NOTE;QUOTED-PRINTABLE:a=", false), ["NOTE;QUOTED-PRINTABLE:a="]);
    }

    #[test]
    fn parses_groups_names_parameters_and_values() {
        let l = ContentLine::parse("Item1.email;type=INTERNET;Type=pref:nadia@example.org").unwrap();
        assert_eq!((l.group.as_str(), l.name.as_str(), l.value.as_str()), ("item1", "EMAIL", "nadia@example.org"));
        assert_eq!(l.types(), ["INTERNET", "PREF"]);
        let plain = ContentLine::parse("NOTE:Budget: phase two; hiring").unwrap();
        assert_eq!(
            (plain.group.as_str(), plain.name.as_str(), plain.params.len(), plain.value.as_str()),
            ("", "NOTE", 0, "Budget: phase two; hiring")
        );
        assert_eq!(ContentLine::parse("FN:").unwrap().value, "");
        for bad in ["no colon here", ":value without a name", "item1.:x", ";WORK:x", "TEL;TYPE=\"never closed:1", "   ", ""] {
            assert_eq!(ContentLine::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn types_come_in_every_spelling() {
        let types = |line: &str| ContentLine::parse(line).unwrap().types();
        assert_eq!(types("TEL;TYPE=WORK,VOICE:1"), ["WORK", "VOICE"]);
        assert_eq!(types("TEL;type=work;type=voice:1"), ["WORK", "VOICE"]);
        assert_eq!(types("TEL;WORK;VOICE:1"), ["WORK", "VOICE"]);
        assert_eq!(types("TEL;VALUE=uri;TYPE=\"voice,cell\";PREF=1:tel:+1-555-0100"), ["VOICE", "CELL"]);
        assert_eq!(types("TEL;TYPE=:1").len(), 0);
        assert_eq!(types("TEL:1").len(), 0);
        let bare = ContentLine::parse("NOTE;CHARSET=utf-8;QUOTED-PRINTABLE;;X-THING:a").unwrap();
        assert_eq!(bare.param("ENCODING"), Some("QUOTED-PRINTABLE"));
        assert_eq!(bare.param("CHARSET"), Some("utf-8"));
        assert_eq!(bare.types(), ["X-THING"]);
        assert_eq!(ContentLine::parse("PHOTO;BASE64;JPEG:AAAA").unwrap().param("ENCODING"), Some("BASE64"));
    }

    #[test]
    fn folds_at_75_octets_without_splitting_characters() {
        let line = format!("NOTE:{}", "é東🎉x".repeat(40));
        let mut out = String::new();
        fold(&mut out, &line);
        let physical: Vec<&str> = out.trim_end_matches("\r\n").split("\r\n").collect();
        assert!(physical.len() > 3);
        assert!(physical.iter().all(|l| l.len() <= 75), "{physical:?}");
        assert!(physical.iter().skip(1).all(|l| l.starts_with(' ')));
        assert_eq!(lines(&out, false), [line]);
        let mut exact = String::new();
        fold(&mut exact, &"x".repeat(76));
        assert_eq!(exact, format!("{}\r\n x\r\n", "x".repeat(75)));
    }

    #[test]
    fn enormous_lines_are_capped() {
        let text = format!("PHOTO;ENCODING=b:{}\r\n {}\r\nFN:after", "A".repeat(MAX_LINE + 5000), "B".repeat(1000));
        let got = lines(&text, false);
        assert_eq!(got.len(), 2);
        assert!(got[0].len() <= MAX_LINE);
        assert_eq!(got[1], "FN:after");
        // A quoted-printable value that never stops.
        let endless = format!("NOTE;QUOTED-PRINTABLE:{}", "é=\r\n".repeat(700_000));
        let got = lines(&endless, true);
        assert_eq!(got.len(), 1);
        assert!(got[0].len() <= MAX_LINE);
        assert_eq!(prefix("héllo", 2), "h");
    }

    proptest::proptest! {
        #[test]
        fn folding_then_unfolding_gives_the_line_back(line in "[^\r\n]{0,400}") {
            let mut out = String::new();
            fold(&mut out, &line);
            proptest::prop_assert!(out.split("\r\n").all(|l| l.len() <= 75));
            // A byte order mark at the very start of a text isn't part of its first line.
            proptest::prop_assert_eq!(lines(&out, false), vec![line.strip_prefix('\u{feff}').unwrap_or(&line).to_string()]);
        }

        #[test]
        fn any_text_splits_into_lines(text in proptest::prelude::any::<String>(), v21 in proptest::prelude::any::<bool>()) {
            for line in lines(&text, v21) {
                let _ = ContentLine::parse(&line).map(|l| l.types());
            }
        }
    }
}
