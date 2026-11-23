//! Markdown export for extracted text: one line per paragraph, conservative markup escaping,
//! styled runs, pipe tables and explicit page separators. Unlike [`crate::export`], this mode
//! does not infer a word-processing document or normalize its block structure. Callers keep
//! layout/reading-order detection and bound the amount of supplied text and table data.
// Extracted from PDFCraft's Markdown writer (Adrian), including the empty-run correction
// by jonathanwxh-cell. The source remains shared here rather than copied into app adapters.

const MAX_HEADING: u8 = 6;
/// Largest span represented by a table cell, matching the supported pipe-table grid.
pub const MAX_SPAN: usize = 63;

fn md_text(s: &str) -> String {
    crate::escaping::escape(s, true)
}

/// A table cell supplied by the caller's existing model, without a second table allocation.
pub trait TableCell {
    fn text(&self) -> &str;
    fn bold(&self) -> bool {
        false
    }
    fn italic(&self) -> bool {
        false
    }
    fn span(&self) -> usize {
        1
    }
}

/// `line` with its start escaped where Markdown would read a list item there (`- `, `+ `,
/// `1. `, `1) `); the other block markers (`#`, `>`) are escaped by [`md_text`] already.
fn block_start(line: String) -> String {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    match line.as_bytes().get(digits) {
        Some(b'.' | b')') if digits > 0 => format!("{}\\{}", line.get(..digits).unwrap_or_default(), line.get(digits..).unwrap_or_default()),
        Some(b'-' | b'+') if digits == 0 => format!("\\{line}"),
        _ => line,
    }
}

/// `text` (already escaped) in bold and/or italic, its outer spaces kept outside. `open` and
/// `close` say whether `*` markers would be read as emphasis on that side (see [`marks`]);
/// where they would not, as in `foo` + bold `.` + `bar`, the run is wrapped in HTML tags, which
/// CommonMark takes inline. The text's own outer spaces count as a side that marks.
pub fn emphasis(text: &str, bold: bool, italic: bool, open: bool, close: bool) -> String {
    let core = text.trim();
    if core.is_empty() || !(bold || italic) {
        return text.to_string();
    }
    let lead = text.get(..text.len().saturating_sub(text.trim_start().len())).unwrap_or_default();
    let trail = text.get(text.trim_end().len()..).unwrap_or_default();
    let markers = (open || !lead.is_empty()) && (close || !trail.is_empty());
    let (before, after) = match (markers, bold, italic) {
        (true, true, true) => ("***", "***"),
        (true, true, false) => ("**", "**"),
        (true, false, _) => ("*", "*"),
        (false, true, true) => ("<strong><em>", "</em></strong>"),
        (false, true, false) => ("<strong>", "</strong>"),
        (false, false, _) => ("<em>", "</em>"),
    };
    format!("{lead}{before}{core}{after}{trail}")
}

/// A paragraph's runs (text, bold, italic) as one line of Markdown. Neighbouring runs in the
/// same style are joined first, so two bold runs never meet as `****`.
pub fn paragraph(runs: &[(&str, bool, bool)]) -> String {
    let mut merged: Vec<(String, bool, bool)> = Vec::new();
    for &(text, bold, italic) in runs {
        let text = md_text(text);
        if text.is_empty() {
            continue;
        }
        match merged.last_mut() {
            Some(last) if last.1 == bold && last.2 == italic => last.0.push_str(&text),
            _ => merged.push((text, bold, italic)),
        }
    }
    let mut out = String::new();
    for (k, (text, bold, italic)) in merged.iter().enumerate() {
        let core = text.trim();
        let open = marks(core.chars().next(), out.chars().last());
        // Markers of a styled neighbour right next to these would join them into one run.
        let close = match merged.get(k.saturating_add(1)) {
            Some((next, b, i)) if (*b || *i) && !next.starts_with(char::is_whitespace) => false,
            next => marks(core.chars().last(), next.and_then(|(n, ..)| n.chars().next())),
        };
        out.push_str(&emphasis(text, *bold, *italic, open, close));
    }
    out
}

/// Whether a `*` marker between the text's `edge` character and the `outside` one is read as
/// emphasis (CommonMark's flanking rules for `*`): always next to a space or the line's edge,
/// never next to another `*`, and otherwise unless the text's edge is punctuation while the
/// outside is a letter or digit.
fn marks(edge: Option<char>, outside: Option<char>) -> bool {
    match (edge, outside) {
        (_, None) => true,
        (_, Some(o)) if o.is_whitespace() => true,
        (_, Some('*')) => false,
        (Some(e), Some(o)) => e.is_alphanumeric() || o.is_ascii_punctuation(),
        (None, Some(_)) => false,
    }
}

fn cell(c: &impl TableCell) -> String {
    emphasis(&md_text(c.text()), c.bold(), c.italic(), true, true)
}

