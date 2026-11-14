//! [`StrokePreview`]: a stroke style drawn as a line sample with the content painter.

use craft_ui::primitives::{ContentPainterCx, PaintShape, StrokeWidth};
use egui::{Pos2, Rect, Shape, Stroke, Vec2};
use kurbo::{BezPath, PathEl, Point};

use super::StrokeStyle;
use crate::stroke::LineCap;

/// The largest distance a circle fan may lie inside the true circle, in points.
const FAN_TOLERANCE: f64 = 0.1;

/// The sample line's width as a share of the rect's height, for a sample sized from its rect.
const LINE_SHARE: f32 = 0.1;
/// The line's distance from the rect's ends, as a share of the rect's height.
const MARGIN_SHARE: f32 = 0.15;

/// A line sample of a [`StrokeStyle`] in a rect: a horizontal line through the rect's centre, drawn
/// left to right in every reading direction and centred whatever the style's alignment. The line
/// is dashed with `kurbo::dash` from [`StrokeStyle::to_kurbo`] (the dashes scaled to the sample's
/// width), then each dash is filled as convex pieces: a quad for the dash, a fan for round caps, a
/// quad for square caps.
///
/// It is a helper of content painters, not a component: [`paint`](Self::paint) fills
/// [`shapes`](Self::shapes) in the text ink of the painter's surface.
///
/// A style that fails [`StrokeStyle::validate`] draws nothing. The style's opacity applies to the
/// ink (`ink`).
pub struct StrokePreview<'a> {
    value: &'a StrokeStyle,
    width: f32,
    margin: Option<f32>,
}

impl<'a> StrokePreview<'a> {
    /// A sample of `value` drawn `width` points wide.
    pub fn new(value: &'a StrokeStyle, width: f32) -> Self {
        Self { value, width, margin: None }
    }

    /// A sample of `value` whose line is a share of `rect`'s height wide, so it scales with the row or
    /// button that holds it.
    pub fn sized(value: &'a StrokeStyle, rect: Rect) -> Self {
        Self::new(value, rect.height() * LINE_SHARE)
    }

    /// The line's distance from the rect's left and right edges, in points; a share of the
    /// rect's height by default.
    #[must_use]
    pub fn margin(mut self, margin: f32) -> Self {
        self.margin = Some(margin);
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
        let Some((start, end)) = self.line(rect) else {
            return Vec::new();
        };
        let stroke = self.sample_stroke();
        if stroke.dash_pattern.is_empty() {
            pieces.run(&[start, end]);
        } else {
            let mut path = BezPath::new();
            path.move_to(start);
            path.line_to(end);
            let mut run: Vec<Point> = Vec::new();
            for el in kurbo::dash(path.iter(), stroke.dash_offset, &stroke.dash_pattern) {
                match el {
                    PathEl::MoveTo(p) => {
                        pieces.run(&run);
                        run.clear();
                        run.push(p);
                    }
                    PathEl::LineTo(p) => run.push(p),
                    _ => {}
                }
            }
            pieces.run(&run);
        }
        pieces.out.into_iter().map(|points| PaintShape::Polygon(points.into_iter().map(to_pos).collect())).collect()
    }

    /// The line the stroke follows, in screen points, when the rect is wide enough for it.
    fn line(&self, rect: Rect) -> Option<(Point, Point)> {
        let y = f64::from(rect.center().y);
        let margin = f64::from(self.margin.unwrap_or(rect.height() * MARGIN_SHARE));
        let (left, right) = (f64::from(rect.left()) + margin, f64::from(rect.right()) - margin);
        (right > left).then(|| (Point::new(left, y), Point::new(right, y)))
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

    /// The sample of no stroke in `rect`: a faint line through the centre, crossed by a slash in the failure colour
    /// from the bottom of the rect to its top, half the height wide.
    pub fn paint_off(cx: &ContentPainterCx, rect: Rect) {
        let style = cx.style_cx();
        let c = &style.tokens.color;
        let (line, slash) = (style.color(c.text.faint), style.color(c.status.bad.fg));
        let half = rect.height() / 2.0;
        let center = rect.center();
        cx.with_draw(|draw| {
            draw.stroke(&PaintShape::Path(vec![rect.left_center(), rect.right_center()]), StrokeWidth::Thin, line);
            let across = vec![center + Vec2::new(-half / 2.0, half), center + Vec2::new(half / 2.0, -half)];
            draw.stroke(&PaintShape::Path(across), StrokeWidth::Medium, slash);
        });
    }
}

fn to_pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

/// The convex pieces of the dashes of a straight line.
struct Pieces<'a> {
    half: f64,
    style: &'a StrokeStyle,
    out: Vec<Vec<Point>>,
}

impl Pieces<'_> {
    /// The pieces of one dash: `points` lie on one straight line, in order.
    fn run(&mut self, points: &[Point]) {
        let mut pts: Vec<Point> = Vec::with_capacity(points.len());
        for &p in points {
            if pts.last().is_none_or(|q| q.distance(p) > 1e-9) {
                pts.push(p);
            }
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
                for pair in pts.windows(2) {
                    self.segment(pair[0], pair[1]);
                }
                self.cap(pts[1], pts[0]);
                self.cap(pts[n - 2], pts[n - 1]);
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
