//! VPlayer (`.txt`, `.vpl`): one start time per caption.
//!
//! ```text
//! 00:00:01.00=First line|second line
//! 00:00:04.00=
//! ```
//!
//! A line is `HH:MM:SS`, an optional `.cc` fraction, one of `:`, `=` or a space, and the text; `|`
//! starts a new line. The file gives only when each caption appears: it stays until the next
//! line, whose empty text clears it. The last caption lasts four seconds if no line ends it.
//! Writing puts a clearing line where a caption ends before the next one starts, drops tags,
//! rounds to a hundredth of a second and writes `|` in text as `｜`.

use crate::text::{escape_markup, last_cue_end, ms_to_tick, number, plain_lines, sequential, tick_to_ms};
use crate::time::Tick;
use crate::{Cue, Document, Error, Result};

/// `HH:MM:SS[.cc]` then a separator and text: the time and the text.
pub(crate) fn split_line(line: &str) -> Option<(Tick, &str)> {
    let mut parts = line.splitn(3, ':');
    let h = number(parts.next()?)?;
    let m = number(parts.next()?)?;
    let tail = parts.next()?;
    let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
    let s = number(tail.get(..digits)?)?;
    let mut rest = tail.get(digits..)?;
    let mut ms = 0;
    if let Some(frac) = rest.strip_prefix('.') {
        let n = frac.bytes().take_while(u8::is_ascii_digit).count().min(3);
        let f = frac.get(..n).filter(|f| !f.is_empty())?;
        ms = number(f)? * 10_i64.pow(3 - n as u32);
        rest = frac.get(n..)?;
        // further digits of a long fraction are below a millisecond
        rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    if m >= 60 || s >= 60 || h > 9999 {
        return None;
    }
    let text = rest.strip_prefix([':', '=', ' '])?;
    Some((ms_to_tick(((h * 60 + m) * 60 + s) * 1000 + ms), text))
}

pub fn parse(text: &str) -> Result<Document> {
    let mut doc = Document::default();
    let mut starts: Vec<(Tick, String)> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match split_line(line.trim()) {
            Some((t, raw)) => {
                let lines: Vec<String> = raw.split('|').map(str::trim).filter(|l| !l.is_empty()).map(escape_markup).collect();
                starts.push((t, lines.join("\n")));
            }
            None => doc.warnings.push(format!("line {}: not a VPlayer line, skipped", n + 1)),
        }
    }
    if starts.is_empty() && !text.trim().is_empty() {
        return Err(Error::NotFormat("VPlayer"));
    }
    for (i, (start, text)) in starts.iter().enumerate() {
        if text.is_empty() {
            continue;
        }
        let end = starts.get(i + 1).map_or_else(|| last_cue_end(*start), |(next, _)| *next);
        if end <= *start {
            doc.warnings.push("a caption is followed by a line at the same time; skipped".to_string());
            continue;
        }
        doc.cues.push(Cue { start: *start, end, text: text.clone(), ..Default::default() });
    }
    Ok(doc)
}

fn clock(t: Tick) -> String {
    let cs = (tick_to_ms(t) + 5) / 10;
    format!("{:02}:{:02}:{:02}.{:02}", cs / 360_000, (cs / 6000) % 60, (cs / 100) % 60, cs % 100)
}

pub fn write(doc: &Document) -> String {
    let cues = sequential(doc);
    let mut out = String::new();
    for (i, c) in cues.iter().enumerate() {
        let lines = plain_lines(&c.text);
        if lines.is_empty() {
            continue;
        }
        let lines: Vec<String> = lines.iter().map(|l| l.replace('|', "\u{ff5c}")).collect();
        out.push_str(&format!("{}={}\n", clock(c.start), lines.join("|")));
        let end = if clock(c.end) == clock(c.start) { Tick(c.end.0.saturating_add(crate::TICKS_PER_MS * 10)) } else { c.end };
        let next = cues.get(i + 1).map(|n| clock(n.start));
        if next.as_deref() != Some(clock(end).as_str()) {
            let next_too_early = cues.get(i + 1).is_some_and(|n| clock(n.start) <= clock(c.start));
            if !next_too_early {
                out.push_str(&format!("{}=\n", clock(end)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_duration_cue_preserves_positive_duration_on_write_and_read() {
        let mut doc = Document::default();
        // A cue from 0 to 1 ms collapses to 00:00:00.00 under centisecond rounding.
        doc.cues.push(Cue { start: ms_to_tick(0), end: ms_to_tick(1), text: "Short".to_string(), ..Default::default() });
        let written = write(&doc);
        assert_eq!(written, "00:00:00.00=Short\n00:00:00.01=\n");
        let parsed = parse(&written).unwrap();
        assert_eq!(parsed.cues.len(), 1);
        assert_eq!(parsed.cues[0].text, "Short");
        assert_eq!(parsed.cues[0].start, ms_to_tick(0));
        assert_eq!(parsed.cues[0].end, ms_to_tick(10));
    }

    #[test]
    fn separators_and_ends() {
        let d = parse("00:00:01:Hello|world\n00:00:04.50=\n00:00:05 later\n").unwrap();
        assert_eq!(d.cues.len(), 2);
        assert_eq!(d.cues[0].text, "Hello\nworld");
        assert_eq!(d.cues[0].end, ms_to_tick(4500));
        assert_eq!(d.cues[1].end, ms_to_tick(9000));
    }

    #[test]
    fn rejects_other_text() {
        assert!(parse("hello\n").is_err());
        assert!(parse("00:00:01,000 --> 00:00:02,000\n").is_err());
    }
}
