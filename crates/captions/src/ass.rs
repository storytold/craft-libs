//! Advanced SubStation Alpha and SubStation Alpha (`.ass`, `.ssa`).
//!
//! Reading takes the `[Events]` section: its `Format:` line gives the field order (ASS v4+ and SSA
//! v4 differ in the first field only), each `Dialogue:` line becomes a cue; `Comment:` lines and
//! the other sections are skipped. A cue gets the `Name` field as its speaker. The text keeps what
//! the neutral model can show: `\N` and `\n` become a line break and a space, `\h` a no-break
//! space, and the override tags `\i`, `\b` and `\u` become `<i>`, `<b>` and `<u>`. Every other
//! override tag (colours, fonts, positions, karaoke, animation) is dropped, and a line that draws
//! vector shapes (`\p1`) is skipped with a warning.
//!
//! Writing gives a minimal ASS v4+ file: one `Default` style, one `Dialogue:` line per cue with
//! times to the centisecond. Braces in the text, which ASS reads as override blocks, are written
//! as the full-width `｛` and `｝`.

use crate::time::{TICKS_PER_SECOND, Tick};
use crate::{Cue, Document, Error, Result};

const TICKS_PER_CS: i64 = TICKS_PER_SECOND / 100;

/// `H:MM:SS.cc`, the form SSA and ASS use (any number of fraction digits is read).
fn parse_time(s: &str) -> Option<Tick> {
    let s = s.trim();
    let (hms, frac) = s.split_once('.').unwrap_or((s, ""));
    let mut parts = hms.split(':');
    let (h, m, sec) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let num = |p: &str| -> Option<i64> { (!p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit())).then(|| p.parse().ok()).flatten() };
    let (h, m, sec) = (num(h)?, num(m)?, num(sec)?);
    if m >= 60 || sec >= 60 || h > 9999 {
        return None;
    }
    let mut ticks = ((h * 60 + m) * 60 + sec) * TICKS_PER_SECOND;
    if !frac.is_empty() {
        if frac.len() > 9 || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let d: i64 = frac.parse().ok()?;
        ticks += (i128::from(d) * i128::from(TICKS_PER_SECOND) / 10i128.pow(frac.len() as u32)) as i64;
    }
    Some(Tick(ticks))
}

fn format_time(t: Tick) -> String {
    let cs = (i128::from(t.0.max(0)) + i128::from(TICKS_PER_CS) / 2) / i128::from(TICKS_PER_CS);
    let (h, m, s, c) = (cs / 360_000, (cs / 6000) % 60, (cs / 100) % 60, cs % 100);
    format!("{h}:{m:02}:{s:02}.{c:02}")
}

/// The Text field of a Dialogue line as cue text; `None` when the line draws shapes.
fn convert_text(raw: &str) -> Option<String> {
    let mut out = String::new();
    // Open `<i>`, `<b>`, `<u>` in the order they were opened.
    let mut open: Vec<char> = Vec::new();
    let mut chars = raw.chars().peekable();
    let mut drawing = false;
    let mut dropped_drawing = false;
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                let mut block = String::new();
                let mut closed = false;
                for b in chars.by_ref() {
                    if b == '}' {
                        closed = true;
                        break;
                    }
                    block.push(b);
                }
                if !closed {
                    // An unterminated block is literal text.
                    out.push('{');
                    out.push_str(&block);
                    continue;
                }
                for tag in block.split('\\').skip(1) {
                    let tag = tag.trim_end();
                    let (name, val) = tag.split_at(tag.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(tag.len()));
                    match name {
                        "i" | "b" | "u" => {
                            let letter = name.chars().next().unwrap_or('i');
                            let on = val.trim() != "0";
                            if on && !open.contains(&letter) {
                                open.push(letter);
                                out.push_str(&format!("<{letter}>"));
                            } else if !on && open.contains(&letter) {
                                // Close back to it, reopening what was opened after.
                                let at = open.iter().position(|&o| o == letter).unwrap_or(0);
                                let after: Vec<char> = open.split_off(at + 1);
                                for o in after.iter().rev() {
                                    out.push_str(&format!("</{o}>"));
                                }
                                open.pop();
                                out.push_str(&format!("</{letter}>"));
                                for o in after {
                                    open.push(o);
                                    out.push_str(&format!("<{o}>"));
                                }
                            }
                        }
                        "p" => drawing = val.trim().parse::<u32>().unwrap_or(0) != 0,
                        _ => {}
                    }
                }
            }
            '\\' => match chars.peek() {
                Some('N') => {
                    chars.next();
                    out.push('\n');
                }
                Some('n') => {
                    chars.next();
                    out.push(' ');
                }
                Some('h') => {
                    chars.next();
                    out.push('\u{a0}');
                }
                _ => out.push('\\'),
            },
            c => {
                if drawing {
                    dropped_drawing = true;
                } else {
                    out.push(c);
                }
            }
        }
    }
    if dropped_drawing && out.trim().is_empty() {
        return None;
    }
    while let Some(o) = open.pop() {
        out.push_str(&format!("</{o}>"));
    }
    Some(out)
}

