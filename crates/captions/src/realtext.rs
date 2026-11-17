//! RealText (`.rt`): streaming HTML-like timed text markup.
//!
//! ```text
//! <window type="generic" width="320" height="60" wordwrap="true">
//! <time begin="00:01.00" end="00:03.00"/>First line<br/>second line
//! </window>
//! ```
//!
//! RealText documents encapsulate timed text inside a `<window>` container with `<time begin="…" [end="…"]/>`
//! markers. Inside each caption block, `<br/>` and `<br>` represent line breaks.

use crate::text::{escape_markup, last_cue_end, ms_to_tick, plain_lines, plain_text, tick_to_ms};
use crate::{Cue, Document, Error, Result};

/// Parse a RealText timestamp string into milliseconds.
/// Supports `SS[.fff]`, `MM:SS[.fff]`, and `HH:MM:SS[.fff]`.
pub(crate) fn parse_time(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let parts: Vec<&str> = s.split(':').collect();
    let (hours, mins, secs_str) = match parts.len() {
        1 => (0i64, 0i64, parts[0]),
        2 => (0i64, parts[0].parse().ok()?, parts[1]),
        3 => (parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2]),
        _ => return None,
    };

    let (secs, ms) = if let Some((sec_p, frac_p)) = secs_str.split_once('.') {
        let sec: i64 = sec_p.parse().ok()?;
        let frac_str = frac_p.trim_end();
        let ms = match frac_str.len() {
            0 => 0i64,
            1 => frac_str.parse::<i64>().ok()?.saturating_mul(100),
            2 => frac_str.parse::<i64>().ok()?.saturating_mul(10),
            3 => frac_str.parse::<i64>().ok()?,
            _ => {
                let first3 = frac_str.get(..3)?;
                first3.parse::<i64>().ok()?
            }
        };
        (sec, ms)
    } else {
        (secs_str.parse().ok()?, 0i64)
    };

    let total_ms = hours.saturating_mul(3600).saturating_add(mins.saturating_mul(60)).saturating_add(secs).saturating_mul(1000).saturating_add(ms);

    Some(total_ms)
}

/// Format milliseconds into RealText timestamp string `MM:SS.ss` or `HH:MM:SS.ss`.
pub(crate) fn format_time(ms: i64) -> String {
    let ms = ms.max(0);
    let total_secs = ms / 1000;
    let centis = (ms % 1000) / 10;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if hours > 0 { format!("{hours:02}:{mins:02}:{secs:02}.{centis:02}") } else { format!("{mins:02}:{secs:02}.{centis:02}") }
}

/// Extract attribute value from a tag (e.g., `begin="00:01.00"`).
fn attr_value<'a>(tag: &'a str, attr_name: &str) -> Option<&'a str> {
    let mut rest = tag;
    while let Some(idx) = rest.find(attr_name) {
        let candidate = &rest[idx + attr_name.len()..];
        let trimmed = candidate.trim_start();
        if let Some(after_eq) = trimmed.strip_prefix('=') {
            let after_eq = after_eq.trim_start();
            let quote = after_eq.chars().next()?;
            if quote == '"' || quote == '\'' {
                let val_start = 1;
                let val_end = after_eq[1..].find(quote)?;
                return Some(&after_eq[val_start..1 + val_end]);
            }
        }
        rest = &rest[idx + attr_name.len()..];
    }
    None
}

fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let needle_bytes = needle.as_bytes();
    if needle_bytes.is_empty() {
        return Some(0);
    }
    let haystack_bytes = haystack.as_bytes();
    if haystack_bytes.len() < needle_bytes.len() {
        return None;
    }
    haystack_bytes.windows(needle_bytes.len()).position(|window| window.iter().zip(needle_bytes).all(|(&h, &n)| h.eq_ignore_ascii_case(&n)))
}

/// Check if text is a RealText file.
pub(crate) fn is_realtext(text: &str) -> bool {
    let trimmed = text.trim_start();
    let lower_prefix: String = trimmed.chars().take(100).flat_map(|c| c.to_lowercase()).collect();
    if lower_prefix.starts_with("<window") {
        return true;
    }
    find_case_insensitive(text, "<window").is_some() && find_case_insensitive(text, "<time").is_some()
}

