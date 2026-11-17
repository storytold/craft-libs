//! GIMP palette (.gpl) reading with neutral RGB8 colours and names.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// Largest accepted palette, bounding all parsing allocations.
pub const MAX_BYTES: usize = 1 << 20;
/// Maximum colours returned; later rows are ignored.
pub const MAX_COLORS: usize = 1024;
/// A named RGB colour. Display-name sanitization belongs to the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Color {
    /// Whitespace-normalized colour label; "Untitled" remains literal.
    pub name: String,
    /// Red, green and blue channels.
    pub rgb: [u8; 3],
}
/// A palette's title and colours, in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Last Name header before the colour limit, with surrounding whitespace removed.
    pub name: String,
    /// Valid colour rows in file order.
    pub colors: Vec<Color>,
}
/// Reads GPL text, accepting BOM/CRLF, comments and metadata headers.
/// Malformed colour rows are skipped. Missing magic, empty palettes and oversized
/// input are errors. Columns and Channels headers are ignored for legacy compatibility.
pub fn parse_gpl(text: &str) -> Result<Palette, String> {
    if text.len() > MAX_BYTES {
        return Err("the palette file is too large".into());
    }
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("").trim_start_matches('\u{feff}').trim();
    if first != "GIMP Palette" {
        return Err("this isn't a GIMP palette (its first line must be \"GIMP Palette\")".into());
    }
    let mut name = String::new();
    let mut out = Vec::new();
    for line in lines {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some(n) = l.strip_prefix("Name:") {
            name = n.trim().to_string();
            continue;
        }
        if l.starts_with("Columns:") || l.starts_with("Channels:") {
            continue;
        }
        let mut parts = l.split_whitespace();
        let channel = |s: Option<&str>| s.and_then(|v| v.parse::<u16>().ok()).filter(|v| *v <= 255).map(|v| v as u8);
        let (Some(r), Some(g), Some(b)) = (channel(parts.next()), channel(parts.next()), channel(parts.next())) else { continue };
        let rest: Vec<&str> = parts.collect();
        let label = rest.join(" ");
        out.push(Color { name: label, rgb: [r, g, b] });
        if out.len() >= MAX_COLORS {
            break;
        }
    }
    if out.is_empty() {
        return Err("the palette has no colours in it".into());
    }
    Ok(Palette { name, colors: out })
}
