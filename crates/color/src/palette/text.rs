//! GIMP `.gpl` palettes and CSS custom properties.

use super::reader::*;
use super::*;

// ---------------------------------------------------------------- .gpl and .css

fn one_line(s: &str) -> String {
    s.split(['\n', '\r', '\t']).collect::<Vec<_>>().join(" ").trim().to_string()
}

pub(super) fn rgb8(c: [f32; 3]) -> [u8; 3] {
    c.map(|v| (unit(v) * 255.0).round() as u8)
}

pub(super) fn read_gpl(bytes: &[u8], file_name: &str) -> Vec<SwatchGroup> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}');
    let mut groups: Vec<SwatchGroup> = Vec::new();
    let mut seen = 0usize;
    let mut grouped = false;
    for line in text.lines().skip(1) {
        let line = line.trim();
        if let Some(g) = line.strip_prefix(GPL_GROUP) {
            let name = clip_name(g);
            groups.push(SwatchGroup { name: if name.is_empty() { "Color Group".into() } else { name }, entries: Vec::new() });
            grouped = true;
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let rgb: Vec<u8> = words.iter().take(3).map_while(|v| v.parse().ok()).collect();
        let [r, g, b] = rgb[..] else { continue };
        let entry = SwatchEntry::new(entry_name(seen, &words[3..].join(" ")), SwatchColor::Rgb([r, g, b].map(|v| f32::from(v) / 255.0)));
        seen += 1;
        if !grouped && groups.is_empty() {
            groups.push(SwatchGroup { name: stem_of(file_name), entries: Vec::new() });
        }
        if let Some(group) = groups.last_mut() {
            group.entries.push(entry);
        }
    }
    groups
}

pub(super) fn write_gpl(groups: &[SwatchGroup], colors: &[Vec<(String, [u8; 3])>]) -> String {
    let title = groups.first().map_or_else(String::new, |g| one_line(&g.name));
    let mut out = format!("{GPL_HEADER}\nName: {title}\nColumns: 0\n#\n");
    for (group, list) in groups.iter().zip(colors) {
        out.push_str(&format!("{GPL_GROUP} {}\n", one_line(&group.name)));
        for (name, [r, g, b]) in list {
            out.push_str(&format!("{r:3} {g:3} {b:3}\t{}\n", one_line(name)));
        }
    }
    out
}

/// `name` as a CSS custom property (`--` and an escaped identifier): lower case, spaces as
/// hyphens, other characters an identifier can't hold escaped with a backslash.
pub(super) fn css_property(name: &str) -> String {
    let mut out = String::from("--");
    for word in name.split_whitespace() {
        if out.len() > 2 {
            out.push('-');
        }
        for ch in word.chars().flat_map(char::to_lowercase) {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || !ch.is_ascii() {
                out.push(ch);
            } else if ch.is_ascii_graphic() {
                out.push('\\');
                out.push(ch);
            } else {
                out.push_str(&format!("\\{:x} ", ch as u32));
            }
        }
    }
    if out.len() == 2 {
        out.push_str("swatch");
    }
    out
}

/// A comment's text: `*/` can't end it early.
fn css_comment(s: &str) -> String {
    one_line(s).replace("*/", "* /")
}

pub(super) fn write_css(groups: &[SwatchGroup], colors: &[Vec<RgbSwatch>]) -> String {
    let mut out = String::from(":root {\n");
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    // The next number to try for each base, so repeated names don't search the ones used before.
    let mut next: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (group, list) in groups.iter().zip(colors) {
        out.push_str(&format!("  /* {} */\n", css_comment(&group.name)));
        for (name, [r, g, b], spot) in list {
            // Names differing only in case or punctuation map to one property: number the others.
            let base = css_property(name);
            let property = if taken.insert(base.clone()) {
                base
            } else {
                let n = next.entry(base.clone()).or_insert(1);
                loop {
                    *n += 1;
                    let candidate = format!("{base}-{n}");
                    if taken.insert(candidate.clone()) {
                        break candidate;
                    }
                }
            };
            let note = if *spot { " /* spot */" } else { "" };
            out.push_str(&format!("  {property}: #{r:02x}{g:02x}{b:02x};{note}\n"));
        }
    }
    out.push_str("}\n");
    out
}

/// Whether the head of `bytes` reads as a GIMP palette.
pub(super) fn is_gpl(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(bytes.get(..1024).unwrap_or(bytes));
    head.trim_start_matches('\u{feff}').trim_start().starts_with(GPL_HEADER)
}
