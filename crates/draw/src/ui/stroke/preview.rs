//! [`StrokePreview`]: a stroke style drawn as a line sample with the content painter.

use craft_ui::primitives::{ContentPainterCx, PaintShape};
use egui::{Pos2, Rect, Shape, Stroke};
use kurbo::{BezPath, PathEl, Point};

use super::StrokeStyle;
use crate::stroke::{LineCap, LineJoin, StrokeAlign};

/// The largest distance a circle fan may lie inside the true circle, in points.
const FAN_TOLERANCE: f64 = 0.1;

/// The sample line's width as a share of the rect's height, for a sample sized from its rect.
const LINE_SHARE: f32 = 0.1;
/// The open line's distance from the rect's ends and the closed rectangle's inset, as a share of the rect's height.
const MARGIN_SHARE: f32 = 0.15;

/// A line sample of a [`StrokeStyle`] in a rect: the outline is dashed with `kurbo::dash` from
/// [`StrokeStyle::to_kurbo`] (the dashes scaled to the sample's width), then each dash is filled
/// as convex pieces: a quad per flattened segment, a fan for round joins and caps, a triangle for
/// bevels, a wedge for miters within the limit, a quad for square caps.
///
/// It is a helper of content painters, not a component: [`paint`](Self::paint) fills
/// [`shapes`](Self::shapes) in the text ink of the painter's surface.
///
/// - Open ([`closed`](Self::closed) false, the default): a horizontal line through the rect's
///   centre, aligned to the centre whatever the style says; drawn left to right in every reading
///   direction.
/// - Closed: a rectangle inset from the rect, so corners and alignment show (inside or outside a
///   closed path's line by half the width).
/// - A style that fails [`StrokeStyle::validate`] draws nothing. The style's opacity applies to
///   the ink ([`ink`](Self::ink)).
pub struct StrokePreview<'a> {
    value: &'a StrokeStyle,
    width: f32,
    closed: bool,
    inset: Option<f32>,
    margin: Option<f32>,
}

impl<'a> StrokePreview<'a> {
    /// A sample of `value` drawn `width` points wide.
    pub fn new(value: &'a StrokeStyle, width: f32) -> Self {
        Self { value, width, closed: false, inset: None, margin: None }
    }

    /// A sample of `value` whose line is a share of `rect`'s height wide, so it scales with the row or
    /// button that holds it.
    pub fn sized(value: &'a StrokeStyle, rect: Rect) -> Self {
        Self::new(value, rect.height() * LINE_SHARE)
    }

    /// Draws a closed rectangle in place of an open line.
    #[must_use]
    pub fn closed(mut self, closed: bool) -> Self {
        self.closed = closed;
        self
    }

    /// The open line's distance from the rect's left and right edges, in points; a share of the
    /// rect's height by default.
    #[must_use]
    pub fn margin(mut self, margin: f32) -> Self {
        self.margin = Some(margin);
        self
    }

    /// The closed sample's inset from the rect, in points; a share of the rect's height by default.
    #[must_use]
    pub fn inset(mut self, inset: f32) -> Self {
        self.inset = Some(inset);
        self
    }

    /// The sample's shapes in `rect` (screen points): every dash as filled convex polygons and
    /// circle fans. A style that fails validation, or a rect too small for the sample, gives none.
    pub fn shapes(&self, rect: Rect) -> Vec<PaintShape> {
        if self.value.validate().is_err() || self.width <= 0.0 || self.value.width <= 0.0 {
            return Vec::new();
        }
        let half = f64::from(self.width) / 2.0;
        let mut pieces = Pieces { half, style: self.value, out: Vec::new() };
        let outline = self.outline(rect, half);
        if outline.len() < 2 {
            return Vec::new();
        }
        let stroke = self.sample_stroke();
        if stroke.dash_pattern.is_empty() {
            pieces.polyline(&outline, self.closed);
        } else {
            let mut path = BezPath::new();
            path.move_to(outline[0]);
            for &point in &outline[1..] {
                path.line_to(point);
            }
            if self.closed {
                path.close_path();
            }
            let mut run: Vec<Point> = Vec::new();
            let mut closed_run = false;
            for el in kurbo::dash(path.iter(), stroke.dash_offset, &stroke.dash_pattern) {
                match el {
                    PathEl::MoveTo(p) => {
                        pieces.polyline(&run, closed_run);
                        run.clear();
                        closed_run = false;
                        run.push(p);
                    }
                    PathEl::LineTo(p) => run.push(p),
                    PathEl::ClosePath => closed_run = true,
                    _ => {}
                }
            }
            pieces.polyline(&run, closed_run);
        }
        pieces.out.into_iter().map(|points| PaintShape::Polygon(points.into_iter().map(to_pos).collect())).collect()
    }

    /// The path the stroke follows, in screen points: the open line, or the closed rectangle moved
    /// in or out by half the width for inside and outside alignment.
    fn outline(&self, rect: Rect, half: f64) -> Vec<Point> {
        let share = rect.height() * MARGIN_SHARE;
        if !self.closed {
            let y = f64::from(rect.center().y);
            let margin = f64::from(self.margin.unwrap_or(share));
            let (left, right) = (f64::from(rect.left()) + margin, f64::from(rect.right()) - margin);
            return if right > left { vec![Point::new(left, y), Point::new(right, y)] } else { Vec::new() };
        }
        let grow = match self.value.align {
            StrokeAlign::Inside => -half,
            StrokeAlign::Center => 0.0,
            StrokeAlign::Outside => half,
        };
        let inset = f64::from(self.inset.unwrap_or(share)) - grow;
        let (l, t) = (f64::from(rect.left()) + inset, f64::from(rect.top()) + inset);
        let (r, b) = (f64::from(rect.right()) - inset, f64::from(rect.bottom()) - inset);
        if r <= l || b <= t {
            return Vec::new();
        }
        vec![Point::new(l, t), Point::new(r, t), Point::new(r, b), Point::new(l, b)]
    }

