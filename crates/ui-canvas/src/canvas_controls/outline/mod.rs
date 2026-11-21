//! `HoverOutline` and `SelectionOutline`: the outline of the object under the pointer and the highlight over the
//! selected shapes, canvas controls (20.6). Both are decoration over the document: no pointer, no key, no focus. They
//! share one stroking function over `kit::outline` (halo and line) and `kit::dashed`.

mod hover;
#[cfg(feature = "suite")]
pub(crate) mod stories;

use crate::canvas::canvas_space;
use crate::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role};
use crate::primitives::textures::{Revision, TextureStore, TextureStoreOptions, UNTINTED};
use crate::primitives::{ContentPainterCx, Dashes, PaintShape, StrokeWidth};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use craft_behavior::kurbo::{self, BezPath, PathEl, Rect};
use craft_behavior::{CanvasView, Input, Metrics};
use egui::epaint::Vertex;
use egui::{Color32, ColorImage, Mesh, Pos2, Response, Shape, Stroke, pos2};

pub use hover::HoverMask;

/// A mask texel's alpha per unit of coverage.
const FILL_ALPHA: f32 = 0.42;

/// A mask edge texel's alpha.
const EDGE_ALPHA: f32 = 0.95;

/// How far, in screen points, a flattened curve may leave the path.
const FLATTEN_TOLERANCE: f64 = 0.25;

/// What the host asks a [`HoverShape::Mask`] tint to look like, resolved once per pass.
#[derive(Clone, Copy, Debug, PartialEq)]
struct MaskLook {
    /// The alpha the tint multiplies a texel's coverage by.
    fill: f32,
    /// The edge band: `None` keeps the framework's one-texel edge at [`EDGE_ALPHA`], `Some(0)` draws
    /// no band, and `Some(depth)` makes it `depth` texels deep.
    band: Option<usize>,
}

impl MaskLook {
    /// The look of `opacity` (a share 0..=1 of the tint's alpha) and `edge_width` (document units of
    /// outline along the mask's edge). A host that gives neither, or a value that is not a number,
    /// keeps the framework's own look: the tint never vanishes or turns opaque on a wild setting.
    fn of(mask: &HoverMask, opacity: Option<f32>, edge_width: Option<f32>) -> Self {
        let fill = match opacity {
            Some(v) if v.is_finite() => v.clamp(0.0, 1.0),
            _ => FILL_ALPHA,
        };
        let band = match edge_width {
            Some(w) if w.is_finite() => Some(band_depth(mask, w)),
            _ => None,
        };
        MaskLook { fill, band }
    }
}

/// The step, in document units, between two texels of `mask`: `HoverMask::sampled` takes one texel
/// per document unit until the long side reaches its ceiling, so this is that step. Its grid is at
/// least 1 x 1 even for an empty mask, so the division has no zero case.
fn texel_step(mask: &HoverMask) -> f64 {
    let [across, _] = mask.size();
    (mask.bounds().width() / across as f64).max(1.0)
}

/// The depth, in texels, of an edge `units` document units wide: at least one texel, since a width
/// under one texel still gets the only texel the grid has, and never more than the mask is deep.
/// 0 units asks for no band.
fn band_depth(mask: &HoverMask, units: f32) -> usize {
    if units <= 0.0 {
        return 0;
    }
    let step = texel_step(mask);
    let [across, down] = mask.size();
    ((f64::from(units) / step).round() as usize).clamp(1, across.max(down))
}

