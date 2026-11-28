//! The page background: a solid colour, a picture or texture, a gradient or a pattern, as Word
//! writes it in `w:background` and its VML fill (ECMA-376 Part 4 §2.2.1, VML §6.1.2.5; the `o:`
//! attributes are Office's extensions, [MS-ODRAWXML] §2.2). Read back by `read.rs`.
//!
//! A pattern is two colours and Word's 8 × 8 one-bit tile: `v:background/@fillcolor` inks the
//! tile's set bits, `v:fill/@color2` the rest, the tile picture is the fill's relationship and
//! `o:title` names the preset. A gradient carries its two ends as `color`/`color2`, further stops
//! in `colors`, and its transparency as `opacity`/`o:opacity2`, all of which `read.rs` reads
//! again.
//!
//! Beyond the two ends a gradient's stops have no transparency of their own in VML: a save gives
//! the interior stops the first stop's.

use craft_wordmodel::fill::{Gradient, GradientKind};
use craft_wordmodel::pattern::PagePattern;
use craft_wordmodel::props::Rgb;
use craft_wordmodel::{Document, MAX_FILL_EXTRA_ATTRS, MAX_FILL_EXTRA_CHILDREN, MAX_FILL_EXTRA_VALUE, PageFillExtra, VmlElement};

use super::{PartRels, W, Writer, rt};

