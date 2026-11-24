//! `HoverOutline` and `SelectionOutline`: the outline of the object under the pointer and the highlight over the
//! selected shapes, canvas controls (20.6). Both are decoration over the document: no pointer, no key, no focus. They
//! share one stroking function over `kit::outline` (halo and line) and `kit::dashed`.

mod hover;
#[cfg(feature = "suite")]
pub(crate) mod stories;

use crate::canvas::canvas_space;
use crate::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role};
use crate::primitives::textures::{Revision, TextureStore, TextureStoreOptions, UNTINTED};
use crate::primitives::{ContentPainterCx, Dashes, PaintShape, dashed, outline_with};
use crate::runtime::env::Label;
use crate::runtime::memory::Remember as _;
use crate::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use craft_behavior::kurbo::{self, BezPath, PathEl, Point, Rect};
use craft_behavior::{CanvasView, Input, Metrics};
use egui::epaint::Vertex;
use egui::{Color32, ColorImage, Mesh, Pos2, Response, Shape, Stroke, Vec2, pos2};
use std::sync::Arc;

pub use hover::HoverMask;

/// A mask texel's alpha per unit of coverage.
const FILL_ALPHA: f32 = 0.42;

/// A mask edge texel's alpha.
const EDGE_ALPHA: f32 = 0.95;

/// How far, in screen points, a flattened curve may leave the path.
const FLATTEN_TOLERANCE: f64 = 0.25;

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
    /// A thin line at the grid colour, with no halo: every editable object's outline in an edit mode.
    Faint,
}

/// The style of the hover and selection outlines.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct OutlineStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The line colour of a shape the host gives no colour: the accent as a bare line.
    line: StyleColor,
    /// The colour of the halo under that line, and its width.
    halo: StyleColor,
    thick: f32,
    /// The width of a hover line and of a dashed selection line.
    medium: f32,
    /// The width of a solid selection line.
    thin: f32,
    /// The length of a dash and of the gap after it.
    dash: f32,
    gap: f32,
    /// The line colour of a faint outline.
    faint: StyleColor,
}

fn outline_style(cx: &StyleCx<'_>) -> OutlineStyle {
    let stroke = &cx.tokens.stroke;
    OutlineStyle {
        metrics: Metrics::new(),
        line: cx.color(cx.tokens.color.overlay.line),
        halo: cx.color(cx.tokens.color.overlay.halo),
        thick: stroke.thick,
        medium: stroke.medium,
        thin: stroke.thin,
        dash: stroke.dash,
        gap: stroke.dash_gap,
        faint: cx.color(cx.tokens.color.overlay.grid),
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

/// The shapes of one run: the kit's halo-and-line outline in the overlay colours, or a line of the host's `color`
/// alone.
fn run_shapes(style: &OutlineStyle, run: Vec<Pos2>, closed: bool, color: Option<DataColor>, width: f32) -> Vec<Shape> {
    match color {
        None => outline_with(
            Stroke::new(style.thick, style.halo.egui()),
            Stroke::new(style.medium, style.line.egui()),
            &[if closed { PaintShape::Polygon(run) } else { PaintShape::Path(run) }],
        ),
        Some(color) => {
            let stroke = Stroke::new(width, color.0);
            vec![if closed { Shape::closed_line(run, stroke) } else { Shape::line(run, stroke) }]
        }
    }
}

/// Strokes one run; see [`run_shapes`].
fn stroke_run(painter: &ContentPainterCx<'_>, style: &OutlineStyle, run: Vec<Pos2>, closed: bool, color: Option<DataColor>, width: f32) {
    painter.painter.extend(run_shapes(style, run, closed, color, width));
}

fn tint(mask: &HoverMask, color: Color32) -> ColorImage {
    let [width, height] = mask.size();
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let covered = mask.coverage(x, y);
            let alpha = if covered <= 0.0 {
                0.0
            } else if mask.is_edge(x, y) {
                EDGE_ALPHA
            } else {
                FILL_ALPHA * covered
            };
            pixels.push(Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (alpha.clamp(0.0, 1.0) * 255.0).round() as u8));
        }
    }
    ColorImage::new([width, height], pixels)
}

fn paint_mask(painter: &ContentPainterCx<'_>, id: egui::Id, view: &CanvasView, mask: &HoverMask, color: Color32) {
    let ctx = painter.painter.ctx();
    let store = TextureStore::<(egui::Id, u64)>::in_context(ctx, "canvas_controls.hover_mask", TextureStoreOptions::frames());
    let texture = store.get_or_put(ctx, (id, mask.generation()), Revision::of(&color.to_array()), || tint(mask, color));
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
}

impl HoverOutline {
    /// The outline of `shape`, named `label`.
    pub fn new(label: impl Into<Label>, shape: Option<HoverShape>) -> Self {
        HoverOutline { common: Common::new(), shape, color: None }.label(label)
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
                stroke_run(&painter, style, view.quad(grown).to_vec(), true, self.color, style.medium);
            }
            HoverShape::Path(path) => {
                for (points, closed) in path_runs(&view, path) {
                    stroke_run(&painter, style, points, closed, self.color, style.medium);
                }
            }
            HoverShape::Mask(mask) => paint_mask(&painter, cx.id, &view, mask, self.color.map_or(style.line.egui(), |color| color.0)),
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
    /// The key outline, stroked after the paths at `stroke.medium` in the outline colour.
    #[prop]
    key: Option<&'a BezPath>,
    /// The revision of the paths and the key: while it holds, the view, the colour and the look, the stroked shapes
    /// are kept in control memory and reused, and a pan by whole device pixels moves them instead of flattening the
    /// paths again. The host changes it whenever a path changes. Without it the shapes are built every pass.
    #[prop]
    revision: Option<u64>,
}

