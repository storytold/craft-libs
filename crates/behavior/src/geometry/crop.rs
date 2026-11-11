//! The crop frame's pure geometry: the box, where a press lands on it, what each gesture makes of it, the
//! straighten and image-mode maths, and the composition guides. Nothing here reads a `Ui`.

use super::marquee::Constraint;
use super::quad::BoxPart;
use crate::canvas::CanvasView;
use craft_keymap::Mods;
use emath::vec2;
use kurbo::{Point, Rect, Size, Vec2};
use serde::{Deserialize, Serialize};

/// Smallest side (document units) a gesture may leave behind.
const MIN_FRAME: f64 = 1.0;

/// The crop frame: a document rect and the angle it is turned by about its centre.
///
/// The rect is the frame before its turn, so ratios, whole-unit readouts and the crop itself are defined on it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CropBox {
    /// The frame before its turn, in document units.
    pub rect: Rect,
    /// Degrees, clockwise on screen, -180..=180.
    pub angle: f64,
}

impl CropBox {
    /// The frame `rect` turned `angle` degrees about its centre.
    pub const fn new(rect: Rect, angle: f64) -> Self {
        Self { rect, angle }
    }

    /// An unturned frame.
    pub const fn upright(rect: Rect) -> Self {
        Self { rect, angle: 0.0 }
    }

    /// The part a press at `point` (document units) lands on, with `reach` document units of reach: a corner or an
    /// edge, [`BoxPart::Inside`] or [`BoxPart::Outside`] (the turn zone). Outside the frame the full reach applies,
    /// inside it at most a quarter of the side, so a small frame stays movable.
    pub fn part_at(&self, point: Point, reach: f64) -> BoxPart {
        match hit(self, point, reach) {
            Hit::Handle(hx, hy) => handle_part(hx, hy),
            Hit::Inside => BoxPart::Inside,
            Hit::Outside => BoxPart::Outside,
        }
    }

    /// Whether the rect and the angle are finite numbers.
    pub fn is_finite(&self) -> bool {
        let r = self.rect;
        [r.x0, r.y0, r.x1, r.y1, self.angle].iter().all(|v| v.is_finite())
    }

    /// The centre the frame turns about.
    pub fn center(&self) -> Point {
        self.rect.center()
    }

