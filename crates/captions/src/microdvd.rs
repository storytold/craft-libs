//! MicroDVD (`.sub`): frame-numbered text.
//!
//! ```text
//! {1}{1}23.976
//! {24}{72}First line|{y:i}second line in italics
//! ```
//!
//! A line is `{first frame}{last frame}text`; `|` starts a new line. A first cue `{1}{1}` or `{0}{0}`
//! whose text is a number gives the frame rate; without it the rate is 23.976 fps (the rate other
//! readers assume). Control codes `{y:i}` / `{y:b}` / `{y:u}` (italic, bold, underline; `i,b` together)
//! and `{c:$BBGGRR}` (colour) style the line they start; the capital forms `{Y:…}` and `{C:…}`
//! style every later line of the cue too. Other controls (`{f:…}` font, `{s:…}` size, `{h:…}`, …)
//! are dropped. Writing always starts with the `{1}{1}` rate line and styles a line only when
//! `<i>`, `<b>` or `<u>` wraps all of it; other tags are dropped, and `{`, `}` and `|` in the
//! text become the full-width `｛`, `｝` and `｜`.

use crate::text::{LineStyle, escape_markup, number, plain_text, whole_line_style, wrap_style};
use crate::time::FrameRate;
use crate::{Cue, Document, Error, Result};

/// The frame rate of a file that does not name one.
pub const DEFAULT_RATE: FrameRate = FrameRate::FPS_23_976;

/// `{a}{b}text` as its parts.
fn split_cue(line: &str) -> Option<(i64, i64, &str)> {
    let rest = line.strip_prefix('{')?;
    let (a, rest) = rest.split_once('}')?;
    let rest = rest.strip_prefix('{')?;
    let (b, text) = rest.split_once('}')?;
    Some((number(a.trim())?, number(b.trim())?, text))
}

/// Whether `line` is a `{first}{last}text` cue line.
pub(crate) fn is_cue(line: &str) -> bool {
    split_cue(line).is_some()
}

/// The frame rate a header cue's text names.
fn header_rate(text: &str) -> Option<FrameRate> {
    let fps: f64 = text.trim().parse().ok()?;
    (fps.is_finite() && (1.0..=1000.0).contains(&fps)).then(|| FrameRate::from_f64(fps))
}

/// A `$BBGGRR` colour as `#rrggbb`.
fn colour(value: &str) -> Option<String> {
    let hex = value.trim().strip_prefix('$')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let (b, rest) = hex.split_at(2);
    let (g, r) = rest.split_at(2);
    Some(format!("#{}{}{}", r.to_ascii_lowercase(), g.to_ascii_lowercase(), b.to_ascii_lowercase()))
}

/// What the styling controls of a cue's lines set.
#[derive(Default)]
struct Sticky {
    style: LineStyle,
    colour: Option<String>,
}

/// The text of one line without its `{x:…}` controls, and the controls in order.
fn split_controls(line: &str) -> (String, Vec<(char, String)>) {
    let mut text = String::new();
    let mut controls = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('{') {
        text.push_str(rest.get(..open).unwrap_or(""));
        let after = rest.get(open + 1..).unwrap_or("");
        // Check the fixed-size control prefix before scanning its body. Invalid opening
        // braces must not repeatedly scan the same suffix for a closing brace.
        let name = after.as_bytes().first().copied().filter(u8::is_ascii_alphabetic);
        if let Some(name) = name.filter(|_| after.as_bytes().get(1) == Some(&b':')) {
            if let Some((body, tail)) = after.get(2..).and_then(|s| s.split_once('}')) {
                controls.push((char::from(name), body.to_string()));
                rest = tail;
            } else {
                // No later control can close either: preserve the entire remaining text.
                text.push('{');
                text.push_str(after);
                rest = "";
                break;
            }
        } else {
            text.push('{');
            rest = after;
        }
    }
    text.push_str(rest);
    (text, controls)
}

fn apply_style(value: &str, style: &mut LineStyle) {
    for flag in value.split([',', ' ', '/']) {
        match flag.trim().to_ascii_lowercase().as_str() {
            "i" => style.italic = true,
            "b" => style.bold = true,
            "u" => style.underline = true,
            _ => {}
        }
    }
}

fn cue_text(raw: &str) -> String {
    let mut sticky = Sticky::default();
    let mut lines = Vec::new();
    for line in raw.split('|') {
        let (text, controls) = split_controls(line);
        let mut style = sticky.style;
        let mut line_colour = sticky.colour.clone();
        for (name, value) in controls {
            match name {
                'y' => apply_style(&value, &mut style),
                'Y' => {
                    apply_style(&value, &mut style);
                    apply_style(&value, &mut sticky.style);
                }
                'c' => line_colour = colour(&value).or(line_colour),
                'C' => {
                    line_colour = colour(&value).or(line_colour);
                    sticky.colour = line_colour.clone();
                }
                _ => {}
            }
        }
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        lines.push(wrap_style(style, line_colour.as_deref(), &escape_markup(text)));
    }
    lines.join("\n")
}