    /// The style's kurbo stroke with the dashes rescaled from the style's width to the sample's, so
    /// the sample keeps the pattern's proportions.
    fn sample_stroke(&self) -> kurbo::Stroke {
        let mut stroke = self.value.to_kurbo();
        let scale = f64::from(self.width / self.value.width);
        stroke.dash_offset *= scale;
        for d in &mut stroke.dash_pattern {
            *d *= scale;
        }
        stroke
    }

    /// Fills [`shapes`](Self::shapes) for `rect` in the painter's ink at the style's opacity.
    pub fn paint(&self, cx: &ContentPainterCx, rect: Rect) {
        let ink = cx.ink();
        let mut faded = cx.painter.clone();
        faded.multiply_opacity(self.value.opacity);
        for shape in self.shapes(rect) {
            if let PaintShape::Polygon(points) = shape {
                faded.add(Shape::convex_polygon(points, ink, Stroke::NONE));
            }
        }
    }
}

fn to_pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

/// The convex pieces of a stroked polyline.
struct Pieces<'a> {
    half: f64,
    style: &'a StrokeStyle,
    out: Vec<Vec<Point>>,
}

impl Pieces<'_> {
    fn polyline(&mut self, points: &[Point], closed: bool) {
        let mut pts: Vec<Point> = Vec::with_capacity(points.len());
        for &p in points {
            if pts.last().is_none_or(|q| q.distance(p) > 1e-9) {
                pts.push(p);
            }
        }
        if closed && pts.len() > 1 && pts[0].distance(pts[pts.len() - 1]) <= 1e-9 {
            pts.pop();
        }
        match pts.len() {
            0 => {}
            1 => {
                // A zero-length dash: a round cap makes a dot, other caps have no direction.
                if self.style.cap == LineCap::Round {
                    self.fan(pts[0]);
                }
            }
            n => {
                let segments = if closed { n } else { n - 1 };
                for i in 0..segments {
                    self.segment(pts[i], pts[(i + 1) % n]);
                }
                let joins = if closed { 0..n } else { 1..n - 1 };
                for i in joins {
                    self.join(pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
                }
                if !closed {
                    self.cap(pts[1], pts[0]);
                    self.cap(pts[n - 2], pts[n - 1]);
                }
            }
        }
    }

    fn normal(&self, a: Point, b: Point) -> kurbo::Vec2 {
        let d = b - a;
        kurbo::Vec2::new(-d.y, d.x).normalize() * self.half
    }

    fn segment(&mut self, a: Point, b: Point) {
        let n = self.normal(a, b);
        self.out.push(vec![a + n, b + n, b - n, a - n]);
    }

    fn join(&mut self, before: Point, at: Point, after: Point) {
        let (n1, n2) = (self.normal(before, at), self.normal(at, after));
        let turn = (at - before).cross(after - at);
        if turn.abs() < 1e-9 {
            return;
        }
        // The outer side of the turn.
        let side = if turn > 0.0 { -1.0 } else { 1.0 };
        let (a, b) = (at + n1 * side, at + n2 * side);
        match self.style.join {
            LineJoin::Round => self.fan(at),
            LineJoin::Bevel => self.out.push(vec![at, a, b]),
            LineJoin::Miter => {
                // The miter length over the width is 1 / sin(theta / 2), theta the angle between
                // the segments; cos(turn) = n1 . n2 / half^2.
                let cos_turn = (n1.dot(n2) / (self.half * self.half)).clamp(-1.0, 1.0);
                let sin_half = ((1.0 + cos_turn) / 2.0).sqrt();
                let ratio = if sin_half > 1e-9 { 1.0 / sin_half } else { f64::INFINITY };
                if ratio <= f64::from(self.style.miter_limit) {
                    let mid = (a - at) + (b - at);
                    let tip = at + mid * (1.0 / (1.0 + cos_turn));
                    self.out.push(vec![at, a, tip, b]);
                } else {
                    self.out.push(vec![at, a, b]);
                }
            }
        }
    }

    /// The cap at `end` of a line arriving from `from`.
    fn cap(&mut self, from: Point, end: Point) {
        match self.style.cap {
            LineCap::Butt => {}
            LineCap::Round => self.fan(end),
            LineCap::Square => {
                let n = self.normal(from, end);
                let out = kurbo::Vec2::new(n.y, -n.x);
                let out = if (end - from).dot(out) < 0.0 { -out } else { out };
                self.out.push(vec![end + n, end + n + out, end - n + out, end - n]);
            }
        }
    }

    fn fan(&mut self, centre: Point) {
        let steps = if self.half > FAN_TOLERANCE {
            (std::f64::consts::PI / (1.0 - FAN_TOLERANCE / self.half).acos()).ceil().clamp(8.0, 64.0) as usize
        } else {
            8
        };
        self.out.push(
            (0..steps)
                .map(|i| {
                    let angle = std::f64::consts::TAU * i as f64 / steps as f64;
                    centre + kurbo::Vec2::from_angle(angle) * self.half
                })
                .collect(),
        );
    }
}