    /// The corners on the page, turned: top left, top right, bottom right, bottom left of the upright frame.
    pub fn corners(&self) -> [Point; 4] {
        let r = self.rect.abs();
        let c = r.center();
        [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)].map(|p| turn(p, c, self.angle))
    }

    /// The document point of `part` on the turned frame: a corner or an edge handle, or the centre for
    /// [`BoxPart::Inside`]; none for the other parts.
    pub fn part_point(&self, part: BoxPart) -> Option<Point> {
        let c = self.center();
        if part == BoxPart::Inside {
            return Some(c);
        }
        let (hx, hy) = part_handle(part)?;
        let r = self.rect.abs();
        Some(turn(Point::new(c.x + f64::from(hx) * r.width() / 2.0, c.y + f64::from(hy) * r.height() / 2.0), c, self.angle))
    }

    /// The direction `part` points out of the frame, as a document vector turned with the frame; none for the
    /// parts that are neither a corner nor an edge.
    pub fn part_direction(&self, part: BoxPart) -> Option<Vec2> {
        let (hx, hy) = part_handle(part)?;
        let d = turn(Point::new(f64::from(hx), f64::from(hy)), Point::ZERO, self.angle);
        Some(Vec2::new(d.x, d.y))
    }

    /// The diagonal, as a document vector, from the frame's corner nearest `point` towards the frame's centre
    /// reversed: it points out of the centre through the corner of the quadrant `point` lies in, along the frame's
    /// own axes. The turn pointer bends toward it.
    pub fn quadrant_diagonal(&self, point: Point) -> Vec2 {
        let (c, local) = (self.center(), turn(point, self.center(), -self.angle));
        let (sx, sy) = (if local.x < c.x { 1.0 } else { -1.0 }, if local.y < c.y { 1.0 } else { -1.0 });
        let d = std::f64::consts::FRAC_1_SQRT_2;
        let toward = turn(Point::new(sx * d, sy * d), Point::ZERO, self.angle);
        Vec2::new(toward.x, toward.y)
    }

    /// The axis-aligned bounds of the turned frame.
    pub fn turned_bounds(&self) -> Rect {
        let [first, rest @ ..] = self.corners();
        rest.iter().fold(Rect::from_points(first, first), |acc, &p| acc.union_pt(p))
    }

    /// The frame moved by `by` on the page, its angle kept.
    pub fn moved(&self, by: Vec2) -> Self {
        Self { rect: self.rect + by, ..*self }
    }

    /// Width and height swapped about the centre, the angle kept (X).
    pub fn swapped(&self) -> Self {
        let c = self.center();
        let (hw, hh) = (self.rect.width() / 2.0, self.rect.height() / 2.0);
        Self { rect: Rect::new(c.x - hh, c.y - hw, c.x + hh, c.y + hw), ..*self }
    }

    /// The frame after an arrow key moves `part` by `by`, as a plain drag of the part would, with `ratio` held.
    pub fn nudged(&self, part: BoxPart, by: Vec2, ratio: Option<f64>) -> Self {
        match part_handle(part) {
            Some((hx, hy)) => resized_turned(self, hx, hy, by, ratio, false),
            None => self.moved(by),
        }
    }

    /// The size Enter would crop to: the frame's own width and height in whole document units, at least 1; `None`
    /// when they are not finite.
    pub fn crop_size(&self) -> Option<(f64, f64)> {
        let (w, h) = (self.rect.width(), self.rect.height());
        (w.is_finite() && h.is_finite()).then(|| (w.round().max(1.0), h.round().max(1.0)))
    }

    /// Whether both sides are at least one document unit.
    pub fn size_ok(&self) -> bool {
        self.rect.width() >= MIN_FRAME && self.rect.height() >= MIN_FRAME
    }

    /// The frame with `part` dragged by the page vector `by`: along the frame's own axes, the opposite edges kept on
    /// the page (or the centre with `alt`), `ratio` (width over height) held. A part that is not a corner or an
    /// edge leaves the frame as it is.
    #[must_use]
    pub fn resized(&self, part: BoxPart, by: Vec2, ratio: Option<f64>, alt: bool) -> Self {
        match part_handle(part) {
            Some((hx, hy)) => resized_turned(self, hx, hy, by, ratio, alt),
            None => *self,
        }
    }

    /// [`resized`](Self::resized) for Auto Center Preview in image mode: `part` dragged by `by` grows or shrinks the
    /// frame about its centre, so the handle stays under the pointer while the frame stays centred on screen. The
    /// opposite edge stays put on the image (the centre with `alt`), so the frame's centre moves by the difference
    /// and the view follows it. `None` when the result is not finite or `part` is not a corner or an edge.
    pub fn resized_centred(&self, part: BoxPart, by: Vec2, ratio: Option<f64>, alt: bool) -> Option<Self> {
        let (hx, hy) = part_handle(part)?;
        let r = self.rect.abs();
        let (w0, h0) = (r.width(), r.height());
        let local = turn(Point::new(by.x, by.y), Point::ZERO, -self.angle);
        let n = resized(Rect::new(-w0 / 2.0, -h0 / 2.0, w0 / 2.0, h0 / 2.0), hx, hy, Vec2::new(local.x, local.y), ratio, true);
        let (w1, h1) = (n.width(), n.height());
        let shift = if alt { Point::ZERO } else { Point::new(f64::from(hx) * (w1 - w0) / 2.0, f64::from(hy) * (h1 - h0) / 2.0) };
        let s = turn(shift, Point::ZERO, self.angle);
        let c = self.center() + Vec2::new(s.x, s.y);
        let next = Self { rect: Rect::new(c.x - w1 / 2.0, c.y - h1 / 2.0, c.x + w1 / 2.0, c.y + h1 / 2.0), angle: self.angle };
        next.is_finite().then_some(next)
    }

    /// The frame turned from this one's angle by the pointer's sweep about the centre, from direction `grab`
    /// (radians, see [`direction`]) to `to`; `step` snaps the angle (⇧). With `image` the frame turns against the
    /// sweep, as the image under an upright frame turns with it.
    #[must_use]
    pub fn turned_to(&self, grab: f64, to: Point, step: Option<f64>, image: bool) -> Self {
        let sweep = (direction(self.center(), to) - grab).to_degrees();
        let a = if image { self.angle - sweep } else { self.angle + sweep };
        let angle = step.map_or(normalized_angle(a), |step| snap_angle(a, step));
        Self { angle, ..*self }
    }

    /// The frame kept inside `bounds`: an unturned frame is cut to it; a turned one is moved by the least that
    /// brings its bounds in, and left as it is when it cannot fit. `None` when a frame that changes size
    /// (`resizing`) cannot be kept inside (the caller keeps its last frame). With `held` (a ratio is held), a cut
    /// that would change the frame's ratio is refused the same way.
    pub fn within(self, bounds: Rect, resizing: bool, held: bool) -> Option<Self> {
        let bounds = bounds.abs();
        if self.angle == 0.0 {
            let r = self.rect.abs();
            let cut = Rect::new(r.x0.max(bounds.x0), r.y0.max(bounds.y0), r.x1.min(bounds.x1), r.y1.min(bounds.y1));
            if resizing {
                let kept = !held || (cut.width() * r.height() - cut.height() * r.width()).abs() <= 1e-9 * (r.width() * r.height()).max(1.0);
                return (kept && cut.width() >= 0.0 && cut.height() >= 0.0).then_some(Self::upright(cut));
            }
            return Some(Self::upright(shifted_in(r, bounds)));
        }
        let t = self.turned_bounds();
        if resizing {
            let inside = t.x0 >= bounds.x0 && t.y0 >= bounds.y0 && t.x1 <= bounds.x1 && t.y1 <= bounds.y1;
            return inside.then_some(self);
        }
        let by = shifted_in(t, bounds).origin() - t.origin();
        Some(self.moved(by))
    }

    /// The frame angle (degrees, clockwise on the page, in -45..=45) that makes the line from `a` to `b` level, or
    /// plumb when it is nearer vertical; `None` for a point or a line that is not finite.
    pub fn level_angle(a: Point, b: Point) -> Option<f64> {
        let d = b - a;
        if !(d.x.is_finite() && d.y.is_finite()) || d.hypot() == 0.0 {
            return None;
        }
        // The line has no direction: fold it into -90..=90, then into the nearer axis.
        let mut deg = d.y.atan2(d.x).to_degrees();
        if deg > 90.0 {
            deg -= 180.0;
        } else if deg <= -90.0 {
            deg += 180.0;
        }
        if deg > 45.0 {
            deg -= 90.0;
        } else if deg <= -45.0 {
            deg += 90.0;
        }
        Some(deg + 0.0)
    }

    /// The largest frame with this one's proportions that fits a canvas of `canvas` at `angle` degrees, centred on
    /// the canvas. `None` when a size or the angle is not a positive finite number.
    pub fn fitted(&self, canvas: Size, angle: f64) -> Option<CropBox> {
        let r = self.rect.abs();
        let (w, h, cw, ch) = (r.width(), r.height(), canvas.width, canvas.height);
        if ![w, h, cw, ch, angle].iter().all(|v| v.is_finite()) || w <= 0.0 || h <= 0.0 || cw <= 0.0 || ch <= 0.0 {
            return None;
        }
        let (s, c) = angle.to_radians().sin_cos();
        let (s, c) = (s.abs(), c.abs());
        // The turned frame's bounds, scaled by `k`, must fit the canvas on both axes.
        let k = (cw / (w * c + h * s)).min(ch / (w * s + h * c));
        if !k.is_finite() || k <= 0.0 {
            return None;
        }
        let (hw, hh, cx, cy) = (w * k / 2.0, h * k / 2.0, cw / 2.0, ch / 2.0);
        Some(CropBox { rect: Rect::new(cx - hw, cy - hh, cx + hw, cy + hh), angle: normalized_angle(angle) })
    }
}

