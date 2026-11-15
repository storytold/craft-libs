//! The drags on the image: moving a Drop Shadow or Inner Shadow (#5701), and a Gradient Overlay's offset.

use super::model::{EffectId, StyleValue};

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

/// The largest offset a Gradient Overlay drag writes, in layer sizes, either way.
const OFFSET_LIMIT: f64 = 4.0;

/// A Gradient Overlay offset drag in progress: the effect instance, the offset it started from
/// and the pointer's travel when it started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientOffsetDrag {
    instance: EffectId,
    origin: [f64; 2],
    base: [f64; 2],
}

impl GradientOffsetDrag {
    /// A drag on `instance` from the shown offset `origin`, begun when the pointer's travel since
    /// the press was `base` (zero at the press; the travel so far when the drag moves to another
    /// instance mid-press, so the offset does not jump).
    pub fn begin(instance: EffectId, origin: [f64; 2], base: [f64; 2]) -> Self {
        GradientOffsetDrag { instance, origin, base }
    }

    /// The offset after the pointer travelled `total` over a layer of `layer_size`: the origin plus
    /// the travel since `base`, in layer sizes (a size under 1 counts as 1), clamped to plus or
    /// minus 4 and rounded to 0.001. A result that is not finite is the origin.
    pub fn moved(&self, total: [f64; 2], layer_size: [f64; 2]) -> [f64; 2] {
        // The result is not an `Option`, so a non-finite result falls back to the origin.
        let axis = |i: usize| {
            let v = self.origin[i] + (total[i] - self.base[i]) / layer_size[i].max(1.0);
            if v.is_finite() { (v.clamp(-OFFSET_LIMIT, OFFSET_LIMIT) * 1000.0).round() / 1000.0 } else { self.origin[i] }
        };
        [axis(0), axis(1)]
    }

    /// The offset a page shows for the stored `value`: its parts clamped to plus or minus 4, or
    /// `default` when it is no point or a part is not finite.
    pub(super) fn shown(value: Option<&StyleValue>, default: [f64; 2]) -> [f64; 2] {
        match value {
            Some(StyleValue::Point(p)) if p.iter().all(|x| x.is_finite()) => p.map(|x| x.clamp(-OFFSET_LIMIT, OFFSET_LIMIT)),
            _ => default,
        }
    }

    /// The offset the drag started from.
    pub(super) fn origin(&self) -> [f64; 2] {
        self.origin
    }

    /// The effect instance the drag moves.
    pub fn instance(&self) -> EffectId {
        self.instance
    }
}
