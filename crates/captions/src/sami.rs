//! SAMI (`.smi`, `.sami`): Microsoft's HTML-like captions.
//!
//! ```text
//! <SAMI><BODY>
//! <SYNC Start=1000><P Class=ENCC>First line<br>second line
//! <SYNC Start=3500><P Class=ENCC>&nbsp;
//! </BODY></SAMI>
//! ```
//!
//! A `<SYNC Start=ms>` block shows its text from that millisecond until the next block; a block
//! holding only `&nbsp;` clears the screen. The last caption lasts four seconds if nothing clears
//! it. `<br>` starts a new line; `<i>`, `<b>`, `<u>` and `<font color>` are kept, other tags
//! dropped; `&amp;`, `&lt;`, `&gt;`, `&nbsp;` and numeric references are read. A file can hold
//! several language classes (`<P Class=ENCC>`, `<P Class=KRCC>`): the first one named is read and
//! the others skipped with a warning. Writing makes one `ENCC` class, an `&nbsp;` block where a
//! caption ends before the next starts, and rounds to milliseconds.

use crate::text::{escape_markup, last_cue_end, ms_to_tick, number, sequential, tick_to_ms};
use crate::time::Tick;
use crate::{Cue, Document, Error, Result};

/// The value of attribute `name` in the tag text `tag` (quotes optional).
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|r| r.find(name)) {
        let at = from + i;
        from = at + name.len();
        let before_ok = at == 0 || lower.as_bytes().get(at - 1).is_some_and(|b| !b.is_ascii_alphanumeric());
        let rest = tag.get(from..).unwrap_or("").trim_start();
        let Some(rest) = rest.strip_prefix('=').filter(|_| before_ok) else { continue };
        let rest = rest.trim_start();
        return Some(match rest.chars().next() {
            Some(q @ ('"' | '\'')) => {
                let inner = rest.get(1..).unwrap_or("");
                inner.split(q).next().unwrap_or("")
            }
            _ => rest.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or(""),
        });
    }
    None
}

/// A numeric character reference body (`#65`, `#x41`) as its character.
fn numeric_reference(body: &str) -> Option<char> {
    let digits = body.strip_prefix('#')?;
    let code = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse().ok()?,
    };
    char::from_u32(code).filter(|c| !c.is_control() || *c == '\n')
}

/// HTML text as cue text: entities the cue model keeps stay, numeric ones are decoded, a stray
/// `&` is escaped, whitespace runs collapse.
fn text_run(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    let mut space = false;
    while let Some(c) = rest.chars().next() {
        let mut advance = c.len_utf8();
        if c.is_whitespace() && c != '\u{a0}' {
            space = true;
            rest = rest.get(advance..).unwrap_or("");
            continue;
        }
        if space {
            out.push(' ');
        }
        space = false;
        if c == '&' {
            if let Some((_, len)) = crate::entity_at(rest) {
                out.push_str(rest.get(..len).unwrap_or(""));
                advance = len;
            } else if let Some(body) = rest.get(1..).and_then(|r| {
                // Numeric entities have at most eight body bytes. Never scan the
                // unbounded remainder again for each malformed ampersand.
                let end = r.bytes().take(9).position(|b| b == b';')?;
                r.get(..end)
            }) && let Some(ch) = numeric_reference(body)
            {
                out.push_str(&escape_markup(&ch.to_string()));
                advance = body.len() + 2;
            } else {
                out.push_str("&amp;");
            }
        } else {
            out.push_str(&escape_markup(&c.to_string()));
        }
        rest = rest.get(advance..).unwrap_or("");
    }
    if space {
        out.push(' ');
    }
    out
}

/// One `<SYNC>` block's body as (class, cue text) segments, one per `<P>`.
fn segments(body: &str) -> Vec<(String, String)> {
    let mut segs: Vec<(String, String)> = vec![(String::new(), String::new())];
    let mut open_fonts = 0usize;
    let mut rest = body;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            if let Some(last) = segs.last_mut() {
                last.1.push_str(&text_run(rest));
            }
            break;
        };
        if let Some(last) = segs.last_mut() {
            last.1.push_str(&text_run(rest.get(..lt).unwrap_or("")));
        }
        let after = rest.get(lt + 1..).unwrap_or("");
        let Some(gt) = after.find('>') else {
            if let Some(last) = segs.last_mut() {
                last.1.push_str("&lt;");
                last.1.push_str(&text_run(after));
            }
            break;
        };
        let tag = after.get(..gt).unwrap_or("");
        rest = after.get(gt + 1..).unwrap_or("");
        let closing = tag.starts_with('/');
        let name: String = tag.trim_start_matches('/').chars().take_while(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
        let Some(last) = segs.last_mut() else { continue };
        match (name.as_str(), closing) {
            ("br", _) => last.1.push('\n'),
            ("p", false) => segs.push((attribute(tag, "class").unwrap_or("").to_string(), String::new())),
            ("i" | "b" | "u", false) => last.1.push_str(&format!("<{name}>")),
            ("i" | "b" | "u", true) => last.1.push_str(&format!("</{name}>")),
            ("font", false) => {
                if let Some(c) = attribute(tag, "color").filter(|c| c.len() <= 20 && c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'#')) {
                    open_fonts += 1;
                    last.1.push_str(&format!("<font color=\"{c}\">"));
                }
            }
            ("font", true) if open_fonts > 0 => {
                open_fonts -= 1;
                last.1.push_str("</font>");
            }
            _ => {}
        }
    }
    segs
}

