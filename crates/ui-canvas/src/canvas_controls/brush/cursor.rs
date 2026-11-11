//! The brush cursor: the tip's outline as the user's cursor preferences ask for it, the aliased tip's pixel cell, and
//! the bitmap a window shows for the painted outline.

use super::{BrushTip, BrushTipOutline, TipPose};
use crate::canvas::ToolContext;
use crate::canvas::canvas_space;
use crate::contract::{Access, Common, Component, Cx, PaintUi, Role};
use crate::primitives::{PaintShape, ellipse};
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::kurbo::Rect as DocRect;
use craft_behavior::{CanvasView, Cursor, CursorBadge, CursorIcon, IconId, Input, Metrics, PaintedCursor};
use craft_theme::Rgba;
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};
use std::sync::Arc;

/// The side of the box a tool icon is drawn on; an icon's hotspot is in these units.
const ICON_BOX: f32 = 24.0;
/// Two mapped tip axes are equal and square within this much of their squared length.
const ISOTROPY_TOLERANCE: f32 = 0.001;
/// No length.
const NO_LENGTH: f32 = 0.0;

/// What the Standard painting cursor shows: the Brush tool icon.
const BRUSH_ICON: IconId = IconId::new("brush");

/// Preferences › Cursors › Painting Cursors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PaintingCursor {
    /// The tool's icon.
    Standard,
    /// The crosshair.
    Precise,
    /// The outline where a dab reaches 50 %.
    #[default]
    NormalBrushTip,
    /// The outline where a dab reaches 0 %.
    FullSizeBrushTip,
}

/// The user's cursor preferences a brush cursor reads, with the reach of the crosshair that replaces an outline too
/// small to read.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BrushCursorOptions {
    /// Which cursor painting tools show.
    pub painting: PaintingCursor,
    /// Show Only Crosshair While Painting.
    pub show_only_crosshair_while_painting: bool,
    /// Caps Lock switches to the precise crosshair.
    pub caps_lock_precise: bool,
}

impl BrushCursorOptions {
    /// Painting cursors `painting`, Caps Lock switching to the crosshair.
    pub fn new(painting: PaintingCursor) -> Self {
        BrushCursorOptions { painting, show_only_crosshair_while_painting: false, caps_lock_precise: true }
    }

    /// Whether a stroke in progress shows only the crosshair.
    #[must_use]
    pub fn show_only_crosshair_while_painting(mut self, on: bool) -> Self {
        self.show_only_crosshair_while_painting = on;
        self
    }

    /// Whether Caps Lock switches to the crosshair.
    #[must_use]
    pub fn caps_lock_precise(mut self, on: bool) -> Self {
        self.caps_lock_precise = on;
        self
    }
}

/// The pointer of a brush tool: built each pass from the app's [`BrushTip`] and turned into the [`Cursor`] the tool
/// returns from `CanvasTool::cursor` at `CursorPriority::Tool`.
///
/// The rules, in order (Photoshop's Preferences › Cursors):
/// 1. Caps Lock with [`BrushCursorOptions::caps_lock_precise`]: the crosshair, whatever else is set.
/// 2. Standard: the tool's [`icon`](Self::icon), hotspot at its centre. Precise: the crosshair.
/// 3. While [`painting`](Self::painting) with [`BrushCursorOptions::show_only_crosshair_while_painting`]: the
///    crosshair.
/// 4. A [`Pixels`](BrushTipOutline::Pixels) tip: the hidden pointer (the tool shows [`tip_cell`](Self::tip_cell)), or
///    the crosshair when [`centre`](Self::centre) is on or the cell is narrower than the crosshair on screen.
/// 5. Otherwise Normal Brush Tip draws the 50 % outline (`normal_radius`, the `normal` rings), Full Size Brush Tip
///    the full reach (`full_radius`, the `full` rings), mapped through the [`pose`](Self::pose) and then the view
///    (zoom, rotation, flip, pixel aspect). A round tip under an upright pose and an isotropic view is a circle;
///    under a skewed pose or a pixel aspect it is an outline ellipse with the mapped axes' lengths; traced rings are
///    an outline. With [`inner_ring`](Self::inner_ring) a second ring is added.
/// 6. An outline whose farthest point is nearer than the hit target (`size.hit.target`, the arm of the crosshair the
///    host paints) to the hotspot: the crosshair.
///
/// A [`badge`](Self::badge) rides on the Standard icon and on the crosshair. NaN, infinite, negative and zero sizes
/// never panic and never give a non-finite ring (they fall to rule 6).
#[derive(Clone, Debug)]
pub struct BrushCursor<'a> {
    tip: &'a BrushTip,
    pose: TipPose,
    centre: bool,
    painting: bool,
    icon: IconId,
    inner_ring: Option<f64>,
    badge: Option<CursorBadge>,
}

