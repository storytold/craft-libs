#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Delimited text (CSV, TSV and their relatives) and SYLK.
//!
//! - [`decode_text`] turns a file's bytes into a string: byte order marks, UTF-8, else
//!   Windows-1256 when the bytes read as Arabic and Windows-1252 otherwise.
//! - [`sniff_delimiter`] picks comma, semicolon or tab.
//! - [`parse_records`] splits text into records of fields (RFC 4180 with the usual leniency),
//!   handing each record to a callback, with a cap on the fields kept per record. [`records`]
//!   collects them.
//! - [`push_field`] and [`field_len`] write one field, quoted when it has to be.
//! - [`sylk`] reads SYLK (`.slk`) files.
//!
//! Fields are text. Reading a field as a number or a date, and deciding how a value is written,
//! belong to the caller.
//!
//! ```
//! let text = craft_csv::decode_text(b"name;note\r\nAda;\"said \"\"hi\"\"\"\r\n");
//! let delimiter = char::from(craft_csv::sniff_delimiter(&text, '"'));
//! let rows = craft_csv::records(&text, delimiter, '"', 1000, 1000);
//! assert_eq!(rows, [["name", "note"], ["Ada", "said \"hi\""]]);
//!
//! let mut line = String::new();
//! craft_csv::push_field(&mut line, "a;b", ';');
//! assert_eq!(line, "\"a;b\"");
//! ```

mod decode;
pub mod sylk;

pub use decode::decode_text;

/// Picks comma, semicolon or tab: the candidate with the most consistent count per line
/// (outside quotes) over the first non-blank lines. A candidate must appear on every such line, otherwise comma wins.
pub fn sniff_delimiter(text: &str, quote: char) -> u8 {
    let mut best = (b',', 0usize, 0usize);
    for d in [',', ';', '\t'] {
        let mut counts = vec![];
        let mut n = 0usize;
        let mut in_q = false;
        let mut blank = true;
        for ch in text.chars().take(64 * 1024) {
            if !in_q && ch == '\n' {
                // Blank lines carry no delimiter evidence.
                if !blank {
                    counts.push(n);
                }
                n = 0;
                blank = true;
                if counts.len() >= 20 {
                    break;
                }
                continue;
            }
            if ch != '\r' {
                blank = false;
            }
            if ch == quote {
                in_q = !in_q;
            } else if !in_q && ch == d {
                n += 1;
            }
        }
        if !blank {
            counts.push(n);
        }
        let min = counts.iter().copied().min().unwrap_or(0);
        let total: usize = counts.iter().sum();
        if min > 0 && (min, total) > (best.1, best.2) {
            best = (d as u8, min, total);
        }
    }
    best.0
}

