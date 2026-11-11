//! Text values: cleaning, escaping, splitting, and the 2.1 encodings (quoted-printable, base64;
//! a `CHARSET` is read by `craft_mime::charset`, as mail is).

/// A one-line field: line breaks and other control characters become spaces, and the ends are
/// trimmed.
pub(crate) fn single(s: &str) -> String {
    let spaced: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    spaced.trim().to_string()
}

/// Text of several lines: `\r\n` and `\r` become `\n`; other control characters except tab are
/// dropped.
pub(crate) fn multi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut after_cr = false;
    for c in s.chars() {
        let cr = c == '\r';
        match c {
            '\n' if after_cr => {}
            '\r' | '\n' => out.push('\n'),
            '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
        after_cr = cr;
    }
    out
}

/// A value ready to write: `\`, `;`, `,` and line breaks escaped.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len().saturating_add(8));
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Undo [`escape`]: `\n` and `\N` are line breaks, `\\`, `\;`, `\,` (and the `\:` some writers
/// add) their second character; any other backslash is kept.
pub(crate) fn unescape(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(x @ ('\\' | ';' | ',' | ':')) => out.push(x),
            Some(x) => {
                out.push('\\');
                out.push(x);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The first `max` pieces of `s`, split on its unescaped `separator`s; the pieces are still
/// escaped.
pub(crate) fn split(s: &str, separator: char, max: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut escaped) = (0usize, false);
    for (i, c) in s.char_indices() {
        if out.len() >= max {
            return out;
        }
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == separator {
            out.push(s.get(start..i).unwrap_or(""));
            start = i.saturating_add(c.len_utf8());
        }
    }
    if out.len() < max {
        out.push(s.get(start..).unwrap_or(""));
    }
    out
}

/// One component of a structured value (a name part, a street) as one line of text. vCard 3.0 and
/// 4.0 let a component list several values with commas, which are joined with `joiner`; in 2.1
/// (`v21`) a comma is just a comma.
pub(crate) fn component(piece: &str, joiner: &str, v21: bool) -> String {
    let values = if v21 { vec![piece] } else { split(piece, ',', 32) };
    match values.as_slice() {
        [one] => single(&unescape(one)),
        many => many.iter().map(|v| single(&unescape(v))).filter(|v| !v.is_empty()).collect::<Vec<_>>().join(joiner),
    }
}

/// The bytes of a quoted-printable value (`=C3=A9`); soft line breaks are already gone, and a `=`
/// that isn't followed by two hex digits is kept.
pub(crate) fn quoted_printable(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let hex = |at: usize| bytes.get(at).and_then(|b| char::from(*b).to_digit(16)).and_then(|d| u8::try_from(d).ok());
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        if b == b'='
            && let (Some(high), Some(low)) = (hex(i.saturating_add(1)), hex(i.saturating_add(2)))
        {
            out.push(high.saturating_mul(16).saturating_add(low));
            i = i.saturating_add(3);
        } else {
            out.push(b);
            i = i.saturating_add(1);
        }
    }
    out
}

/// The bytes of a base64 value, white space and padding or not; empty for junk.
pub(crate) fn base64_bytes(s: &str) -> Vec<u8> {
    let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    craft_mime::base64::STANDARD_NO_PAD.decode(compact.trim_end_matches('=')).unwrap_or_default()
}

/// Whether `c` is one of Unicode's format characters (general category Cf, Unicode 17.0):
/// bidirectional marks, zero-width marks and joiners, the byte-order mark, the soft hyphen, tags
/// and the rest. They show nothing and can turn the text around them.
pub(crate) fn is_format(c: char) -> bool {
    matches!(
        c,
        '\u{00ad}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061c}'
            | '\u{06dd}'
            | '\u{070f}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08e2}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_one_line_and_many_line_text() {
        assert_eq!(single("  Ada\r\nLovelace\t\u{0}x  "), "Ada  Lovelace  x");
        assert_eq!(single("\n\u{85}\u{7f} "), "");
        assert_eq!(multi("a\r\nb\rc\n\nd\u{0}\u{7f}\te "), "a\nb\nc\n\nd\te ");
        assert_eq!(multi(""), "");
    }

    #[test]
    fn escapes_and_unescapes() {
        assert_eq!(escape("Research, Inc.; Lab \\ 7\nSecond\r\n"), "Research\\, Inc.\\; Lab \\\\ 7\\nSecond\\n");
        assert_eq!(unescape("Research\\, Inc.\\; Lab \\\\ 7\\nSecond\\NThird"), "Research, Inc.; Lab \\ 7\nSecond\nThird");
        assert_eq!(unescape("http\\://example.org C:\\Users\\x tail\\"), "http://example.org C:\\Users\\x tail\\");
        assert_eq!(unescape("plain"), "plain");
    }

    #[test]
    fn splits_on_unescaped_separators() {
        assert_eq!(split("Okafor;Nadia;;Dr.;", ';', 9), ["Okafor", "Nadia", "", "Dr.", ""]);
        assert_eq!(split("a\\;b;c\\\\;d", ';', 9), ["a\\;b", "c\\\\", "d"]);
        assert_eq!(split("a,b,c", ',', 2), ["a", "b"]);
        assert_eq!(split("", ';', 9), [""]);
        assert!(split("a;b", ';', 0).is_empty());
        assert_eq!(split("é;東;🎉", ';', 9), ["é", "東", "🎉"]);
    }

    #[test]
    fn components_join_their_lists() {
        assert_eq!(component("Quinlan,Paul", " ", false), "Quinlan Paul");
        assert_eq!(component("Quinlan\\,Paul", " ", false), "Quinlan,Paul");
        assert_eq!(component(" 41 Harbor Road , Suite 300,, ", ", ", false), "41 Harbor Road, Suite 300");
        assert_eq!(component("Smith, Jr.", " ", true), "Smith, Jr.");
        assert_eq!(component("  one\\nline  ", " ", false), "one line");
        assert_eq!(component("", ", ", false), "");
    }

    #[test]
    fn decodes_quoted_printable() {
        assert_eq!(quoted_printable("J=C3=BCrgen M=c3=bcller"), "Jürgen Müller".as_bytes());
        assert_eq!(quoted_printable("a=3Db=0D=0Ac"), b"a=b\r\nc");
        // What isn't an escape stays.
        assert_eq!(quoted_printable("1+1=2 =ZZ =4"), b"1+1=2 =ZZ =4");
        assert_eq!(quoted_printable("="), b"=");
        assert_eq!(quoted_printable("é=41"), "éA".as_bytes());
    }

    #[test]
    fn decodes_base64() {
        assert_eq!(base64_bytes("SGVsbG8sIHdvcmxk"), b"Hello, world");
        assert_eq!(base64_bytes(" SGVs\r\n bG8=\n"), b"Hello");
        assert_eq!(base64_bytes("SGVsbG8"), b"Hello");
        assert!(base64_bytes("not base64!").is_empty());
        assert!(base64_bytes("").is_empty());
    }

    proptest::proptest! {
        #[test]
        fn clean_text_survives_escaping(s in "(\\PC|[\n\t;,\\\\:]){0,80}") {
            proptest::prop_assert_eq!(unescape(&escape(&s)), s.clone());
            // And as one piece of a structured value or a list.
            proptest::prop_assert_eq!(split(&format!("{};x", escape(&s)), ';', 9).first().map(|p| unescape(p)), Some(s.clone()));
            proptest::prop_assert_eq!(split(&format!("{},x", escape(&s)), ',', 9).first().map(|p| unescape(p)), Some(s));
        }

        #[test]
        fn text_helpers_never_panic(s in proptest::prelude::any::<String>(), max in 0usize..5) {
            let _ = (single(&s), multi(&s), unescape(&s), split(&s, ';', max), component(&s, ", ", false), quoted_printable(&s), base64_bytes(&s));
            proptest::prop_assert!(!escape(&multi(&s)).contains(['\r', '\n']));
            proptest::prop_assert!(!single(&s).contains(['\r', '\n']));
        }
    }
}
