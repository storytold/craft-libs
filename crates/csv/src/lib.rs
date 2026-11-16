#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Delimited text (CSV, TSV and their relatives) and SYLK.
//!
//! - [`decode_text`] turns a file's bytes into a string: byte order marks, UTF-8, else
//!   Windows-1256 when the bytes read as Arabic and Windows-1252 otherwise.
//! - [`decode_utf8`] requires valid UTF-8 and strips a leading UTF-8 byte order mark.
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

pub use decode::{decode_text, decode_utf8};

/// Picks comma, semicolon or tab by consistent field counts in the first non-blank records.
/// Quotes start only at a candidate field boundary; candidates with incoherent quoted fields
/// are discarded when another coherent candidate separates the records. Ties prefer tab,
/// then comma, then semicolon. Comma is the fallback when no candidate separates the text.
pub fn sniff_delimiter(text: &str, quote: char) -> u8 {
    let mut candidates = [(b'\t', 0usize, 0usize, true), (b',', 0, 0, true), (b';', 0, 0, true)];
    const SAMPLE_BYTES: usize = 64 * 1024;
    let mut sample_end = text.len().min(SAMPLE_BYTES);
    while !text.is_char_boundary(sample_end) {
        sample_end -= 1;
    }
    let sample = &text[..sample_end];
    let truncated = sample_end < text.len();
    for (delimiter, min_count, total, coherent) in &mut candidates {
        let mut counts = Vec::new();
        let mut count = 0usize;
        let mut quoted = false;
        let mut after_quote = false;
        let mut field_start = true;
        let mut record_has_content = false;
        let mut record_coherent = true;
        let mut candidate_coherent = true;
        let mut reached_record_limit = false;
        let mut chars = sample.chars().peekable();
        while let Some(ch) = chars.next() {
            if quoted {
                if ch == quote {
                    if chars.peek() == Some(&quote) {
                        chars.next();
                        record_has_content = true;
                    } else {
                        quoted = false;
                        after_quote = true;
                    }
                } else if !ch.is_whitespace() {
                    record_has_content = true;
                }
                continue;
            }
            if ch == '\r' || ch == '\n' {
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                if record_has_content {
                    counts.push(count);
                    candidate_coherent &= record_coherent;
                    if counts.len() >= 20 {
                        reached_record_limit = true;
                        break;
                    }
                }
                count = 0;
                quoted = false;
                after_quote = false;
                field_start = true;
                record_has_content = false;
                record_coherent = true;
                continue;
            }
            if ch == quote && field_start {
                quoted = true;
                field_start = false;
                record_has_content = true;
            } else if ch == char::from(*delimiter) {
                count += 1;
                field_start = true;
                after_quote = false;
            } else {
                record_coherent &= !after_quote || ch.is_whitespace();
                record_has_content |= !ch.is_whitespace();
                field_start = false;
            }
        }
        if !reached_record_limit && !truncated {
            if quoted {
                candidate_coherent = false;
            } else if record_has_content {
                candidate_coherent &= record_coherent;
                counts.push(count);
            }
        }
        *coherent &= candidate_coherent;
        *min_count = counts.iter().copied().min().unwrap_or(0);
        *total = counts.iter().sum();
    }
    let prefer_coherent = candidates.iter().any(|(_, min, _, coherent)| *min > 0 && *coherent);
    let mut best = (b',', 0usize, 0usize);
    for (delimiter, min, total, coherent) in candidates {
        if min > 0 && (!prefer_coherent || coherent) && (min, total) > (best.1, best.2) {
            best = (delimiter, min, total);
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
        assert_eq!(sniff_delimiter("a,b\tc\n1,2\td\n", '"'), b'\t');
        assert_eq!(sniff_delimiter("a,b;c\n1,2;c\n", '"'), b',');
        assert_eq!(sniff_delimiter("a;b\tc\n1;2\td\n", '"'), b'\t');
        // A delimiter missing from some lines is text, not a separator (single-column export round trip).
        assert_eq!(sniff_delimiter("alpha;beta\r\ngamma\r\n", '"'), b',');
        assert_eq!(sniff_delimiter("alpha\tbeta\r\ngamma\r\n", '"'), b',');
        assert_eq!(sniff_delimiter("a;b\r\n\r\nc;d\r\n", '"'), b';');
        assert_eq!(sniff_delimiter("x;y\n1,5;2", '"'), b';');
    }

    #[test]
    fn delimiter_sniffing_preserves_candidate_quote_boundaries() {
        for ending in ["\n", "\r\n", "\r"] {
            for (text, expected) in [
                (format!("Name,\"preferred\tCity\"{ending}Ada\tLondon{ending}"), b'\t'),
                (format!("\"Last, First\";City{ending}Ada;London{ending}"), b';'),
                (format!("\"Name\";\"Last, First\"{ending}\"Ada\";\"Lovelace, Countess\"{ending}\"Grace\";\"Hopper, Admiral\"{ending}"), b';'),
                (format!("\"Name\"suffix,Title\tCity{ending}\"Ada\"suffix,Analyst\tLondon\tignored{ending}"), b'\t'),
                (format!("\r\n \r\n\"Name\";City{ending}Ada;London{ending}"), b';'),
            ] {
                assert_eq!(sniff_delimiter(&text, '"'), expected, "{text:?}");
            }
        }
    }

    #[test]
    fn delimiter_sniffing_counts_empty_quoted_fields_and_ignores_whitespace_records() {
        assert_eq!(sniff_delimiter("\"\";name\n\"\";Ada\n", '"'), b';');

        let blank_rows = "\t \t\n".repeat(25);
        assert_eq!(sniff_delimiter(&format!("{blank_rows}a;b\nc;d\n"), '"'), b';');
    }

    #[test]
    fn delimiter_sniffing_does_not_count_the_twentieth_record_twice() {
        let mut text = "a;b\tc\td\n".repeat(19);
        text.push('a');
        text.push_str(&";".repeat(19));
        text.push_str("\tc\n");
        assert_eq!(sniff_delimiter(&text, '"'), b'\t');
    }

    #[test]
    fn delimiter_sniffing_discards_an_incomplete_record_at_the_sample_boundary() {
        let complete = "a\tb\tc\n1\t2\t3\n";
        let remaining = 64 * 1024 - complete.len();
        let partial = format!("{}x\t\"unfinished", "z".repeat(remaining - 4));
        assert!(partial.len() >= remaining);
        let text = format!("{complete}{partial}\"\n");
        assert_eq!(sniff_delimiter(&text, '"'), b'\t');
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