impl<'a> BrushCursor<'a> {
    /// The cursor of `tip`, upright, no centre mark, not painting, the Brush icon for Standard.
    pub fn new(tip: &'a BrushTip) -> Self {
        BrushCursor { tip, pose: TipPose::upright(), centre: false, painting: false, icon: BRUSH_ICON, inner_ring: None, badge: None }
    }

    /// The pen's pose this pass ([`TipPose::pen`]); default upright.
    #[must_use]
    pub fn pose(mut self, pose: TipPose) -> Self {
        self.pose = pose;
        self
    }

    /// Whether an aliased tip shows the crosshair rather than hiding the pointer: the app's rule (Show Crosshair in
    /// Brush Tip; Clone Stamp only while Alt is held; the Background Eraser always). Default off.
    #[must_use]
    pub fn centre(mut self, on: bool) -> Self {
        self.centre = on;
        self
    }

    /// Whether a stroke is being painted (for Show Only Crosshair While Painting). Default off.
    #[must_use]
    pub fn painting(mut self, on: bool) -> Self {
        self.painting = on;
        self
    }

    /// The tool's icon for the Standard painting cursor. Default the Brush icon.
    #[must_use]
    pub fn icon(mut self, icon: IconId) -> Self {
        self.icon = icon;
        self
    }

    /// A second ring at `radius` document units inside the outline, mapped like it (LightCraft's feather core).
    #[must_use]
    pub fn inner_ring(mut self, radius: f64) -> Self {
        self.inner_ring = Some(radius);
        self
    }

    /// A mark on the Standard icon and on the crosshair: the + or - of the selection mode. Default none.
    #[must_use]
    pub fn badge(mut self, badge: Option<CursorBadge>) -> Self {
        self.badge = badge;
        self
    }

    /// The pointer for this pass, by the rules above. The reach is [`ToolContext::reach`] and Caps Lock is the pass's toggle state.
    pub fn cursor(&self, cx: &ToolContext<'_>, options: &BrushCursorOptions, view: &CanvasView) -> Cursor {
        Cursor::Painted(self.painted(cx.reach(), options, view, cx.caps_lock))
    }

    fn crosshair(&self) -> PaintedCursor {
        match self.badge {
            Some(badge) => PaintedCursor::badged(CursorIcon::Crosshair, badge),
            None => PaintedCursor::Crosshair,
        }
    }

