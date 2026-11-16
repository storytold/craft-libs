//! Shape outlines and the drag rules that size them: [`ShapeGeometry`] (a live shape's outline as
//! a [`BezPath`]), the box drag ([`box_drag`]), the line drag ([`line_drag`]) and the radial drag
//! ([`RadialFrame::drag`]). No egui: an engine, importer or exporter builds the same outline the
//! Shape tools preview (`craft_draw::ui::shape_drag`).
//!
//! Document units throughout; y grows downwards. Corner radii run top-left, top-right,
//! bottom-right, bottom-left (PhotoCraft's engine order).

use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::ops::RangeInclusive;

use kurbo::{BezPath, Ellipse, Point, Rect, RoundedRect, RoundedRectRadii, Shape, Vec2};

/// The sides a polygon or the points a star may have (PhotoCraft's Sides field, 3–100).
pub const SIDES: RangeInclusive<u32> = 3..=100;

/// The inner radius of a star over its outer radius; 1 is a plain polygon (PhotoCraft's Star
/// ratio field, 1–100 %, stored 0.01–1).
pub const STAR_RATIO: RangeInclusive<f64> = 0.01..=1.0;

/// The smallest side a box drag draws and the shortest line, in document units: anything smaller
/// draws nothing (PhotoCraft).
pub const MIN_SIDE: f64 = 1.0;

/// A shape's outline: what a Shape tool drag, the Create Shape dialog and the Properties panel
/// describe, and what the app's engine draws.
#[derive(Clone, Debug, PartialEq)]
pub enum ShapeGeometry {
    /// A rectangle; `radii` round the corners top-left, top-right, bottom-right, bottom-left (all
    /// 0 for square corners). Photoshop's Rectangle with a radius and VectorCraft's Rounded
    /// Rectangle are this with radii.
    Rect {
        /// The rectangle's bounds.
        rect: Rect,
        /// Corner radii, in document units.
        radii: [f64; 4],
    },
    /// An ellipse inscribed in `rect`.
    Ellipse {
        /// The bounds of the ellipse.
        rect: Rect,
    },
    /// A triangle filling `rect`: the apex at the top centre, the base on the bottom edge (PhotoCraft's Triangle
    /// tool and Create Triangle).
    Triangle {
        /// The triangle's bounds.
        rect: Rect,
    },
    /// A regular polygon or star inscribed in the ellipse of `rect` (the Box frame: PhotoCraft's Polygon).
    /// `sides` in [`SIDES`]; `star_ratio` in [`STAR_RATIO`], 1 for a plain polygon.
    Polygon {
        /// The box whose inscribed ellipse holds the polygon.
        rect: Rect,
        /// Number of sides.
        sides: u32,
        /// Inner radius over outer radius for a star; 1 for a plain polygon.
        star_ratio: f64,
    },
    /// A regular polygon or star about `center` (the Radial frame: VectorCraft's Polygon and
    /// Star). `radius` reaches the outer points; `rotation` turns the first point from straight
    /// up, in radians, clockwise on screen.
    RadialPolygon {
        /// The centre of the polygon.
        center: Point,
        /// Distance from the centre to the outer points, in document units.
        radius: f64,
        /// Turn of the first point from straight up, in radians, clockwise on screen.
        rotation: f64,
        /// Number of sides.
        sides: u32,
        /// Inner radius over outer radius for a star; 1 for a plain polygon.
        star_ratio: f64,
    },
    /// A line from `from` to `to`, `weight` wide (Photoshop's Line tool: a filled bar).
    Line {
        /// The start of the line.
        from: Point,
        /// The end of the line.
        to: Point,
        /// The bar's width, in document units.
        weight: f64,
    },
    /// A custom shape's box: the app fits its preset path into `rect`, keeping the preset's
    /// proportions when `keep_proportions` (⇧ held, PhotoCraft's `finish_custom_shape`).
    Custom {
        /// The box the preset path is fitted into.
        rect: Rect,
        /// Whether the preset keeps its own proportions inside `rect`.
        keep_proportions: bool,
    },
}

