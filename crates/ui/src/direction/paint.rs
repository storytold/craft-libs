//! Direction-aware painting: elided galleys and popup anchors. Text is handed to egui in logical
//! order; where a right-to-left name may be cut, its *start* is kept and the "…" lands at its
//! logical end.

use std::sync::Arc;

use egui::{Align2, Area, AsIdSalt, Color32, FontId, Galley, Painter, PopupAnchor, Rect, Response, Ui, pos2, vec2};

use super::{Direction, HasDirection, forward, is_rtl, text_direction};

/// Whether `text` reads right to left as a paragraph: its first strong letter is Hebrew, Arabic…
fn is_rtl_paragraph(text: &str) -> bool {
    craft_i18n::base_direction(text) == Some(Direction::Rtl)
}

/// A right-to-left `text` cut at its logical end with "…" where it doesn't fit (`fits` measures a
/// candidate). The start of the name stays, and egui puts the "…" at the left, the end of a
/// right-to-left line. `None`: `text` fits as it is (or a left-to-right one, which egui elides at
/// its right end as usual).
fn elide(text: &str, fits: impl Fn(&str) -> bool) -> Option<String> {
    if !is_rtl_paragraph(text) || fits(text) {
        return None;
    }
    // The longest logical prefix (whole characters) that fits with "…" after it.
    let ends: Vec<usize> = text.char_indices().map(|(i, _)| i).skip(1).chain(std::iter::once(text.len())).collect();
    let cut = |n: usize| -> String {
        let end = if n == 0 { 0 } else { ends.get(n - 1).copied().unwrap_or(text.len()) };
        format!("{}…", text.get(..end).unwrap_or_default().trim_end())
    };
    let (mut lo, mut hi) = (0_usize, ends.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(&cut(mid)) { lo = mid } else { hi = mid - 1 }
    }
    Some(cut(lo))
}

/// Single-line galley elided with "…" to `max_width` (nothing when there's no room).
pub fn elided_galley(
    d: &impl HasDirection,
    painter: &Painter,
    text: &str,
    font: FontId,
    color: Color32,
    italics: bool,
    max_width: f32,
) -> Option<Arc<Galley>> {
    if max_width < 8.0 {
        return None;
    }
    let format = egui::text::TextFormat { font_id: font, color, italics, ..Default::default() };
    let layout = |s: &str, wrap: bool| {
        let mut job = egui::text::LayoutJob::default();
        job.append(s, 0.0, format.clone());
        // Base the paragraph on the UI's direction: an RTL name in an LTR UI then elides with "…"
        // at the visual right, not at its own reading end.
        job.direction = text_direction(d);
        if wrap {
            job.wrap = egui::text::TextWrapping { max_width, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        }
        painter.layout_job(job)
    };
    // A right-to-left name (Arabic, Hebrew) too long for the row is cut at its logical end, so its
    // start stays and "…" is at the left.
    if let Some(shown) = elide(text, |s| layout(s, false).size().x <= max_width) {
        return Some(layout(&shown, false));
    }
    // egui puts a right-to-left name into display order itself; left-to-right names are cut at
    // their right end by egui.
    Some(layout(text, true))
}

/// Text to paint by hand: the string, its font and its color.
pub struct PaintText<'a> {
    pub text: &'a str,
    pub font: FontId,
    pub color: Color32,
}

/// Paint `text` in the span anchor→limit at height `cy`, elided to fit, aligned into the reading
/// direction (`RIGHT_CENTER` at the anchor in RTL, `LEFT_CENTER` in LTR). A span whose anchor sits
/// past its limit in LTR (an unconverted caller) keeps the left-to-right contract. Returns the
/// painted rect.
pub fn paint_label_span(d: &impl HasDirection, painter: &Painter, anchor: f32, cy: f32, limit: f32, text: PaintText<'_>) -> Option<Rect> {
    let PaintText { text, font, color } = text;
    if is_rtl(d) && anchor > limit {
        let g = elided_galley(d, painter, text, font, color, false, anchor - limit)?;
        let r = Align2::RIGHT_CENTER.anchor_size(pos2(anchor, cy), g.size());
        painter.galley(r.min, g, color);
        Some(r)
    } else {
        let g = elided_galley(d, painter, text, font, color, false, limit - anchor)?;
        let r = Align2::LEFT_CENTER.anchor_size(pos2(anchor, cy), g.size());
        painter.galley(r.min, g, color);
        Some(r)
    }
}

/// Paint a single-line elided label at the x-anchor `x` (a reading-start edge from
/// [`super::start_x`]), vertically centred on `cy`; returns (painted rect, elided). RTL paints
/// right-aligned from the reading-start edge.
pub fn paint_elided(d: &impl HasDirection, painter: &Painter, x: f32, cy: f32, max_w: f32, text: PaintText<'_>) -> Option<(Rect, bool)> {
    let PaintText { text, font, color } = text;
    let mut job = egui::text::LayoutJob::default();
    job.append(text, 0.0, egui::text::TextFormat { font_id: font.clone(), color, ..Default::default() });
    let elided = painter.layout_job(job).size().x > max_w;
    let g = elided_galley(d, painter, text, font, color, false, max_w)?;
    let align = if is_rtl(d) { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER };
    let r = align.anchor_size(pos2(x, cy), g.size());
    painter.galley(r.min, g, color);
    Some((r, elided))
}

/// An [`Area`] hanging off `anchor`'s reading-end edge, `side_offset` px toward the canvas:
/// RTL pivot `RIGHT_TOP` at `anchor.left_top() - (side_offset, 0)`; LTR `LEFT_TOP` at
/// `anchor.right_top() + (side_offset, 0)`.
pub fn popup_area_near(d: &impl HasDirection, ui: &Ui, id: impl AsIdSalt, anchor: Rect, side_offset: f32) -> Area {
    let id = ui.scope_id().with(id);
    if is_rtl(d) {
        Area::new(id).pivot(Align2::RIGHT_TOP).fixed_pos(anchor.left_top() - vec2(side_offset, 0.0))
    } else {
        Area::new(id).pivot(Align2::LEFT_TOP).fixed_pos(anchor.right_top() + vec2(side_offset, 0.0))
    }
}

/// The [`PopupAnchor`] of `resp` translated `offset` px past its reading-end edge
/// (`ParentRect` moved by `(forward(d) * offset, 0)`; the other variants unchanged).
pub fn popup_anchor_reading_end(d: &impl HasDirection, resp: &Response, offset: f32) -> PopupAnchor {
    match PopupAnchor::from(resp) {
        PopupAnchor::ParentRect(rect) => PopupAnchor::ParentRect(rect.translate(vec2(forward(d) * offset, 0.0))),
        other => other,
    }
}