    fn painted(&self, reach: f32, options: &BrushCursorOptions, view: &CanvasView, caps_lock: bool) -> PaintedCursor {
        // Rules 1 and 2.
        if caps_lock && options.caps_lock_precise {
            return self.crosshair();
        }
        match options.painting {
            PaintingCursor::Precise => return self.crosshair(),
            PaintingCursor::Standard => return PaintedCursor::tool(self.icon.clone(), (Vec2::X + Vec2::Y) * (ICON_BOX / 2.0), self.badge),
            PaintingCursor::NormalBrushTip | PaintingCursor::FullSizeBrushTip => {}
        }
        // Rule 3.
        if self.painting && options.show_only_crosshair_while_painting {
            return self.crosshair();
        }
        let full = options.painting == PaintingCursor::FullSizeBrushTip;
        // Rule 5.
        let unit = sanitize(self.tip.diameter) / 2.0;
        let axes_for = |radius: f64| [self.pose.x_axis, self.pose.y_axis].map(|axis| view.map_vec(axis * radius));
        let unit_axes = axes_for(unit);
        let (rings, round) = match &self.tip.outline {
            BrushTipOutline::Round { normal_radius, full_radius } => {
                let axes = axes_for(sanitize(if full { *full_radius } else { *normal_radius }));
                (vec![ellipse(Pos2::ZERO, axes)], Some(axes))
            }
            BrushTipOutline::Traced { normal, full: widest } => {
                let rings = if full { widest } else { normal };
                let mapped: Vec<Vec<Pos2>> =
                    rings.rings().iter().map(|ring| ring.iter().map(|p| (unit_axes[0] * p[0] + unit_axes[1] * p[1]).to_pos2()).collect()).collect();
                (mapped, None)
            }
            // Rule 4. A cell narrower than the crosshair, or not finite, shows the crosshair.
            BrushTipOutline::Pixels { cell, .. } => {
                let corners = view.quad(*cell);
                let (min, max) = corners.iter().fold((corners[0], corners[0]), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
                let side = (max.x - min.x).min(max.y - min.y);
                let narrow = matches!(side.partial_cmp(&reach), None | Some(std::cmp::Ordering::Less));
                return if self.centre || narrow { self.crosshair() } else { PaintedCursor::Hidden };
            }
        };
        // Rule 6, also for a non-finite outline.
        let extent = match round {
            Some(axes) => semi_major(axes),
            None => rings.iter().flatten().fold(NO_LENGTH, |farthest, p| farthest.max(p.to_vec2().length())),
        };
        let finite = rings.iter().flatten().all(|p| p.x.is_finite() && p.y.is_finite());
        if !finite || !extent.is_finite() || extent <= NO_LENGTH || extent < reach {
            return self.crosshair();
        }
        let mut rings = rings;
        if let Some(radius) = self.inner_ring {
            let inner = ellipse(Pos2::ZERO, axes_for(sanitize(radius)));
            if inner.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
                if let Some(axes) = round {
                    rings = vec![ellipse(Pos2::ZERO, axes), inner];
                } else {
                    rings.push(inner);
                }
            }
        } else if let Some([x, y]) = round {
            // A round tip stays a plain circle while the mapped axes are equal and square.
            let anisotropic = (x.length_sq() - y.length_sq()).abs() > ISOTROPY_TOLERANCE || x.dot(y).abs() > ISOTROPY_TOLERANCE;
            if !anisotropic {
                return PaintedCursor::circle(x.length());
            }
        }
        PaintedCursor::outline(rings)
    }

    /// The aliased tip's cell as a canvas control, for the tool to show beside the hidden pointer; `None` for another
    /// outline.
    pub fn tip_cell(&self) -> Option<TipCell> {
        match &self.tip.outline {
            BrushTipOutline::Pixels { cell, round } => Some(TipCell::new(*cell, *round)),
            _ => None,
        }
    }
}

/// `v` if it is a finite size, else 0; negative sizes count as 0.
fn sanitize(v: f64) -> f64 {
    if v.is_finite() { v.max(0.0) } else { 0.0 }
}

/// The farthest distance from the centre of the ellipse with semi-axis vectors `axes`.
fn semi_major([x, y]: [Vec2; 2]) -> f32 {
    let (a, b, d) = (x.length_sq(), y.length_sq(), x.dot(y));
    ((a + b) / 2.0 + (((a - b) / 2.0).powi(2) + d * d).sqrt()).sqrt()
}

/// The screen rect of `cell` through `view` on the pixel grid of `pixels_per_point`; `None` for a cell that is not
/// finite. A `pixels_per_point` that is not finite and positive counts as 1.
fn snapped_cell(view: &CanvasView, cell: DocRect, pixels_per_point: f32) -> Option<Rect> {
    if ![cell.x0, cell.y0, cell.x1, cell.y1].iter().all(|v| v.is_finite()) {
        return None;
    }
    let ppp = if pixels_per_point.is_finite() && pixels_per_point > 0.0 { pixels_per_point } else { 1.0 };
    let snap = |v: f32| (v * ppp).round() / ppp;
    let corners = view.quad(cell);
    let bounds = corners.iter().skip(1).fold(Rect::from_min_max(corners[0], corners[0]), |r, p| r.union(Rect::from_min_max(*p, *p)));
    Some(Rect::from_min_max(Pos2::new(snap(bounds.min.x), snap(bounds.min.y)), Pos2::new(snap(bounds.max.x), snap(bounds.max.y))))
}

