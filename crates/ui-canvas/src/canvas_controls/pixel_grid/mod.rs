//! `PixelGrid`: View > Show > Pixel Grid, a canvas control (20.6). Shown above 500 percent, one device pixel wide,
//! lightening a dark pixel by about a quarter and darkening a light one, only over pixels that have content (never over
//! the checkerboard). The line colour fades from white to black across a band of luma instead of switching at one cut,
//! so mid-tone images get an even grid.
//!
//! The grid follows the composite of the part in view: it reads it through the same `RasterSource` as the raster
//! display and keeps the merged line segments per revision and padded region in the control's memory, so panning
//! within the padding costs nothing and the work is bounded by the viewport.

#[cfg(feature = "testing")]
pub(super) mod stories;

use crate::canvas::{PixelRect, RasterSource, canvas_space};
use crate::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role};
use crate::primitives::{ContentPainterCx, Revision, Rule};
use crate::runtime::env::Label;
use crate::runtime::memory::Remember as _;
use crate::style::{ComponentStyle, RuleLook, StyleCx};
use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, Input, Metrics};
use egui::{Color32, ColorImage, Response, Shape};
use std::sync::Arc;

/// One grid line piece in document pixels, axis-aligned, with its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct GridSegment {
    /// First end of the piece, in document pixels.
    pub from: [i32; 2],
    /// Last end of the piece, in document pixels.
    pub to: [i32; 2],
    /// The line's colour.
    pub color: Color32,
}

/// The line is fully white at or below the first luma and fully black (at `LIGHT_SCALE` strength) at or above the
/// second; between them the two blend.
const FADE_LUMA: [f32; 2] = [0.35, 0.75];

/// The share of black in the line is kept to this many steps, so noisy mid-tone pixels fall into a few colours and
/// equal-colour runs still merge into long segments.
const FADE_STEPS: f32 = 8.0;

/// Luminance of the checker under a see-through pixel when deciding light or dark; between the default checker's two
/// greys.
const CHECKER_LUMA: f32 = 0.9;

/// The grid of a region at one revision, kept in the control's memory.
#[derive(Clone, Default)]
struct GridCache {
    revision: Option<Revision>,
    region: PixelRect,
    segments: Arc<Vec<GridSegment>>,
}

/// Merges consecutive cells of equal colour along one line into `emit(start, end, colour)` runs.
fn runs(len: i32, color_at: impl Fn(i32) -> Option<Color32>, mut emit: impl FnMut(i32, i32, Color32)) {
    let mut open: Option<(i32, Color32)> = None;
    for i in 0..len {
        let color = color_at(i);
        match (open, color) {
            (Some((_, open_color)), Some(color)) if open_color == color => {}
            _ => {
                if let Some((start, open_color)) = open {
                    emit(start, i, open_color);
                }
                open = color.map(|color| (i, color));
            }
        }
    }
    if let Some((start, color)) = open {
        emit(start, len, color);
    }
}

/// The parts of `segment` that are not edges of the cells in `dirty`: a vertical run at x in `dirty` loses its rows in
/// `dirty`, a horizontal run at y in `dirty` loses its columns in it.
fn outside(segment: &GridSegment, dirty: PixelRect) -> impl Iterator<Item = GridSegment> {
    let segment = *segment;
    let vertical = segment.from[0] == segment.to[0];
    let (across, along) = if vertical { (0, 1) } else { (1, 0) };
    let (owned, span) = if vertical { ((dirty.x0, dirty.x1), (dirty.y0, dirty.y1)) } else { ((dirty.y0, dirty.y1), (dirty.x0, dirty.x1)) };
    let piece = move |from: i32, to: i32| {
        let (mut a, mut b) = (segment.from, segment.to);
        (a[along], b[along]) = (from, to);
        GridSegment { from: a, to: b, color: segment.color }
    };
    let (start, end) = (segment.from[along], segment.to[along]);
    let cut = (owned.0..owned.1).contains(&segment.from[across]);
    let before = (!cut).then_some((start, end)).or((start < span.0).then_some((start, end.min(span.0))));
    let after = cut.then_some((start.max(span.1), end)).filter(|(from, to)| from < to);
    before.into_iter().chain(after).map(move |(from, to)| piece(from, to))
}

/// The style of the pixel grid: the control's lines are data-derived, so only the metrics.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PixelGridStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The pixel grid over a raster document.
#[derive(crate::contract::Component)]
#[component(canvas_control, content_painter)]
pub struct PixelGrid<'a> {
    #[common]
    common: Common,
    source: &'a dyn RasterSource,
}

