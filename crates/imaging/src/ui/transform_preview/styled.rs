//! The styled transform preview: a layer with a style (Stroke, Drop Shadow, …) composited by the
//! app at the resolution it is shown at, within a time budget.
//!
//! A style is not transformed with its layer: it is drawn again from the layer's new shape (a
//! stroke keeps its width, a shadow its angle). So the moving texels are resampled through the box
//! by the library ([`StylePlan`], the homography), the app's compositor lays the style over them
//! ([`StyledSource::compose`]), and the result is drawn in place of the mesh. The work follows
//! the screen, never finer than the document, and shrinks to fit [`StyledBudget`]; it is rebuilt
//! only when its key changes (box, zoom, document revision, display key, texture level, filter).
//! The composite is drawn over transparency with Normal blending, like the mesh it replaces.

use std::sync::{Mutex, PoisonError};

use craft_behavior::geometry::Homography;
use craft_behavior::kurbo::{Point, Rect};
use craft_ui::canvas::PixelRect;
use craft_ui::primitives::{Revision, TextureStore, TextureStoreOptions};
use egui::{Color32, ColorImage, Context, TextureOptions};

use super::{PreviewDisplay, PreviewQuality, PreviewTextures};

/// Work coordinates stay within this, so rectangle arithmetic on them cannot overflow.
const COORD_LIMIT: f64 = 5.0e8;
/// Rounds of shrinking the scale until the content fits the budget.
const ROUNDS: u32 = 6;
/// The salt of the styled composite's texture stores.
const STORE: &str = "craft_imaging.styled_preview";

/// How long a styled rebuild may take (LC25, LC26): the work pixels a composite may have are the
/// time over the source's [cost per pixel](StyledSource::pixel_cost_ns) plus the library's own
/// resample cost per pixel, clamped. Separate from
/// S7-adjust's live-preview damage tracking (different inputs).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct StyledBudget {
    /// Milliseconds a rebuild may take while the pointer is down (a drag rebuilds every pass).
    pub drag_ms: f64,
    /// Milliseconds once the box rests.
    pub rest_ms: f64,
    /// The fewest work pixels a budget asks for, however costly the style.
    pub min_pixels: f64,
    /// The most work pixels in a composite.
    pub max_pixels: f64,
    /// Nanoseconds the library's single-threaded resample takes per content pixel, added to the
    /// source's cost.
    pub resample_ns: f64,
}

impl StyledBudget {
    /// PhotoCraft's drag time, 30 ms.
    pub const DRAG_MS: f64 = 30.0;
    /// PhotoCraft's rest time, 120 ms.
    pub const REST_MS: f64 = 120.0;
    /// 0.25 million work pixels.
    pub const MIN_PIXELS: f64 = 0.25e6;
    /// 8 million work pixels.
    pub const MAX_PIXELS: f64 = 8.0e6;
    /// A bilinear resample of one texel through the homography, on one thread (new: PhotoCraft's
    /// cost model has no term for it, its resample runs on Rayon).
    pub const RESAMPLE_NS: f64 = 15.0;

    /// The work pixels for a style costing `cost_ns` per pixel: `ms × 10⁶ / (cost_ns +
    /// resample_ns)` with the drag or rest time, clamped to `min_pixels..=max_pixels`.
    pub fn pixels(&self, cost_ns: f64, dragging: bool) -> f64 {
        // pc: crates/ui-egui/src/tools/transform_fx.rs:94 (`budget`).
        let ms = if dragging { self.drag_ms } else { self.rest_ms };
        (ms * 1.0e6 / (cost_ns + self.resample_ns)).clamp(self.min_pixels, self.max_pixels)
    }
}

impl Default for StyledBudget {
    fn default() -> Self {
        Self {
            drag_ms: Self::DRAG_MS,
            rest_ms: Self::REST_MS,
            min_pixels: Self::MIN_PIXELS,
            max_pixels: Self::MAX_PIXELS,
            resample_ns: Self::RESAMPLE_NS,
        }
    }
}

/// Where and how finely a styled preview is composited. Rectangles are in work pixels: document
/// pixels times `scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct StylePlan {
    /// Work pixels per document pixel, at most 1.
    pub scale: f64,
    /// The box: the layer's bounds after the transform.
    pub bounds: PixelRect,
    /// What is composited and drawn: `bounds` grown by the style's reach, inside the canvas.
    pub shown: PixelRect,
    /// The moving texels resampled for it: `shown` grown by twice the reach, so every effect over
    /// `shown` sees all of the shape it depends on, also past the canvas edge.
    pub content: PixelRect,
}