pub fn parse(text: &str) -> Result<Document> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut doc = Document::default();
    let mut in_events = false;
    let mut saw_events = false;
    // Column of each field in a Dialogue line.
    let mut cols: Vec<String> =
        ["layer", "start", "end", "style", "name", "marginl", "marginr", "marginv", "effect", "text"].map(String::from).to_vec();
    let mut drawings = 0usize;
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_events = line.eq_ignore_ascii_case("[events]");
            saw_events |= in_events;
            continue;
        }
        if !in_events {
            continue;
        }
        let Some((key, rest)) = line.split_once(':') else { continue };
        let rest = rest.trim_start();
        match key.trim().to_ascii_lowercase().as_str() {
            "format" => cols = rest.split(',').map(|f| f.trim().to_ascii_lowercase()).collect(),
            "dialogue" => {
                // The last column (Text) keeps its commas.
                let fields: Vec<&str> = rest.splitn(cols.len(), ',').collect();
                let get = |name: &str| cols.iter().position(|c| c == name).and_then(|i| fields.get(i)).copied();
                let (Some(start), Some(end), Some(raw)) = (get("start").and_then(parse_time), get("end").and_then(parse_time), get("text")) else {
                    doc.warnings.push(format!("line {}: not a Dialogue line this reader understands", n + 1));
                    continue;
                };
                let Some(text) = convert_text(raw) else {
                    drawings += 1;
                    continue;
                };
                let name = get("name").map(str::trim).filter(|s| !s.is_empty());
                doc.cues.push(Cue { start, end, text, speaker: name.map(str::to_string), ..Default::default() });
            }
            _ => {}
        }
    }
    if !saw_events {
        return Err(Error::NotFormat("ASS"));
    }
    if drawings > 0 {
        doc.warnings.push(format!("{drawings} vector drawing line(s) were skipped"));
    }
    Ok(doc)
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("\r\n") {
            out.push_str("\\N");
            rest = r;
        } else if let Some(r) = rest.strip_prefix("<i>") {
            out.push_str("{\\i1}");
            rest = r;
        } else if let Some(r) = rest.strip_prefix("</i>") {
            out.push_str("{\\i0}");
            rest = r;
        } else if let Some(r) = rest.strip_prefix("<b>") {
            out.push_str("{\\b1}");
            rest = r;
        } else if let Some(r) = rest.strip_prefix("</b>") {
            out.push_str("{\\b0}");
            rest = r;
        } else if let Some(r) = rest.strip_prefix("<u>") {
            out.push_str("{\\u1}");
            rest = r;
        } else if let Some(r) = rest.strip_prefix("</u>") {
            out.push_str("{\\u0}");
            rest = r;
        } else if let Some(c) = rest.chars().next() {
            match c {
                '\n' | '\r' => out.push_str("\\N"),
                // Empty override block separates a literal slash from N/n/h.
                '\\' => out.push_str("\\{}"),
                '{' => out.push('\u{ff5b}'),
                '}' => out.push('\u{ff5d}'),
                '\u{a0}' => out.push_str("\\h"),
                c => out.push(c),
            }
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

pub fn write(doc: &Document) -> String {
    let mut out = String::from(
        "[Script Info]\n; Written by craft-captions\nScriptType: v4.00+\nWrapStyle: 0\nScaledBorderAndShadow: yes\nPlayResX: 384\nPlayResY: 288\n\n\
         [V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: Default,Arial,16,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,1\n\n\
         [Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );
    for c in &doc.cues {
        // A comma in the Name field would shift the columns; the other fields are fixed text.
        let name = c.speaker.as_deref().unwrap_or("").replace([',', '\n', '\r'], " ");
        out.push_str(&format!("Dialogue: 0,{},{},Default,{},0,0,0,,{}\n", format_time(c.start), format_time(c.end), name, escape(&c.text)));
    }
    out
}