pub fn parse(text: &str) -> Result<Document> {
    let mut rows = Vec::new();
    let mut rate = DEFAULT_RATE;
    let mut has_rate_line = false;
    let mut doc = Document::default();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match split_cue(line) {
            Some((0, 0, t) | (1, 1, t)) if rows.is_empty() && !has_rate_line && header_rate(t).is_some() => {
                rate = header_rate(t).unwrap_or(rate);
                has_rate_line = true;
            }
            Some(row) => rows.push(row),
            None => doc.warnings.push(format!("line {}: not a MicroDVD cue, skipped", n + 1)),
        }
    }
    if rows.is_empty() && !has_rate_line && !text.trim().is_empty() {
        return Err(Error::NotFormat("MicroDVD"));
    }
    for (first, last, raw) in rows {
        let (start, end) = (rate.tick_of(first), rate.tick_of(last));
        let text = cue_text(raw);
        if end <= start || text.is_empty() {
            doc.warnings.push(format!("the cue at frame {first} is empty or ends at frame {last}; skipped"));
            continue;
        }
        doc.cues.push(Cue { start, end, text, ..Default::default() });
    }
    Ok(doc)
}

/// Write `doc` with its frames counted at `rate` (None: [`DEFAULT_RATE`]).
pub fn write(doc: &Document, rate: Option<FrameRate>) -> String {
    let rate = rate.map_or(DEFAULT_RATE, FrameRate::sane);
    let mut out = format!("{{1}}{{1}}{:.3}\n", rate.as_f64());
    let frame = |t| rate.frame_at(rate.snap_nearest(t)).max(0);
    for c in &doc.cues {
        let first = frame(c.start);
        let last = frame(c.end).max(first.saturating_add(1));
        let mut lines = Vec::new();
        for line in c.text.lines() {
            let (style, inner) = whole_line_style(line);
            let text = plain_text(inner).trim().replace('{', "\u{ff5b}").replace('}', "\u{ff5d}").replace('|', "\u{ff5c}");
            if text.is_empty() {
                continue;
            }
            let mut flags = Vec::new();
            for (on, f) in [(style.italic, "i"), (style.bold, "b"), (style.underline, "u")] {
                if on {
                    flags.push(f);
                }
            }
            lines.push(if flags.is_empty() { text } else { format!("{{y:{}}}{text}", flags.join(",")) });
        }
        if lines.is_empty() {
            continue;
        }
        out.push_str(&format!("{{{first}}}{{{last}}}{}\n", lines.join("|")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_style_lines() {
        assert_eq!(cue_text("{y:i,b}x|plain"), "<i><b>x</b></i>\nplain");
        assert_eq!(cue_text("{Y:i}a|b"), "<i>a</i>\n<i>b</i>");
        assert_eq!(cue_text("{c:$0000ff}red"), "<font color=\"#ff0000\">red</font>");
        assert_eq!(cue_text("{f:Arial}{s:20}a & b {x"), "a &amp; b {x");
    }

    #[test]
    fn malformed_controls_inside_valid_cues_are_preserved() {
        for raw in ["{".repeat(32_768), "{y:".repeat(16_384)] {
            let doc = parse(&format!("{{0}}{{25}}{raw}")).unwrap();
            assert_eq!(doc.cues.len(), 1);
            assert_eq!(doc.cues[0].text, raw);
        }
        assert_eq!(cue_text("{{y:i}hello"), "<i>{hello</i>");
        assert_eq!(cue_text("{xx:no}{y:b}hello"), "<b>{xx:no}hello</b>");
    }

    #[test]
    fn rate_header() {
        let d = parse("{1}{1}25.000\n{25}{50}x\n").unwrap();
        assert_eq!(d.cues[0].start, FrameRate::FPS_25.tick_of(25));
        let d = parse("{0}{0}25.000\n{25}{50}x\n{50}{75}29.97\n").unwrap();
        assert_eq!(d.cues.len(), 2);
        assert_eq!(d.cues[0].start, FrameRate::FPS_25.tick_of(25));
        assert_eq!(d.cues[1].start, FrameRate::FPS_25.tick_of(50));
        assert_eq!(d.cues[1].text, "29.97");
        let d = parse("{25}{50}x\n").unwrap();
        assert_eq!(d.cues[0].start, DEFAULT_RATE.tick_of(25));
    }

    #[test]
    fn rejects_other_text() {
        assert!(parse("hello\nworld\n").is_err());
        assert!(parse("").unwrap().cues.is_empty());
    }
}