impl ShapeGeometry {
    /// The outline as a closed path; `tolerance` (document units) bounds the error where a curve
    /// is approximated.
    ///
    /// - `Rect`: the four corners clockwise from the top left, each rounded by its radius (clamped
    ///   to half the shorter side, as kurbo's `RoundedRect` does); all radii 0 is the plain
    ///   rectangle.
    /// - `Ellipse`: kurbo's `Ellipse::from_rect`.
    /// - `Triangle`: the top centre, the bottom right and the bottom left of `rect`.
    /// - `Polygon`: the outline of [`RadialPolygon`](Self::RadialPolygon) with the first point
    ///   straight up, its outer points on the ellipse inscribed in `rect` (PhotoCraft's engine:
    ///   a 3-sided polygon's base lies a quarter of the height below the centre, not on the bottom
    ///   edge); a star alternates outer and inner points, the inner ones on the ellipse
    ///   scaled by `star_ratio`.
    /// - `RadialPolygon`: `sides` outer points on the circle of `radius`, the first at `rotation`
    ///   from straight up; a star (`star_ratio` < 1) puts an inner point between each pair at
    ///   `radius * star_ratio`.
    /// - `Line`: the bar `weight` wide centred on the segment, square ends at `from` and `to`.
    /// - `Custom`: the box itself (the app draws the preset).
    pub fn to_path(&self, tolerance: f64) -> BezPath {
        // pc: crates/ui-egui/src/tools/vector_ui/shapes.rs:73-79 (the kinds: `roundedRect` with four
        // equal radii, ellipse, `triangle` filling its rect, `polygon_sides.max(3)`).
        // new: PhotoCraft's engine draws the outline; here it is kurbo's `RoundedRect`, `Ellipse`
        // and a point list, so engines without one share it.
        let tolerance = tolerance.max(f64::EPSILON);
        match *self {
            Self::Rect { rect, radii } => {
                let rect = rect.abs();
                if radii.iter().all(|r| *r <= 0.0) {
                    rect.to_path(tolerance)
                } else {
                    RoundedRect::from_rect(rect, RoundedRectRadii::new(radii[0], radii[1], radii[2], radii[3])).to_path(tolerance)
                }
            }
            Self::Ellipse { rect } => Ellipse::from_rect(rect.abs()).to_path(tolerance),
            Self::Triangle { rect } => {
                // pc: #13350 (the triangle fills its box, apex on top).
                let rect = rect.abs();
                closed(&[Point::new(rect.center().x, rect.y0), Point::new(rect.x1, rect.y1), Point::new(rect.x0, rect.y1)])
            }
            Self::Polygon { rect, sides, star_ratio } => {
                let rect = rect.abs();
                let (c, rx, ry) = (rect.center(), rect.width() / 2.0, rect.height() / 2.0);
                let points: Vec<Point> =
                    polygon_points(Point::ZERO, 1.0, 0.0, sides, star_ratio).iter().map(|p| Point::new(c.x + p.x * rx, c.y + p.y * ry)).collect();
                closed(&points)
            }
            Self::RadialPolygon { center, radius, rotation, sides, star_ratio } => {
                closed(&polygon_points(center, radius, rotation, sides, star_ratio))
            }
            Self::Line { from, to, weight } => {
                let v = to - from;
                let len = v.hypot();
                let n = if len > 0.0 { Vec2::new(-v.y, v.x) * (weight / 2.0 / len) } else { Vec2::ZERO };
                closed(&[from + n, to + n, to - n, from - n])
            }
            Self::Custom { rect, .. } => rect.abs().to_path(tolerance),
        }
    }

    /// The axis-aligned box the shape occupies: the rect of the box kinds, the circle's box of a
    /// radial polygon, the box of a line's ends. Space-drag snapping uses it (PhotoCraft's
    /// `shape_bounds`).
    pub fn bounds(&self) -> Rect {
        // pc: crates/ui-egui/src/tools/vector_ui/shapes.rs:85-96 `shape_bounds` (a line's ends' box).
        match *self {
            Self::Rect { rect, .. } | Self::Ellipse { rect } | Self::Triangle { rect } | Self::Polygon { rect, .. } | Self::Custom { rect, .. } => {
                rect.abs()
            }
            Self::RadialPolygon { center, radius, .. } => Rect::new(center.x - radius, center.y - radius, center.x + radius, center.y + radius),
            Self::Line { from, to, .. } => Rect::from_points(from, to),
        }
    }
}