/// The style of the pixel cell.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TipCellStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// An aliased tip's whole-pixel cell on the pixel grid, outlined as a square or a disc: what a pencil shows under the
/// hidden pointer. Shown at the canvas rect, in a canvas given a view.
#[derive(crate::contract::Component)]
#[component(canvas_control, content_painter)]
pub struct TipCell {
    #[common]
    common: Common,
    cell: DocRect,
    round: bool,
}

impl TipCell {
    /// The cell `cell` in document units, a disc when `round`.
    pub fn new(cell: DocRect, round: bool) -> Self {
        TipCell { common: Common::new(), cell, round }
    }
}

impl Component for TipCell {
    const ROLE: Role = Role::GROUP;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = TipCellStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: TipCellStyleKey) -> TipCellStyle {
        TipCellStyle::default()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let Some(rect) = snapped_cell(&space.view, self.cell, space.pixels_per_point) else { return };
        let painter = ui.content_painter(cx);
        let shape = if self.round { PaintShape::Circle(rect.center(), rect.width() / 2.0) } else { PaintShape::Rect(rect, CornerRadius::ZERO) };
        painter.outline(&shape);
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// What a window needs to rasterise a painted brush cursor as its own cursor image, so the pointer shows without a
/// frame of lag.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CursorBitmapSpec {
    /// Device pixels per point.
    pub scale: f32,
    /// The colour of the line.
    pub fill: Rgba,
    /// The colour of the outline round the line.
    pub halo: Rgba,
    /// The line width in points.
    pub stroke: f32,
    /// The arm of the crosshair in points.
    pub crosshair: f32,
    /// The largest side of the image in device pixels; a cursor that needs more has no bitmap and stays painted.
    pub max_side: u16,
}

impl CursorBitmapSpec {
    /// A spec for a window at `scale` device pixels per point, with a line of `stroke` points in `fill` over `halo`,
    /// crosshair arms of `crosshair` points and images up to `max_side` pixels on a side.
    pub fn new(scale: f32, fill: Rgba, halo: Rgba, stroke: f32, crosshair: f32, max_side: u16) -> Self {
        CursorBitmapSpec { scale, fill, halo, stroke, crosshair, max_side }
    }
}

/// A cursor image in straight RGBA, its hotspot on the centre pixel.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CursorBitmap {
    /// Straight (not premultiplied) RGBA, row by row.
    pub rgba: Arc<[u8]>,
    /// Width and height in pixels; odd, so the hotspot lies on the centre pixel at every scale.
    pub size: [u16; 2],
    /// The hotspot in pixels from the top left.
    pub hotspot: [u16; 2],
}

/// What a bitmap cursor shows.
enum Shape<'a> {
    Circle(f32),
    Crosshair(f32),
    Outline(&'a [Vec<Pos2>]),
}

