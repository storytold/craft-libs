//! The painted pointers (8.5): hand, magnifier, brush outline, tool icon, badges and the rotate, distort and skew
//! pointers. Their art is drawn in points around the hotspot, like an OS cursor bitmap, so its coordinates are data
//! tables, not layout sizes.

use crate::primitives::{Draw, IconSpec};
use crate::style::StyleColor;
use craft_behavior::{CursorBadge, IconId};
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

/// The icon sets draw on a box this many units on a side; a tool pointer's hotspot is in these.
const ICON_UNITS: f32 = 24.0;
/// Where a badge sits beside a system pointer arrow: toward its lower right.
const BADGE_BESIDE: [f32; 2] = [12.0, 12.0];
/// Half the side of a badge glyph.
const BADGE_HALF: f32 = 3.5;
/// The scissors badge: how far its blades' ends sit from the axis, and the radius of its handle rings, as fractions of
/// `BADGE_HALF`; and the radius of the close badge's circle.
const SCISSORS_SPREAD: f32 = 0.5;
const SCISSORS_HANDLE: f32 = 0.35;
const CLOSE_RADIUS: f32 = 0.8;
/// The rotate pointer: radius of its arc, half its sweep in radians, and its segment count.
const TURN_RADIUS: f32 = 12.0;
const HALF_SWEEP: f32 = 0.75;
const ARC_SEGMENTS: usize = 16;
/// A rotate arrowhead's unit of size per radius, and its proportions in those units: the tip lies `HEAD_REACH` past the
/// arc's end, the head is `HEAD_LENGTH` long, its base lies back by `HEAD_BACK` and is `2 * HEAD_HALF_WIDTH` wide.
const HEAD_PER_RADIUS: f32 = 1.0 / 12.0;
const HEAD_REACH: f32 = 2.0;
const HEAD_LENGTH: f32 = 3.5;
const HEAD_BACK: f32 = 3.0;
const HEAD_HALF_WIDTH: f32 = 3.5;
/// The distort dart: length, half width, and how far back from the tip its notch lies as a fraction of the length.
const DART_LENGTH: f32 = 16.0;
const DART_HALF_WIDTH: f32 = 7.0;
const DART_NOTCH: f32 = 0.65;
/// The skew arrows: distance of each from the pointer across the edge, half length, head length and head back.
const SKEW_GAP: f32 = 4.0;
const SKEW_HALF_LENGTH: f32 = 9.0;
const SKEW_HEAD: f32 = 3.5;
const SKEW_BACK: f32 = 2.5;
/// The open hand: the offset of its box from the pointer, the palm's centre, half size and corner radius, the
/// fingers' x and tip heights, where the fingers start, their radius, and the thumb's ends and radius.
const HAND_SHIFT: [f32; 2] = [2.0, 0.5];
const PALM_CENTER: [f32; 2] = [0.0, 4.5];
const PALM_HALF: [f32; 2] = [6.8, 5.5];
const PALM_ROUND: f32 = 3.0;
const FINGER_X: [f32; 4] = [-5.1, -1.7, 1.7, 5.1];
const FINGER_TIP: [f32; 4] = [-7.0, -9.5, -8.5, -5.5];
const FINGER_BASE: f32 = 3.0;
const FINGER_R: f32 = 1.7;
const THUMB: ([f32; 2], [f32; 2], f32) = ([-4.5, 7.0], [-10.5, 1.5], 1.9);
/// The fist: the box, the knuckles' ends and radius, the thumb, the curl line across the fingers and where the finger
/// creases start.
const FIST_CENTER: [f32; 2] = [0.0, 3.0];
const FIST_HALF: [f32; 2] = [7.2, 6.5];
const FIST_ROUND: f32 = 3.6;
const KNUCKLE_FROM: f32 = -1.0;
const KNUCKLE_TO: f32 = -2.2;
const KNUCKLE_R: f32 = 1.9;
const FIST_THUMB: ([f32; 2], [f32; 2], f32) = ([-4.5, 7.0], [-9.6, 3.6], 2.0);
const FIST_CURL: ([f32; 2], [f32; 2]) = ([-4.0, 4.4], [6.0, 4.4]);
const FIST_CREASE_FROM: f32 = -2.6;
const CREASE_X: [f32; 3] = [-3.4, 0.0, 3.4];
const CREASE_END: f32 = 3.5;
/// The magnifier: the lens radius (its centre is the pointer), the handle's ends and radius, and the arms of the plus
/// or minus.
const LENS: f32 = 6.5;
const HANDLE: ([f32; 2], [f32; 2], f32) = ([5.2, 5.2], [10.8, 10.8], 2.2);
const GLYPH_ARM: f32 = 3.4;