/// What the shapes of a [`SelectionOutline`] are built from besides the view's centre: when none of it changed, the
/// kept shapes are the shapes to paint. The paths themselves are covered by `revision`.
#[derive(Clone, PartialEq)]
struct OutlineKey {
    revision: u64,
    /// The view with its centre zeroed: a pan keeps the key.
    view: CanvasView,
    color: Color32,
    look: SelectionOutlineLook,
    has_key: bool,
    style: OutlineStyle,
    pixels_per_point: f32,
}

/// The shapes of a selection outline under the key they were built for, and the view centre they were built at.
#[derive(Clone, Default)]
struct OutlineShapes {
    key: Option<OutlineKey>,
    centre: Point,
    shapes: Arc<Vec<Shape>>,
}

impl<'a> SelectionOutline<'a> {
    /// A solid outline of `paths` (document units), named `label`.
    pub fn solid(label: impl Into<Label>, paths: &'a [BezPath]) -> Self {
        SelectionOutline { common: Common::new(), paths, look: SelectionOutlineLook::Solid, color: None, key: None, revision: None }.label(label)
    }

    /// A thin outline of `paths` (document units) with no halo, for every editable object in an edit mode, named
    /// `label`.
    pub fn faint(label: impl Into<Label>, paths: &'a [BezPath]) -> Self {
        SelectionOutline { common: Common::new(), paths, look: SelectionOutlineLook::Faint, color: None, key: None, revision: None }.label(label)
    }

    /// A dashed outline of `paths` (document units), named `label`.
    pub fn dashed(label: impl Into<Label>, paths: &'a [BezPath]) -> Self {
        SelectionOutline { common: Common::new(), paths, look: SelectionOutlineLook::Dashed, color: None, key: None, revision: None }.label(label)
    }
}

impl SelectionOutline<'_> {
    /// The shapes of the paths, then of the key path, under `view`.
    fn build(&self, view: &CanvasView, style: &OutlineStyle) -> Vec<Shape> {
        let mut shapes = Vec::new();
        for path in self.paths {
            for (mut points, closed) in path_runs(view, path) {
                match self.look {
                    SelectionOutlineLook::Solid => shapes.extend(run_shapes(style, points, closed, self.color, style.thin)),
                    SelectionOutlineLook::Faint => {
                        let color = self.color.map_or(style.line.egui(), |color| color.0);
                        let stroke = egui::Stroke::new(style.thin, color);
                        shapes.push(if closed { Shape::closed_line(points, stroke) } else { Shape::line(points, stroke) });
                    }
                    SelectionOutlineLook::Dashed => {
                        if closed {
                            points.extend(points.first().copied());
                        }
                        let color = self.color.map_or(style.line.egui(), |color| color.0);
                        shapes.extend(dashed(&points, Dashes::new(style.dash, style.gap), style.medium, color));
                    }
                }
            }
        }
        if let Some(key) = self.key {
            for (points, closed) in path_runs(view, key) {
                shapes.extend(run_shapes(style, points, closed, self.color, style.medium));
            }
        }
        shapes
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

    /// Keeps the shapes in memory when the host gives a revision; without one `paint` builds them.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let (Some(revision), Some(space)) = (self.revision, canvas_space(&cx.env)) else { return };
        let style = cx.style.clone();
        let pixels_per_point = cx.ctx.pixels_per_point();
        let mut view = space.view;
        view.center = Point::ZERO;
        let key = OutlineKey {
            revision,
            view,
            color: self.color.map_or(style.line.egui(), |color| color.0),
            look: self.look,
            has_key: self.key.is_some(),
            style,
            pixels_per_point,
        };
        let centre = space.view.center;
        let kept = cx.memory::<OutlineShapes>();
        if kept.key.as_ref() == Some(&key) {
            // A pan moves every point of the picture by the same screen vector; it is exact only on whole device
            // pixels, where the strokes' pixel alignment is the one a rebuild gives.
            let shift = space.view.map_vec(kept.centre - centre);
            let whole = |v: f32| (v * pixels_per_point - (v * pixels_per_point).round()).abs() < 1e-3;
            if shift == Vec2::ZERO {
                return;
            }
            if whole(shift.x) && whole(shift.y) {
                let mut moved = (*kept.shapes).clone();
                moved.iter_mut().for_each(|shape| shape.translate(shift));
                kept.shapes = Arc::new(moved);
                kept.centre = centre;
                return;
            }
        }
        let shapes = Arc::new(self.build(&space.view, &key.style));
        *cx.memory::<OutlineShapes>() = OutlineShapes { key: Some(key), centre, shapes };
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let painter = ui.content_painter(cx);
        if self.revision.is_some() {
            painter.painter.extend(cx.recall::<OutlineShapes>().shapes.iter().cloned());
        } else {
            painter.painter.extend(self.build(&space.view, cx.style));
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
