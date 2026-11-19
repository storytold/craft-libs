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
/// then comma, then semicolon. When no candidate separates the records that hold other text,
/// the same rules pick among the candidates whose records hold only their delimiters and
/// whitespace, such as an exported row of empty cells; a candidate that finds a record with
/// other text is left out of that pick. Comma is the fallback when neither pick finds one.
/// Only the first 65,536 characters are read. A record cut off at the end of that sample is left out,
/// except when no candidate finds coherent complete records with a delimiter in each. Then a candidate
/// that finds no complete record with other text counts the delimiters read so far in the first
/// such record, unless it found rows of its empty cells and that record has none of its
/// delimiters. That record is incoherent when it has a quote inside a field that does not start
/// with one. When one of those candidates ends the sample inside a quoted field, each of them
/// counts only up to that field's opening quote.
pub fn sniff_delimiter(text: &str, quote: char) -> u8 {
    const SAMPLE_CHARS: usize = 64 * 1024;
    let sample_end = text.char_indices().nth(SAMPLE_CHARS).map_or(text.len(), |(at, _)| at);
    let sample = &text[..sample_end];
    let truncated = sample_end < text.len();
    let mut scans = b"\t,;".map(|delimiter| scan_delimiter(sample, delimiter, quote));
    if truncated {
        if scans.iter().any(|scan| scan.coherent && scan.counts.iter().min().is_some_and(|&min| min > 0)) {
            return pick_delimiter(scans);
        }
        // Every quote in the scans is at a char boundary, so the cut is one too.
        let cut = scans.iter().filter(|scan| scan.counts.is_empty()).filter_map(|scan| scan.open_quote).min().unwrap_or(sample_end);
        for scan in &mut scans {
            if !scan.counts.is_empty() {
                continue;
            }
            if cut < sample_end {
                let before_cut = scan_delimiter(&sample[..cut], scan.delimiter, quote);
                (scan.count, scan.record_has_content, scan.record_coherent, scan.record_bare_quote) =
                    (before_cut.count, before_cut.record_has_content, before_cut.record_coherent, before_cut.record_bare_quote);
            }
            if scan.delimiter_only_counts.is_empty() {
                if scan.record_has_content {
                    scan.coherent &= scan.record_coherent && !scan.record_bare_quote;
                    scan.counts.push(scan.count);
                } else {
                    scan.push_delimiter_only_record();
                }
            } else if scan.record_has_content && scan.count > 0 {
                // After rows of empty cells, a cut record without this delimiter is left out.
                scan.coherent &= scan.record_coherent && !scan.record_bare_quote;
                scan.counts.push(scan.count);
            }
        }
    } else {
        for scan in &mut scans {
            if scan.reached_record_limit {
                continue;
            }
            if scan.open_quote.is_some() {
                scan.coherent = false;
            } else if scan.record_has_content {
                scan.coherent &= scan.record_coherent;
                scan.counts.push(scan.count);
            } else {
                scan.push_delimiter_only_record();
            }
        }
    }
    pick_delimiter(scans)
}

fn pick_delimiter(scans: [DelimiterScan; 3]) -> u8 {
    let with_text = scans.each_ref().map(|scan| (scan.delimiter, scan.counts.as_slice(), scan.coherent));
    // Records of delimiters alone have no quotes, so they are coherent.
    let delimiters_only =
        scans.each_ref().map(|scan| (scan.delimiter, if scan.counts.is_empty() { scan.delimiter_only_counts.as_slice() } else { &[] }, true));
    best_delimiter(with_text).or_else(|| best_delimiter(delimiters_only)).unwrap_or(b',')
}

fn best_delimiter(candidates: [(u8, &[usize], bool); 3]) -> Option<u8> {
    let candidates = candidates
        .map(|(delimiter, counts, coherent)| (delimiter, counts.iter().copied().min().unwrap_or(0), counts.iter().sum::<usize>(), coherent));
    let prefer_coherent = candidates.iter().any(|(_, min, _, coherent)| *min > 0 && *coherent);
    let mut best: Option<(u8, usize, usize)> = None;
    for (delimiter, min, total, coherent) in candidates {
        if min > 0 && (!prefer_coherent || coherent) && best.is_none_or(|(_, best_min, best_total)| (min, total) > (best_min, best_total)) {
            best = Some((delimiter, min, total));
        }
    }
    best.map(|(delimiter, _, _)| delimiter)
}

