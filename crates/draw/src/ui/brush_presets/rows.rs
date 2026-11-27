//! The rows of the Brush Preset picker: a preset row shows the tip or the stroke by the parts the picker shows.

use craft_ui::list::ThumbAspect;
use craft_ui::patterns::presets::PresetRow;
use craft_ui::primitives::FacePaint;
use craft_ui::runtime::env::Env;
use egui::Rect;
use std::hash::{Hash, Hasher};

use super::settings::BrushCardParts;
use super::words::BrushPresetWords;

/// The stroke band is this many parts wide to one high.
const STROKE_ASPECT: (u16, u16) = (4, 1);

/// How small the tip thumbnail of a preset `diameter` pixels across is drawn, relative to its
/// cell: `(diameter / 40).clamp(0.3, 1.0)`, so small tips read smaller and 40 px and up fill it.
/// A diameter that isn't a finite number gives 1.
pub fn tip_thumb_scale(diameter: f64) -> f32 {
    if diameter.is_finite() { (diameter / 40.0).clamp(0.3, 1.0) as f32 } else { 1.0 }
}

/// How the app's engine paints a preset: its tip and its stroke, each into the rect it is given.
#[derive(Clone)]
#[non_exhaustive]
pub struct BrushRowPaint {
    /// The tip, white on dark themes as the engine renders it.
    pub tip: FacePaint,
    /// The stroke preview.
    pub stroke: FacePaint,
}

impl BrushRowPaint {
    /// The faces of one preset, each keyed by the host by the brush's identity.
    pub fn new(tip: FacePaint, stroke: FacePaint) -> Self {
        Self { tip, stroke }
    }
}

/// The key of a tip drawn at `scale`: the tip's key and the scale, since one tip face draws at several scales.
fn scaled_key(tip: u64, scale: f32) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (tip, scale.to_bits()).hash(&mut hasher);
    hasher.finish()
}

/// A preset row for the picker: `name` at `depth`, its size as the caption and in the accessible name
/// ("{name}, {size}"), and its picture by `parts`: the stroke band while the stroke shows, else the tip drawn at
/// [`tip_thumb_scale`] of `diameter`.
pub fn brush_row<K>(
    key: K,
    name: impl Into<String>,
    depth: usize,
    diameter: f64,
    parts: BrushCardParts,
    env: &Env,
    paint: &BrushRowPaint,
) -> PresetRow<K> {
    let name = name.into();
    let words = env.words::<BrushPresetWords>();
    let size = env.units().pixels(if diameter.is_finite() { diameter.round() } else { 0.0 });
    let row =
        PresetRow::item(key, name.clone(), depth).caption(size.clone()).accessible_name(words.preset_size.fill(&[("name", &name), ("size", &size)]));
    if parts.normalised().stroke {
        row.preview_face(ThumbAspect::new(STROKE_ASPECT.0, STROKE_ASPECT.1), paint.stroke.clone())
    } else {
        let (tip, scale) = (paint.tip.clone(), tip_thumb_scale(diameter));
        let face =
            FacePaint::new(scaled_key(tip.key(), scale), move |cx, rect| tip.paint(cx, Rect::from_center_size(rect.center(), rect.size() * scale)));
        row.preview_face(ThumbAspect::SQUARE, face)
    }
}