impl StylePlan {
    /// The plan for a box `quad` (document units, clockwise from the top left) in a document of
    /// `canvas` pixels shown at `zoom` screen pixels per document pixel, within `pixels` work
    /// pixels. The scale starts at `min(zoom, 1)` and shrinks by `sqrt(pixels / area) × 0.98` until
    /// `content`'s area fits, in at most 6 rounds; the reach is the source's at each scale.
    /// `None` when the box is not finite, nothing of it is on the canvas, or it does not fit after
    /// 6 rounds. Work coordinates are clamped to ±5 × 10⁸ so rectangle arithmetic cannot overflow.
    pub fn new(quad: [Point; 4], canvas: [u32; 2], zoom: f32, pixels: f64, reach: impl Fn(f64) -> i32) -> Option<StylePlan> {
        // pc: crates/ui-egui/src/tools/transform_fx.rs:117 (`plan`).
        if quad.iter().any(|p| !(p.x.is_finite() && p.y.is_finite())) {
            return None;
        }
        let (min, max) =
            quad.iter().fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), p| ([lo[0].min(p.x), lo[1].min(p.y)], [hi[0].max(p.x), hi[1].max(p.y)]));
        let budget = if pixels.is_finite() { pixels.clamp(1.0, StyledBudget::MAX_PIXELS) } else { StyledBudget::MIN_PIXELS };
        let mut scale = if zoom.is_finite() && zoom > 0.0 { f64::from(zoom).min(1.0) } else { 1.0 };
        // The reach is in work pixels too, so the area is not simply the scale squared: shrink
        // until it fits (a few rounds settle it).
        for round in 0..ROUNDS {
            let reach = reach(scale).max(0);
            let edge = |v: f64, up: bool| {
                let v = (v * scale).clamp(-COORD_LIMIT, COORD_LIMIT);
                (if up { v.ceil() } else { v.floor() }) as i32
            };
            let bounds = PixelRect::new(edge(min[0], false), edge(min[1], false), edge(max[0], true), edge(max[1], true));
            let canvas = PixelRect::new(0, 0, edge(f64::from(canvas[0]), true), edge(f64::from(canvas[1]), true));
            let shown = grow(&bounds, reach).intersect(&canvas);
            if bounds.is_empty() || shown.is_empty() {
                return None;
            }
            let content = grow(&shown, reach.saturating_mul(2));
            let area = f64::from(content.width()) * f64::from(content.height());
            if area <= budget {
                return Some(StylePlan { scale, bounds, shown, content });
            }
            if round + 1 == ROUNDS {
                break;
            }
            scale *= (budget / area).sqrt() * 0.98;
            if !(scale.is_finite() && scale > 0.0) {
                return None;
            }
        }
        None
    }

    /// `shown` in document units: where the composite is drawn.
    pub fn document_rect(&self) -> Rect {
        // pc: crates/ui-egui/src/tools/transform_fx.rs:67 (`doc_rect`).
        let k = 1.0 / self.scale;
        Rect::new(f64::from(self.shown.x0) * k, f64::from(self.shown.y0) * k, f64::from(self.shown.x1) * k, f64::from(self.shown.y1) * k)
    }
}

/// `rect` grown by `by` on every side (saturating).
fn grow(rect: &PixelRect, by: i32) -> PixelRect {
    PixelRect::new(rect.x0.saturating_sub(by), rect.y0.saturating_sub(by), rect.x1.saturating_add(by), rect.y1.saturating_add(by))
}

/// The moving texels resampled over a work rectangle: straight-alpha RGBA floats, row by row,
/// `rect.width() × rect.height()` of them. The edges of the source fade over one texel, as a
/// resampled layer's do.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct StyledContent {
    /// The work rectangle in document pixels.
    pub rect: PixelRect,
    /// The straight-alpha RGBA texels, row by row.
    pub texels: Vec<[f32; 4]>,
}

/// The app's side of a styled preview: its compositor, which the library cannot have (the layer's
/// style and the document are the app's).
pub trait StyledSource {
    /// Changes whenever the document or the moving layer does (its style, opacity, fill).
    fn revision(&self) -> u64;

    /// Rough cost of compositing one work pixel with the layer's style, in nanoseconds (PhotoCraft
    /// measures it per effect, `pixel_cost_ns`).
    fn pixel_cost_ns(&self) -> f64;

    /// How far the style reaches past the layer's pixels at `scale` work pixels per document
    /// pixel, in work pixels (PhotoCraft `effects::margin` of the scaled style).
    fn reach(&self, scale: f64) -> i32;

    /// The layer with its style at `plan.scale`, over `content`, composited over `plan.shown`:
    /// premultiplied texels, `shown.width() × shown.height()`. `None` when it cannot be made.
    fn compose(&self, plan: &StylePlan, content: &StyledContent) -> Option<ColorImage>;
}