/// How a box drag turns the press and the pointer into a box ([`box_drag`]). The default draws the dragged box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct BoxRule {
    /// Both sides take the drag's shorter extent, each keeping its direction (⇧).
    pub square: bool,
    /// The press is the centre: the box is twice the extent (⌥). With `fixed`, the release is the centre.
    pub from_centre: bool,
    /// A fixed width and height: the box has this size with its top left at the release, whatever the drag.
    pub fixed: Option<(f64, f64)>,
    /// A width to height ratio: the largest box of this ratio inside the dragged one, from the press.
    pub ratio: Option<(f64, f64)>,
}

/// The box a drag from `start` to `end` draws under `rule`, in whole pixels, or `None` when a side is under
/// [`MIN_SIDE`].
///
/// - `fixed` (both sides finite and above 0): that size at the release; `square` and `ratio` do not apply.
/// - `square`: both sides take the drag's shorter extent, each keeping its direction.
/// - `ratio` (both parts finite and above 0): the largest box of that ratio inside the (squared) drag.
/// - `from_centre`: `start` is the centre; the box is twice the extent.
/// - The top left and the size round to whole pixels, as PhotoCraft rounds `[x, y, w, h]`.
pub fn box_drag(start: Point, end: Point, rule: BoxRule) -> Option<Rect> {
    // pc: crates/ui-egui/src/tools/vector_ui/shapes.rs `drag_box` (⇧: `m = w.abs().min(h.abs())` with the
    // signs kept, #13222; ⌥: origin `start - (w, h)`, size doubled; the Rectangle gear's Fixed Size and
    // Proportional, #13343), `shape_geometry` (`[x0.min(x0 + w).round(), …, w.abs().round(), …]`;
    // `rect[2] < 1.0 || rect[3] < 1.0` draws nothing).
    let (x0, y0, w, h) = if let Some((fw, fh)) = rule.fixed.and_then(positive) {
        // From Center with Fixed Size is not measured: the size is centred on the release.
        if rule.from_centre { (end.x - fw / 2.0, end.y - fh / 2.0, fw, fh) } else { (end.x, end.y, fw, fh) }
    } else {
        let (mut w, mut h) = (end.x - start.x, end.y - start.y);
        if rule.square {
            let m = w.abs().min(h.abs());
            (w, h) = (m * sign(w), m * sign(h));
        }
        if let Some((rw, rh)) = rule.ratio.and_then(positive) {
            let s = (w.abs() / rw).min(h.abs() / rh);
            (w, h) = (rw * s * sign(w), rh * s * sign(h));
        }
        if rule.from_centre { (start.x - w, start.y - h, w * 2.0, h * 2.0) } else { (start.x, start.y, w, h) }
    };
    let (x, y, w, h) = (x0.min(x0 + w).round(), y0.min(y0 + h).round(), w.abs().round(), h.abs().round());
    (w >= MIN_SIDE && h >= MIN_SIDE).then(|| Rect::new(x, y, x + w, y + h))
}

/// `pair` when both parts are finite and above 0 (a gear field that is not empty).
fn positive(pair: (f64, f64)) -> Option<(f64, f64)> {
    (pair.0.is_finite() && pair.1.is_finite() && pair.0 > 0.0 && pair.1 > 0.0).then_some(pair)
}

/// The tight box of the equilateral triangle, apex up, inscribed in the circle of `square`: what ⇧ draws with the
/// Triangle tool (#13350). With the square centred on the press (⇧⌥), the triangle's centre is the press.
pub fn equilateral_in(square: Rect) -> Rect {
    let square = square.abs();
    let r = square.width() / 2.0;
    let half_base = r * 3f64.sqrt() / 2.0;
    let x = square.center().x - half_base;
    Rect::new(x, square.y0, x + 2.0 * half_base, square.y0 + 1.5 * r)
}

/// The end of a line dragged from `from` to `to`, or `None` when it is shorter than
/// [`MIN_SIDE`].
pub fn line_drag(from: Point, to: Point) -> Option<Point> {
    // pc: crates/ui-egui/src/tools/vector_ui/shapes.rs:52-61 (shorter than 1 draws nothing).
    ((to - from).hypot() >= MIN_SIDE).then_some(to)
}

