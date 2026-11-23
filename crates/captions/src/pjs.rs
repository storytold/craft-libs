//! Phoenix Japanimation Society (PJS) subtitles (`.pjs`): times in tenths of a second.
//!
//! ```text
//! 10,50,"First line"
//! 60,120,"Second line|with break"
//! ```
//!
//! Lines are comma-separated `start,end,"text"` with times in deciseconds (tenths of a second).
//! Inside the quotes, `|` breaks a line.

use crate::text::{escape_markup, ms_to_tick, number, plain_lines, tick_to_ms};
use crate::{Cue, Document, Error, Result};

/// `start,end,"text"` as its parts.
pub(crate) fn split_cue(line: &str) -> Option<(i64, i64, &str)> {
    let (start_str, rest) = line.split_once(',')?;
    let (end_str, text_str) = rest.split_once(',')?;
    let start = number(start_str.trim())?;
    let end = number(end_str.trim())?;
    let text = text_str.trim();
    let text = if let Some(stripped) = text.strip_prefix('"').and_then(|t| t.strip_suffix('"')) { stripped } else { text };
    Some((start, end, text))
}

/// Parse a PJS subtitle document.
pub fn parse(text: &str) -> Result<Document> {
    let mut doc = Document::default();
    let mut found = false;
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((start_ds, end_ds, raw)) = split_cue(line) else {
            doc.warnings.push(format!("line {}: not a PJS cue, skipped", n + 1));
            continue;
        };
        found = true;
        let mut lines = Vec::new();
        for l in raw.split('|') {
            let trimmed = l.trim();
            if !trimmed.is_empty() {
                let unquoted = trimmed.replace("\"\"", "\"");
                lines.push(escape_markup(&unquoted));
            }
        }
        let start = ms_to_tick(start_ds.saturating_mul(100));
        let end = ms_to_tick(end_ds.saturating_mul(100));
        if end <= start || lines.is_empty() {
            doc.warnings.push(format!("line {}: empty or ends before it starts; skipped", n + 1));
            continue;
        }
        doc.cues.push(Cue { start, end, text: lines.join("\n"), ..Default::default() });
    }
    if !found && !text.trim().is_empty() {
        return Err(Error::NotFormat("PJS"));
    }
    Ok(doc)
}

/// Serialize a document into PJS format.
pub fn write(doc: &Document) -> String {
    let mut out = String::new();
    let ds = |t| (tick_to_ms(t) + 50) / 100;
    for c in &doc.cues {
        let start = ds(c.start);
        let end = ds(c.end).max(start + 1);
        let lines: Vec<String> =
            plain_lines(&c.text).into_iter().map(|l| l.replace('"', "\"\"").replace('|', "｜")).filter(|l| !l.is_empty()).collect();
        if !lines.is_empty() {
            out.push_str(&format!("{start},{end},\"{}\"\n", lines.join("|")));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_write_roundtrip() {
        let pjs = "10,50,\"First line\"\n60,120,\"Second line|with break\"\n130,150,\"Say \"\"Hi\"\"\"\n";
        let doc = parse(pjs).expect("parse");
        assert_eq!(doc.cues.len(), 3);
        assert_eq!(doc.cues[0].start, ms_to_tick(1000));
        assert_eq!(doc.cues[0].end, ms_to_tick(5000));
        assert_eq!(doc.cues[0].text, "First line");
        assert_eq!(doc.cues[1].start, ms_to_tick(6000));
        assert_eq!(doc.cues[1].end, ms_to_tick(12000));
        assert_eq!(doc.cues[1].text, "Second line\nwith break");
        assert_eq!(doc.cues[2].text, "Say \"Hi\"");

        let written = write(&doc);
        assert_eq!(written, pjs);
    }

    #[test]
    fn rejects_non_pjs() {
        assert!(parse("1\n00:00:01,000 --> 00:00:02,000\nHello\n").is_err());
    }
}