/// Which covered texels of `mask` sit within `depth` texels of a texel that is not covered, or of the
/// mask's own border. The distance is counted over the eight neighbours (Chebyshev), which costs two
/// passes over the grid: the mask is a preview at `HoverMask::MAX_TEXELS` at most, and an exact
/// Euclidean band would need a second grid and a square root per texel for a difference no preview
/// shows.
fn band_texels(mask: &HoverMask, depth: usize) -> Vec<bool> {
    let [across, down] = mask.size();
    let depth = depth.min(across.max(down));
    let covered: Vec<bool> = (0..down).flat_map(|y| (0..across).map(move |x| mask.coverage(x, y) >= 0.5)).collect();
    let far = (depth + 1) as u32;
    let mut dist: Vec<u32> = covered.iter().map(|&on| if on { far } else { 0 }).collect();
    // A covered texel on the grid's border is beside an uncovered one (outside the mask).
    for y in 0..down {
        for x in 0..across {
            if covered[y * across + x] && (x == 0 || y == 0 || x + 1 == across || y + 1 == down) {
                dist[y * across + x] = dist[y * across + x].min(1);
            }
        }
    }
    let near = |dist: &mut Vec<u32>, here: usize, from: usize| {
        let through = dist[from].saturating_add(1);
        if through < dist[here] {
            dist[here] = through;
        }
    };
    for y in 0..down {
        for x in 0..across {
            let here = y * across + x;
            if y > 0 {
                near(&mut dist, here, (y - 1) * across + x);
                if x > 0 {
                    near(&mut dist, here, (y - 1) * across + x - 1);
                }
                if x + 1 < across {
                    near(&mut dist, here, (y - 1) * across + x + 1);
                }
            }
            if x > 0 {
                near(&mut dist, here, y * across + x - 1);
            }
        }
    }
    for y in (0..down).rev() {
        for x in (0..across).rev() {
            let here = y * across + x;
            if y + 1 < down {
                near(&mut dist, here, (y + 1) * across + x);
                if x > 0 {
                    near(&mut dist, here, (y + 1) * across + x - 1);
                }
                if x + 1 < across {
                    near(&mut dist, here, (y + 1) * across + x + 1);
                }
            }
            if x + 1 < across {
                near(&mut dist, here, y * across + x + 1);
            }
        }
    }
    (0..dist.len()).map(|i| covered[i] && dist[i] <= depth as u32).collect()
}

fn tint(mask: &HoverMask, color: Color32, look: &MaskLook) -> ColorImage {
    let [width, height] = mask.size();
    // The edge, worked out once for the whole mask: no host width keeps the framework's own
    // one-texel edge, width 0 asks for no band, and any other depth is the band's texels.
    let band: Option<Vec<bool>> = match look.band {
        None => None,
        Some(0) => Some(Vec::new()),
        Some(depth) => Some(band_texels(mask, depth)),
    };
    let on_edge = |x: usize, y: usize| match &band {
        None => mask.is_edge(x, y),
        Some(band) => band.get(y * width + x).is_some_and(|on| *on),
    };
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let covered = mask.coverage(x, y);
            let alpha = if covered <= 0.0 {
                0.0
            } else if on_edge(x, y) {
                EDGE_ALPHA
            } else {
                look.fill * covered
            };
            pixels.push(Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (alpha.clamp(0.0, 1.0) * 255.0).round() as u8));
        }
    }
    ColorImage::new([width, height], pixels)
}

/// About how many points one flattened segment may have (the tolerance stays at `FLATTEN_TOLERANCE` for segments
/// up to about 250k screen points across).
const MAX_SEGMENT_POINTS: f64 = 1000.0;

/// What a click under the pointer would pick, in document units.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum HoverShape {
    /// A layer's or group's bounds: the line sits just outside the rectangle, so it never covers the layer's edge
    /// pixels.
    Rect(Rect),
    /// An object's outline: the line along the path.
    Path(BezPath),
    /// An object's coverage: a tint of the outline colour at the fill alpha times the coverage, with a one-texel edge
    /// at the edge alpha.
    Mask(HoverMask),
}

/// How a [`SelectionOutline`] strokes its shapes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SelectionOutlineLook {
    /// A thin line: the selected paths of a vector or design app.
    #[default]
    Solid,
    /// A medium line in dashes: a CAD app's selection effect.
    Dashed,
}

/// The style of the hover and selection outlines.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct OutlineStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The line colour of a shape the host gives no colour: the accent as a bare line.
    line: StyleColor,
    /// The width of a hover line and of a dashed selection line.
    medium: f32,
    /// The width of a solid selection line.
    thin: f32,
    /// The length of a dash and of the gap after it.
    dash: f32,
    gap: f32,
}

fn outline_style(cx: &StyleCx<'_>) -> OutlineStyle {
    let stroke = &cx.tokens.stroke;
    OutlineStyle {
        metrics: Metrics::new(),
        line: cx.color(cx.tokens.color.overlay.line),
        medium: stroke.medium,
        thin: stroke.thin,
        dash: stroke.dash,
        gap: stroke.dash_gap,
    }
}