/// Write `w:background` (with its VML fill) when the document has a page fill.
pub(super) fn background(w: &mut W, doc: &Document, wr: &mut Writer<'_>, rels: &mut PartRels) {
    let picture = doc.settings.page_picture.as_ref().filter(|p| wr.media_files.contains_key(&p.media) && doc.media.contains_key(&p.media));
    let gradient = doc.settings.page_gradient.as_ref().and_then(Gradient::sanitized);
    // A gradient of one stop is that colour, as the model takes it elsewhere.
    let (gradient, solid) = match gradient {
        Some(g) if g.stops.len() > 1 => (Some(g), None),
        Some(g) => (None, g.stops.first().map(|s| s.color)),
        None => (None, None),
    };
    let mut pattern = doc.settings.page_pattern.as_ref().map(PagePattern::sanitized);
    // The tile the pattern is written with: the preset's, else the file's own picture.
    let tile = pattern.as_ref().and_then(|p| match PagePattern::bits(&p.preset) {
        Some(bits) => Some(Tile::Made(bitmap(bits))),
        None => wr.media_files.get(&p.media).filter(|_| doc.media.contains_key(&p.media)).map(|file| Tile::Kept(file.clone())),
    });
    if pattern.is_some() && tile.is_none() {
        // A pattern with neither a known preset nor a picture this save can carry.
        pattern = None;
    }
    // The page colour older readers show for a gradient or a pattern fill.
    let color = doc
        .settings
        .page_color
        .or(solid)
        .or_else(|| gradient.as_ref().and_then(|g| g.stops.first()).map(|s| s.color))
        .or_else(|| pattern.as_ref().map(|p| p.back));
    if picture.is_none() && gradient.is_none() && pattern.is_none() && doc.settings.page_color.is_none() {
        return;
    }
    let hex = color.map(|c| c.hex());
    let attrs: Vec<(&str, &str)> = hex.iter().map(|c| ("w:color", c.as_str())).collect();
    w.open("w:background", &attrs);
    if let Some(p) = picture {
        if let Some(file) = wr.media_files.get(&p.media).cloned() {
            let rid = rels.add(rt::IMAGE, &format!("media/{file}"), false);
            wr.used_media.insert(p.media.clone());
            let mut fa: Vec<(&str, String)> = vec![("r:id", rid)];
            // VML has no centred fill (an ODT page background's): stretched to the page.
            let kind = if p.fill_type.eq_ignore_ascii_case("center") { "frame" } else { p.fill_type.as_str() };
            if !kind.is_empty() {
                fa.push(("type", kind.to_string()));
            }
            if !p.title.is_empty() {
                fa.push(("o:title", p.title.clone()));
            }
            if p.transparency > 0.0 {
                fa.push(("opacity", opacity(p.transparency)));
            }
            w.open("v:background", &[("id", "_x0000_s1025"), ("o:bwmode", "white")]);
            write_fill(w, &fa, &doc.settings.page_fill_extra);
            w.close("v:background");
        }
    } else if let Some(g) = &gradient {
        let (first, last) = (g.stops.first(), g.stops.last());
        let (c1, c2) = (first.map_or(Rgb::WHITE, |s| s.color), last.map_or(Rgb::WHITE, |s| s.color));
        let fillcolor = format!("#{}", c1.hex());
        w.open("v:background", &[("id", "_x0000_s1025"), ("o:bwmode", "white"), ("fillcolor", &fillcolor)]);
        let mut fa: Vec<(&str, String)> = vec![("color", format!("#{}", c1.hex())), ("color2", format!("#{}", c2.hex()))];
        if g.stops.len() > 2 {
            // Every stop, as `position colour` pairs (1/65536ths, VML §6.1.2.5).
            let list: Vec<String> =
                g.stops.iter().map(|s| format!("{}f #{}", (s.pos.clamp(0.0, 100.0) / 100.0 * 65_536.0).round() as i64, s.color.hex())).collect();
            fa.push(("colors", list.join(";")));
            fa.push(("method", "none".to_string()));
        }
        match g.kind {
            GradientKind::Linear { angle, .. } => {
                fa.push(("angle", number((90.0 - angle).rem_euclid(360.0))));
                fa.push(("type", "gradient".to_string()));
            }
            GradientKind::Path { focus, .. } => {
                fa.push(("focusposition", format!("{},{}", number(focus[0] / 100.0), number(focus[1] / 100.0))));
                fa.push(("type", "gradientRadial".to_string()));
            }
        }
        if let Some(s) = first.filter(|s| s.transparency > 0.0) {
            fa.push(("opacity", opacity(s.transparency)));
        }
        if let Some(s) = last.filter(|s| s.transparency > 0.0) {
            fa.push(("o:opacity2", opacity(s.transparency)));
        }
        write_fill(w, &fa, &doc.settings.page_fill_extra);
        w.close("v:background");
    } else if let (Some(p), Some(tile)) = (&pattern, tile) {
        let rid = match tile {
            Tile::Made(bytes) => {
                let file = wr.extra_media_name("page-pattern", "bmp");
                wr.extra_media.push((file.clone(), bytes));
                rels.add(rt::IMAGE, &format!("media/{file}"), false)
            }
            Tile::Kept(file) => {
                wr.used_media.insert(p.media.clone());
                rels.add(rt::IMAGE, &format!("media/{file}"), false)
            }
        };
        let fillcolor = format!("#{}", p.fore.hex());
        w.open("v:background", &[("id", "_x0000_s1025"), ("o:bwmode", "white"), ("fillcolor", &fillcolor)]);
        let label = PagePattern::label(&p.preset).map_or(p.preset.as_str(), |l| l);
        let mut fa: Vec<(&str, String)> =
            vec![("r:id", rid), ("o:title", label.to_string()), ("color2", format!("#{}", p.back.hex())), ("type", "pattern".to_string())];
        if p.transparency > 0.0 {
            fa.push(("opacity", opacity(p.transparency)));
        }
        write_fill(w, &fa, &doc.settings.page_fill_extra);
        w.close("v:background");
    }
    w.close("w:background");
}

/// A pattern's tile as this save writes it.
enum Tile {
    /// The preset's 8 × 8 one-bit BMP, made for this save.
    Made(Vec<u8>),
    /// The file's own picture, kept in the document's media.
    Kept(String),
}