/// `p` turned `deg` degrees clockwise on screen (y down) about `c`.
pub fn turn(p: Point, c: Point, deg: f64) -> Point {
    if deg == 0.0 {
        return p;
    }
    let (s, k) = deg.to_radians().sin_cos();
    let (dx, dy) = (p.x - c.x, p.y - c.y);
    Point::new(c.x + k * dx - s * dy, c.y + s * dx + k * dy)
}

/// An angle in degrees brought into (-180, 180]; 0 for anything not finite. An angle already in range comes back
/// exactly.
pub fn normalized_angle(deg: f64) -> f64 {
    if !deg.is_finite() {
        return 0.0;
    }
    let a = deg % 360.0;
    if a > 180.0 {
        a - 360.0
    } else if a <= -180.0 {
        a + 360.0
    } else {
        a
    }
}

/// `deg` on the nearest multiple of `step` degrees, brought into (-180, 180]; a step that is not finite and positive
/// is ignored.
pub fn snap_angle(deg: f64, step: f64) -> f64 {
    if !(step.is_finite() && step > 0.0) {
        return normalized_angle(deg);
    }
    normalized_angle((deg / step).round() * step)
}

/// The direction of `p` from `c`, in radians.
pub fn direction(c: Point, p: Point) -> f64 {
    (p.y - c.y).atan2(p.x - c.x)
}