impl<'a> PixelGrid<'a> {
    /// The grid over the composite `source`, named `label`.
    pub fn new(label: impl Into<Label>, source: &'a dyn RasterSource) -> Self {
        PixelGrid { common: Common::new(), source }.label(label)
    }

    /// The grid shows above this device zoom (500 percent).
    pub const MIN_ZOOM: f64 = 5.0;
    /// How far a line moves its pixel toward white or black.
    pub const STRENGTH: f32 = 0.25;
    /// How much of [`STRENGTH`](Self::STRENGTH) a lightening line uses (white reads stronger).
    pub const LIGHT_SCALE: f32 = 0.64;
    /// The cached region snaps outward to multiples of this many pixels.
    pub const REGION_SNAP: i32 = 64;
    /// Segments drawn at most; past this the rest of the grid is left off.
    pub const MAX_SEGMENTS: usize = 60_000;

    /// Whether the grid shows at `device_zoom` (device pixels per document pixel): strictly above
    /// [`MIN_ZOOM`](Self::MIN_ZOOM).
    pub fn shows_at(device_zoom: f64) -> bool {
        device_zoom > Self::MIN_ZOOM
    }

    /// How light the line is over a pixel of `luma`: 0 a white line, 1 a black one; smoothstep across luma 0.35..0.75,
    /// `x = clamp((luma - 0.35) / 0.4, 0, 1)`, `round(x² (3 - 2x) · 8) / 8`. A luma that is not finite reads as 0.
    pub fn light_share(luma: f32) -> f32 {
        let luma = if luma.is_finite() { luma } else { 0.0 };
        let x = ((luma - FADE_LUMA[0]) / (FADE_LUMA[1] - FADE_LUMA[0])).clamp(0.0, 1.0);
        (x * x * (3.0 - 2.0 * x) * FADE_STEPS).round() / FADE_STEPS
    }

    /// The line colour over a pixel of straight-alpha `rgba` (0..=1), or `None` where alpha is NaN or at most 0, or the
    /// result rounds to alpha 0. `a = min(alpha, 1)`, `luma = 0.299 r + 0.587 g + 0.114 b`,
    /// `shown = luma·a + 0.9·(1 - a)` (the checker's luma), `t = light_share(shown)`, `white = STRENGTH·a·(1 - t)`,
    /// `black = STRENGTH·a·LIGHT_SCALE·t`; the colour is grey `round(white / (white + black) · 255)` at alpha
    /// `round((white + black) · 255)`, unmultiplied.
    pub fn cell_color(rgba: [f32; 4]) -> Option<Color32> {
        let alpha = rgba[3];
        if alpha.is_nan() || alpha <= 0.0 {
            return None;
        }
        let alpha = alpha.min(1.0);
        let luma = 0.299 * rgba[0] + 0.587 * rgba[1] + 0.114 * rgba[2];
        let shown = luma * alpha + CHECKER_LUMA * (1.0 - alpha);
        let share = Self::light_share(shown);
        // A white layer and a black layer over the pixel, as one grey with the combined alpha.
        let (white, black) = (Self::STRENGTH * alpha * (1.0 - share), Self::STRENGTH * alpha * Self::LIGHT_SCALE * share);
        let total = white + black;
        let coverage = (total * 255.0).round() as u8;
        if coverage == 0 {
            return None;
        }
        let grey = (white / total * 255.0).round() as u8;
        Some(Color32::from_rgba_unmultiplied(grey, grey, grey, coverage))
    }

    /// The merged segments of `rect`, whose pixels `px` are row by row: runs of one colour along each cell edge become
    /// one segment.
    pub fn segments(px: &[[f32; 4]], rect: PixelRect) -> Vec<GridSegment> {
        let (width, height) = (rect.width(), rect.height());
        let mut out = Vec::new();
        if width == 0 || height == 0 || px.len() < width as usize * height as usize {
            return out;
        }
        let at = |x: i32, y: i32| px.get(y as usize * width as usize + x as usize).and_then(|p| Self::cell_color(*p));
        // Each pixel owns its left and top edge, so the region's right and bottom edges carry no line.
        for x in 0..width {
            if out.len() >= Self::MAX_SEGMENTS {
                return out;
            }
            runs(
                height,
                |y| at(x, y),
                |start, end, color| {
                    out.push(GridSegment { from: [rect.x0 + x, rect.y0 + start], to: [rect.x0 + x, rect.y0 + end], color });
                },
            );
        }
        for y in 0..height {
            if out.len() >= Self::MAX_SEGMENTS {
                return out;
            }
            runs(
                width,
                |x| at(x, y),
                |start, end, color| {
                    out.push(GridSegment { from: [rect.x0 + start, rect.y0 + y], to: [rect.x0 + end, rect.y0 + y], color });
                },
            );
        }
        out.truncate(Self::MAX_SEGMENTS);
        out
    }