/// The moving texels (`image`, premultiplied, of which `uv` covers the source frame) resampled
/// over `rect`. `to_unit` takes a work pixel to the unit square of the source frame and `inside`
/// is a work point within the box; `nearest` keeps texels whole.
pub fn resample(image: &ColorImage, uv: [f32; 2], to_unit: &Homography, inside: Point, rect: PixelRect, nearest: bool) -> StyledContent {
    // pc: crates/ui-egui/src/tools/transform_fx.rs:164 (`resample`).
    let width = rect.width() as usize;
    let mut texels = vec![[0.0f32; 4]; width * rect.height() as usize];
    let [iw, ih] = image.size;
    let (tw, th) = (f64::from(uv[0]) * iw as f64, f64::from(uv[1]) * ih as f64);
    let [m0, m1, m2] = to_unit.row(0);
    let [m3, m4, m5] = to_unit.row(1);
    let [m6, m7, m8] = to_unit.row(2);
    // The side of the horizon the box is on: a projective map sends the far side elsewhere.
    let side = (m6 * inside.x + m7 * inside.y + m8).signum();
    if iw == 0 || ih == 0 || !(tw > 0.0 && th > 0.0) || !side.is_finite() {
        return StyledContent { rect, texels };
    }
    let texel = |x: i64, y: i64| -> [f32; 4] {
        let (x, y) = (x.clamp(0, iw as i64 - 1) as usize, y.clamp(0, ih as i64 - 1) as usize);
        image.pixels.get(y * iw + x).map_or([0.0; 4], |c| c.to_array().map(f32::from))
    };
    let fade = |t: f64, n: f64| (t + 0.5).min(n - t + 0.5).clamp(0.0, 1.0) as f32;
    let straight = |p: [f32; 4]| -> [f32; 4] {
        let c = Color32::from_rgba_premultiplied(p[0].round() as u8, p[1].round() as u8, p[2].round() as u8, p[3].round() as u8);
        let [r, g, b, _] = c.to_srgba_unmultiplied();
        // The alpha keeps its fraction: a soft edge would band at 8 bits.
        [f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, (p[3] / 255.0).clamp(0.0, 1.0)]
    };
    for (y, row) in texels.chunks_mut(width.max(1)).enumerate() {
        let py = f64::from(rect.y0) + y as f64 + 0.5;
        for (i, o) in row.iter_mut().enumerate() {
            let px = f64::from(rect.x0) + i as f64 + 0.5;
            let w = m6 * px + m7 * py + m8;
            if w * side <= 1e-12 {
                continue;
            }
            let (tx, ty) = ((m0 * px + m1 * py + m2) / w * tw, (m3 * px + m4 * py + m5) / w * th);
            if !(tx.is_finite() && ty.is_finite()) {
                continue;
            }
            if nearest {
                if tx >= 0.0 && ty >= 0.0 && tx < tw && ty < th {
                    *o = straight(texel(tx as i64, ty as i64));
                }
                continue;
            }
            let cover = fade(tx, tw) * fade(ty, th);
            if cover <= 0.0 {
                continue;
            }
            let (fx, fy) = (tx - 0.5, ty - 0.5);
            let (x0, y0) = (fx.floor(), fy.floor());
            let (ax, ay) = ((fx - x0) as f32, (fy - y0) as f32);
            let (x0, y0) = (x0 as i64, y0 as i64);
            let (a, b, c, d) = (texel(x0, y0), texel(x0 + 1, y0), texel(x0, y0 + 1), texel(x0 + 1, y0 + 1));
            let mut p = [0.0f32; 4];
            for (k, v) in p.iter_mut().enumerate() {
                let (top, bottom) = (a[k] + (b[k] - a[k]) * ax, c[k] + (d[k] - c[k]) * ax);
                *v = (top + (bottom - top) * ay) * cover;
            }
            if p[3] > 0.0 {
                *o = straight(p);
            }
        }
    }
    StyledContent { rect, texels }
}

/// What a styled composite is made from in one pass.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct StyledFrame<'a> {
    /// The moving pixels.
    pub textures: &'a PreviewTextures,
    /// The box, clockwise from the top left, in document units.
    pub quad: [Point; 4],
    /// How much the box enlarges its source (1 is the same size).
    pub quad_scale: f32,
    /// Nearest Neighbor interpolation.
    pub nearest: bool,
    /// Screen pixels per document pixel.
    pub zoom: f32,
    /// The pointer is down: the drag budget applies.
    pub dragging: bool,
    /// The document's size in pixels.
    pub canvas: [u32; 2],
    /// How long a rebuild may take.
    pub budget: StyledBudget,
}

impl<'a> StyledFrame<'a> {
    /// The frame of `quad` over `textures` in a document of `canvas` pixels shown at `zoom` screen
    /// pixels per document pixel: unscaled, linear, at rest, with the default budget.
    pub fn new(textures: &'a PreviewTextures, quad: [Point; 4], canvas: [u32; 2], zoom: f32) -> Self {
        Self { textures, quad, quad_scale: 1.0, nearest: false, zoom, dragging: false, canvas, budget: StyledBudget::default() }
    }
}