/// Splits delimited text into records of fields and hands each to `on_record`, which returns
/// whether to go on. A record ends at CR, LF or CRLF; a blank line is a record with one empty
/// field. A field that starts with `quote` runs to the closing quote (a doubled quote is one
/// quote character, and line ends inside are text); a quote elsewhere in a field is text, and an
/// unterminated quoted field runs to the end of the text.
///
/// A record keeps at most `max_fields` fields; the rest are dropped as they are read, so a row
/// of endless delimiters cannot allocate a string per field.
pub fn parse_records(text: &str, delim: char, quote: char, max_fields: usize, mut on_record: impl FnMut(Vec<String>) -> bool) {
    let mut field = String::new();
    let mut record: Vec<String> = vec![];
    let mut in_q = false;
    let mut chars = text.chars().peekable();
    let mut field_started_quoted = false;
    while let Some(c) = chars.next() {
        if in_q {
            if c == quote {
                if chars.peek() == Some(&quote) {
                    field.push(quote);
                    chars.next();
                } else {
                    in_q = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        if c == quote && field.is_empty() && !field_started_quoted {
            in_q = true;
            field_started_quoted = true;
        } else if c == delim {
            let f = std::mem::take(&mut field);
            if record.len() < max_fields {
                record.push(f);
            }
            field_started_quoted = false;
        } else if c == '\r' || c == '\n' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            let f = std::mem::take(&mut field);
            if record.len() < max_fields {
                record.push(f);
            }
            field_started_quoted = false;
            if !on_record(std::mem::take(&mut record)) {
                return;
            }
        } else {
            field.push(c);
        }
    }
    if !field.is_empty() || !record.is_empty() || field_started_quoted {
        if record.len() < max_fields {
            record.push(field);
        }
        on_record(record);
    }
}

/// The first `max_records` records of [`parse_records`], each with at most `max_fields` fields.
pub fn records(text: &str, delim: char, quote: char, max_records: usize, max_fields: usize) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    parse_records(text, delim, quote, max_fields, |record| {
        if out.len() < max_records {
            out.push(record);
        }
        out.len() < max_records
    });
    out
}

/// Whether a field must be quoted when written with `delim`: it holds the delimiter, a double
/// quote or a line break.
fn needs_quotes(field: &str, delim: char) -> bool {
    field.contains(delim) || field.contains('"') || field.contains('\n') || field.contains('\r')
}

/// The bytes [`push_field`] appends for `field`, so a writer can enforce a size cap before it
/// writes (no quoted copy of the text is built).
pub fn field_len(field: &str, delim: char) -> usize {
    if needs_quotes(field, delim) { field.len() + field.matches('"').count() + 2 } else { field.len() }
}

/// Appends one field to `out`: as it is, or in double quotes with its quotes doubled when it
/// holds the delimiter, a quote or a line break. Delimiters between fields and the line end
/// (CRLF in RFC 4180) are the caller's.
pub fn push_field(out: &mut String, field: &str, delim: char) {
    if !needs_quotes(field, delim) {
        out.push_str(field);
        return;
    }
    out.push('"');
    for ch in field.chars() {
        if ch == '"' {
            out.push('"');
        }
        out.push(ch);
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str) -> Vec<Vec<String>> {
        records(text, ',', '"', usize::MAX, usize::MAX)
    }

    #[test]
    fn quoting_and_newlines() {
        let src = "a,\"b,c\",\"say \"\"hi\"\"\"\r\n\"multi\nline\",,3\n\nlast";
        assert_eq!(rows(src), [vec!["a", "b,c", "say \"hi\""], vec!["multi\nline", "", "3"], vec![""], vec!["last"]]);
        // A quote inside an unquoted field is text; an unterminated quote runs to the end.
        assert_eq!(rows("a\"b,c\r1"), [vec!["a\"b", "c"], vec!["1"]]);
        assert_eq!(rows("\"unterminated,quote\n1,2"), [vec!["unterminated,quote\n1,2"]]);
        // An empty quoted last field is still a field; a trailing line end adds no record.
        assert_eq!(rows("a,\"\""), [vec!["a", ""]]);
        assert_eq!(rows("a\n"), [vec!["a"]]);
        assert!(rows("").is_empty());
        // Other delimiters and quotes.
        assert_eq!(records("'a;b';c", ';', '\'', 9, 9), [vec!["a;b", "c"]]);
    }

    #[test]
    fn delimiter_sniffing() {
        assert_eq!(sniff_delimiter("a;b;c\n1;2;3\n", '"'), b';');
        assert_eq!(sniff_delimiter("a\tb\n1\t2\n", '"'), b'\t');
        assert_eq!(sniff_delimiter("a,b\n\"x;y;z\",2\n", '"'), b',');
        assert_eq!(sniff_delimiter("single", '"'), b',');
        // A delimiter missing from some lines is text, not a separator (single-column export round trip).
        assert_eq!(sniff_delimiter("alpha;beta\r\ngamma\r\n", '"'), b',');
        assert_eq!(sniff_delimiter("alpha\tbeta\r\ngamma\r\n", '"'), b',');
        assert_eq!(sniff_delimiter("a;b\r\n\r\nc;d\r\n", '"'), b';');
        assert_eq!(sniff_delimiter("x;y\n1,5;2", '"'), b';');
    }

    #[test]
    fn parser_stops_collecting_fields_at_the_cap() {
        let (max, extra) = (16_384usize, 50);
        // max + extra fields on the first row (the last one filled), then a short second row.
        let mut text = ",".repeat(max + extra - 1);
        text.push_str("end\nx,y\n");
        let mut lens = vec![];
        parse_records(&text, ',', '"', max, |rec| {
            lens.push(rec.len());
            true
        });
        assert_eq!(lens, vec![max, 2]);
        // An unterminated last row is capped the same way.
        assert_eq!(records(&",".repeat(max + extra), ',', '"', 9, max).iter().map(Vec::len).collect::<Vec<_>>(), [max]);
    }

    #[test]
    fn reading_stops_when_the_callback_says_so_and_at_the_record_cap() {
        let mut seen = 0;
        parse_records("1\n2\n3\n", ',', '"', 9, |_| {
            seen += 1;
            seen < 2
        });
        assert_eq!(seen, 2);
        assert_eq!(records("1\n2\n3\n", ',', '"', 2, 9), [["1"], ["2"]]);
        assert!(records("1\n2\n", ',', '"', 0, 9).is_empty());
    }

    #[test]
    fn writing_quotes_only_what_needs_it() {
        let line = |fields: &[&str], delim: char| {
            let mut out = String::new();
            let mut len = 0;
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    out.push(delim);
                }
                len += field_len(f, delim);
                push_field(&mut out, f, delim);
            }
            assert_eq!(out.len(), len + (fields.len() - 1) * delim.len_utf8(), "field_len is exact");
            out
        };
        assert_eq!(line(&["", "a,b", "q\"t", "0.1", "x\ny", "r\rs"], ','), ",\"a,b\",\"q\"\"t\",0.1,\"x\ny\",\"r\rs\"");
        assert_eq!(line(&["a,b", "c;d"], ';'), "a,b;\"c;d\"");
        // Round trip.
        let fields = ["a,b", "say \"hi\"", "", "x\r\ny", "plain"];
        assert_eq!(rows(&line(&fields, ',')), [fields]);
    }
}