/// The runs of `path` on the screen under `view`, curves flattened: the vertices and whether the run closes. Runs
/// of one point are dropped. The outline widgets and a tool that paints its own path share this flattening.
pub fn path_runs(view: &CanvasView, path: &BezPath) -> Vec<(Vec<Pos2>, bool)> {
    let screen = view.affine() * path.clone();
    let mut out = Vec::new();
    let mut points = Vec::new();
    let end = |out: &mut Vec<_>, points: &mut Vec<Pos2>, closed: bool| {
        let run = std::mem::take(points);
        if run.len() > 1 {
            out.push((run, closed));
        }
    };
    let (mut start, mut current) = (kurbo::Point::ZERO, kurbo::Point::ZERO);
    for element in screen.elements().iter().copied() {
        match element {
            PathEl::MoveTo(p) => {
                end(&mut out, &mut points, false);
                points.push(pos2(p.x as f32, p.y as f32));
                (start, current) = (p, p);
            }
            PathEl::ClosePath => {
                end(&mut out, &mut points, true);
                current = start;
            }
            segment => {
                let Some(to) = segment.end_point() else { continue };
                let hull = segment_hull(current, segment);
                current = to;
                if !hull.iter().all(|p| p.is_finite()) {
                    // A segment with a non-finite point has no drawable shape; the run breaks there and the next
                    // one starts at its end.
                    end(&mut out, &mut points, false);
                    if to.is_finite() {
                        points.push(pos2(to.x as f32, to.y as f32));
                    }
                    continue;
                }
                let extent = hull.iter().fold(Rect::from_points(hull[0], hull[0]), |r, p| r.union_pt(*p));
                // The point count of a flattened curve grows with the square root of its size over the tolerance;
                // a segment far larger than any screen keeps a bounded count at a coarser tolerance.
                let tolerance = FLATTEN_TOLERANCE.max(extent.width().max(extent.height()) / MAX_SEGMENT_POINTS.powi(2));
                kurbo::flatten([PathEl::MoveTo(hull[0]), segment], tolerance, |flat| {
                    if let PathEl::LineTo(p) = flat {
                        points.push(pos2(p.x as f32, p.y as f32));
                    }
                });
            }
        }
    }
    end(&mut out, &mut points, false);
    out
}

/// The start point and every point of one drawing segment.
fn segment_hull(from: kurbo::Point, segment: PathEl) -> Vec<kurbo::Point> {
    match segment {
        PathEl::LineTo(p) => vec![from, p],
        PathEl::QuadTo(a, p) => vec![from, a, p],
        PathEl::CurveTo(a, b, p) => vec![from, a, b, p],
        PathEl::MoveTo(_) | PathEl::ClosePath => vec![from],
    }
}

/// Strokes one run: the kit's halo-and-line outline in the overlay colours, or a line of the host's `color` alone.
fn stroke_run(painter: &ContentPainterCx<'_>, run: Vec<Pos2>, closed: bool, color: Option<DataColor>, width: f32) {
    match color {
        None => painter.outline(&if closed { PaintShape::Polygon(run) } else { PaintShape::Path(run) }),
        Some(color) => {
            let stroke = Stroke::new(width, color.0);
            painter.painter.add(if closed { Shape::closed_line(run, stroke) } else { Shape::line(run, stroke) });
        }
    }
}

fn paint_mask(painter: &ContentPainterCx<'_>, view: &CanvasView, mask: &HoverMask, color: Color32, look: &MaskLook) {
    let ctx = painter.painter.ctx();
    let store = TextureStore::<u64>::in_context(ctx, "canvas_controls.hover_mask", TextureStoreOptions::frames());
    // The look is part of the key: the same mask tinted at another opacity or with another edge is a
    // different image, and `get_or_put` would otherwise keep the first one.
    let key = (color.to_array(), look.fill.to_bits(), look.band);
    let texture = store.get_or_put(ctx, mask.generation(), Revision::of(&key), || tint(mask, color, look));
    let uvs = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
    let mut mesh = Mesh::with_texture(texture.sized().id);
    for (pos, uv) in view.quad(mask.bounds()).into_iter().zip(uvs) {
        mesh.vertices.push(Vertex { pos, uv, color: UNTINTED });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.painter.add(Shape::mesh(mesh));
}

/// The outline of the object under the pointer: decoration over the canvas, drawn after the document and before the
/// tool's own marks. No shape draws nothing; the host passes `None` while a button is down, another tool is up, the
/// pointer is off the canvas or the option is off. Nothing mirrors in a right-to-left interface: the shape is the
/// document's.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct HoverOutline {
    #[common]
    common: Common,
    shape: Option<HoverShape>,
    /// The line and tint colour (a layer's colour); the overlay line colour by default.
    #[prop]
    color: Option<DataColor>,
    /// The share of the tint's alpha a mask's coverage is multiplied by (0..=1, clamped); the
    /// framework's own [`FILL_ALPHA`] by default.
    #[prop]
    tint_opacity: Option<f32>,
    /// How far, in document units, the near-opaque band along a mask's edge reaches; the
    /// framework's one-texel edge by default, `Some(0.0)` for no band at all.
    #[prop]
    edge_width: Option<f32>,
}