/// What a press lands on, in the frame's own axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    /// An edge or corner: `hx`, `hy` are -1 (left, top), 1 (right, bottom) or 0.
    Handle(i8, i8),
    Inside,
    Outside,
}

/// The part for handle (`hx`, `hy`): corners clockwise from the top left, edges clockwise from the top.
fn handle_part(hx: i8, hy: i8) -> BoxPart {
    match (hx.signum(), hy.signum()) {
        (-1, -1) => BoxPart::Corner(0),
        (1, -1) => BoxPart::Corner(1),
        (1, 1) => BoxPart::Corner(2),
        (-1, 1) => BoxPart::Corner(3),
        (0, -1) => BoxPart::Edge(0),
        (1, 0) => BoxPart::Edge(1),
        (0, 1) => BoxPart::Edge(2),
        (-1, 0) => BoxPart::Edge(3),
        _ => BoxPart::Inside,
    }
}

/// The handle (`hx`, `hy`) of a corner or edge part.
fn part_handle(part: BoxPart) -> Option<(i8, i8)> {
    match part {
        BoxPart::Corner(i) => Some([(-1, -1), (1, -1), (1, 1), (-1, 1)][usize::from(i % 4)]),
        BoxPart::Edge(i) => Some([(0, -1), (1, 0), (0, 1), (-1, 0)][usize::from(i % 4)]),
        _ => None,
    }
}

fn hit(b: &CropBox, p: Point, tol: f64) -> Hit {
    let r = b.rect.abs();
    let p = turn(p, r.center(), -b.angle);
    let axis = |lo: f64, hi: f64, v: f64, other_in: bool| -> i8 {
        let t = tol.min((hi - lo) / 4.0).max(0.0);
        let near = |e: f64, outward: f64| (v - e) * outward <= tol && (e - v) * outward <= t;
        match (other_in, near(lo, -1.0), near(hi, 1.0)) {
            (false, ..) => 0,
            (_, true, true) => {
                if (v - lo).abs() <= (v - hi).abs() {
                    -1
                } else {
                    1
                }
            }
            (_, true, false) => -1,
            (_, false, true) => 1,
            _ => 0,
        }
    };
    let in_x = p.x >= r.x0 - tol && p.x <= r.x1 + tol;
    let in_y = p.y >= r.y0 - tol && p.y <= r.y1 + tol;
    let (hx, hy) = (axis(r.x0, r.x1, p.x, in_y), axis(r.y0, r.y1, p.y, in_x));
    if hx != 0 || hy != 0 {
        Hit::Handle(hx, hy)
    } else if p.x > r.x0 && p.x < r.x1 && p.y > r.y0 && p.y < r.y1 {
        Hit::Inside
    } else {
        Hit::Outside
    }
}

/// [`CropBox::part_at`] for a plain upright `rect`, with no frame or view.
pub fn hit_rect(rect: Rect, point: Point, reach: f64) -> BoxPart {
    CropBox::upright(rect).part_at(point, reach)
}

