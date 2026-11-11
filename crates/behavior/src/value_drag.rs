//! `ValueDrag`: the drag maths of sliders, dials, scrubbing number fields and rulers, written once (8.7).

use crate::flow::Axis;
use crate::press::{PressPhase, PressState};
use emath::{Pos2, Rect, Vec2};
use std::ops::{Range, RangeInclusive};

/// How the pointer's position along the axis becomes a value.
#[derive(Clone, Debug, PartialEq)]
enum Mapping {
    /// The pointer position on a track from `track.start` to `track.end` along the axis, in layer points.
    Absolute { track: Range<f32> },
    /// Value units per point of travel from where the press began.
    Relative { rate: f64 },
}

/// Turns a press into a value. Components set it as data and call [`ValueDrag::value`] in `update`.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueDrag {
    mapping: Mapping,
    axis: Axis,
    range: RangeInclusive<f64>,
    warp: bool,
    hide_cursor: bool,
    fine: f64,
    coarse: Option<f64>,
}

/// The factor Alt (Option) applies to travel when a drag sets none.
const FINE: f64 = 0.1;

/// How close to an edge of the window, in points, the pointer warps.
const EDGE: f32 = 2.0;

/// How far from the far edge the pointer lands after a warp: outside the zone that warps.
const LANDING: f32 = 2.0 * EDGE;

impl ValueDrag {
    /// The value follows the pointer on `track`: `range.start()` at `track.start`, `range.end()` at `track.end`.
    /// A track whose `start` lies after its `end` runs backwards, as a vertical slider's does, and a range whose start
    /// exceeds its end maps the track the other way; the value stays within the range either way.
    pub fn absolute(track: Range<f32>, range: RangeInclusive<f64>) -> Self {
        ValueDrag { mapping: Mapping::Absolute { track }, axis: Axis::X, range, warp: false, hide_cursor: false, fine: FINE, coarse: None }
    }

    /// The value changes by `rate` units per point of travel from the press. Vertical travel upwards increases it.
    pub fn relative(rate: f64, range: RangeInclusive<f64>) -> Self {
        ValueDrag { mapping: Mapping::Relative { rate }, axis: Axis::X, range, warp: false, hide_cursor: false, fine: FINE, coarse: None }
    }

    /// The axis the pointer travels on; the default is horizontal.
    pub const fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    /// The factor Alt (Option) applies to travel.
    pub const fn fine(mut self, factor: f64) -> Self {
        self.fine = factor;
        self
    }

    /// The factor Shift applies to travel. A drag sets none by default: on an absolute drag such as a slider's a
    /// larger step has no meaning, so Shift changes nothing.
    pub const fn coarse(mut self, factor: f64) -> Self {
        self.coarse = Some(factor);
        self
    }

    /// Warps the pointer to the other side of the window when the drag reaches an edge, so the drag has no end
    /// ([`ValueDrag::warp_by`], [`DragWarp`]). It sets no wrap of the value.
    pub const fn warp(mut self, on: bool) -> Self {
        self.warp = on;
        self
    }

    /// Hides the pointer while the drag runs, as a scrub does that warps the pointer.
    pub const fn hide_cursor(mut self, on: bool) -> Self {
        self.hide_cursor = on;
        self
    }

    /// Whether the pointer warps at a window edge.
    pub const fn warps(&self) -> bool {
        self.warp
    }

    /// Whether the pointer is hidden while the drag runs.
    pub const fn hides_cursor(&self) -> bool {
        self.hide_cursor
    }

    /// How far to move the pointer when the drag at `pos` has reached an edge of `bounds` on the drag's axis: to
    /// the far side, inside the zone that warps. `None` away from the edges and for a drag that does not warp.
    pub fn warp_by(&self, pos: Pos2, bounds: Rect) -> Option<Vec2> {
        if !self.warp {
            return None;
        }
        let (at, min, max) = match self.axis {
            Axis::X => (pos.x, bounds.min.x, bounds.max.x),
            Axis::Y => (pos.y, bounds.min.y, bounds.max.y),
        };
        let to = if at <= min + EDGE {
            max - LANDING
        } else if at >= max - EDGE {
            min + LANDING
        } else {
            return None;
        };
        Some(match self.axis {
            Axis::X => Vec2::new(to - at, 0.0),
            Axis::Y => Vec2::new(0.0, to - at),
        })
    }

    /// The value the press gives, `None` while it gives none: before a drag starts, and after a cancel. `start` is the
    /// value when the drag began. Horizontal travel mirrors when `rtl`.
    pub fn value(&self, start: f64, press: &PressState, rtl: bool) -> Option<f64> {
        self.value_shifted(start, press, rtl, Vec2::ZERO)
    }