fn v(point: [f32; 2]) -> Vec2 {
    Vec2::new(point[0], point[1])
}

/// What the pointers paint with: the painter, the pointer, the mark colour, its halo and the hairline width.
pub(super) struct Pen<'a> {
    pub painter: &'a Painter,
    pub at: Pos2,
    pub fill: Color32,
    pub halo: Color32,
    pub thin: f32,
}

/// A line mark, in points from the pointer.
enum Mark {
    Line(Vec2, Vec2),
    Circle(Vec2, f32),
    Rect(Rect),
}

impl Pen<'_> {
    /// `marks` as hairlines in the mark colour, each over a halo line a hairline wider on each side.
    fn marks(&self, marks: &[Mark]) {
        self.lines(self.thin, self.fill, marks);
    }

    /// `marks` as lines of `width` in `color`, each over a halo line a hairline wider on each side.
    fn lines(&self, width: f32, color: Color32, marks: &[Mark]) {
        for (width, color) in [(width + self.thin * 2.0, self.halo), (width, color)] {
            let stroke = Stroke::new(width, color);
            for mark in marks {
                match mark {
                    Mark::Line(a, b) => {
                        self.painter.line_segment([self.at + *a, self.at + *b], stroke);
                    }
                    Mark::Circle(center, radius) => {
                        self.painter.circle_stroke(self.at + *center, *radius, stroke);
                    }
                    Mark::Rect(rect) => {
                        self.painter.rect_stroke(rect.translate(self.at.to_vec2()), 0.0, stroke, egui::StrokeKind::Middle);
                    }
                }
            }
        }
    }

    fn disc(&self, center: Vec2, radius: f32, color: Color32) {
        self.painter.circle_filled(self.at + center, radius, color);
    }

    /// The segment `a` to `b` grown by `radius`.
    fn capsule(&self, a: Vec2, b: Vec2, radius: f32, color: Color32) {
        self.painter.line_segment([self.at + a, self.at + b], Stroke::new(2.0 * radius, color));
        self.disc(a, radius, color);
        self.disc(b, radius, color);
    }

    /// A polygon filled with the mark colour and outlined in the halo.
    fn polygon(&self, points: Vec<Pos2>) {
        self.painter.add(Shape::convex_polygon(points, self.fill, Stroke::new(self.thin, self.halo)));
    }
}

/// A circle of `radius` points around the pointer.
pub(super) fn circle(pen: &Pen<'_>, radius: f32, width: f32, color: Color32) {
    pen.lines(width, color, &[Mark::Circle(Vec2::ZERO, radius)]);
}

/// A crosshair with arms of `arm` points.
pub(super) fn crosshair(pen: &Pen<'_>, arm: f32, width: f32, color: Color32) {
    pen.lines(width, color, &[Mark::Line(v([-arm, 0.0]), v([arm, 0.0])), Mark::Line(v([0.0, -arm]), v([0.0, arm]))]);
}

/// Rings of a brush tip, as offsets from the pointer.
pub(super) fn outline(pen: &Pen<'_>, rings: &[Vec<Pos2>]) {
    for ring in rings.iter().filter(|ring| ring.len() >= 2) {
        let points: Vec<Pos2> = ring.iter().map(|p| pen.at + p.to_vec2()).collect();
        pen.painter.add(Shape::closed_line(points.clone(), Stroke::new(pen.thin * 3.0, pen.halo)));
        pen.painter.add(Shape::closed_line(points, Stroke::new(pen.thin, pen.fill)));
    }
}