/// A caption text without the whitespace and non-breaking spaces round its lines.
fn tidy(text: &str) -> String {
    let lines: Vec<String> = text
        .split('\n')
        .map(|l| l.replace("&nbsp;", " ").trim().to_string())
        .filter(|l| !l.is_empty() && !crate::plain_text(l).trim().is_empty())
        .collect();
    lines.join("\n")
}

pub fn parse(text: &str) -> Result<Document> {
    let lower = text.to_ascii_lowercase();
    let mut starts: Vec<usize> = Vec::new();
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|r| r.find("<sync")) {
        starts.push(from + i);
        from += i + 5;
    }
    if starts.is_empty() {
        return if text.trim().is_empty() || lower.contains("<sami") { Ok(Document::default()) } else { Err(Error::NotFormat("SAMI")) };
    }
    let mut doc = Document::default();
    let mut class: Option<String> = None;
    let mut skipped_class = false;
    let mut syncs: Vec<(Tick, String)> = Vec::new();
    for (k, &at) in starts.iter().enumerate() {
        let stop = starts.get(k + 1).copied().unwrap_or(text.len());
        let chunk = text.get(at..stop).unwrap_or("");
        let chunk = match chunk.to_ascii_lowercase().find("</body") {
            Some(i) => chunk.get(..i).unwrap_or(chunk),
            None => chunk,
        };
        let Some((tag, body)) = chunk.get(1..).and_then(|c| c.split_once('>')) else { continue };
        let Some(start) = attribute(tag, "start").and_then(|v| number(v.trim())) else {
            doc.warnings.push("a <SYNC> without a Start time was skipped".to_string());
            continue;
        };
        let mut shown = String::new();
        for (cls, seg) in segments(body) {
            if class.is_none() && !cls.is_empty() {
                class = Some(cls.to_ascii_lowercase());
            }
            if class.as_deref().is_none_or(|c| c == cls.to_ascii_lowercase() || cls.is_empty()) {
                shown.push_str(&seg);
            } else {
                skipped_class = true;
            }
        }
        syncs.push((ms_to_tick(start), tidy(&shown)));
    }
    if skipped_class {
        doc.warnings.push("the file has other language classes; only the first was read".to_string());
    }
    syncs.sort_by_key(|(t, _)| *t);
    for (i, (start, text)) in syncs.iter().enumerate() {
        if text.is_empty() {
            continue;
        }
        let end = syncs.get(i + 1).map_or_else(|| last_cue_end(*start), |(next, _)| *next);
        if end <= *start {
            doc.warnings.push("two captions start at the same time; the first was skipped".to_string());
            continue;
        }
        doc.cues.push(Cue { start: *start, end, text: text.clone(), ..Default::default() });
    }
    Ok(doc)
}