impl CursorBitmap {
    /// The image of a painted `cursor` (a circle, the crosshair or an outline) by signed-distance coverage per pixel,
    /// so the line stays one hardware pixel crisp at every scale. `None` for another cursor, a shape that is not
    /// finite, or one larger than `spec.max_side`.
    pub fn render(cursor: &PaintedCursor, spec: &CursorBitmapSpec) -> Option<CursorBitmap> {
        let scale = spec.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let shape = match cursor {
            PaintedCursor::Circle { radius, .. } => Shape::Circle(*radius),
            PaintedCursor::Crosshair => Shape::Crosshair(spec.crosshair),
            PaintedCursor::Outline { rings, .. } => Shape::Outline(rings),
            _ => return None,
        };
        let extent = extent(&shape)?;
        // Odd dimensions put the hotspot on the centre pixel at every scale. Include the line, its halo and the
        // antialiasing fringe, then check before allocating.
        let half = ((extent + 0.5 * spec.stroke + 1.0) * scale + 1.0).ceil();
        if !half.is_finite() || half > f32::from(spec.max_side.saturating_sub(1) / 2) {
            return None;
        }
        let hot = half as u16;
        let side = hot * 2 + 1;
        let n = usize::from(side);
        let hotf = f32::from(hot);
        let (dark, light) = (spec.halo.0, spec.fill.0);
        let mut rgba = vec![0u8; n.checked_mul(n)?.checked_mul(4)?];
        let mut put = |col: usize, row: usize, cover: (f32, f32)| {
            if let Some(px) = rgba.get_mut((row * n + col) * 4..(row * n + col) * 4 + 4) {
                px.copy_from_slice(&pixel(cover, dark, light));
            }
        };
        match shape {
            Shape::Outline(rings) => {
                // Splat each segment into its bounding box instead of measuring every ring per pixel: tip outlines
                // may carry hundreds of points.
                let reach = (0.5 * spec.stroke + 1.0) * scale + 2.0;
                let mut dist = vec![f32::INFINITY; n * n];
                for ring in rings {
                    let len = ring.len();
                    if len < 2 {
                        continue;
                    }
                    for (i, &a) in ring.iter().enumerate() {
                        splat_segment(&mut dist, n, hotf, a.to_vec2() * scale, ring[(i + 1) % len].to_vec2() * scale, reach);
                    }
                }
                for (i, &d) in dist.iter().enumerate() {
                    if d.is_finite() {
                        put(i % n, i / n, line_coverage(d, scale, spec.stroke));
                    }
                }
            }
            Shape::Circle(_) | Shape::Crosshair(_) => {
                // Columns to evaluate in each row: the ring's two arcs for a circle, the whole row otherwise.
                let last = f32::from(side - 1);
                for row in 0..n {
                    let dy = row as f32 - hotf;
                    let spans: [Option<(f32, f32)>; 2] = match shape {
                        Shape::Circle(radius) => {
                            let fringe = (0.5 * spec.stroke + 1.0) * scale + 0.5;
                            match ring_spans(dy, radius * scale, fringe) {
                                Some([l, r]) => [Some(l), Some(r)],
                                None => [None, None],
                            }
                        }
                        _ => [Some((-hotf, hotf)), None],
                    };
                    for (a, b) in spans.into_iter().flatten() {
                        let (c0, c1) = ((hotf + a).floor().clamp(0.0, last) as usize, (hotf + b).ceil().clamp(0.0, last) as usize);
                        for col in c0..=c1 {
                            put(col, row, coverage(&shape, col as f32 - hotf, dy, scale, spec.stroke));
                        }
                    }
                }
            }
        }
        Some(CursorBitmap { rgba: Arc::from(rgba), size: [side, side], hotspot: [hot, hot] })
    }
}

/// The reach of `shape` from the hotspot in points, `None` for a shape that cannot be drawn.
fn extent(shape: &Shape<'_>) -> Option<f32> {
    match shape {
        Shape::Circle(length) | Shape::Crosshair(length) => (length.is_finite() && *length > 0.0).then_some(*length),
        Shape::Outline(rings) => {
            let mut farthest = 0.0f32;
            for p in rings.iter().flatten() {
                if !p.x.is_finite() || !p.y.is_finite() {
                    return None;
                }
                farthest = farthest.max(p.to_vec2().length());
            }
            (farthest > 0.0).then_some(farthest)
        }
    }
}

/// Distance to the nearest arm of a crosshair of arm `length` (round caps), in the units of `x` and `y`.
fn cross_distance(x: f32, y: f32, length: f32) -> f32 {
    let horizontal = (x - x.clamp(0.0, length)).hypot(y);
    let vertical = x.hypot(y - y.clamp(0.0, length));
    horizontal.min(vertical)
}