/// The hand: open, or the fist while `closed`. Its silhouette is in the mark colour over the same grown by a hairline in
/// the halo colour, with the creases between the fingers.
pub(super) fn hand(pen: &Pen<'_>, closed: bool) {
    let shift = |p: [f32; 2]| v(p) + v(HAND_SHIFT);
    for (grow, color) in [(pen.thin, pen.halo), (0.0, pen.fill)] {
        let (center, half, round) = if closed { (FIST_CENTER, FIST_HALF, FIST_ROUND) } else { (PALM_CENTER, PALM_HALF, PALM_ROUND) };
        let palm = Rect::from_center_size(pen.at + shift(center), v(half) * 2.0).expand(grow);
        pen.painter.rect_filled(palm, round + grow, color);
        if closed {
            for x in FINGER_X {
                pen.capsule(shift([x, KNUCKLE_FROM]), shift([x, KNUCKLE_TO]), KNUCKLE_R + grow, color);
            }
        } else {
            for (x, tip) in FINGER_X.into_iter().zip(FINGER_TIP) {
                pen.capsule(shift([x, FINGER_BASE]), shift([x, tip]), FINGER_R + grow, color);
            }
        }
        let (a, b, r) = if closed { FIST_THUMB } else { THUMB };
        pen.capsule(shift(a), shift(b), r + grow, color);
    }
    let stroke = Stroke::new(pen.thin, pen.halo);
    let crease = |a: [f32; 2], b: [f32; 2]| {
        pen.painter.line_segment([pen.at + shift(a), pen.at + shift(b)], stroke);
    };
    if closed {
        crease(FIST_CURL.0, FIST_CURL.1);
        for x in CREASE_X {
            crease([x, FIST_CREASE_FROM], [x, FIST_CURL.0[1]]);
        }
    } else {
        for (x, (a, b)) in CREASE_X.into_iter().zip(FINGER_TIP.into_iter().zip(FINGER_TIP.into_iter().skip(1))) {
            // Each crease starts just below the lower of the two fingertips it separates.
            crease([x, a.max(b) + 1.0], [x, CREASE_END]);
        }
    }
}

/// The magnifier with a plus, or a minus when `out`.
pub(super) fn zoom(pen: &Pen<'_>, out: bool) {
    let (a, b, r) = HANDLE;
    for (grow, color) in [(pen.thin, pen.halo), (0.0, pen.fill)] {
        pen.disc(Vec2::ZERO, LENS + grow, color);
        pen.capsule(v(a), v(b), r + grow, color);
    }
    let stroke = Stroke::new(pen.thin, pen.halo);
    pen.painter.line_segment([pen.at + v([-GLYPH_ARM, 0.0]), pen.at + v([GLYPH_ARM, 0.0])], stroke);
    if !out {
        pen.painter.line_segment([pen.at + v([0.0, -GLYPH_ARM]), pen.at + v([0.0, GLYPH_ARM])], stroke);
    }
}

/// A tool's `icon` of `side` points with `hotspot` (a point of the icon's 24-unit box) on the pointer, in the mark
/// colour over a halo of the icon drawn a hairline off in every direction, and its badge at the icon's lower right.
pub(super) fn tool(pen: &Pen<'_>, draw: &Draw<'_>, icon: &IconId, hotspot: Vec2, badge: Option<CursorBadge>, side: f32) {
    let rect = Rect::from_min_size(pen.at - hotspot * (side / ICON_UNITS), Vec2::splat(side));
    let spec = |color: Color32| IconSpec::svg(icon.clone()).color(StyleColor(color));
    for dx in [-1.0, 0.0, 1.0] {
        for dy in [-1.0, 0.0, 1.0] {
            if dx != 0.0 || dy != 0.0 {
                draw.icon(rect.translate(Vec2::new(dx, dy) * pen.thin), &spec(pen.halo));
            }
        }
    }
    draw.icon(rect, &spec(pen.fill));
    if let Some(badge) = badge {
        self::badge(pen, rect.right_bottom() - pen.at, badge);
    }
}

/// A badge centred `center` from the pointer.
pub(super) fn badge(pen: &Pen<'_>, center: Vec2, badge: CursorBadge) {
    let h = BADGE_HALF;
    let at = |x: f32, y: f32| center + Vec2::new(x, y);
    let marks = match badge {
        CursorBadge::Add => vec![Mark::Line(at(-h, 0.0), at(h, 0.0)), Mark::Line(at(0.0, -h), at(0.0, h))],
        CursorBadge::Subtract => vec![Mark::Line(at(-h, 0.0), at(h, 0.0))],
        CursorBadge::Intersect => {
            let d = 0.8 * h;
            vec![Mark::Line(at(-d, -d), at(d, d)), Mark::Line(at(-d, d), at(d, -d))]
        }
        CursorBadge::Copy => {
            // A sheet behind a sheet.
            let sheet = Vec2::splat(h * 1.4);
            let near = h / 3.0;
            vec![
                Mark::Rect(Rect::from_center_size(Pos2::ZERO + at(-near, -near), sheet)),
                Mark::Rect(Rect::from_center_size(Pos2::ZERO + at(near, near), sheet)),
            ]
        }
        CursorBadge::NotAllowed => {
            let d = h * std::f32::consts::FRAC_1_SQRT_2;
            vec![Mark::Circle(center, h), Mark::Line(at(-d, d), at(d, -d))]
        }
        CursorBadge::Scissors => {
            let (spread, handle) = (h * SCISSORS_SPREAD, h * SCISSORS_HANDLE);
            vec![
                Mark::Line(at(h, -spread), at(-h, spread)),
                Mark::Line(at(h, spread), at(-h, -spread)),
                Mark::Circle(at(-h, spread), handle),
                Mark::Circle(at(-h, -spread), handle),
            ]
        }
        CursorBadge::Close => vec![Mark::Circle(center, h * CLOSE_RADIUS)],
        _ => Vec::new(),
    };
    pen.marks(&marks);
}

