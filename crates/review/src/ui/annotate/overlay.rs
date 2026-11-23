//! Painting annotations on the canvas: [`AnnotationOverlay`].

use craft_behavior::{Input, Metrics};
use craft_ui::canvas::canvas_space;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role};
use craft_ui::fonts::FontRef;
use craft_ui::primitives::{DataWidth, DocRun, DocText, Draw, Handle, HandleShape, PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, DataColor, HandleLook, StateSet, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Response};
use kurbo::{Point, Rect};

use super::tool::{color32, corners};
use crate::annotate::{Annotation, AnnotationHandle, AnnotationId, AnnotationKind, handles};

/// The ellipse inscribed in `rect`, as a closed run of page points.
pub(super) fn ellipse_points(rect: Rect) -> Vec<Point> {
    const STEPS: u32 = 48;
    let (c, rx, ry) = (rect.center(), rect.width() / 2.0, rect.height() / 2.0);
    (0..STEPS)
        .map(|i| {
            let t = std::f64::consts::TAU * f64::from(i) / f64::from(STEPS);
            Point::new(c.x + rx * t.cos(), c.y + ry * t.sin())
        })
        .collect()
}

/// An arrow head's length and a cloud scallop's width, in stroke widths.
const ARROW_HEAD: f32 = 6.0;
const CLOUD_BUMP: f32 = 8.0;
/// A squiggle's wavelength, in stroke widths.
const WAVE: f32 = 4.0;
/// The points a cloud scallop is drawn with.
const SCALLOP_STEPS: u32 = 8;

fn scalloped(corners: &[Pos2], bump: f32) -> Vec<Pos2> {
    let area: f32 = corners.iter().zip(corners.iter().cycle().skip(1)).map(|(a, b)| a.x * b.y - b.x * a.y).sum();
    let outward = if area >= 0.0 { -1.0 } else { 1.0 };
    let mut out = Vec::new();
    for (a, b) in corners.iter().zip(corners.iter().cycle().skip(1)) {
        let edge = *b - *a;
        let count = (edge.length() / bump).round().max(1.0) as usize;
        let normal = egui::vec2(-edge.y, edge.x).normalized() * outward;
        for i in 0..count {
            let from = *a + edge * (i as f32 / count as f32);
            let to = *a + edge * ((i + 1) as f32 / count as f32);
            let radius = (to - from).length() / 2.0;
            let centre = from + (to - from) / 2.0;
            for s in 0..SCALLOP_STEPS {
                let t = std::f32::consts::PI * s as f32 / SCALLOP_STEPS as f32;
                // Rotates from the segment's start, over the outward side, to its end.
                let along = (from - centre).normalized();
                out.push(centre + along * radius * t.cos() + normal * radius * t.sin());
            }
        }
    }
    out
}

fn screen_rect(rect: Rect, to_screen: &impl Fn(Point) -> Pos2) -> egui::Rect {
    egui::Rect::from_two_pos(to_screen(Point::new(rect.x0, rect.y0)), to_screen(Point::new(rect.x1, rect.y1)))
}

/// The screen rect that holds `points`.
fn around(points: impl IntoIterator<Item = Pos2>) -> egui::Rect {
    points.into_iter().fold(egui::Rect::NOTHING, |r, p| r.union(egui::Rect::from_min_max(p, p)))
}

/// The outline of the icon of a note, an attachment or a caret in the unit square, as open or closed runs.
fn pictogram(kind: AnnotationKind) -> Vec<(Vec<(f32, f32)>, bool)> {
    match kind {
        AnnotationKind::Note => vec![(vec![(0.1, 0.1), (0.9, 0.1), (0.9, 0.7), (0.5, 0.7), (0.25, 0.92), (0.25, 0.7), (0.1, 0.7)], true)],
        AnnotationKind::Attach => vec![(vec![(0.72, 0.15), (0.3, 0.6), (0.4, 0.85), (0.82, 0.4), (0.62, 0.12), (0.2, 0.55)], false)],
        _ => vec![(vec![(0.15, 0.85), (0.5, 0.15), (0.85, 0.85)], false), (vec![(0.5, 0.15), (0.5, 0.95)], false)],
    }
}

/// `points` as a closed path.
fn closed(mut points: Vec<Pos2>) -> PaintShape {
    if let Some(&first) = points.first() {
        points.push(first);
    }
    PaintShape::Path(points)
}