/// Frame `r` with handle (`hx`, `hy`) dragged by `d`. `alt` resizes about the centre; `ratio` (width / height) keeps
/// the proportions, the larger relative extent winning at a corner.
fn resized(r: Rect, hx: i8, hy: i8, d: Vec2, ratio: Option<f64>, alt: bool) -> Rect {
    let r = r.abs();
    // Per axis: the fixed point, the signed extent from it, and whether it is centred (the extent is then a
    // half-extent).
    let axis = |lo: f64, hi: f64, h: i8, d: f64| -> (f64, f64, bool) {
        let c = (lo + hi) / 2.0;
        match h {
            1 if alt => (c, hi + d - c, true),
            1 => (lo, hi + d - lo, false),
            -1 if alt => (c, lo + d - c, true),
            -1 => (hi, lo + d - hi, false),
            _ => (c, (hi - lo) / 2.0, true),
        }
    };
    let (fx, mut ex, cx) = axis(r.x0, r.x1, hx, d.x);
    let (fy, mut ey, cy) = axis(r.y0, r.y1, hy, d.y);
    if let Some(k) = ratio.filter(|k| k.is_finite() && *k > 0.0) {
        let full = |e: f64, c: bool| e.abs() * if c { 2.0 } else { 1.0 };
        let (w, h) = (full(ex, cx), full(ey, cy));
        let (w, h) = match (hx != 0, hy != 0) {
            (true, true) => {
                let s = (w / k).max(h);
                (s * k, s)
            }
            (true, false) => (w, w / k),
            (false, true) => (h * k, h),
            (false, false) => (w, h),
        };
        let back = |e: f64, len: f64, c: bool| if e < 0.0 { -1.0 } else { 1.0 } * if c { len / 2.0 } else { len };
        ex = back(ex, w, cx);
        ey = back(ey, h, cy);
    }
    let span = |f: f64, e: f64, c: bool| if c { (f - e, f + e) } else { (f, f + e) };
    let ((ax, bx), (ay, by)) = (span(fx, ex, cx), span(fy, ey, cy));
    Rect::new(ax, ay, bx, by).abs()
}

/// [`resized`] for a frame turned by its angle: the handle moves along the frame's own axes by the page drag `d`,
/// and the edges opposite stay where they are on the page.
fn resized_turned(b: &CropBox, hx: i8, hy: i8, d: Vec2, ratio: Option<f64>, alt: bool) -> CropBox {
    if b.angle == 0.0 {
        return CropBox::upright(resized(b.rect, hx, hy, d, ratio, alt));
    }
    let c = b.center();
    let local = turn(Point::new(d.x, d.y), Point::ZERO, -b.angle);
    let n = resized(b.rect, hx, hy, Vec2::new(local.x, local.y), ratio, alt);
    let nc = turn(n.center(), c, b.angle);
    let (hw, hh) = (n.width() / 2.0, n.height() / 2.0);
    CropBox { rect: Rect::new(nc.x - hw, nc.y - hh, nc.x + hw, nc.y + hh), angle: b.angle }
}

/// A new frame drawn from `anchor` to `cur`: ⇧ square, ⌥ from the centre, `ratio` held.
pub fn drawn(anchor: Point, cur: Point, ratio: Option<f64>, mods: Mods) -> Rect {
    let constraint = match ratio.filter(|k| k.is_finite() && *k > 0.0) {
        Some(k) => Constraint::Ratio(k, 1.0),
        None => Constraint::Free,
    };
    let (a, b) = constraint.corners(anchor, cur, mods.shift, mods.alt);
    Rect::from_points(a, b).abs()
}

/// `r` moved by the least that brings it inside `bounds` (an edge of `bounds` first when `r` is larger).
fn shifted_in(r: Rect, bounds: Rect) -> Rect {
    let fit = |lo: f64, hi: f64, b_lo: f64, b_hi: f64| {
        if hi - lo >= b_hi - b_lo || lo < b_lo {
            b_lo - lo
        } else if hi > b_hi {
            b_hi - hi
        } else {
            0.0
        }
    };
    r + Vec2::new(fit(r.x0, r.x1, bounds.x0, bounds.x1), fit(r.y0, r.y1, bounds.y0, bounds.y1))
}

/// The turn the image-mode readout shows: clockwise positive, the frame's angle undone, never -0.
pub fn image_turn(frame_angle: f64) -> f64 {
    let a = normalized_angle(-frame_angle);
    if a == 0.0 { 0.0 } else { a }
}

/// `view` turned so the frame `b` shows upright (rotation `-b.angle`), the frame's centre kept where it is on screen.
pub fn crop_view(b: &CropBox, view: &CanvasView) -> CanvasView {
    crop_view_about(b, view, b.center())
}

/// [`crop_view`] keeping document point `pivot` where it is on screen. `view` itself when anything is not finite.
pub fn crop_view_about(b: &CropBox, view: &CanvasView, pivot: Point) -> CanvasView {
    let rotation = normalized_angle(-b.angle);
    let at = view.affine() * pivot;
    let turned = CanvasView { rotation, center: pivot, ..*view };
    let back = turned.affine().inverse() * at;
    let center = Point::new(2.0 * pivot.x - back.x, 2.0 * pivot.y - back.y);
    if b.is_finite() && center.x.is_finite() && center.y.is_finite() { CanvasView { center, ..turned } } else { *view }
}