    /// [`ValueDrag::value`] for a pointer that warped by `shift` in total ([`DragWarp::shift`]): the pointer
    /// counts as where it would be without the warps, so the value stays continuous across them.
    pub fn value_shifted(&self, start: f64, press: &PressState, rtl: bool, shift: Vec2) -> Option<f64> {
        let (origin, pos) = match press.phase {
            PressPhase::DragStart { start, pos } | PressPhase::Drag { start, pos, .. } | PressPhase::Release { start, pos, .. } => {
                (start.origin, pos)
            }
            _ => return None,
        };
        let pos = pos - shift;
        let fine = if press.mods.alt {
            self.fine
        } else if press.mods.shift {
            self.coarse.unwrap_or(1.0)
        } else {
            1.0
        };
        let raw = match &self.mapping {
            Mapping::Absolute { track } => {
                let at = |point: Pos2| self.fraction(point, track, rtl) * (self.range.end() - self.range.start()) + self.range.start();
                if fine == 1.0 { at(pos) } else { start + (at(pos) - at(origin)) * fine }
            }
            Mapping::Relative { rate } => {
                let (from, to) = (self.along(origin), self.along(pos));
                let travel = match self.axis {
                    Axis::X if rtl => from - to,
                    Axis::X => to - from,
                    Axis::Y => from - to,
                };
                let travel = f64::from(travel);
                start + travel * rate * fine
            }
        };
        Some(self.fit(raw))
    }

    fn along(&self, point: Pos2) -> f32 {
        match self.axis {
            Axis::X => point.x,
            Axis::Y => point.y,
        }
    }

    /// Where along the track `point` lies, 0 at its start and 1 at its end, mirrored under `rtl` on the horizontal axis.
    fn fraction(&self, point: Pos2, track: &Range<f32>, rtl: bool) -> f64 {
        let span = track.end - track.start;
        let x = if self.axis == Axis::X && rtl { track.start + track.end - self.along(point) } else { self.along(point) };
        if span == 0.0 { 0.0 } else { f64::from((x - track.start) / span) }
    }

    fn fit(&self, value: f64) -> f64 {
        let (low, high) = (self.range.start().min(*self.range.end()), self.range.start().max(*self.range.end()));
        value.clamp(low, high)
    }
}

/// `value` moved by whole periods into `low..low + period`, as a wrapping slider or dial holds it. A period that is
/// not finite and positive leaves the value as it is.
pub fn wrap_value(value: f64, low: f64, period: f64) -> f64 {
    if !(period.is_finite() && period > 0.0) {
        return value;
    }
    low + (value - low).rem_euclid(period)
}

/// The pointer warps of one drag. The OS reports the pointer at its new place a frame after a warp, so a warp
/// counts once the pointer is seen there.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DragWarp {
    shift: Vec2,
    sent: Option<(Pos2, Vec2)>,
}

impl DragWarp {
    /// The total warp so far, given the pointer `pos` of this pass. Pass it to [`ValueDrag::value_shifted`].
    pub fn shift(&mut self, pos: Pos2) -> Vec2 {
        if let Some((from, by)) = self.sent
            && (pos - (from + by)).length() < (pos - from).length()
        {
            self.shift += by;
            self.sent = None;
        }
        self.shift
    }

    /// Whether a warp waits for the pointer to show up at its new place: no second warp starts before it does.
    pub const fn pending(&self) -> bool {
        self.sent.is_some()
    }

    /// Records that the pointer at `from` was warped by `by`.
    pub fn sent(&mut self, from: Pos2, by: Vec2) {
        self.sent = Some((from, by));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::press::{PointerButton, PressStart};
    use craft_keymap::Mods;
    use emath::{Vec2, pos2};

    fn drag(from: Pos2, to: Pos2, mods: Mods) -> PressState {
        let start = PressStart::new(from, PointerButton::Primary, 0.0, 1);
        PressState::new(PressPhase::Drag { start, pos: to, delta: Vec2::ZERO }).mods(mods)
    }

    #[test]
    fn a_warped_scrub_keeps_its_value_continuous() {
        let scrub = ValueDrag::relative(1.0, 0.0..=1000.0).warp(true);
        let bounds = Rect::from_min_max(pos2(0.0, 0.0), pos2(500.0, 300.0));
        let origin = pos2(400.0, 100.0);
        let mut warp = DragWarp::default();
        let at = |warp: &mut DragWarp, pos: Pos2| scrub.value_shifted(100.0, &drag(origin, pos, Mods::NONE), false, warp.shift(pos));

        assert_eq!(at(&mut warp, pos2(450.0, 100.0)), Some(150.0));
        let edge = pos2(499.0, 100.0);
        assert_eq!(at(&mut warp, edge), Some(199.0));
        let by = scrub.warp_by(edge, bounds).expect("the pointer is at the right edge");
        assert_eq!(edge + by, pos2(LANDING, 100.0), "the pointer lands inside the left edge");
        warp.sent(edge, by);
        let landed = edge + by;
        assert_eq!(at(&mut warp, landed), Some(199.0), "the first pass at the new place continues from the value before the warp");
        assert_eq!(at(&mut warp, landed + Vec2::new(10.0, 0.0)), Some(209.0));
        assert_eq!(scrub.warp_by(pos2(250.0, 100.0), bounds), None, "no warp away from the edges");
        assert_eq!(ValueDrag::relative(1.0, 0.0..=1.0).warp_by(edge, bounds), None, "a drag that does not warp stays put");
    }

    #[test]
    fn wrap_value_brings_values_into_one_period() {
        assert_eq!(wrap_value(-10.0, 0.0, 360.0), 350.0);
        assert_eq!(wrap_value(5.0, 0.0, 0.0), 5.0, "a period of 0 leaves the value, not NaN");
    }
}