    /// `segments` with the edges of the cells in `dirty` redrawn from `px`, whose pixels are `dirty` row by row: the
    /// parts of old runs inside the cells' edges go, and the new runs of `dirty` join the rest. A run that crossed
    /// `dirty` stays two pieces of one colour, which draw as the one run.
    pub fn patched(segments: &[GridSegment], px: &[[f32; 4]], dirty: PixelRect) -> Vec<GridSegment> {
        let mut out: Vec<GridSegment> = segments.iter().flat_map(|segment| outside(segment, dirty)).collect();
        out.extend(Self::segments(px, dirty));
        out.truncate(Self::MAX_SEGMENTS);
        out
    }

    /// The region to cache for a view showing `visible`: inside `bounds`, grown outward to
    /// [`REGION_SNAP`](Self::REGION_SNAP).
    pub fn cache_region(visible: PixelRect, bounds: PixelRect) -> PixelRect {
        let seen = visible.intersect(&bounds);
        if seen.is_empty() {
            return PixelRect::default();
        }
        let down = |n: i32| n.div_euclid(Self::REGION_SNAP) * Self::REGION_SNAP;
        let up = |n: i32| down(n.saturating_add(Self::REGION_SNAP - 1));
        PixelRect::new(down(seen.x0), down(seen.y0), up(seen.x1), up(seen.y1)).intersect(&bounds)
    }

    /// The composite of `region`, as straight-alpha floats row by row.
    fn read_pixels(&self, region: PixelRect) -> Vec<[f32; 4]> {
        let mut image = ColorImage::filled([region.width() as usize, region.height() as usize], Color32::TRANSPARENT);
        self.source.read(region, 1, &mut image);
        image.pixels.iter().map(|c| c.to_srgba_unmultiplied().map(|v| f32::from(v) / 255.0)).collect()
    }

    /// The document pixels `view` shows.
    fn visible(view: &CanvasView) -> PixelRect {
        let shown = view.visible();
        let pixel = |n: f64, round: fn(f64) -> f64| round(n).clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
        PixelRect::new(pixel(shown.x0, f64::floor), pixel(shown.y0, f64::floor), pixel(shown.x1, f64::ceil), pixel(shown.y1, f64::ceil))
    }
}

fn paint_segment(painter: &ContentPainterCx<'_>, view: &CanvasView, segment: &GridSegment) -> Vec<Shape> {
    let point = |p: [i32; 2]| view.to_screen(Point::new(f64::from(p[0]), f64::from(p[1])));
    let ppp = painter.pixels_per_point();
    Rule::shapes(&RuleLook { color: segment.color, width: 1.0 / ppp }, point(segment.from), point(segment.to), ppp)
}

impl Component for PixelGrid<'_> {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = PixelGridStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PixelGridStyleKey) -> PixelGridStyle {
        PixelGridStyle { metrics: Metrics::new() }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    /// Reads the composite when the view left the cached region (all of the new region), or the revision changed (the
    /// part the source reports damaged since the cached one, or all of the region where it cannot tell).
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env).filter(|space| Self::shows_at(space.zoom_device())) else { return };
        let bounds = PixelRect::of_size(self.source.size());
        let visible = Self::visible(&space.view);
        let need = visible.intersect(&bounds);
        if need.is_empty() {
            return;
        }
        let revision = self.source.revision();
        let cache = cx.memory::<GridCache>().clone();
        let covers = cache.region.intersect(&need) == need;
        if cache.revision == Some(revision) && covers {
            return;
        }
        // The cached region still shows the view: only what the source reports changed since its revision is read.
        let damage = cache.revision.filter(|_| covers).and_then(|since| self.source.damage(since));
        let (region, segments) = match damage {
            Some(damage) => {
                let dirty = damage.intersect(&cache.region);
                if dirty.is_empty() {
                    (cache.region, cache.segments)
                } else {
                    (cache.region, Arc::new(Self::patched(&cache.segments, &self.read_pixels(dirty), dirty)))
                }
            }
            None => {
                let region = Self::cache_region(visible, bounds);
                (region, Arc::new(Self::segments(&self.read_pixels(region), region)))
            }
        };
        *cx.memory::<GridCache>() = GridCache { revision: Some(revision), region, segments };
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env).filter(|space| Self::shows_at(space.zoom_device())) else { return };
        let cache = cx.recall::<GridCache>();
        let painter = ui.content_painter(cx);
        for segment in cache.segments.iter() {
            painter.painter.extend(paint_segment(&painter, &space.view, segment));
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