/// A radial drag's frame: the press is the centre, the pointer sets the radius and the turn
/// (VectorCraft's Polygon and Star tools).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadialFrame {
    /// The press point, the frame's centre.
    pub center: Point,
    /// From the centre to the pointer, document units.
    pub radius: f64,
    /// The first point's turn from straight up, radians, clockwise on screen.
    pub rotation: f64,
}

impl RadialFrame {
    /// The frame of a drag from `center` to `at`, or `None` when the radius is under
    /// [`MIN_SIDE`]. The first point follows the pointer; `upright` (⇧) keeps the base
    /// orientation (rotation 0, the first point straight up).
    pub fn drag(center: Point, at: Point, upright: bool) -> Option<RadialFrame> {
        // inv-vectorcraft: crates/tools/src/shape.rs:131-137 (polygon and star: the press is the
        // centre, `r = v.hypot()`, rotation `(v.atan2().to_degrees() + 90.0) % 360.0`; Shift keeps the
        // base orientation). VectorCraft's radius floor is 0.01; the library's is `MIN_SIDE`.
        let v = at - center;
        let radius = v.hypot();
        if radius < MIN_SIDE {
            return None;
        }
        let rotation = if upright { 0.0 } else { (v.atan2() + FRAC_PI_2).rem_euclid(TAU) };
        Some(RadialFrame { center, radius, rotation })
    }

    /// The shape of this frame with `sides` points and `star_ratio`.
    pub fn geometry(self, sides: u32, star_ratio: f64) -> ShapeGeometry {
        ShapeGeometry::RadialPolygon { center: self.center, radius: self.radius, rotation: self.rotation, sides, star_ratio }
    }

    /// The star ratio that keeps the inner radius `inner` (held when Ctrl/Cmd went down) while
    /// the outer radius follows the pointer to [`radius`](Self::radius); clamped to
    /// [`STAR_RATIO`].
    pub fn ratio_holding(self, inner: f64) -> f64 {
        // inv-vectorcraft: crates/tools/src/shape.rs:5-7 (Ctrl/Cmd keeps the inner radius while the
        // outer one follows), :32-33 (`inner` held while Ctrl/Cmd is down).
        if self.radius > 0.0 { (inner / self.radius).clamp(*STAR_RATIO.start(), *STAR_RATIO.end()) } else { *STAR_RATIO.end() }
    }

    /// The star ratio whose shoulders are straight (⌥, five points or more): the line between
    /// two outer points two apart passes through the inner points between them. `None` under five
    /// points.
    pub fn straight_shoulders(sides: u32) -> Option<f64> {
        // inv-vectorcraft: crates/tools/src/shape.rs:42-48 `straight_ratio`: `(points >= 5).then(||
        // (2π / n).cos() / (π / n).cos())`.
        (sides >= 5).then(|| {
            let n = f64::from(sides);
            (TAU / n).cos() / (PI / n).cos()
        })
    }
}

fn sign(v: f64) -> f64 {
    if v < 0.0 { -1.0 } else { 1.0 }
}

/// The corners of a polygon or star about `center`, the first outer point `rotation` clockwise
/// from straight up (y down); a star puts an inner point between each pair of outer ones.
fn polygon_points(center: Point, radius: f64, rotation: f64, sides: u32, star_ratio: f64) -> Vec<Point> {
    let n = sides.clamp(*SIDES.start(), *SIDES.end());
    let step = TAU / f64::from(n);
    let at = |angle: f64, r: f64| Point::new(center.x + r * (rotation + angle).sin(), center.y - r * (rotation + angle).cos());
    let star = star_ratio < 1.0;
    let inner = radius * star_ratio.max(*STAR_RATIO.start());
    let mut points = Vec::with_capacity(if star { 2 * n as usize } else { n as usize });
    for i in 0..n {
        let angle = step * f64::from(i);
        points.push(at(angle, radius));
        if star {
            points.push(at(angle + step / 2.0, inner));
        }
    }
    points
}

fn closed(points: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    for (i, p) in points.iter().enumerate() {
        if i == 0 {
            path.move_to(*p);
        } else {
            path.line_to(*p);
        }
    }
    path.close_path();
    path
}