pub fn write(doc: &Document) -> String {
    let cues = sequential(doc);
    let mut out = String::from(
        "<SAMI>\n<HEAD>\n<TITLE></TITLE>\n<STYLE TYPE=\"text/css\">\n<!--\nP { margin-left: 8pt; margin-right: 8pt; text-align: center; font-size: 20pt; color: white; }\n.ENCC { Name: English; lang: en-US; SAMIType: CC; }\n-->\n</STYLE>\n</HEAD>\n<BODY>\n",
    );
    let active_cues: Vec<(&Cue, Vec<&str>)> = cues
        .iter()
        .filter_map(|c| {
            let lines: Vec<&str> = c.text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            if lines.is_empty() { None } else { Some((c, lines)) }
        })
        .collect();
    let mut resolved_cues: Vec<(i64, i64, &Vec<&str>)> = Vec::with_capacity(active_cues.len());
    let mut prev_end = 0i64;
    for (c, lines) in &active_cues {
        let start_ms = tick_to_ms(c.start).max(prev_end);
        let end_ms = tick_to_ms(c.end).max(start_ms + 1);
        prev_end = end_ms;
        resolved_cues.push((start_ms, end_ms, lines));
    }
    for (i, &(start_ms, end_ms, lines)) in resolved_cues.iter().enumerate() {
        out.push_str(&format!("<SYNC Start={start_ms}><P Class=ENCC>{}\n", lines.join("<br>")));
        let next_start = resolved_cues.get(i + 1).map(|&(next_s, _, _)| next_s);
        if next_start != Some(end_ms) {
            out.push_str(&format!("<SYNC Start={end_ms}><P Class=ENCC>&nbsp;\n"));
        }
    }
    out.push_str("</BODY>\n</SAMI>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_clear_and_style() {
        let d = parse("<SAMI><BODY>\n<SYNC Start=1000><P Class=ENCC>Hello<br>world &amp; <i>you</i>\n<SYNC Start=3500><P Class=ENCC>&nbsp;\n<SYNC Start=4000><P Class=ENCC>x &#65; y\n</BODY></SAMI>").unwrap();
        assert_eq!(d.cues.len(), 2);
        assert_eq!(d.cues[0].text, "Hello\nworld &amp; <i>you</i>");
        assert_eq!(d.cues[0].end, ms_to_tick(3500));
        assert_eq!(d.cues[1].text, "x A y");
        assert_eq!(d.cues[1].end, last_cue_end(ms_to_tick(4000)));
    }

    #[test]
    fn malformed_entities_inside_valid_cues_are_bounded() {
        let raw = "&".repeat(32_768);
        let doc = parse(&format!("<SAMI><SYNC Start=0><P>{raw}<SYNC Start=1000><P>&nbsp;")).unwrap();
        assert_eq!(doc.cues.len(), 1);
        assert_eq!(doc.cues[0].text, "&amp;".repeat(32_768));
        assert_eq!(text_run("&#0000065; &#00000065; &#x41; &é;"), "A &amp;#00000065; A &amp;é;");
    }

    #[test]
    fn only_the_first_class_is_read() {
        let d = parse("<SAMI><BODY><SYNC Start=1000><P Class=ENCC>en<P Class=KRCC>kr<SYNC Start=2000><P Class=ENCC>&nbsp;</BODY></SAMI>").unwrap();
        assert_eq!(d.cues.len(), 1);
        assert_eq!(d.cues[0].text, "en");
        assert_eq!(d.warnings.len(), 1);
    }

    #[test]
    fn rejects_other_text() {
        assert!(parse("hello").is_err());
    }

    #[test]
    fn short_duration_and_empty_cues() {
        let doc = Document {
            cues: vec![
                Cue { start: ms_to_tick(100), end: Tick(ms_to_tick(100).0 + 1), text: "Brief".into(), ..Default::default() },
                Cue { start: ms_to_tick(101), end: ms_to_tick(200), text: "   ".into(), ..Default::default() },
                Cue { start: ms_to_tick(500), end: ms_to_tick(600), text: "After gap".into(), ..Default::default() },
            ],
            ..Default::default()
        };
        let written = write(&doc);
        assert!(written.contains("<SYNC Start=100><P Class=ENCC>Brief\n<SYNC Start=101><P Class=ENCC>&nbsp;\n"));
        let parsed = parse(&written).unwrap();
        assert_eq!(parsed.cues.len(), 2);
        assert_eq!(parsed.cues[0].start, ms_to_tick(100));
        assert_eq!(parsed.cues[0].end, ms_to_tick(101));
        assert_eq!(parsed.cues[1].start, ms_to_tick(500));
    }

    #[test]
    fn adjacent_short_cues_do_not_collide_after_rounding() {
        let t100 = ms_to_tick(100);
        let t100_1 = Tick(t100.0 + crate::TICKS_PER_MS / 10);
        let t200 = ms_to_tick(200);
        let doc = Document {
            cues: vec![
                Cue { start: t100, end: t100_1, text: "First".into(), ..Default::default() },
                Cue { start: t100_1, end: t200, text: "Second".into(), ..Default::default() },
            ],
            ..Default::default()
        };
        let encoded = write(&doc);
        let back = parse(&encoded).unwrap();
        assert_eq!(back.cues.len(), 2);
        assert_eq!(back.cues[0].text, "First");
        assert_eq!(tick_to_ms(back.cues[0].start), 100);
        assert_eq!(tick_to_ms(back.cues[0].end), 101);
        assert_eq!(back.cues[1].text, "Second");
        assert_eq!(tick_to_ms(back.cues[1].start), 101);
        assert_eq!(tick_to_ms(back.cues[1].end), 200);
    }
}
