//! Direction locking for ⇧ drags: [`Directions`], [`lock_direction`] and [`round_to_direction`].

use kurbo::Vec2;

/// Extra angle in degrees the pointer must pass the half-way line between two directions by
/// before a locked direction switches, so it doesn't flicker near the diagonal.
pub const LOCK_HYSTERESIS_DEGREES: f64 = 8.0;

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
/// [`turn_to_45`](super::draft::turn_to_45) (rotation).
pub fn round_to_direction(angle: f64, directions: Directions) -> f64 {
    let step = std::f64::consts::TAU / f64::from(directions.count());
    (angle / step).round() * step
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