/// Incremental block writer. Input text is escaped; no caller-provided markup is trusted.
#[derive(Default)]
pub struct Writer {
    blocks: Vec<String>,
}

impl Writer {
    /// Create an empty extracted-text document.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a paragraph or heading (levels above six are clamped). Heading weight comes
    /// from its heading marker; callers may still supply italic runs.
    pub fn paragraph(&mut self, runs: &[(&str, bool, bool)], level: u8) {
        let text = paragraph(runs);
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.blocks.push(match level {
            0 => block_start(text.to_string()),
            l => format!("{} {text}", "#".repeat(usize::from(l.min(MAX_HEADING)))),
        });
    }

    /// Append a pipe table with the first row as its header. Short rows are padded to the
    /// widest row, and each span is clamped to 1..=MAX_SPAN. Callers bound input rows/cells.
    pub fn table<C: TableCell>(&mut self, rows: &[Vec<C>]) {
        let span = |c: &C| c.span().clamp(1, MAX_SPAN);
        let width = rows.iter().map(|r| r.iter().map(span).fold(0usize, usize::saturating_add)).max().unwrap_or(0);
        if width == 0 {
            return;
        }
        let mut lines = Vec::with_capacity(rows.len().saturating_add(1));
        for (i, row) in rows.iter().enumerate() {
            let mut cells: Vec<String> = Vec::with_capacity(width);
            for c in row {
                cells.push(cell(c));
                cells.extend(std::iter::repeat_n(String::new(), span(c).saturating_sub(1)));
            }
            cells.resize(width, String::new());
            lines.push(format!("| {} |", cells.join(" | ")));
            if i == 0 {
                lines.push(format!("|{}", " --- |".repeat(width)));
            }
        }
        self.blocks.push(lines.join("\n"));
    }

    /// Append a thematic break, including consecutive and leading page separators.
    pub fn page_break(&mut self) {
        self.blocks.push("---".into());
    }

    /// Join blocks with blank lines and finish with a newline (also for an empty document).
    pub fn finish(self) -> String {
        let mut out = self.blocks.join("\n\n");
        out.push('\n');
        out
    }
}
#[cfg(test)]
mod tests {
    use super::paragraph;

    #[test]
    fn empty_or_control_run_does_not_split_styled_runs() {
        assert_eq!(paragraph(&[("hello", true, false), ("", false, false), ("world", true, false)]), "**helloworld**");
        assert_eq!(paragraph(&[("hello", true, false), ("\u{1}", false, false), ("world", true, false)]), "**helloworld**");
    }

    #[test]
    fn empty_styled_run_does_not_affect_neighbor_emphasis() {
        assert_eq!(paragraph(&[("x", true, false), ("", false, true)]), "**x**");
        assert_eq!(paragraph(&[("foo", false, false), (".", true, false), ("", true, true), ("bar", false, false)]), "foo<strong>.</strong>bar");
        // Empty/control runs between differently styled runs still look ahead to the next non-empty run.
        assert_eq!(paragraph(&[("bold", true, false), ("", false, false), ("italic", false, true)]), "<strong>bold</strong>*italic*");
        assert_eq!(paragraph(&[("bold", true, false), ("\u{1}", false, false), (" italic", false, true)]), "**bold** *italic*");
        // Leading empty/control runs do not affect the opening delimiter.
        assert_eq!(paragraph(&[("", false, true), ("\u{1}", true, false), ("lead", true, false)]), "**lead**");
        // All-empty/control paragraphs produce empty output.
        assert_eq!(paragraph(&[("", true, false), ("\u{1}", false, true), ("\u{7f}", false, false)]), "");
    }
}

#[cfg(test)]
mod writer_tests {
    use super::*;

    struct Cell(&'static str, usize);
    impl TableCell for Cell {
        fn text(&self) -> &str {
            self.0
        }
        fn span(&self) -> usize {
            self.1
        }
    }

    #[test]
    fn preserves_block_spacing_heading_clamp_and_table_spans() {
        let mut w = Writer::new();
        w.page_break();
        w.paragraph(&[("12. item\nnext", false, false)], 0);
        w.paragraph(&[("Title", false, true)], 255);
        w.table(&[vec![Cell("A|B", 2)], vec![Cell("C", 0)]]);
        w.page_break();
        assert_eq!(w.finish(), "---\n\n12\\. item next\n\n###### *Title*\n\n| A\\|B |  |\n| --- | --- |\n| C |  |\n\n---\n");
        assert_eq!(Writer::new().finish(), "\n");
    }

    #[test]
    fn control_filter_and_literal_punctuation_stay_explicit() {
        assert_eq!(paragraph(&[("\0\u{7f}\u{fffe}\u{ffff}\n\r\t#!&", false, false)]), "   \\#\\!\\&");
        // Ordinary export retains its existing newline/control policy and #/! behavior.
        assert_eq!(crate::escaping::escape("\0\n#!\u{c}\u{e}&", false), "\0\n#!\\&");
    }
}