/// One candidate delimiter's pass over the sample in [`sniff_delimiter`].
struct DelimiterScan {
    delimiter: u8,
    /// Delimiter counts of the complete records that hold other text, at most 20.
    counts: Vec<usize>,
    /// Delimiter counts of the complete records that hold only this candidate's delimiters and
    /// whitespace, at most 20. They do not count toward the 20 records above.
    delimiter_only_counts: Vec<usize>,
    /// Whether every counted record had coherent quoted fields.
    coherent: bool,
    reached_record_limit: bool,
    /// The record still open at the end of the sample: its delimiter count so far, whether it
    /// has content, and whether its quoted fields so far are coherent.
    count: usize,
    record_has_content: bool,
    record_coherent: bool,
    /// Whether the open record has a quote inside a field that does not start with one.
    record_bare_quote: bool,
    /// The byte offset of the quote that opens a field still unterminated at the end.
    open_quote: Option<usize>,
}

impl DelimiterScan {
    /// Keeps the open record's delimiter count when the record has no other text and at least
    /// one delimiter.
    fn push_delimiter_only_record(&mut self) {
        if !self.record_has_content && self.count > 0 && self.delimiter_only_counts.len() < 20 {
            self.delimiter_only_counts.push(self.count);
        }
    }
}

fn scan_delimiter(sample: &str, delimiter: u8, quote: char) -> DelimiterScan {
    let mut scan = DelimiterScan {
        delimiter,
        counts: Vec::new(),
        delimiter_only_counts: Vec::new(),
        coherent: true,
        reached_record_limit: false,
        count: 0,
        record_has_content: false,
        record_coherent: true,
        record_bare_quote: false,
        open_quote: None,
    };
    let mut after_quote = false;
    let mut field_start = true;
    let mut chars = sample.char_indices().peekable();
    while let Some((at, ch)) = chars.next() {
        if scan.open_quote.is_some() {
            if ch == quote {
                if chars.peek().map(|&(_, next)| next) == Some(quote) {
                    chars.next();
                    scan.record_has_content = true;
                } else {
                    scan.open_quote = None;
                    after_quote = true;
                }
            } else if !ch.is_whitespace() {
                scan.record_has_content = true;
            }
            continue;
        }
        if ch == '\r' || ch == '\n' {
            if ch == '\r' && chars.peek().map(|&(_, next)| next) == Some('\n') {
                chars.next();
            }
            if scan.record_has_content {
                scan.counts.push(scan.count);
                scan.coherent &= scan.record_coherent;
                if scan.counts.len() >= 20 {
                    scan.reached_record_limit = true;
                    break;
                }
            } else {
                scan.push_delimiter_only_record();
            }
            scan.count = 0;
            after_quote = false;
            field_start = true;
            scan.record_has_content = false;
            scan.record_coherent = true;
            scan.record_bare_quote = false;
            continue;
        }
        if ch == quote && field_start {
            scan.open_quote = Some(at);
            field_start = false;
            scan.record_has_content = true;
        } else if ch == char::from(delimiter) {
            scan.count += 1;
            field_start = true;
            after_quote = false;
        } else {
            scan.record_coherent &= !after_quote || ch.is_whitespace();
            scan.record_bare_quote |= ch == quote;
            scan.record_has_content |= !ch.is_whitespace();
            field_start = false;
        }
    }
    scan
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
    fn delimiter_sniffing_counts_a_first_record_longer_than_the_sample() {
        let wide = |field: &str, delimiter: char| {
            let mut record = String::new();
            while record.len() <= 64 * 1024 {
                record.push_str(field);
                record.push(delimiter);
            }
            record
        };
        assert_eq!(sniff_delimiter(&format!("{}end\n1;2\n", wide("cell", ';')), '"'), b';');
        assert_eq!(sniff_delimiter(&format!("{}end\n1\t2\n", wide("cell", '\t')), '"'), b'\t');
        // Delimiters inside quoted fields of the long record are text.
        assert_eq!(sniff_delimiter(&format!("{}end\n", wide("\"a,b\tc\"", ';')), '"'), b';');
        assert_eq!(sniff_delimiter(&format!("{}end\n", wide("\"a;b\tc\"", ',')), '"'), b',');
        // A candidate that reads a quoted field as text inside an unquoted one is incoherent.
        assert_eq!(sniff_delimiter(&format!("a,\"{}\",{}", "x;y".repeat(20_000), "z".repeat(10_000)), '"'), b',');
        // The sample ends inside a quoted field: the delimiters before that field count.
        let text = format!("a;b;c;\"{}\"\n1;2;3;4\n", "long text ".repeat(8_000));
        assert_eq!(sniff_delimiter(&text, '"'), b';');
        // Candidates without a complete record do not count delimiters after that quote, also
        // when they read it as text.
        assert_eq!(sniff_delimiter(&format!("a,\"{}\"\n1,2\n", "x;y".repeat(30_000)), '"'), b',');
        assert_eq!(sniff_delimiter(&format!("a;b;\"{}\"\n1;2;3\n", "x,y\t".repeat(20_000)), '"'), b';');
        // A line break inside the open quoted field gives the other candidates a record without
        // a delimiter, which does not stop the first record from counting.
        assert_eq!(sniff_delimiter(&format!("a;b;\"hello\n{}\"\n", "x".repeat(70_000)), '"'), b';');
        assert_eq!(sniff_delimiter(&format!("\"a\";b;\"hello,\n{}\"\n", "x".repeat(70_000)), '"'), b';');
        // When a candidate finds coherent complete records with a delimiter in each, a record cut
        // off at the end is left out.
        assert_eq!(sniff_delimiter(&format!("a;\"hello,\";b\n{}", "x;y;z\n".repeat(12_000)), '"'), b';');
        assert_eq!(sniff_delimiter(&format!("a,b,c;\"hello,\";d\n{}", "x;y;z\n".repeat(12_000)), '"'), b';');
    }

    #[test]
    fn delimiter_sniffing_samples_characters_not_bytes() {
        // 30,000 three-byte characters are 90,000 bytes, more than 64 KiB.
        assert_eq!(sniff_delimiter(&format!("{};b;c\n1;2;3\n", "€".repeat(30_000)), '"'), b';');
        // The 65,536th character is in the sample and the 65,537th is not.
        assert_eq!(sniff_delimiter(&format!("{};x\n1;2\n", "€".repeat(65_535)), '"'), b';');
        assert_eq!(sniff_delimiter(&format!("{};x\n1;2\n", "€".repeat(65_536)), '"'), b',');
    }

    // A sheet whose only value is far down exports rows of empty cells that fill the whole sample.
    #[test]
    fn delimiter_sniffing_reads_a_sparse_semicolon_export() {
        assert_eq!(sniff_delimiter(&format!("{};7\n", ";\n".repeat(40_000)), '"'), b';');
    }

    #[test]
    fn delimiter_sniffing_reads_a_sparse_semicolon_export_with_crlf() {
        assert_eq!(sniff_delimiter(&format!("{};7\r\n", ";\r\n".repeat(40_000)), '"'), b';');
    }

    #[test]
    fn delimiter_sniffing_reads_a_sparse_tab_export() {
        assert_eq!(sniff_delimiter(&format!("{}\t7\r\n", "\t\r\n".repeat(40_000)), '"'), b'\t');
    }

    #[test]
    fn delimiter_sniffing_reads_a_short_file_of_empty_semicolon_cells() {
        assert_eq!(sniff_delimiter(";;\n;;\n", '"'), b';');
        assert_eq!(sniff_delimiter(";;\r\n;;", '"'), b';');
    }

    #[test]
    fn delimiter_sniffing_reads_empty_semicolon_cells_padded_with_whitespace() {
        assert_eq!(sniff_delimiter(&"; ;  \n".repeat(20_000), '"'), b';');
    }

    #[test]
    fn delimiter_sniffing_reads_a_first_record_of_empty_cells_longer_than_the_sample() {
        assert_eq!(sniff_delimiter(&format!("{}\n1;2\n", ";".repeat(70_000)), '"'), b';');
    }

    #[test]
    fn delimiter_sniffing_leaves_out_a_cut_record_after_a_row_of_empty_cells() {
        assert_eq!(sniff_delimiter(&format!(";\n{};7\n", "x".repeat(70_000)), '"'), b';');
    }

    #[test]
    fn delimiter_sniffing_counts_a_long_record_after_a_row_of_empty_cells() {
        // Delimiters inside the long record's quoted field are text for every candidate.
        assert_eq!(sniff_delimiter(&format!("\t\nname\t\"{}\"\n", "x,y".repeat(30_000)), '"'), b'\t');
        assert_eq!(sniff_delimiter(&format!(";\nname;\"{}\"\n", "x,y\t".repeat(20_000)), '"'), b';');
    }

    #[test]
    fn delimiter_sniffing_lets_records_with_text_outweigh_records_of_delimiters_alone() {
        // Rows of tabs alone are blank to the other candidates and do not outweigh them.
        assert_eq!(sniff_delimiter(&format!("{}a;b\nc;d\n", "\t\n".repeat(25)), '"'), b';');
        assert_eq!(sniff_delimiter(&format!("{}a;b\nc;d\n", "\t\t\n".repeat(25)), '"'), b';');
        // The same holds for a record with text cut off at the end of the sample.
        assert_eq!(sniff_delimiter(&format!("\t\n{}", "x;".repeat(40_000)), '"'), b';');
        // A candidate with a record of other text is judged on those records alone.
        assert_eq!(sniff_delimiter(&format!("title\n{}", ";\n".repeat(25)), '"'), b',');
        // Rows of empty cells in a comma file; comma is also the fallback.
        assert_eq!(sniff_delimiter("a,b,c,d\n,,,\n,,,\n1,2,3,4\n", '"'), b',');
        assert_eq!(sniff_delimiter(&format!("{}x;y\n", ",,,\n".repeat(20_000)), '"'), b',');
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