/// What a composite depends on: the box (bit for bit), the plan, the revision, the display key,
/// the texture level and the filter.
#[derive(Clone, Debug, PartialEq)]
struct StyledKey {
    revision: u64,
    display_key: u64,
    quad: [u64; 8],
    plan: StylePlan,
    level: u32,
    nearest: bool,
    crisp: bool,
}

struct Shown {
    key: StyledKey,
    revision: Revision,
    crisp: bool,
    rect: Rect,
}

#[derive(Default)]
struct StyledState {
    shown: Option<Shown>,
    /// The key whose composite could not be made (the source gave nothing or an image of the
    /// wrong size): the source is not asked again until the key changes.
    failed: Option<StyledKey>,
    builds: u64,
}

/// The styled composite of a session's moving layer, kept until its key changes. The app keeps one
/// per session while the layer has a style.
pub struct StyledPreview {
    name: String,
    state: Mutex<StyledState>,
}

impl StyledPreview {
    /// `name` keys the texture in egui.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), state: Mutex::new(StyledState::default()) }
    }

    /// How many times the composite was made: once per change of its key.
    pub fn builds(&self) -> u64 {
        // pc: crates/ui-egui/src/tools/transform_fx.rs:386.
        self.state.lock().unwrap_or_else(PoisonError::into_inner).builds
    }

    /// The store of the composite's picture: one per filter, one entry each.
    fn store(&self, ctx: &Context, crisp: bool) -> TextureStore<()> {
        let filter = if crisp { TextureOptions::NEAREST } else { TextureOptions::LINEAR };
        TextureStore::in_context(ctx, (STORE, self.name.as_str(), crisp), TextureStoreOptions::frames().filter(filter))
    }

    /// The composite for `frame` and the document rectangle it covers, compositing again through
    /// `source` when the key changed. Crisp (nearest) when the plan is at full scale and the zoom
    /// is at least `CRISP_ZOOM`. `None` when there is nothing to draw this way; the caller draws
    /// the plain mesh.
    pub fn composite(
        &self,
        ctx: &Context,
        frame: &StyledFrame<'_>,
        source: &dyn StyledSource,
        display: Option<&PreviewDisplay>,
    ) -> Option<(egui::TextureId, Rect)> {
        // pc: crates/ui-egui/src/tools/transform_fx.rs:393 (`texture`), :335 (`Frame::plan`).
        let plan = StylePlan::new(frame.quad, frame.canvas, frame.zoom, frame.budget.pixels(source.pixel_cost_ns(), frame.dragging), |scale| {
            source.reach(scale)
        })?;
        let level = PreviewTextures::choose(plan.scale as f32, frame.quad_scale, frame.nearest).level;
        let crisp = plan.scale >= 1.0 && frame.zoom >= PreviewQuality::CRISP_ZOOM;
        let key = StyledKey {
            revision: source.revision(),
            display_key: display.map_or(0, |d| d.key),
            quad: frame.quad.map(|p| [p.x.to_bits(), p.y.to_bits()]).concat().try_into().ok()?,
            plan,
            level,
            nearest: frame.nearest,
            crisp,
        };
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(shown) = state.shown.as_ref().filter(|s| s.key == key)
            && let Some(texture) = self.store(ctx, shown.crisp).state(ctx, &(), shown.revision).shown()
        {
            return Some((texture.id, shown.rect));
        }
        if state.failed.as_ref() == Some(&key) {
            return None;
        }
        let (image, uv) = frame.textures.level_image(level)?;
        let s = plan.scale;
        let to_unit = Homography::unit_to_quad(frame.quad.map(|p| Point::new(p.x * s, p.y * s)))?.inverse()?;
        let inside = Point::new(frame.quad.iter().map(|p| p.x * s / 4.0).sum(), frame.quad.iter().map(|p| p.y * s / 4.0).sum());
        let content = resample(&image, uv, &to_unit, inside, plan.content, frame.nearest);
        let expected = [plan.shown.width() as usize, plan.shown.height() as usize];
        let Some(composed) = source.compose(&plan, &content).filter(|c| c.size == expected) else {
            state.failed = Some(key);
            return None;
        };
        let pixels = match display {
            Some(d) => (d.convert)(&composed),
            None => composed,
        };
        let revision = Revision::new(state.builds + 1);
        self.store(ctx, !crisp).forget(ctx, &());
        let texture = self.store(ctx, crisp).put(ctx, (), revision, pixels);
        let rect = plan.document_rect();
        state.failed = None;
        state.shown = Some(Shown { key, revision, crisp, rect });
        state.builds += 1;
        Some((texture.id, rect))
    }
}
