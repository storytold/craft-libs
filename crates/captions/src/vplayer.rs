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

use crate::text::{escape_markup, last_cue_end, ms_to_tick, number, plain_lines, sequential};
use crate::time::{TICKS_PER_SECOND, Tick};
use crate::{Cue, Document, Error, Result};

const TICKS_PER_CS: i64 = TICKS_PER_SECOND / 100;

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

fn cs(t: Tick) -> i64 {
    ((i128::from(t.0.max(0)) + i128::from(TICKS_PER_CS) / 2) / i128::from(TICKS_PER_CS)) as i64
}

fn clock(cs: i64) -> String {
    format!("{:02}:{:02}:{:02}.{:02}", cs / 360_000, (cs / 6000) % 60, (cs / 100) % 60, cs % 100)
}

pub fn write(doc: &Document) -> String {
    let cues = sequential(doc);
    let mut out = String::new();
    let active_cues: Vec<(&Cue, Vec<String>)> = cues
        .iter()
        .filter_map(|c| {
            let lines = plain_lines(&c.text);
            if lines.is_empty() {
                None
            } else {
                let lines = lines.into_iter().map(|l| l.replace('|', "\u{ff5c}")).collect();
                Some((c, lines))
            }
        })
        .collect();

    let mut resolved_cues: Vec<(i64, i64, &Vec<String>)> = Vec::with_capacity(active_cues.len());
    let mut prev_end = 0i64;
    for (c, lines) in &active_cues {
        let start_cs = cs(c.start).max(prev_end);
        let end_cs = cs(c.end).max(start_cs + 1);
        prev_end = end_cs;
        resolved_cues.push((start_cs, end_cs, lines));
    }

    for (i, &(start_cs, end_cs, lines)) in resolved_cues.iter().enumerate() {
        out.push_str(&format!("{}={}\n", clock(start_cs), lines.join("|")));
        let next_start_cs = resolved_cues.get(i + 1).map(|&(next_s, _, _)| next_s);
        if next_start_cs != Some(end_cs) {
            out.push_str(&format!("{}=\n", clock(end_cs)));
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
    fn adjacent_short_cues_do_not_collide_after_rounding() {
        let doc = Document {
            cues: vec![
                Cue { start: ms_to_tick(1000), end: ms_to_tick(1001), text: "First".to_string(), ..Default::default() },
                Cue { start: ms_to_tick(1001), end: ms_to_tick(1020), text: "Second".to_string(), ..Default::default() },
            ],
            ..Default::default()
        };
        let written = write(&doc);
        assert_eq!(written, "00:00:01.00=First\n00:00:01.01=Second\n00:00:01.02=\n");
        let parsed = parse(&written).unwrap();
        assert_eq!(parsed.cues.len(), 2);
        assert_eq!(parsed.cues[0].text, "First");
        assert_eq!(parsed.cues[0].start, ms_to_tick(1000));
        assert_eq!(parsed.cues[0].end, ms_to_tick(1010));
        assert_eq!(parsed.cues[1].text, "Second");
        assert_eq!(parsed.cues[1].start, ms_to_tick(1010));
        assert_eq!(parsed.cues[1].end, ms_to_tick(1020));
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

    #[test]
    fn empty_cue_skipped_and_numerical_timestamp_order() {
        let doc = Document {
            cues: vec![
                Cue {
                    start: Tick(99 * 3600 * TICKS_PER_SECOND),
                    end: Tick(99 * 3600 * TICKS_PER_SECOND + 2 * TICKS_PER_SECOND),
                    text: "At 99h".into(),
                    ..Default::default()
                },
                Cue {
                    start: Tick(99 * 3600 * TICKS_PER_SECOND + 3 * TICKS_PER_SECOND),
                    end: Tick(99 * 3600 * TICKS_PER_SECOND + 4 * TICKS_PER_SECOND),
                    text: "  ".into(),
                    ..Default::default()
                },
                Cue {
                    start: Tick(100 * 3600 * TICKS_PER_SECOND),
                    end: Tick(100 * 3600 * TICKS_PER_SECOND + 2 * TICKS_PER_SECOND),
                    text: "At 100h".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let written = write(&doc);
        assert_eq!(written, "99:00:00.00=At 99h\n99:00:02.00=\n100:00:00.00=At 100h\n100:00:02.00=\n");
        let parsed = parse(&written).unwrap();
        assert_eq!(parsed.cues.len(), 2);
        assert_eq!(parsed.cues[0].text, "At 99h");
        assert_eq!(parsed.cues[1].text, "At 100h");
    }
}
