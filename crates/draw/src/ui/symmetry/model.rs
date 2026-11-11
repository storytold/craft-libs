//! [`Symmetry`]: a painting symmetry's straight axes and their geometry. No egui.

use craft_behavior::kurbo::{Line, Point, Size, Vec2};

/// The largest centre coordinate a symmetry keeps, in document units (PhotoCraft's).
pub const MAX_COORDINATE: f64 = 1_000_000.0;
/// ⇧ turns the axes in steps of this many degrees, absolute from 0.
pub const TURN_STEP_DEGREES: f64 = 15.0;

/// The straight symmetry modes PhotoCraft has (lead call 23). Photoshop's Wavy, Circle, Spiral,
/// Parallel Lines, Radial and Mandala modes need engine work and are not built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum SymmetryMode {
    /// One vertical axis (90° before the rotation).
    Vertical,
    /// One horizontal axis (0°).
    Horizontal,
    /// A vertical and a horizontal axis: three mirrored copies.
    Dual,
    /// One diagonal axis (45°).
    Diagonal,
}

impl SymmetryMode {
    /// Every mode, in menu order.
    pub const ALL: [SymmetryMode; 4] = [SymmetryMode::Vertical, SymmetryMode::Horizontal, SymmetryMode::Dual, SymmetryMode::Diagonal];
}

/// A painting symmetry: its mode, the centre the axes run through (document units) and the
/// rotation in degrees clockwise (document y points down), 0 to 360.
/// Serialises camelCase, the centre as kurbo's point: brush presets store the symmetry.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Symmetry {
    /// Which axes the symmetry has.
    pub mode: SymmetryMode,
    /// The point the axes run through, in document units.
    pub center: Point,
    /// Rotation in degrees clockwise, 0 to 360.
    pub rotation: f64,
}

impl Symmetry {
    /// The symmetry, or `None` for a rotation or centre that is not finite or a centre coordinate
    /// beyond ±[`MAX_COORDINATE`]. The rotation wraps into 0 to 360.
    pub fn new(mode: SymmetryMode, center: Point, rotation: f64) -> Option<Symmetry> {
        // pc: crates/paint/src/symmetry.rs:41 `PresetSymmetry::new`.
        if !rotation.is_finite()
            || !center.x.is_finite()
            || !center.y.is_finite()
            || center.x.abs() > MAX_COORDINATE
            || center.y.abs() > MAX_COORDINATE
        {
            return None;
        }
        Some(Symmetry { mode, center, rotation: rotation.rem_euclid(360.0) })
    }

    /// The axes' unit directions: the first at 90° (Vertical, Dual), 0° (Horizontal) or 45°
    /// (Diagonal) plus the rotation, clockwise on screen; Dual adds the perpendicular.
    pub fn axes(&self) -> Vec<Vec2> {
        // pc: crates/paint/src/symmetry.rs:47 `directions`.
        let initial: f64 = match self.mode {
            SymmetryMode::Horizontal => 0.0,
            SymmetryMode::Diagonal => 45.0,
            SymmetryMode::Vertical | SymmetryMode::Dual => 90.0,
        };
        let angle = (initial + self.rotation).to_radians();
        let first = Vec2::new(angle.cos(), angle.sin());
        if self.mode == SymmetryMode::Dual { vec![first, Vec2::new(-first.y, first.x)] } else { vec![first] }
    }

    /// Each axis clipped to the document `size` (`(0, 0)` to `size`), whether or not the centre is
    /// inside it; an axis that misses the document gives no segment.
    pub fn segments(&self, size: Size) -> Vec<Line> {
        // pc: crates/paint/src/symmetry.rs:73 `segments`.
        self.axes()
            .into_iter()
            .filter_map(|d| {
                let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
                for ((origin, direction), extent) in [(self.center.x, d.x), (self.center.y, d.y)].into_iter().zip([size.width, size.height]) {
                    if direction.abs() < 1e-12 {
                        if origin < 0.0 || origin > extent {
                            return None;
                        }
                    } else {
                        let (a, b) = (-origin / direction, (extent - origin) / direction);
                        lo = lo.max(a.min(b));
                        hi = hi.min(a.max(b));
                    }
                }
                if !lo.is_finite() || !hi.is_finite() || hi < lo {
                    return None;
                }
                Some(Line::new(self.center + d * lo, self.center + d * hi))
            })
            .collect()
    }

    /// The rotation handle: along the first axis from the centre, at `share` of the document's
    /// shorter side (the handle share, 0.35), at least 1 unit.
    pub fn rotation_handle(&self, size: Size, share: f64) -> Point {
        // pc: crates/ui-egui/src/tools/symmetry_ui.rs:227 `rotation_handle`.
        let d = self.axes().into_iter().next().unwrap_or(Vec2::from_angle(0.0));
        let r = (size.width.min(size.height) * share).max(1.0);
        self.center + d * r
    }
}
