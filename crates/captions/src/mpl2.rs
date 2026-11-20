//! MPL2 (`.txt`, `.mpl`): times in tenths of a second.
//!
//! ```text
//! [10][35]First line|/second line in italics
//! ```
//!
//! A line is `[start][end]text` with times in deciseconds. `|` starts a new line; a `/` at the
//! start of a line makes it italic. Writing marks a line italic only when `<i>` wraps all of it,
//! drops other tags, rounds times to a tenth of a second, writes `|` in text as `｜`, and writes
//! a leading `/` in non-italic text as `／`.

use crate::text::{escape_markup, ms_to_tick, number, plain_text, tick_to_ms, whole_line_style, wrap_style};
use crate::{Cue, Document, Error, Result};

/// `[a][b]text` as its parts.
pub(crate) fn split_cue(line: &str) -> Option<(i64, i64, &str)> {
    let rest = line.strip_prefix('[')?;
    let (a, rest) = rest.split_once(']')?;
    let rest = rest.strip_prefix('[')?;
    let (b, text) = rest.split_once(']')?;
    Some((number(a.trim())?, number(b.trim())?, text))
}

pub fn parse(text: &str) -> Result<Document> {
    let mut doc = Document::default();
    let mut found = false;
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((a, b, raw)) = split_cue(line) else {
            doc.warnings.push(format!("line {}: not an MPL2 cue, skipped", n + 1));
            continue;
        };
        found = true;
        let mut lines = Vec::new();
        for l in raw.split('|') {
            let (italic, l) = match l.trim().strip_prefix('/') {
                Some(r) => (true, r.trim().to_string()),
                None => {
                    let t = l.trim();
                    if let Some(rest) = t.strip_prefix('\u{ff0f}') { (false, format!("/{rest}")) } else { (false, t.to_string()) }
                }
            };
            if !l.is_empty() {
                lines.push(wrap_style(crate::text::LineStyle { italic, ..Default::default() }, None, &escape_markup(&l)));
            }
        }
        let (start, end) = (ms_to_tick(a.saturating_mul(100)), ms_to_tick(b.saturating_mul(100)));
        if end <= start || lines.is_empty() {
            doc.warnings.push(format!("line {}: empty or ends before it starts; skipped", n + 1));
            continue;
        }
        doc.cues.push(Cue { start, end, text: lines.join("\n"), ..Default::default() });
    }
    if !found && !text.trim().is_empty() {
        return Err(Error::NotFormat("MPL2"));
    }
    Ok(doc)
}

pub fn write(doc: &Document) -> String {
    let mut out = String::new();
    let ds = |t| (tick_to_ms(t) + 50) / 100;
    for c in &doc.cues {
        let start = ds(c.start);
        let end = ds(c.end).max(start + 1);
        let mut lines = Vec::new();
        for line in c.text.lines() {
            let (style, inner) = whole_line_style(line);
            let mut text = plain_text(inner).trim().replace('|', "\u{ff5c}");
            if !style.italic && text.starts_with('/') {
                text.replace_range(..1, "\u{ff0f}");
            }
            if !text.is_empty() {
                lines.push(if style.italic { format!("/{text}") } else { text });
            }
        }
        if !lines.is_empty() {
            out.push_str(&format!("[{start}][{end}]{}\n", lines.join("|")));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn italics_and_lines() {
        let d = parse("[10][35]Hello|/italic\n").unwrap();
        assert_eq!(d.cues[0].text, "Hello\n<i>italic</i>");
        assert_eq!(d.cues[0].end, ms_to_tick(3500));
    }

    #[test]
    fn rejects_other_text() {
        assert!(parse("1\n00:00:01,000 --> 00:00:02,000\nx\n").is_err());
    }

    #[test]
    fn non_italic_leading_slash_roundtrip() {
        let doc = Document {
            cues: vec![Cue { start: ms_to_tick(1000), end: ms_to_tick(2000), text: "/not italic\n<i>/italic</i>".into(), ..Default::default() }],
            ..Default::default()
        };
        let written = write(&doc);
        assert_eq!(written, "[10][20]／not italic|//italic\n");
        let parsed = parse(&written).unwrap();
        assert_eq!(parsed.cues[0].text, "/not italic\n<i>/italic</i>");
    }
}