/// Write `v:fill` with its modelled `attrs` and what the file said beyond them: its other
/// attributes and child elements (Office's `o:fill` and a producer's own), so they survive a save.
/// Names the model owns are not repeated, and only the VML/Office namespaces' are written.
fn write_fill(w: &mut W, attrs: &[(&str, String)], extra: &PageFillExtra) {
    let mut seen: Vec<&str> = attrs.iter().map(|(k, _)| *k).collect();
    let mut pairs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    for (name, value) in extra.attrs.iter().take(MAX_FILL_EXTRA_ATTRS) {
        if seen.contains(&name.as_str()) || !PageFillExtra::writable_name(name) || value.len() > MAX_FILL_EXTRA_VALUE {
            continue;
        }
        seen.push(name.as_str());
        pairs.push((name.as_str(), value.as_str()));
    }
    let children: Vec<&VmlElement> =
        extra.children.iter().take(MAX_FILL_EXTRA_CHILDREN).filter(|child| PageFillExtra::writable_name(&child.name)).collect();
    if children.is_empty() {
        w.empty("v:fill", &pairs);
        return;
    }
    w.open("v:fill", &pairs);
    for child in children {
        let mut seen: Vec<&str> = Vec::new();
        let mut ca: Vec<(&str, &str)> = Vec::new();
        for (name, value) in child.attrs.iter().take(MAX_FILL_EXTRA_ATTRS) {
            if seen.contains(&name.as_str()) || !PageFillExtra::writable_name(name) || value.len() > MAX_FILL_EXTRA_VALUE {
                continue;
            }
            seen.push(name.as_str());
            ca.push((name.as_str(), value.as_str()));
        }
        w.empty(&child.name, &ca);
    }
    w.close("v:fill");
}

/// A number for a VML attribute: up to four decimals, no trailing zeros.
fn number(v: f32) -> String {
    let s = format!("{:.4}", if v.is_finite() { v } else { 0.0 });
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// VML `opacity` for a fill that is `transparency` percent transparent: a fraction, 1 = opaque.
fn opacity(transparency: f32) -> String {
    number((100.0 - transparency.clamp(0.0, 100.0)) / 100.0)
}

/// A pattern preset's 8 × 8 one-bit BMP, the format Word stores pattern tiles in: palette index 1
/// (white) where the tile's bit is set, index 0 (black) elsewhere, rows bottom-up and each padded
/// to four bytes. Word inks index 1 with the fill's colour and index 0 with `color2`.
fn bitmap(bits: [u8; 8]) -> Vec<u8> {
    const SIZE: usize = 8;
    const ROW_BYTES: usize = 4;
    let mut b = Vec::with_capacity(62 + ROW_BYTES * SIZE);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&(94u32).to_le_bytes());
    b.extend_from_slice(&[0, 0, 0, 0]);
    b.extend_from_slice(&62u32.to_le_bytes());
    for v in [40u32, 8, 8] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    for v in [0u32, 32, 2835, 2835, 2, 2] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    // Palette index 0 black, 1 white.
    b.extend_from_slice(&[0, 0, 0, 0, 255, 255, 255, 0]);
    for row in bits.iter().rev() {
        // The leftmost pixel is the high bit.
        let byte = (0..8).fold(0u8, |acc, x| if row >> x & 1 == 1 { acc | 0x80 >> x } else { acc });
        b.extend_from_slice(&[byte, 0, 0, 0]);
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pattern_tile_is_the_preset_cell() {
        let Some(bits) = PagePattern::bits("pct50") else { panic!("pct50 is a preset") };
        let b = bitmap(bits);
        assert_eq!((b.len(), &b[..2]), (94, &b"BM"[..]));
        // 8 x 8, 1 bit a pixel, two palette entries.
        assert_eq!((&b[18..26], u16::from_le_bytes([b[26], b[27]]), u16::from_le_bytes([b[28], b[29]])), (&[8, 0, 0, 0, 8, 0, 0, 0][..], 1, 1));
        // Rows bottom-up: the file's first row is the tile's last, its leftmost pixel the high
        // bit (a tile bit `x` is column `x` from the left); a set tile bit is palette index 1,
        // which Word inks with the fill's colour.
        assert_eq!(b[62], bits[7].reverse_bits());
        assert_eq!(b[90], bits[0].reverse_bits());
        assert_eq!(&b[63..66], &[0, 0, 0]);
    }

    #[test]
    fn numbers_and_opacity_are_vml_fractions() {
        assert_eq!(number(90.0), "90");
        assert_eq!(number(0.5), "0.5");
        assert_eq!(number(f32::NAN), "0");
        assert_eq!(opacity(25.0), "0.75");
        assert_eq!(opacity(0.0), "1");
        assert_eq!(opacity(100.0), "0");
        assert_eq!(opacity(250.0), "0");
    }
}
