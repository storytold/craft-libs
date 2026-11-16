//! JACOsub (`.jss`, `.js`): multi-feature scriptable subtitle format.
//!
//! ```text
//! # JACOsub file
//! #TimeRes 100
//! 0:00:01.00 0:00:03.00 D0 First line\nsecond line
//! ```
//!
//! Lines beginning with `#` are comments or header directives. Timed caption lines begin with
//! `START STOP [DIRECTIVE] Text`, where timestamps are `H:MM:SS.FF` (or `@units`) relative to
//! the declared `#TimeRes` (default 30). Inside the text, `\n` represents a newline.

use crate::text::{escape_markup, ms_to_tick, plain_text, tick_to_ms};
use crate::{Cue, Document, Error, Result};

/// Parse a JACOsub timestamp `H:MM:SS.FF` or `@units` into milliseconds.
pub(crate) fn parse_time(s: &str, time_res: i64) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    if let Some(units_str) = s.strip_prefix('@') {
        let units: i64 = units_str.trim().parse().ok()?;
        let time_res = if time_res > 0 { time_res } else { 30 };
        return Some(units.saturating_mul(1000) / time_res);
    }

    let (h_str, rest) = s.split_once(':')?;
    let (m_str, rest) = rest.split_once(':')?;
    let (s_str, f_str) = rest.split_once('.')?;

    let h: i64 = h_str.trim().parse().ok()?;
    let m: i64 = m_str.trim().parse().ok()?;
    let sec: i64 = s_str.trim().parse().ok()?;
    let frac: i64 = f_str.trim().parse().ok()?;

    let time_res = if time_res > 0 { time_res } else { 30 };
    let frac_ms = frac.saturating_mul(1000) / time_res;

    let total_secs = h.saturating_mul(3600).saturating_add(m.saturating_mul(60)).saturating_add(sec);
    Some(total_secs.saturating_mul(1000).saturating_add(frac_ms))
}

/// Format milliseconds into JACOsub timestamp string `H:MM:SS.cc` (at 100 centiseconds/sec).
pub(crate) fn format_time(ms: i64) -> String {
    let ms = ms.max(0);
    let total_secs = ms / 1000;
    let centis = (ms % 1000) / 10;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    format!("{hours}:{mins:02}:{secs:02}.{centis:02}")
}

/// Check if a line is a JACOsub timed caption line: `H:MM:SS.FF H:MM:SS.FF ...` or `@units @units ...`.
pub(crate) fn is_timed_line(line: &str) -> bool {
    let line = line.trim();
    if line.starts_with('#') {
        return false;
    }
    let mut parts = line.split_whitespace();
    let Some(t0) = parts.next() else { return false };
    let Some(t1) = parts.next() else { return false };
    (t0.contains(':') && t0.contains('.') && t1.contains(':') && t1.contains('.')) || (t0.starts_with('@') && t1.starts_with('@'))
}