/// An arrow step `by` (screen points) as the document vector it is in `view`, at one document unit per point.
pub fn screen_step(view: &CanvasView, by: Vec2) -> Vec2 {
    CanvasView { zoom: 1.0, aspect: 1.0, ..*view }.unmap_vec(vec2(by.x as f32, by.y as f32))
}

/// Golden section: 1/phi.
const INV_PHI: f64 = 0.618_033_988_749_894_9;
/// Golden ratio.
const PHI: f64 = 1.0 + INV_PHI;

/// The composition guide drawn inside a crop frame, in Photoshop's menu order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum CropGuide {
    /// Two lines each way at the thirds.
    #[default]
    Thirds,
    /// Square cells of about the grid step.
    Grid,
    /// 45 degrees on screen from each corner.
    Diagonal,
    /// A diagonal and the perpendiculars to it from the other corners.
    Triangle,
    /// Two lines each way at the golden section.
    GoldenRatio,
    /// A spiral through nested golden rectangles.
    GoldenSpiral,
}

impl CropGuide {
    /// Every guide, in the order `next` cycles through.
    pub const ALL: [CropGuide; 6] = [Self::Thirds, Self::Grid, Self::Diagonal, Self::Triangle, Self::GoldenRatio, Self::GoldenSpiral];

    /// How many distinct orientations `next_orientation` steps through: the Triangle's diagonal runs either way, the
    /// Golden Spiral winds into any of the four corners; the others are symmetric.
    pub fn orientations(self) -> u8 {
        match self {
            Self::Triangle => 2,
            Self::GoldenSpiral => 4,
            _ => 1,
        }
    }

    /// The next guide (O); the orientation starts over at 0.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|k| *k == self).unwrap_or(0);
        Self::ALL.get((i + 1) % Self::ALL.len()).copied().unwrap_or_default()
    }

    /// The guide's next orientation (Shift+O) after `orientation`; a symmetric guide stays at 0 and a stray value
    /// comes back in range.
    pub fn next_orientation(self, orientation: u8) -> u8 {
        let n = self.orientations().max(1);
        (orientation % n + 1) % n
    }

    /// The guide's polylines for a frame `w` x `h` screen points, in its unit square (0..1 along its top edge, 0..1
    /// down its left edge), so one geometry maps onto an upright or a turned frame. The Grid guide has cells of
    /// about `grid_step` points and at most `grid_max` lines per axis. `orientation` is taken modulo
    /// [`orientations`](Self::orientations). Sizes under 1 point or not finite give no lines.
    pub fn guides(self, orientation: u8, w: f64, h: f64, grid_step: f64, grid_max: usize) -> Vec<Vec<Point>> {
        if !(w.is_finite() && h.is_finite()) || w < 1.0 || h < 1.0 {
            return Vec::new();
        }
        let seg = |a: [f64; 2], b: [f64; 2]| vec![a, b];
        let cross = |fs: &[f64]| -> Vec<Vec<[f64; 2]>> { fs.iter().flat_map(|&f| [seg([f, 0.0], [f, 1.0]), seg([0.0, f], [1.0, f])]).collect() };
        let lines = match self {
            Self::Thirds => cross(&[1.0 / 3.0, 2.0 / 3.0]),
            Self::GoldenRatio => cross(&[1.0 - INV_PHI, INV_PHI]),
            Self::Grid => {
                let axis = |len: f64, vertical: bool| -> Vec<Vec<[f64; 2]>> {
                    // Square cells of about `grid_step` on screen, at most `grid_max` lines per axis.
                    let n = ((len / grid_step.max(1.0)) as usize).clamp(2, grid_max.max(2));
                    (1..n)
                        .map(|i| {
                            let f = i as f64 / n as f64;
                            if vertical { seg([f, 0.0], [f, 1.0]) } else { seg([0.0, f], [1.0, f]) }
                        })
                        .collect()
                };
                let mut v = axis(w, true);
                v.extend(axis(h, false));
                v
            }
            Self::Diagonal => {
                // 45 degrees on screen from each corner until the line meets the far edge.
                let m = w.min(h);
                let (u, v) = (m / w, m / h);
                vec![seg([0.0, 0.0], [u, v]), seg([1.0, 0.0], [1.0 - u, v]), seg([0.0, 1.0], [u, 1.0 - v]), seg([1.0, 1.0], [1.0 - u, 1.0 - v])]
            }
            Self::Triangle => {
                // The top-left to bottom-right diagonal, and the perpendiculars to it (on screen) from the other
                // two corners; their feet sit at `t` and `s` along it. t = w^2 / (w^2 + h^2) and s = 1 - t, from
                // the smaller ratio so huge sizes can't overflow.
                let (t, s) = if w >= h {
                    let t = 1.0 / (1.0 + (h / w).powi(2));
                    (t, 1.0 - t)
                } else {
                    let s = 1.0 / (1.0 + (w / h).powi(2));
                    (1.0 - s, s)
                };
                vec![seg([0.0, 0.0], [1.0, 1.0]), seg([1.0, 0.0], [t, t]), seg([0.0, 1.0], [s, s])]
            }
            Self::GoldenSpiral => vec![golden_spiral()],
        };
        let o = orientation % self.orientations().max(1);
        let (fx, fy) = (o & 1 != 0, o & 2 != 0);
        lines
            .into_iter()
            .map(|l| l.into_iter().map(|[x, y]| Point::new(if fx { 1.0 - x } else { x }, if fy { 1.0 - y } else { y })).collect())
            .collect()
    }
}