impl HoverOutline {
    /// The outline of `shape`, named `label`.
    pub fn new(label: impl Into<Label>, shape: Option<HoverShape>) -> Self {
        HoverOutline { common: Common::new(), shape, color: None, tint_opacity: None, edge_width: None }.label(label)
    }
}

impl Component for HoverOutline {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = OutlineStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: HoverOutlineStyleKey) -> OutlineStyle {
        outline_style(cx)
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (Some(shape), Some(space)) = (&self.shape, canvas_space(&cx.env)) else { return };
        let view = space.view;
        let painter = ui.content_painter(cx);
        let style = &cx.style;
        match shape {
            HoverShape::Rect(rect) => {
                // The line sits outside the rectangle: grown by half its width.
                let (half_y, half_x) = (view.doc_len(style.medium / 2.0), f64::from(style.medium / 2.0) / view.zoom_x());
                let rect = rect.abs();
                let grown = Rect::new(rect.x0 - half_x, rect.y0 - half_y, rect.x1 + half_x, rect.y1 + half_y);
                stroke_run(&painter, view.quad(grown).to_vec(), true, self.color, style.medium);
            }
            HoverShape::Path(path) => {
                for (points, closed) in path_runs(&view, path) {
                    stroke_run(&painter, points, closed, self.color, style.medium);
                }
            }
            HoverShape::Mask(mask) => {
                let look = MaskLook::of(mask, self.tint_opacity, self.edge_width);
                paint_mask(&painter, &view, mask, self.color.map_or(style.line.egui(), |color| color.0), &look);
            }
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// Highlights over the selected shapes, in the host's colour (a layer's colour), else the overlay line colour.
/// Decoration: no pointer, key or accessibility node; the selection's own frame or list carries those. Dashes start
/// afresh on each subpath, in screen points, so they do not crawl as the view zooms.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct SelectionOutline<'a> {
    #[common]
    common: Common,
    paths: &'a [BezPath],
    look: SelectionOutlineLook,
    /// The line colour (a layer's colour); the overlay line colour by default.
    #[prop]
    color: Option<DataColor>,
}

impl<'a> SelectionOutline<'a> {
    /// A solid outline of `paths` (document units), named `label`.
    pub fn solid(label: impl Into<Label>, paths: &'a [BezPath]) -> Self {
        SelectionOutline { common: Common::new(), paths, look: SelectionOutlineLook::Solid, color: None }.label(label)
    }

