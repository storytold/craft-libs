//! SubViewer 2 (`.sub`): hundredths of a second, with an information header.
//!
//! ```text
//! [INFORMATION]
//! [TITLE]…
//! [END INFORMATION]
//! [SUBTITLE]
//! 00:00:01.00,00:00:03.50
//! First line[br]second line
//!
//! ```
//!
//! Reading skips the bracketed header lines and takes every `start,end` time line as a cue whose
//! text runs to the next blank line; `[br]` starts a new line. Fractions of one to three digits
//! are accepted. Writing adds a minimal header, rounds times to a hundredth of a second, drops
//! tags and writes `[br]` inside text as `［br］`.

use crate::text::{escape_markup, plain_lines, tick_to_ms};
use crate::time::Tick;
use crate::{Cue, Document, Error, Result, parse_clock};

/// `h:mm:ss.cc,h:mm:ss.cc` as its two times.
pub(crate) fn parse_timing(line: &str) -> Option<(Tick, Tick)> {
    let (a, b) = line.trim().split_once(',')?;
    let ok = |s: &str| s.contains(':') && s.contains('.');
    if !ok(a) || !ok(b) {
        return None;
    }
    Some((parse_clock(a)?, parse_clock(b)?))
}

/// Split `s` at each `[br]`, whatever its case.
fn split_br(s: &str) -> Vec<&str> {
    let lower = s.to_ascii_lowercase();
    let mut parts = Vec::new();
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|r| r.find("[br]")) {
        parts.push(s.get(from..from + i).unwrap_or(""));
        from += i + 4;
    }
    parts.push(s.get(from..).unwrap_or(""));
    parts
}

pub fn parse(text: &str) -> Result<Document> {
    let lines: Vec<&str> = text.lines().collect();
    let mut doc = Document::default();
    let mut found = false;
    let mut i = 0;
    while i < lines.len() {
        let Some((start, end)) = lines.get(i).and_then(|l| parse_timing(l)) else {
            i += 1;
            continue;
        };
        found = true;
        i += 1;
        let mut out: Vec<String> = Vec::new();
        while let Some(l) = lines.get(i).filter(|l| !l.trim().is_empty()) {
            if parse_timing(l).is_some() {
                break;
            }
            out.extend(split_br(l).into_iter().map(str::trim).filter(|p| !p.is_empty()).map(escape_markup));
            i += 1;
        }
        if end <= start || out.is_empty() {
            doc.warnings.push(format!("the cue at line {i} is empty or ends before it starts; skipped"));
            continue;
        }
        doc.cues.push(Cue { start, end, text: out.join("\n"), ..Default::default() });
    }
    let has_header = text.contains("[INFORMATION]") || text.contains("[SUBTITLE]");
    if !found && !has_header && !text.trim().is_empty() {
        return Err(Error::NotFormat("SubViewer"));
    }
    Ok(doc)
}

fn clock(t: Tick) -> String {
    let cs = (tick_to_ms(t) + 5) / 10;
    format!("{:02}:{:02}:{:02}.{:02}", cs / 360_000, (cs / 6000) % 60, (cs / 100) % 60, cs % 100)
}

pub fn write(doc: &Document) -> String {
    let mut out = String::from(
        "[INFORMATION]\n[TITLE]\n[AUTHOR]\n[SOURCE]\n[PRG]\n[FILEPATH]\n[DELAY]0\n[CD TRACK]0\n[COMMENT]\n[END INFORMATION]\n[SUBTITLE]\n[COLF]&HFFFFFF,[STYLE]no,[SIZE]18,[FONT]Arial\n",
    );
    for c in &doc.cues {
        let lines = plain_lines(&c.text);
        if lines.is_empty() {
            continue;
        }
        let end = if clock(c.end) == clock(c.start) { Tick(c.end.0.saturating_add(crate::TICKS_PER_MS * 10)) } else { c.end };
        out.push_str(&format!("{},{}\n", clock(c.start), clock(end)));
        let lines: Vec<String> = lines
            .iter()
            .map(|l| {
                l.replace("[br]", "\u{ff3b}br\u{ff3d}")
                    .replace("[BR]", "\u{ff3b}BR\u{ff3d}")
                    .replace("[Br]", "\u{ff3b}Br\u{ff3d}")
                    .replace("[bR]", "\u{ff3b}bR\u{ff3d}")
            })
            .collect();
        out.push_str(&lines.join("[br]"));
        out.push_str("\n\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_breaks() {
        let d = parse("[INFORMATION]\n[TITLE]x\n[END INFORMATION]\n[SUBTITLE]\n00:00:01.00,00:00:03.50\nHello[BR]world\n\n").unwrap();
        assert_eq!(d.cues[0].text, "Hello\nworld");
        assert_eq!(d.cues[0].end, crate::text::ms_to_tick(3500));
    }

    #[test]
    fn writer_escapes_every_case_of_literal_break_tokens() {
        let doc = Document {
            cues: vec![Cue { start: Tick(0), end: crate::text::ms_to_tick(1000), text: "a[br]b[BR]c[Br]d[bR]é".into(), ..Default::default() }],
            ..Default::default()
        };
        let decoded = parse(&write(&doc)).unwrap();
        assert_eq!(decoded.cues[0].text, "a［br］b［BR］c［Br］d［bR］é");
    }

    #[test]
    fn rejects_other_text() {
        assert!(parse("1\n00:00:01,000 --> 00:00:02,000\nx\n").is_err());
    }
}
