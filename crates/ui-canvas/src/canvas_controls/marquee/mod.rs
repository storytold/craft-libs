//! `Marquee`: the outline of a selection being made or held, a canvas control (20.6). It paints marching ants, a
//! selection band or an outline being drawn over the canvas, mapped from document geometry through the canvas's view.
//! It takes no pointer and no key: the press, the drag and the modifier rules are the canvas tool's.

#[cfg(feature = "testing")]
pub(super) mod stories;

use super::words::CanvasControlWords;
use crate::canvas::{MARCH_SECONDS, MARCH_TICK, canvas_space, march_shift};
use crate::contract::{Access, Animate as _, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, WordSetCx as _};
use crate::primitives::{ContentPainterCx, Dashes, StrokeWidth, ellipse};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use craft_behavior::kurbo::{Point, Rect, Vec2};
use craft_behavior::{CanvasView, Input, Metrics};
use egui::{Color32, Pos2, Response, Shape, Stroke};

/// Opacity of the halo under an outline being drawn.
const HALO_ALPHA: f32 = 0.55;

/// How a marquee is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum MarqueeLook {
    /// Marching ants laid out in device pixels, for selections, marquee drags and zoom boxes.
    #[default]
    Ants,
    /// A static thin line with dashes over it, for selecting objects by dragging in a layout or vector app.
    Band,
    /// An outline still being drawn (a lasso): a dark thin line over a wider translucent light line. A polygon is an
    /// open path in this look.
    Drawing,
    /// A line in the given colour over a halo, for a shape being drawn over any image: a rectangle, an ellipse or a
    /// line, closed like the ants' outline.
    Contrast(DataColor),
}

/// What a marquee outlines, in document units.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MarqueeShape {
    /// A rectangle.
    Rect(Rect),
    /// The ellipse inscribed in a rectangle.
    Ellipse(Rect),
    /// A closed polygon (an open path in the drawing look).
    Polygon(Vec<Point>),
    /// An open path through the points; no segment joins its last point to its first in any look.
    Polyline(Vec<Point>),
    /// Separate segments, such as a traced outline's edges.
    Segments(Vec<(Point, Point)>),
}

impl MarqueeShape {
    /// The document bounds of the outline.
    pub fn bounds(&self) -> Rect {
        let around = |points: &mut dyn Iterator<Item = Point>| {
            points.fold(None, |bounds: Option<Rect>, point| Some(bounds.map_or(Rect::from_points(point, point), |bounds| bounds.union_pt(point))))
        };
        match self {
            MarqueeShape::Rect(rect) | MarqueeShape::Ellipse(rect) => rect.abs(),
            MarqueeShape::Polygon(points) | MarqueeShape::Polyline(points) => around(&mut points.iter().copied()).unwrap_or_default(),
            MarqueeShape::Segments(segments) => around(&mut segments.iter().flat_map(|&(a, b)| [a, b])).unwrap_or_default(),
        }
    }

    /// The outline's runs on the screen under `view`; `look` decides whether a polygon is closed.
    pub fn runs(&self, view: &CanvasView, look: MarqueeLook) -> Vec<Run> {
        let map = |points: &[Point]| points.iter().map(|&point| view.to_screen(point)).collect::<Vec<_>>();
        match self {
            MarqueeShape::Rect(rect) => vec![Run { points: view.quad(rect.abs()).to_vec(), closed: true }],
            MarqueeShape::Ellipse(rect) => {
                let rect = rect.abs();
                let axes = [view.map_vec(Vec2::new(rect.width() / 2.0, 0.0)), view.map_vec(Vec2::new(0.0, rect.height() / 2.0))];
                vec![Run { points: ellipse(view.to_screen(rect.center()), axes), closed: true }]
            }
            MarqueeShape::Polygon(points) => vec![Run { points: map(points), closed: look != MarqueeLook::Drawing }],
            MarqueeShape::Polyline(points) => vec![Run { points: map(points), closed: false }],
            MarqueeShape::Segments(segments) => segments.iter().map(|&(a, b)| Run { points: map(&[a, b]), closed: false }).collect(),
        }
    }
}

/// One run of an outline on the screen.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Run {
    /// The vertices, in screen points.
    pub points: Vec<Pos2>,
    /// Whether the last vertex joins the first.
    pub closed: bool,
}

/// The style of a marquee.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct MarqueeStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The band's line and the halo of an outline being drawn.
    light: StyleColor,
    /// The band's dashes, the line of an outline being drawn and the halo under a contrast line.
    dark: StyleColor,
    /// The band's line width.
    band_width: f32,
    /// The band's dash length and gap.
    band_dash: f32,
    /// The width of the halo under an outline being drawn.
    drawing_halo: f32,
    /// The ants' dash length, which sets how far a march covers.
    ants_dash: f32,
}

/// An outline over the document, shown at the canvas's rect with `show_at` and mapped through the canvas's view.
/// It is one graphics object in the accessibility tree, named by its label and valued by `value`, else by its bounds.
/// With reduced motion the ants stand still.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct Marquee {
    #[common]
    common: Common,
    shape: MarqueeShape,
    /// How the outline is drawn.
    #[prop]
    look: MarqueeLook,
    /// The accessible value; the outline's bounds by default.
    #[prop(into)]
    value: Option<Label>,
}

