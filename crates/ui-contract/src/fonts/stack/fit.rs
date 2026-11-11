//! Fitting CJK faces into the rows of the face that leads a family.
//!
//! A CJK face that follows the app's face sits on that face's baseline and keeps its row height, scaled down only as far
//! as keeps a few tall and deep characters inside the row.

use egui::FontData;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, MetadataProvider as _};

/// Tall and deep ideographs, Hangul, kana, bopomofo and corner brackets: the characters a CJK face
/// fitted into a Latin row keeps inside it.
const ROW_SAMPLES: &str = "漢中國體鬱한뷁あぎアギㄅ「」";

/// Fit `face`, a CJK fallback after `primary`, into `primary`'s rows: its glyphs sit on
/// `primary`'s baseline and the row keeps `primary`'s height. The face is scaled down only as far
/// as keeps [`ROW_SAMPLES`] inside the row.
pub(super) fn into_row(primary: &FontData, face: &mut FontData) {
    let (Some(p), Some(f)) = (VerticalMetrics::of(primary), VerticalMetrics::of(face)) else { return };
    let ascent = primary.tweak.scale * p.ascent;
    let height = primary.tweak.scale * p.height();
    let (above, below) = ink_extent(face, ROW_SAMPLES);
    let mut scale = 1.0_f32;
    if above > ascent {
        scale = scale.min(ascent / above);
    }
    if below > height - ascent {
        scale = scale.min((height - ascent) / below);
    }
    // egui centres a fallback's row in the primary's row without growing it; the offset moves the
    // centred baseline onto the primary's. Both are in units of the font size times the scale.
    face.tweak.set_baseline_fallback(false);
    face.tweak.scale = scale;
    let centred = 0.5 * (height - scale * f.height()) + scale * f.ascent;
    face.tweak.y_offset_factor = (ascent - centred + primary.tweak.scale * primary.tweak.y_offset_factor) / scale;
    face.tweak.y_offset = primary.tweak.y_offset;
}

/// A face's ascent, descent and line gap as epaint reads them, in ems.
struct VerticalMetrics {
    ascent: f32,
    descent: f32,
    line_gap: f32,
}

impl VerticalMetrics {
    fn of(data: &FontData) -> Option<Self> {
        let font = FontRef::from_index(data.bytes(), data.index).ok()?;
        let metrics = font.metrics(Size::unscaled(), LocationRef::default());
        let em = f32::from(metrics.units_per_em);
        (em > 0.0).then(|| Self { ascent: metrics.ascent / em, descent: -metrics.descent / em, line_gap: metrics.leading / em })
    }

    fn height(&self) -> f32 {
        self.ascent + self.descent + self.line_gap
    }
}

/// How far the outlines of `text`'s characters in `data` reach above and below the baseline, in
/// ems, from their control points.
fn ink_extent(data: &FontData, text: &str) -> (f32, f32) {
    let Ok(font) = FontRef::from_index(data.bytes(), data.index) else { return (0.0, 0.0) };
    let em = f32::from(font.metrics(Size::unscaled(), LocationRef::default()).units_per_em);
    let (charmap, outlines) = (font.charmap(), font.outline_glyphs());
    let mut pen = Extent::default();
    for glyph in text.chars().filter_map(|c| charmap.map(c)).filter_map(|id| outlines.get(id)) {
        let _ = glyph.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen);
    }
    if em > 0.0 { (pen.top / em, -pen.bottom / em) } else { (0.0, 0.0) }
}

#[derive(Default)]
struct Extent {
    top: f32,
    bottom: f32,
}

impl Extent {
    fn add(&mut self, y: f32) {
        self.top = self.top.max(y);
        self.bottom = self.bottom.min(y);
    }
}

impl OutlinePen for Extent {
    fn move_to(&mut self, _x: f32, y: f32) {
        self.add(y);
    }
    fn line_to(&mut self, _x: f32, y: f32) {
        self.add(y);
    }
    fn quad_to(&mut self, _cx0: f32, cy0: f32, _x: f32, y: f32) {
        self.add(cy0);
        self.add(y);
    }
    fn curve_to(&mut self, _cx0: f32, cy0: f32, _cx1: f32, cy1: f32, _x: f32, y: f32) {
        self.add(cy0);
        self.add(cy1);
        self.add(y);
    }
    fn close(&mut self) {}
}