pub fn parse(text: &str) -> Result<Document> {
    if !is_realtext(text) {
        return Err(Error::NotFormat("RealText"));
    }

    let mut doc = Document::default();
    struct RawCue<'a> {
        begin: i64,
        end: Option<i64>,
        body: &'a str,
    }

    let mut raw_cues: Vec<RawCue> = Vec::new();
    let mut search_pos = 0;

    while let Some(time_idx) = find_case_insensitive(&text[search_pos..], "<time") {
        let actual_idx = search_pos + time_idx;
        let tag_end = match text[actual_idx..].find('>') {
            Some(end) => actual_idx + end + 1,
            None => break,
        };

        let tag_str = &text[actual_idx..tag_end];
        let begin = attr_value(tag_str, "begin").and_then(parse_time);
        let end = attr_value(tag_str, "end").and_then(parse_time);

        if let Some(begin_ms) = begin {
            // Text runs from tag_end to next '<time' or '</window>' or EOF
            let remaining = &text[tag_end..];
            let body_end =
                find_case_insensitive(remaining, "<time").or_else(|| find_case_insensitive(remaining, "</window>")).unwrap_or(remaining.len());

            let body = &text[tag_end..tag_end + body_end];
            raw_cues.push(RawCue { begin: begin_ms, end, body });
            search_pos = tag_end;
        } else {
            search_pos = tag_end;
        }
    }

    for i in 0..raw_cues.len() {
        let rc = &raw_cues[i];
        let start = ms_to_tick(rc.begin);
        let end = if let Some(explicit_end) = rc.end {
            ms_to_tick(explicit_end)
        } else if i + 1 < raw_cues.len() {
            ms_to_tick(raw_cues[i + 1].begin)
        } else {
            last_cue_end(start)
        };

        // Convert <br/> and <br> to newlines, decode markup to clean text
        let normalized = rc.body.replace("<br/>", "\n").replace("<br>", "\n").replace("<BR/>", "\n").replace("<BR>", "\n");
        let plain = plain_text(&normalized);
        let lines: Vec<&str> = plain.lines().map(str::trim).filter(|l| !l.is_empty()).collect();

        if end <= start || lines.is_empty() {
            continue;
        }

        let escaped_lines: Vec<String> = lines.into_iter().map(escape_markup).collect();
        doc.cues.push(Cue { start, end, text: escaped_lines.join("\n"), ..Default::default() });
    }

    if doc.cues.is_empty() && !text.trim().is_empty() {
        return Err(Error::NotFormat("RealText"));
    }

    Ok(doc)
}

pub fn write(doc: &Document) -> String {
    let mut out = String::new();
    out.push_str("<window type=\"generic\" width=\"320\" height=\"60\" wordwrap=\"true\">\n");

    for c in &doc.cues {
        let start_ms = tick_to_ms(c.start);
        let mut end_ms = tick_to_ms(c.end);
        if (end_ms / 10) <= (start_ms / 10) {
            end_ms = start_ms.saturating_add(10);
        }
        let lines: Vec<String> = plain_lines(&c.text).into_iter().map(|l| escape_markup(&l)).filter(|l| !l.is_empty()).collect();

        if !lines.is_empty() {
            let body = lines.join("<br/>");
            out.push_str(&format!("<time begin=\"{}\" end=\"{}\"/>{}\n", format_time(start_ms), format_time(end_ms), body));
        }
    }

    out.push_str("</window>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_write_roundtrip() {
        let rt = "<window type=\"generic\" width=\"320\" height=\"60\" wordwrap=\"true\">\n<time begin=\"00:01.00\" end=\"00:03.00\"/>Hello world\n<time begin=\"00:04.50\" end=\"00:06.25\"/>Second cue<br/>with newline\n</window>\n";
        let doc = parse(rt).expect("parse");
        assert_eq!(doc.cues.len(), 2);
        assert_eq!(doc.cues[0].start, ms_to_tick(1000));
        assert_eq!(doc.cues[0].end, ms_to_tick(3000));
        assert_eq!(doc.cues[0].text, "Hello world");
        assert_eq!(doc.cues[1].start, ms_to_tick(4500));
        assert_eq!(doc.cues[1].end, ms_to_tick(6250));
        assert_eq!(doc.cues[1].text, "Second cue\nwith newline");

        let written = write(&doc);
        let doc2 = parse(&written).expect("parse written");
        assert_eq!(doc2.cues.len(), 2);
        assert_eq!(doc2.cues[0].text, doc.cues[0].text);
        assert_eq!(doc2.cues[1].text, doc.cues[1].text);
    }

    #[test]
    fn parse_timestamps() {
        assert_eq!(parse_time("5"), Some(5000));
        assert_eq!(parse_time("5.5"), Some(5500));
        assert_eq!(parse_time("00:02.50"), Some(2500));
        assert_eq!(parse_time("01:02.500"), Some(62500));
        assert_eq!(parse_time("01:02:03.40"), Some(3723400));
    }

    #[test]
    fn does_not_misidentify_embedded_window_in_pjs() {
        let pjs = "10,50,\"Use <window> here\"\n";
        assert!(!is_realtext(pjs));
    }

    #[test]
    fn short_cue_quantization_roundtrip() {
        let mut doc = Document::default();
        doc.cues.push(Cue { start: ms_to_tick(1000), end: ms_to_tick(1001), text: "Short".into(), ..Default::default() });
        let written = write(&doc);
        let parsed = parse(&written).expect("parse short cue document");
        assert_eq!(parsed.cues.len(), 1);
        assert_eq!(parsed.cues[0].text, "Short");
    }
}