/// Distance from (`x`, `y`) to the segment `a`-`b`, all in the same units.
fn seg_distance(x: f32, y: f32, a: Vec2, b: Vec2) -> f32 {
    let (abx, aby) = (b.x - a.x, b.y - a.y);
    let t = (((x - a.x) * abx + (y - a.y) * aby) / (abx * abx + aby * aby).max(1e-12)).clamp(0.0, 1.0);
    ((x - a.x - abx * t).powi(2) + (y - a.y - aby * t).powi(2)).sqrt()
}

/// How much of the pixel at (`dx`, `dy`) physical pixels from the hotspot the line covers and how much its halo does,
/// each 0 to 1, for a circle or a crosshair.
fn coverage(shape: &Shape<'_>, dx: f32, dy: f32, scale: f32, width: f32) -> (f32, f32) {
    let (x, y) = (dx.abs(), dy.abs());
    match shape {
        Shape::Circle(radius) => line_coverage((x.hypot(y) - radius * scale).abs(), scale, width),
        Shape::Crosshair(length) => line_coverage(cross_distance(x, y, length * scale), scale, width),
        Shape::Outline(_) => (0.0, 0.0),
    }
}

/// A line `width` points wide with a one pixel halo on each side, `distance` physical pixels from its centre.
fn line_coverage(distance: f32, scale: f32, width: f32) -> (f32, f32) {
    let half = 0.5 * width * scale;
    ((half + 0.5 - distance).clamp(0.0, 1.0), (half + scale + 0.5 - distance).clamp(0.0, 1.0))
}

/// Splat the distance to one ring segment into an outline's side x side distance buffer.
fn splat_segment(dist: &mut [f32], side: usize, hot: f32, a: Vec2, b: Vec2, reach: f32) {
    let n = side as f32;
    let (x0, x1) = (a.x.min(b.x) + hot - reach, a.x.max(b.x) + hot + reach);
    let (y0, y1) = (a.y.min(b.y) + hot - reach, a.y.max(b.y) + hot + reach);
    if x1 < 0.0 || y1 < 0.0 || x0 > n - 1.0 || y0 > n - 1.0 {
        return;
    }
    let (c0, c1) = (x0.max(0.0).floor() as i32, x1.min(n - 1.0).ceil() as i32);
    let (r0, r1) = (y0.max(0.0).floor() as i32, y1.min(n - 1.0).ceil() as i32);
    for row in r0..=r1 {
        let dy = row as f32 - hot;
        for col in c0..=c1 {
            let d = seg_distance(col as f32 - hot, dy, a, b);
            let i = row as usize * side + col as usize;
            if d < dist[i] {
                dist[i] = d;
            }
        }
    }
}

/// One bitmap pixel in straight RGBA: the line (`fill` coverage) over its halo (`outline` coverage). No coverage is
/// transparent.
fn pixel((fill, outline): (f32, f32), dark: [u8; 4], light: [u8; 4]) -> [u8; 4] {
    let a_dark = outline * f32::from(dark[3]) / 255.0;
    let a_light = fill * f32::from(light[3]) / 255.0;
    let alpha = a_light + a_dark * (1.0 - a_light);
    if alpha <= 0.0 {
        return [0; 4];
    }
    let channel = |at: usize| ((f32::from(light[at]) * a_light + f32::from(dark[at]) * a_dark * (1.0 - a_light)) / alpha).round() as u8;
    [channel(0), channel(1), channel(2), (255.0 * alpha).round() as u8]
}

/// The columns of row offset `dy` (from the hotspot, in pixels) that a ring of radius `r` drawn `reach` wide either
/// side can paint: the left and right arcs of the annulus, padded by a pixel. Every other pixel of the row is
/// transparent.
fn ring_spans(dy: f32, r: f32, reach: f32) -> Option<[(f32, f32); 2]> {
    let (ro, ri) = (r + reach + 1.0, (r - reach - 1.0).max(0.0));
    if dy.abs() > ro {
        return None;
    }
    let xo = (ro * ro - dy * dy).max(0.0).sqrt();
    let xi = if dy.abs() < ri { (ri * ri - dy * dy).sqrt() } else { 0.0 };
    Some([(-xo, -xi), (xi, xo)])
}