    /// A dashed outline of `paths` (document units), named `label`.
    pub fn dashed(label: impl Into<Label>, paths: &'a [BezPath]) -> Self {
        SelectionOutline { common: Common::new(), paths, look: SelectionOutlineLook::Dashed, color: None }.label(label)
    }
}

impl Component for SelectionOutline<'_> {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = OutlineStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: SelectionOutlineStyleKey) -> OutlineStyle {
        outline_style(cx)
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let painter = ui.content_painter(cx);
        let style = &cx.style;
        for path in self.paths {
            for (mut points, closed) in path_runs(&space.view, path) {
                match self.look {
                    SelectionOutlineLook::Solid => stroke_run(&painter, points, closed, self.color, style.thin),
                    SelectionOutlineLook::Dashed => {
                        if closed {
                            points.extend(points.first().copied());
                        }
                        let color = self.color.map_or(style.line.egui(), |color| color.0);
                        painter.dashed(&points, Dashes::new(style.dash, style.gap), StrokeWidth::Medium, color);
                    }
                }
            }
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The alpha a covered edge texel carries, as the image stores it.
    const EDGE_A8: u8 = (EDGE_ALPHA * 255.0).round() as u8;
    /// The alpha a fully covered texel carries at the framework's own fill alpha.
    const FILL_A8: u8 = (FILL_ALPHA * 255.0).round() as u8;

    /// A 6 x 6 mask with a 4 x 4 covered block at (1, 1), one texel per document unit: the block's
    /// ring sits one texel from an uncovered texel, its middle does not.
    fn block() -> HoverMask {
        HoverMask::sampled(Rect::new(0.0, 0.0, 6.0, 6.0), |x, y| if (1.0..5.0).contains(&x) && (1.0..5.0).contains(&y) { 1.0 } else { 0.0 })
    }

    fn alpha(image: &ColorImage, x: usize, y: usize) -> u8 {
        image.pixels[y * image.size[0] + x].a()
    }

    /// The look of a mask over the block: `opacity` and `edge` as a host would pass them.
    fn look(opacity: Option<f32>, edge: Option<f32>) -> MaskLook {
        MaskLook::of(&block(), opacity, edge)
    }

    /// How many texels of the image carry the edge's alpha.
    fn edged(image: &ColorImage) -> usize {
        (0..6).flat_map(|y| (0..6).map(move |x| (x, y))).filter(|(x, y)| alpha(image, *x, *y) == EDGE_A8).count()
    }

    #[test]
    fn a_host_that_gives_nothing_gets_the_frameworks_own_tint() {
        let image = tint(&block(), Color32::RED, &look(None, None));
        assert!(alpha(&image, 1, 1) > alpha(&image, 2, 2), "the one-texel edge stands out over the fill");
        assert_eq!(alpha(&image, 1, 1), EDGE_A8, "the ring is the edge");
        assert_eq!(alpha(&image, 2, 2), FILL_A8, "the middle is the fill");
        assert_eq!(alpha(&image, 0, 0), 0, "an uncovered texel tints nothing");
    }

    #[test]
    fn the_host_opacity_moves_the_fill_and_leaves_the_edge_and_the_emptiness_alone() {
        let weak = tint(&block(), Color32::RED, &look(Some(0.1), None));
        let full = tint(&block(), Color32::RED, &look(Some(1.0), None));
        assert!(alpha(&weak, 2, 2) < alpha(&full, 2, 2), "more opacity, more fill");
        assert_eq!(alpha(&weak, 1, 1), alpha(&full, 1, 1), "the edge keeps its own alpha");
        for image in [&weak, &full] {
            assert_eq!(alpha(image, 0, 0), 0, "opacity still tints nothing where nothing is covered");
        }
        // Past the ends of its range the share is clamped; a value that is not a number keeps the
        // framework's own fill.
        assert_eq!(alpha(&tint(&block(), Color32::RED, &look(Some(7.0), None)), 2, 2), 255, "above 1 reads as 1");
        assert_eq!(alpha(&tint(&block(), Color32::RED, &look(Some(-3.0), None)), 2, 2), 0, "below 0 reads as 0");
        let nan = tint(&block(), Color32::RED, &look(Some(f32::NAN), None));
        assert_eq!(alpha(&nan, 2, 2), alpha(&tint(&block(), Color32::RED, &look(None, None)), 2, 2), "not a number tints as the framework does");
    }

    #[test]
    fn the_host_edge_replaces_the_one_texel_edge_and_a_deeper_outline_reaches_deeper() {
        let default = tint(&block(), Color32::RED, &look(None, None));
        let none = tint(&block(), Color32::RED, &look(None, Some(0.0)));
        let deep = tint(&block(), Color32::RED, &look(None, Some(3.0)));
        assert_eq!(edged(&none), 0, "0 width asks for no band at all");
        assert!(edged(&deep) > edged(&default), "a wider outline reaches further in");
        assert_eq!(alpha(&none, 1, 1), FILL_A8, "with no band the ring is plain fill");
        assert_eq!(alpha(&deep, 2, 2), EDGE_A8, "the block's middle joins a 3-unit band");
        assert_eq!(alpha(&deep, 0, 0), 0, "the band stays inside the covered area");
        let nan = tint(&block(), Color32::RED, &look(None, Some(f32::NAN)));
        assert_eq!(edged(&nan), edged(&default), "a width that is not a number keeps the framework's edge");
    }

    #[test]
    fn a_width_under_one_texel_still_gets_that_texel_and_an_empty_mask_nothing() {
        let thin = tint(&block(), Color32::RED, &look(None, Some(0.4)));
        let one = tint(&block(), Color32::RED, &look(None, Some(1.0)));
        assert_eq!(edged(&thin), edged(&one), "the grid's own texel is the narrowest band");
        let empty = HoverMask::sampled(Rect::new(0.0, 0.0, 6.0, 6.0), |_, _| 0.0);
        let image = tint(&empty, Color32::RED, &MaskLook::of(&empty, Some(1.0), Some(30.0)));
        assert!(image.pixels.iter().all(|p| p.a() == 0), "an object with no coverage draws nothing");
    }
}