/// A badge beside a system pointer arrow.
pub(super) fn badge_beside(pen: &Pen<'_>, kind: CursorBadge) {
    badge(pen, v(BADGE_BESIDE), kind);
}

/// The rotate pointer: an arc with an arrowhead at each end, bent round the corner in direction `toward`.
pub(super) fn turn(pen: &Pen<'_>, toward: Vec2) {
    let len = toward.length();
    if !len.is_finite() || len < f32::EPSILON {
        return;
    }
    let u = toward / len;
    let centre = pen.at + u * TURN_RADIUS;
    let mid = (-u.y).atan2(-u.x);
    let on_arc = |a: f32| centre + Vec2::new(a.cos(), a.sin()) * TURN_RADIUS;
    let points: Vec<Pos2> = (0..=ARC_SEGMENTS).map(|i| on_arc(mid - HALF_SWEEP + 2.0 * HALF_SWEEP * i as f32 / ARC_SEGMENTS as f32)).collect();
    pen.painter.add(Shape::line(points.clone(), Stroke::new(pen.thin * 3.0, pen.halo)));
    pen.painter.add(Shape::line(points, Stroke::new(pen.thin, pen.fill)));
    let head = TURN_RADIUS * HEAD_PER_RADIUS;
    for (a, sign) in [(mid - HALF_SWEEP, -1.0f32), (mid + HALF_SWEEP, 1.0)] {
        let tangent = Vec2::new(-a.sin(), a.cos()) * sign;
        let normal = Vec2::new(a.cos(), a.sin());
        let tip = on_arc(a) + tangent * HEAD_REACH * head;
        pen.polygon(vec![
            tip + tangent * HEAD_LENGTH * head,
            tip - tangent * HEAD_BACK * head + normal * HEAD_HALF_WIDTH * head,
            tip - tangent * HEAD_BACK * head - normal * HEAD_HALF_WIDTH * head,
        ]);
    }
}

/// The unit vector along `dir`, or `fallback` for a zero or non-finite one.
fn unit(dir: Vec2, fallback: Vec2) -> Vec2 {
    let len = dir.length();
    if len.is_finite() && len > f32::EPSILON { dir / len } else { fallback }
}

/// The free-distort pointer: a dart with its tip on the pointer, pointing along `dir` (up and left for zero).
pub(super) fn distort(pen: &Pen<'_>, dir: Vec2) {
    let u = unit(dir, v([-1.0, -1.0]).normalized());
    let perp = Vec2::new(-u.y, u.x);
    let base = pen.at - u * DART_LENGTH;
    let (left, right, notch) = (base + perp * DART_HALF_WIDTH, base - perp * DART_HALF_WIDTH, pen.at - u * (DART_LENGTH * DART_NOTCH));
    // Two triangles meeting along the tip and the notch: the dart is not convex.
    for triangle in [vec![pen.at, left, notch], vec![pen.at, notch, right]] {
        pen.painter.add(Shape::convex_polygon(triangle, pen.fill, Stroke::NONE));
    }
    pen.painter.add(Shape::closed_line(vec![pen.at, left, notch, right], Stroke::new(pen.thin * 2.0, pen.halo)));
}

/// The skew pointer: two opposed arrows beside each other along `dir` (along x for zero).
pub(super) fn skew(pen: &Pen<'_>, dir: Vec2) {
    let u = unit(dir, Vec2::X);
    let perp = Vec2::new(-u.y, u.x);
    for (offset, sign) in [(SKEW_GAP, 1.0f32), (-SKEW_GAP, -1.0)] {
        let centre = pen.at + perp * offset;
        let end = centre + u * (sign * SKEW_HALF_LENGTH);
        let start = centre - u * (sign * SKEW_HALF_LENGTH);
        pen.painter.line_segment([start, end], Stroke::new(pen.thin * 3.0, pen.halo));
        pen.painter.line_segment([start, end], Stroke::new(pen.thin, pen.fill));
        pen.polygon(vec![
            end + u * (sign * SKEW_HEAD),
            end - u * (sign * SKEW_BACK) + perp * SKEW_HEAD,
            end - u * (sign * SKEW_BACK) - perp * SKEW_HEAD,
        ]);
    }
}