impl Marquee {
    /// A marquee named `label` over `shape`.
    pub fn new(label: impl Into<Label>, shape: MarqueeShape) -> Self {
        Marquee { common: Common::new(), shape, look: MarqueeLook::Ants, value: None }.label(label)
    }

    /// A rectangle named `label`.
    pub fn rect(label: impl Into<Label>, rect: Rect) -> Self {
        Marquee::new(label, MarqueeShape::Rect(rect))
    }

    /// The ellipse inscribed in `rect`, named `label`.
    pub fn ellipse(label: impl Into<Label>, rect: Rect) -> Self {
        Marquee::new(label, MarqueeShape::Ellipse(rect))
    }

    /// A closed polygon (an open path in the drawing look), named `label`.
    pub fn polygon(label: impl Into<Label>, points: impl IntoIterator<Item = Point>) -> Self {
        Marquee::new(label, MarqueeShape::Polygon(points.into_iter().collect()))
    }

    /// An open path through `points`, named `label`.
    pub fn polyline(label: impl Into<Label>, points: impl IntoIterator<Item = Point>) -> Self {
        Marquee::new(label, MarqueeShape::Polyline(points.into_iter().collect()))
    }

    /// Separate segments, such as a traced outline's edges, named `label`.
    pub fn segments(label: impl Into<Label>, segments: impl IntoIterator<Item = (Point, Point)>) -> Self {
        Marquee::new(label, MarqueeShape::Segments(segments.into_iter().collect()))
    }

    fn shape_name<'w>(&self, words: &'w CanvasControlWords) -> &'w Label {
        match self.shape {
            MarqueeShape::Rect(_) => &words.shape_rect,
            MarqueeShape::Ellipse(_) => &words.shape_ellipse,
            MarqueeShape::Polygon(_) => &words.shape_polygon,
            MarqueeShape::Polyline(_) => &words.shape_polyline,
            MarqueeShape::Segments(_) => &words.shape_segments,
        }
    }
}

fn line(points: &[Pos2], closed: bool, stroke: Stroke) -> Shape {
    if closed { Shape::closed_line(points.to_vec(), stroke) } else { Shape::line(points.to_vec(), stroke) }
}

fn paint_band(painter: &ContentPainterCx<'_>, run: &Run, style: &MarqueeStyle) {
    let (light, dark) = (style.light.egui(), style.dark.egui());
    painter.painter.add(line(&run.points, run.closed, Stroke::new(style.band_width, light)));
    let mut dashed = run.points.clone();
    if run.closed
        && let Some(&first) = run.points.first()
    {
        dashed.push(first);
    }
    painter.dashed(&dashed, Dashes::even(style.band_dash), StrokeWidth::Thin, dark);
}

/// A thin line over a wider translucent halo.
fn paint_haloed(painter: &ContentPainterCx<'_>, run: &Run, style: &MarqueeStyle, halo: Color32, ink: Color32) {
    painter.painter.add(line(&run.points, run.closed, Stroke::new(style.drawing_halo, halo.gamma_multiply(HALO_ALPHA))));
    painter.painter.add(line(&run.points, run.closed, Stroke::new(style.band_width, ink)));
}

impl Component for Marquee {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = MarqueeStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: MarqueeStyleKey) -> MarqueeStyle {
        let (overlay, stroke) = (&cx.tokens.color.overlay, &cx.tokens.stroke);
        MarqueeStyle {
            metrics: Metrics::new(),
            light: cx.color(overlay.marquee),
            dark: cx.color(overlay.halo),
            band_width: stroke.thin,
            band_dash: stroke.dash_gap,
            drawing_halo: stroke.thick,
            ants_dash: stroke.dash,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let runs = self.shape.runs(&space.view, self.look);
        let painter = ui.content_painter(cx);
        match self.look {
            MarqueeLook::Ants => {
                // One march covers a dash and its gap; where motion is off the ants stand still.
                let phase = march_shift(cx.phase(MARCH_SECONDS, Some(MARCH_TICK)), cx.style.ants_dash);
                for run in &runs {
                    painter.marching_ants(&run.points, run.closed, phase);
                }
            }
            MarqueeLook::Band => runs.iter().for_each(|run| paint_band(&painter, run, cx.style)),
            MarqueeLook::Drawing => runs.iter().for_each(|run| paint_haloed(&painter, run, cx.style, cx.style.light.egui(), cx.style.dark.egui())),
            MarqueeLook::Contrast(color) => runs.iter().for_each(|run| paint_haloed(&painter, run, cx.style, cx.style.dark.egui(), color.0)),
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<CanvasControlWords>();
        access.description(self.shape_name(&words).as_str());
        let value = match &self.value {
            Some(value) => value.as_str().to_owned(),
            None => {
                let (bounds, units) = (self.shape.bounds(), cx.env.units());
                let numbers = [bounds.width(), bounds.height(), bounds.x0, bounds.y0].map(|n| units.pixels(n));
                words.bounds.fill(&[("width", &numbers[0]), ("height", &numbers[1]), ("x", &numbers[2]), ("y", &numbers[3])])
            }
        };
        access.value(value);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