/// The golden spiral: quarter arcs through nested golden rectangles, squares cut off the left, top, right and bottom
/// in turn, winding in towards the lower right. Stretched to the unit square.
fn golden_spiral() -> Vec<[f64; 2]> {
    const STEPS: usize = 12;
    // The remaining rectangle [x0, y0, x1, y1] in a phi x 1 space, and the spiral's current point.
    let mut r = [0.0f64, 0.0, PHI, 1.0];
    let mut p = [0.0f64, 1.0];
    let mut pts = vec![p];
    for i in 0..12 {
        let (w, h) = (r[2] - r[0], r[3] - r[1]);
        let s = w.min(h);
        if s < 1e-4 {
            break;
        }
        // The square cut off, the arc's centre (a corner of it) and the end point; the rest stays.
        let (centre, end) = match i % 4 {
            0 => {
                r[0] += s;
                ([r[0], r[3]], [r[0], r[1]])
            }
            1 => {
                r[1] += s;
                ([r[0], r[1]], [r[2], r[1]])
            }
            2 => {
                r[2] -= s;
                ([r[2], r[1]], [r[2], r[3]])
            }
            _ => {
                r[3] -= s;
                ([r[2], r[3]], [r[0], r[3]])
            }
        };
        let a0 = (p[1] - centre[1]).atan2(p[0] - centre[0]);
        let a1 = (end[1] - centre[1]).atan2(end[0] - centre[0]);
        let mut da = a1 - a0;
        if da > std::f64::consts::PI {
            da -= std::f64::consts::TAU;
        } else if da < -std::f64::consts::PI {
            da += std::f64::consts::TAU;
        }
        for k in 1..=STEPS {
            let a = a0 + da * k as f64 / STEPS as f64;
            pts.push([centre[0] + s * a.cos(), centre[1] + s * a.sin()]);
        }
        p = end;
    }
    pts.into_iter().map(|[x, y]| [(x / PHI).clamp(0.0, 1.0), y.clamp(0.0, 1.0)]).collect()
}

/// When the guide shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum GuideShow {
    /// Only while the frame is being dragged (drawn, moved, resized or turned).
    #[default]
    Auto,
    /// Always shown.
    Always,
    /// Never shown.
    Never,
}

impl GuideShow {
    /// Every rule, in menu order.
    pub const ALL: [GuideShow; 3] = [Self::Auto, Self::Always, Self::Never];

    /// Whether the guide shows, `gesture` being a crop drag in progress.
    pub fn shows(self, gesture: bool) -> bool {
        match self {
            Self::Auto => gesture,
            Self::Always => true,
            Self::Never => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angles_wrap_into_the_half_open_turn() {
        assert_eq!(normalized_angle(-180.0), 180.0, "-180 is 180");
        assert_eq!(normalized_angle(-0.1), -0.1, "an angle in range comes back exactly");
        assert_eq!(normalized_angle(f64::NAN), 0.0);
        assert_eq!(snap_angle(-170.0, 45.0), 180.0, "a snap past -180 wraps");
        assert_eq!(snap_angle(37.5, 0.0), 37.5, "a step of 0 is ignored");
    }
}
