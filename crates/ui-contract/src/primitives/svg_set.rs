//! The SVG icon catalog the build embeds, and its rasteriser.

use egui::{Color32, ColorImage};
use resvg::tiny_skia;

use super::svg_tree;

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

/// The position of the icon called `name` in the catalog.
pub(crate) fn find(name: &str) -> Option<usize> {
    NAMES.binary_search(&name).ok()
}

/// Every icon name, sorted.
pub(crate) fn names() -> &'static [&'static str] {
    &NAMES
}

/// Whether the icon at `index` flips under right-to-left.
pub(crate) fn directional(index: usize) -> bool {
    DIRECTIONAL[index]
}

/// The icon in white at `px` x `px` physical pixels, its alpha from the drawing; tinted when drawn.
///
/// Stroke sets are drawn `stroke_px` physical pixels wide, fill sets as drawn. `mirrored` flips the drawing
/// horizontally. A document that fails to parse gives a transparent image and a log line.
pub(crate) fn rasterise(index: usize, set: IconSet, px: u32, stroke_px: f32, mirrored: bool) -> ColorImage {
    let size = [px as usize; 2];
    let (svg, own) = svg(index, set);
    let drawn_in = if own { set } else { IconSet::BASE };
    match draw(svg, drawn_in, px, stroke_px, mirrored) {
        Ok(pixmap) => ColorImage::new(size, pixmap.pixels().iter().map(|p| Color32::from_white_alpha(p.alpha())).collect()),
        Err(error) => {
            log::error!("icon {} in {}: {error}", NAMES[index], set.id());
            ColorImage::filled(size, Color32::TRANSPARENT)
        }
    }
}

/// `set` is the set whose file `svg` is: strokes are rewritten only for a stroke set's own files.
fn draw(svg: &[u8], set: IconSet, px: u32, stroke_px: f32, mirrored: bool) -> Result<tiny_skia::Pixmap, String> {
    let mut tree = svg_tree::parse(svg)?;
    let view = tree.size();
    if let Some(own) = set.stroke_width() {
        // Every `stroke-width` scales by the same factor, so a set's thinner details stay thinner.
        let units = stroke_px * view.width() / px as f32;
        let text = std::str::from_utf8(svg).map_err(|e| e.to_string())?;
        tree = svg_tree::parse(scale_stroke_widths(text, units / own).as_bytes())?;
    }
    let mut pixmap = tiny_skia::Pixmap::new(px, px).ok_or("zero size")?;
    let (sx, sy) = (px as f32 / view.width(), px as f32 / view.height());
    let transform =
        if mirrored { tiny_skia::Transform::from_row(-sx, 0.0, 0.0, sy, px as f32, 0.0) } else { tiny_skia::Transform::from_scale(sx, sy) };
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(pixmap)
}

/// `svg` with each `stroke-width="..."` attribute multiplied by `factor`.
fn scale_stroke_widths(svg: &str, factor: f32) -> String {
    const ATTR: &str = "stroke-width=\"";
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(start) = rest.find(ATTR) {
        let value_start = start + ATTR.len();
        out.push_str(&rest[..value_start]);
        rest = &rest[value_start..];
        let end = rest.find('"').unwrap_or(rest.len());
        match rest[..end].parse::<f32>() {
            Ok(width) => out.push_str(&format!("{:.3}", width * factor)),
            Err(_) => out.push_str(&rest[..end]),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}
