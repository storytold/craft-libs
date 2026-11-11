//! The shadow drag: moving a Drop Shadow or Inner Shadow by dragging on the image (#5701).

/// A shadow drag in progress: the shadow's angle and distance when the press came. The offset of
/// a shadow is (−cos angle, sin angle) × distance (the light sits opposite the shadow; y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowDrag {
    /// Degrees, as the effect's `angle`.
    pub start_angle: f64,
    /// Pixels, as the effect's `distance`.
    pub start_distance: f64,
}

impl ShadowDrag {
    /// A drag from a shadow at `angle` degrees and `distance` pixels.
    pub fn begin(angle: f64, distance: f64) -> ShadowDrag {
        ShadowDrag { start_angle: angle, start_distance: distance }
    }

    /// The `(angle, distance)` after the pointer moved `total` document pixels since the press:
    /// the start offset plus `total`; the distance is its length rounded to a whole pixel and
    /// clamped to 0..=30000; the angle `atan2(oy, −ox)` in whole degrees, or the start angle while
    /// the new offset is shorter than 0.5 px. `None` when the result is not finite.
    pub fn moved(self, total: [f64; 2]) -> Option<(f64, f64)> {
        // pc: crates/ui-egui/src/dialogs/layer_style.rs:517-527 `drag_shadow`.
        let a = self.start_angle.to_radians();
        let (ox, oy) = (-a.cos() * self.start_distance + total[0], a.sin() * self.start_distance + total[1]);
        let d = ox.hypot(oy);
        if !d.is_finite() {
            return None;
        }
        let angle = if d >= 0.5 { oy.atan2(-ox).to_degrees().round() } else { self.start_angle };
        Some((angle, d.round().clamp(0.0, 30000.0)))
    }
}
