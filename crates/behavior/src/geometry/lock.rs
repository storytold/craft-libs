//! Direction locking for ⇧ drags: [`Directions`], [`lock_direction`], [`round_to_direction`], [`turn_to_45`],
//! [`polygon_step`] and [`axis_or_diagonal`].

use kurbo::{Point, Vec2};

/// Extra angle in degrees the pointer must pass the half-way line between two directions by
/// before a locked direction switches, so it doesn't flicker near the diagonal.
pub const LOCK_HYSTERESIS_DEGREES: f64 = 8.0;

/// How near an axis, in degrees, a ⇧ line or anchor drag must point to snap onto the axis rather than the diagonal
/// ([`axis_or_diagonal`]).
pub const AXIS_BAND_DEGREES: f64 = 30.0;

/// The directions ⇧ locks a move to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Directions {
    /// Horizontal and vertical: the Move tool dragging layers (Photoshop).
    #[default]
    Four,
    /// The axes and the 45° diagonals: selected pixels, a selection outline, a transform box's
    /// body.
    Eight,
}

impl Directions {
    /// How many evenly spaced directions: 4 or 8.
    pub const fn count(self) -> u32 {
        match self {
            Directions::Four => 4,
            Directions::Eight => 8,
        }
    }
}

/// `angle` (radians, 0 = right, document y down) rounded to the nearest of `directions` evenly
/// spaced directions: the one angle-rounding rule behind [`lock_direction`] (projection) and
/// [`turn_to_45`] (rotation).
pub fn round_to_direction(angle: f64, directions: Directions) -> f64 {
    let step = std::f64::consts::TAU / f64::from(directions.count());
    (angle / step).round() * step
}

/// Shift's polygon vertex: `point` moved to the nearest of eight directions from `from`; an axis keeps the
/// segment's length, a diagonal keeps the shorter offset on both axes. `point` itself for a zero-length or
/// non-finite offset.
pub fn polygon_step(from: Point, point: Point) -> Point {
    let delta = point - from;
    let length = delta.hypot();
    if !length.is_finite() || length == 0.0 {
        return point;
    }
    let direction = Vec2::from_angle(round_to_direction(delta.atan2(), Directions::Eight));
    let (x, y) = (direction.x.round(), direction.y.round());
    if x == 0.0 || y == 0.0 {
        return from + Vec2::new(x, y) * length;
    }
    // Inside a diagonal's sector both offsets have the diagonal's signs.
    let shorter = delta.x.abs().min(delta.y.abs());
    from + Vec2::new(shorter.copysign(delta.x), shorter.copysign(delta.y))
}

/// Shift's rule for the Line tool and anchor drags: within [`AXIS_BAND_DEGREES`] of an axis, `point` moves onto the
/// axis through `from` and keeps its offset along it; otherwise onto the 45° diagonal, at the shorter of its two
/// offsets on both axes. `point` itself for a zero or non-finite offset.
pub fn axis_or_diagonal(from: Point, point: Point) -> Point {
    // Photoshop flattens a 26.6° drag and keeps a 39.8° one diagonal (#13243); anchors stay on the axis at a 5:12
    // slope (22.6°) and go diagonal at 45° (#13207).
    let delta = point - from;
    if !delta.x.is_finite() || !delta.y.is_finite() || delta == Vec2::ZERO {
        return point;
    }
    let tan = AXIS_BAND_DEGREES.to_radians().tan();
    let (ax, ay) = (delta.x.abs(), delta.y.abs());
    if ay < ax * tan {
        return Point::new(point.x, from.y);
    }
    if ax < ay * tan {
        return Point::new(from.x, point.y);
    }
    let shorter = ax.min(ay);
    from + Vec2::new(shorter.copysign(delta.x), shorter.copysign(delta.y))
}

/// `point` turned about `from` to the nearest multiple of 45°, keeping its distance: the ⇧ rule for a line, a handle
/// or a segment drawn from a fixed point. A polygon's vertex takes [`polygon_step`] instead. `point` itself when it
/// is on `from` or not finite.
pub fn turn_to_45(from: Point, point: Point) -> Point {
    let delta = point - from;
    let length = delta.hypot();
    if !length.is_finite() || length == 0.0 {
        return point;
    }
    let angle = round_to_direction(delta.atan2(), Directions::Eight);
    from + Vec2::from_angle(angle) * length
}

/// `d` locked to the nearest of `directions` evenly spaced directions (0° = right, document y
/// down). `previous` is the offset this returned last in the same drag: its direction is kept
/// until `d` is more than half a step plus [`LOCK_HYSTERESIS_DEGREES`] away from it. The length
/// is the projection of `d` on the chosen direction, never negative; components under 1e-9
/// become exactly 0, so an axis-locked move stays on the axis. A zero or non-finite `d` comes
/// back unchanged.
pub fn lock_direction(d: Vec2, previous: Option<Vec2>, directions: Directions) -> Vec2 {
    let len = d.hypot();
    if !len.is_finite() || len == 0.0 {
        return d;
    }
    let step = std::f64::consts::TAU / f64::from(directions.count());
    let a = d.atan2();
    let mut locked = round_to_direction(a, directions);
    if let Some(p) = previous.filter(|p| p.hypot() > 0.0) {
        let kept = round_to_direction(p.atan2(), directions);
        let off = (a - kept).rem_euclid(std::f64::consts::TAU);
        let off = off.min(std::f64::consts::TAU - off);
        if off < step / 2.0 + LOCK_HYSTERESIS_DEGREES.to_radians() {
            locked = kept;
        }
    }
    let (s, c) = locked.sin_cos();
    let t = (d.x * c + d.y * s).max(0.0);
    let clean = |v: f64| if v.abs() < 1e-9 { 0.0 } else { v };
    Vec2::new(clean(t * c), clean(t * s))
}