pub fn parse(text: &str) -> Result<Document> {
    let mut doc = Document::default();
    let mut time_res: i64 = 30; // standard default
    let mut shift_ms: i64 = 0;
    let mut found = false;

    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(cmd) = line.strip_prefix('#') {
            let cmd_lower = cmd.to_ascii_lowercase();
            if cmd_lower.starts_with("timeres") || cmd_lower.starts_with('t') {
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                if parts.len() >= 2
                    && let Ok(res) = parts[1].parse::<i64>()
                    && res > 0
                {
                    time_res = res;
                }
            } else if cmd_lower.starts_with("shift") || cmd_lower.starts_with('s') {
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                if parts.len() >= 2 {
                    let s_val = parts[1].trim();
                    let (neg, s_raw) = if let Some(stripped) = s_val.strip_prefix('-') {
                        (true, stripped)
                    } else if let Some(stripped) = s_val.strip_prefix('+') {
                        (false, stripped)
                    } else {
                        (false, s_val)
                    };
                    if let Some(ms) = parse_time(s_raw, time_res) {
                        shift_ms = if neg { -ms } else { ms };
                    }
                }
            }
            continue;
        }

        if !is_timed_line(line) {
            continue;
        }

        let mut parts = line.split_whitespace();
        let Some(start_str) = parts.next() else { continue };
        let Some(end_str) = parts.next() else { continue };

        let start_ms = match parse_time(start_str, time_res) {
            Some(ms) => ms.saturating_add(shift_ms).max(0),
            None => {
                doc.warnings.push(format!("line {}: invalid start timestamp, skipped", n + 1));
                continue;
            }
        };
        let end_ms = match parse_time(end_str, time_res) {
            Some(ms) => ms.saturating_add(shift_ms).max(0),
            None => {
                doc.warnings.push(format!("line {}: invalid end timestamp, skipped", n + 1));
                continue;
            }
        };

        found = true;

        // Skip to after the end timestamp in the original line to preserve exact spaces
        let after_end_pos = match line.find(end_str) {
            Some(idx) => idx + end_str.len(),
            None => continue,
        };
        let remainder = &line[after_end_pos..];
        let trimmed_rem = remainder.trim_start();

        // Check if there is a directive prefix: either [Directive] or a single non-space word starting with D
        let text_part = if let Some(stripped) = trimmed_rem.strip_prefix('[') {
            match stripped.find(']') {
                Some(end_bracket) => stripped[end_bracket + 1..].trim_start(),
                None => trimmed_rem,
            }
        } else if let Some(space_idx) = trimmed_rem.find(' ') {
            let first_token = &trimmed_rem[..space_idx];
            if first_token.starts_with('D') && first_token[1..].chars().all(|c| c.is_ascii_digit()) {
                trimmed_rem[space_idx + 1..].trim_start()
            } else {
                trimmed_rem
            }
        } else {
            trimmed_rem
        };

        // Text parsing: `\n` represents newline
        let raw_lines: Vec<&str> = text_part.split(r"\n").map(str::trim).filter(|l| !l.is_empty()).collect();
        let lines: Vec<String> = raw_lines.into_iter().map(escape_markup).collect();

        let start = ms_to_tick(start_ms);
        let end = ms_to_tick(end_ms);
        if end <= start || lines.is_empty() {
            doc.warnings.push(format!("line {}: empty or ends before it starts; skipped", n + 1));
            continue;
        }

        doc.cues.push(Cue { start, end, text: lines.join("\n"), ..Default::default() });
    }

    if !found && !text.trim().is_empty() {
        return Err(Error::NotFormat("JACOsub"));
    }

    Ok(doc)
}

pub fn write(doc: &Document) -> String {
    let mut out = String::new();
    out.push_str("# JACOsub file\n");
    out.push_str("#TimeRes 100\n\n");

    for c in &doc.cues {
        let start_ms = tick_to_ms(c.start);
        let end_ms = tick_to_ms(c.end).max(start_ms + 10);
        let raw_lines = plain_text(&c.text);
        let lines: Vec<&str> = raw_lines.lines().map(str::trim).filter(|l| !l.is_empty()).collect();

        if !lines.is_empty() {
            let text = lines.join(r"\n");
            out.push_str(&format!("{} {} D0 {}\n", format_time(start_ms), format_time(end_ms), text));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_write_roundtrip() {
        let jss = "# JACOsub file\n#TimeRes 100\n\n0:00:01.00 0:00:03.00 D0 First line\\nsecond line\n0:00:04.50 0:00:06.00 D0 Second cue\n";
        let doc = parse(jss).expect("parse");
        assert_eq!(doc.cues.len(), 2);
        assert_eq!(doc.cues[0].start, ms_to_tick(1000));
        assert_eq!(doc.cues[0].end, ms_to_tick(3000));
        assert_eq!(doc.cues[0].text, "First line\nsecond line");
        assert_eq!(doc.cues[1].start, ms_to_tick(4500));
        assert_eq!(doc.cues[1].end, ms_to_tick(6000));
        assert_eq!(doc.cues[1].text, "Second cue");

        let written = write(&doc);
        assert_eq!(written, jss);
    }

    #[test]
    fn parse_bracketed_directive() {
        let jss = "#TimeRes 100\n0:00:01.00 0:00:03.00 [Style1] Bracketed style text\n";
        let doc = parse(jss).expect("parse");
        assert_eq!(doc.cues.len(), 1);
        assert_eq!(doc.cues[0].text, "Bracketed style text");
    }

    #[test]
    fn negative_shift_under_one_hour() {
        let jss = "#TimeRes 100\n#Shift -0:00:01.00\n0:00:02.00 0:00:04.00 D0 Shifted line\n";
        let doc = parse(jss).expect("parse");
        assert_eq!(doc.cues.len(), 1);
        assert_eq!(doc.cues[0].start, ms_to_tick(1000));
        assert_eq!(doc.cues[0].end, ms_to_tick(3000));
    }

    #[test]
    fn rejects_non_jacosub() {
        assert!(parse("1\n00:00:01,000 --> 00:00:02,000\nHello\n").is_err());
    }
}