fn paint_annotation(draw: &Draw<'_>, a: &Annotation, to_screen: &impl Fn(Point) -> Pos2, (text_size, icon_side): (f32, f32), hairline: f32) {
    use AnnotationKind as K;
    let scale = (to_screen(Point::new(1.0, 0.0)) - to_screen(Point::ZERO)).length();
    let color = DataColor::new(color32(a));
    // The document's width through the view, never thinner than a physical pixel.
    let width = (a.stroke_width as f32 * scale).max(hairline);
    let line = |shape: PaintShape| draw.stroke(&shape, DataWidth::points(width), color);
    let map = |points: &[Point]| -> Vec<Pos2> { points.iter().map(|&p| to_screen(p)).collect() };
    let quads = || -> Vec<egui::Rect> { a.points.chunks(4).map(|quad| around(quad.iter().map(|&p| to_screen(p)))).collect() };
    match a.kind {
        K::Highlight => {
            for r in quads() {
                draw.data_fill(&PaintShape::Rect(r, CornerRadius::ZERO), color);
            }
        }
        K::Underline | K::StrikeOut | K::Squiggly | K::ReplaceText => {
            for r in quads() {
                let y = if matches!(a.kind, K::StrikeOut | K::ReplaceText) { r.center().y } else { r.bottom() };
                if a.kind == K::Squiggly {
                    let (amplitude, length) = (width, width * WAVE);
                    let steps = (r.width() / length * 2.0).ceil().max(1.0) as usize;
                    let wave: Vec<Pos2> = (0..=steps)
                        .map(|i| egui::pos2(r.left() + r.width() * i as f32 / steps as f32, y + if i % 2 == 0 { -amplitude } else { amplitude }))
                        .collect();
                    line(PaintShape::Path(wave));
                } else {
                    line(PaintShape::Path(vec![egui::pos2(r.left(), y), egui::pos2(r.right(), y)]));
                }
            }
        }
        K::Line | K::Arrow | K::PolyLine => {
            let path = map(&a.points);
            if let [.., from, to] = path[..] {
                line(PaintShape::Path(path.clone()));
                if a.kind == K::Arrow {
                    let back = (from - to).normalized() * width * ARROW_HEAD;
                    for turn in [0.5_f32, -0.5] {
                        let (s, c) = turn.sin_cos();
                        line(PaintShape::Path(vec![to, to + egui::vec2(back.x * c - back.y * s, back.x * s + back.y * c)]));
                    }
                }
            }
        }
        K::Polygon => line(closed(map(&a.points))),
        K::Cloud => {
            let corners = map(&a.points);
            if corners.len() > 2 {
                line(closed(scalloped(&corners, width * CLOUD_BUMP)));
            }
        }
        K::Ink => {
            for s in &a.strokes {
                line(PaintShape::Path(map(s)));
            }
        }
        K::Rectangle => line(closed(map(&corners(a.rect)))),
        K::Oval => line(closed(map(&ellipse_points(a.rect)))),
        K::TextBox | K::Callout => {
            let r = screen_rect(a.rect, to_screen);
            line(PaintShape::Rect(r, CornerRadius::ZERO));
            // The text is document-scaled: the body size at zoom 1, wrapped at the box's width.
            let run = DocRun::new(a.text.as_str(), FontRef::proportional(), text_size * scale, color);
            draw.clipped(r, |draw| {
                draw.doc(r, &DocText::new(&[run]));
            });
        }
        K::Note | K::Attach | K::Caret => {
            let slot = egui::Rect::from_min_size(to_screen(Point::new(a.rect.x0, a.rect.y0)), egui::Vec2::splat(icon_side));
            for (run, is_closed) in pictogram(a.kind) {
                let points: Vec<Pos2> = run.iter().map(|&(x, y)| slot.min + egui::vec2(x * slot.width(), y * slot.height())).collect();
                line(if is_closed { closed(points) } else { PaintShape::Path(points) });
            }
        }
    }
}

/// The four corners of `rect` as a closed path, for an outline centred on the rect's edge.
fn ring(rect: egui::Rect) -> PaintShape {
    closed(vec![rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()])
}

/// The style of the annotation overlay.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct AnnotationOverlayStyle {
    #[style(metrics)]
    metrics: Metrics,
    handle: HandleLook,
    line: StyleColor,
    text_size: f32,
    /// The side of a note, attachment or caret icon on screen, whatever the zoom.
    icon_side: f32,
}

/// Draws a page's annotations over it: markups as translucent quads (highlight at the annotation's opacity, lines
/// for underline and strike-out, a wave for squiggly), shapes and ink with their stroke, notes as their icon, text
/// boxes with their text, and the handles of the selected ones. A canvas control shown with `show_at` at the
/// canvas's rect and mapped through its view. It takes no pointer: the tool does.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct AnnotationOverlay<'a> {
    #[common]
    common: Common,
    annotations: &'a [Annotation],
    selected: &'a [AnnotationId],
}

impl<'a> AnnotationOverlay<'a> {
    /// An overlay named `label` for `annotations`, with `selected` drawing handles.
    pub fn new(label: impl Into<Label>, annotations: &'a [Annotation], selected: &'a [AnnotationId]) -> Self {
        AnnotationOverlay { common: Common::new(), annotations, selected }.label(label)
    }
}

impl Component for AnnotationOverlay<'_> {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = AnnotationOverlayStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: AnnotationOverlayStyleKey) -> AnnotationOverlayStyle {
        AnnotationOverlayStyle {
            metrics: Metrics::new(),
            handle: cx.handle(),
            line: cx.color(cx.tokens.color.overlay.line),
            text_size: cx.tokens.typography.body.size,
            icon_side: cx.tokens.size.icon.md,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let view = space.view;
        let to_screen = |p: Point| view.to_screen(p);
        let painter = ui.content_painter(cx);
        let draw = ui.draw();
        let hairline = 1.0 / cx.response.ctx.pixels_per_point();
        for a in self.annotations {
            paint_annotation(&draw, a, &to_screen, (cx.style.text_size, cx.style.icon_side), hairline);
        }
        let outline = |shape: PaintShape| draw.stroke(&shape, StrokeWidth::Thin, cx.style.line);
        let raw = &painter.painter;
        for a in self.annotations.iter().filter(|a| self.selected.contains(&a.id)) {
            if a.kind.is_markup() {
                for quad in a.points.chunks(4) {
                    outline(ring(around(quad.iter().map(|&p| to_screen(p)))));
                }
            } else if a.kind.is_pointwise() && !a.points.is_empty() {
                outline(PaintShape::Path(a.points.iter().map(|&p| to_screen(p)).collect()));
            } else {
                outline(ring(screen_rect(a.rect, &to_screen)));
            }
            for (handle, at) in handles(a) {
                let shape = match handle {
                    AnnotationHandle::Box(..) => HandleShape::Square,
                    AnnotationHandle::Point(_) => HandleShape::Circle,
                };
                raw.extend(Handle::shapes(&cx.style.handle, shape, to_screen(at), &StateSet::default()));
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
