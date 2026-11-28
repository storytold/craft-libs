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
//! tags, writes `[br]` inside text as `［br］`, and writes `,` in lines that match the timing
//! syntax as `，`.

use crate::text::{escape_markup, plain_lines};
use crate::time::{TICKS_PER_SECOND, Tick};
use crate::{Cue, Document, Error, Result, parse_clock};

const TICKS_PER_CS: i64 = TICKS_PER_SECOND / 100;

/// Returns true if `head` begins with a genuine SubViewer header structure.
pub(crate) fn is_header(head: &str) -> bool {
    let mut lines = head.lines().map(str::trim).filter(|l| !l.is_empty());
    match lines.next() {
        Some("[INFORMATION]") => lines.any(|l| {
            l == "[SUBTITLE]"
                || l == "[END INFORMATION]"
                || l.starts_with("[TITLE")
                || l.starts_with("[AUTHOR")
                || l.starts_with("[SOURCE")
                || l.starts_with("[FILEPATH")
                || l.starts_with("[DELAY")
                || l.starts_with("[CD TRACK")
                || l.starts_with("[COMMENT")
                || (l.starts_with('[') && l.contains(']'))
        }),
        Some("[SUBTITLE]") => true,
        _ => false,
    }
}

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

fn cs(t: Tick) -> u64 {
    ((i128::from(t.0.max(0)) + i128::from(TICKS_PER_CS) / 2) / i128::from(TICKS_PER_CS)) as u64
}

fn clock(cs: u64) -> String {
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
        let start_cs = cs(c.start);
        let end_cs = cs(c.end).max(start_cs + 1);
        out.push_str(&format!("{},{}\n", clock(start_cs), clock(end_cs)));
        let lines: Vec<String> = lines
            .into_iter()
            .map(|mut l| {
                if parse_timing(&l).is_some() {
                    l = l.replacen(',', "\u{ff0c}", 1);
                }
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

    #[test]
    fn centisecond_rounding_without_double_rounding_error() {
        // 1.0046s is 255_184_473_600 ticks (at 254_016_000_000 ticks/sec).
        // Direct rounding to hundredths gives 100cs (00:00:01.00).
        // Double-rounding through ms gives 1005ms -> 101cs (00:00:01.01).
        let t = Tick(255_184_473_600);
        assert_eq!(clock(cs(t)), "00:00:01.00");
    }

    #[test]
    fn timing_shaped_text_escaped_and_roundtripped() {
        let doc = Document {
            cues: vec![
                Cue { start: Tick(TICKS_PER_SECOND), end: Tick(3 * TICKS_PER_SECOND), text: "00:00:01.00,00:00:02.00".into(), ..Default::default() },
                Cue { start: Tick(4 * TICKS_PER_SECOND), end: Tick(5 * TICKS_PER_SECOND), text: "Second cue".into(), ..Default::default() },
            ],
            ..Default::default()
        };
        let written = write(&doc);
        assert!(written.contains("00:00:01.00，00:00:02.00"));
        let parsed = parse(&written).unwrap();
        assert_eq!(parsed.cues.len(), 2);
        assert_eq!(parsed.cues[0].text, "00:00:01.00\u{ff0c}00:00:02.00");
        assert_eq!(parsed.cues[1].text, "Second cue");
    }

    #[test]
    fn literal_fullwidth_comma_preserved_on_read() {
        let raw = "[INFORMATION]\n[SUBTITLE]\n00:00:01.00,00:00:02.00\n00:00:01.00\u{ff0c}00:00:02.00\n\n";
        let doc = parse(raw).unwrap();
        assert_eq!(doc.cues[0].text, "00:00:01.00\u{ff0c}00:00:02.00");
    }
}
