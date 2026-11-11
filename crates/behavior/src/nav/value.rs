//! Stepping a numeric value from keys and the wheel.

use crate::intent::{MoveMods, NavMove};
use crate::wheel::Wheel;
use std::ops::RangeInclusive;

/// The axes a single-value control reads moves on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValueAxes {
    /// Left and right arrows.
    Horizontal,
    /// Up and down arrows.
    Vertical,
    /// Both pairs.
    Both,
}

/// How many small steps one large step is, when a control sets no large step of its own.
pub const LARGE_STEPS: f64 = 10.0;

/// The share of a value's span that one small step is, when a control sets no step.
pub const STEP_SHARE: f64 = 0.01;

/// The value and steps of a single-value control.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ValueSteps {
    /// The current value.
    pub value: f64,
    /// The smallest value.
    pub min: f64,
    /// The largest value.
    pub max: f64,
    /// One arrow press.
    pub small: f64,
    /// One Page key, or an arrow with the extend modifier.
    pub large: f64,
    /// An arrow with Alt.
    pub fine: f64,
    /// One wheel notch.
    pub notch: f64,
}

impl ValueSteps {
    /// Steps over `min..=max` from `value`. The fine step is `small / LARGE_STEPS` and a wheel notch is `small`.
    pub const fn new(value: f64, min: f64, max: f64, small: f64, large: f64) -> Self {
        ValueSteps { value, min, max, small, large, fine: small / LARGE_STEPS, notch: small }
    }

    /// The same steps with an Alt arrow of `fine`.
    pub const fn fine(mut self, fine: f64) -> Self {
        self.fine = fine;
        self
    }

    /// The same steps with a wheel notch of `notch`.
    pub const fn notch(mut self, notch: f64) -> Self {
        self.notch = notch;
        self
    }
}

/// What a single-value move does.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum StepResult {
    /// The control takes this value.
    Set(f64),
    /// The move is not a value move on these axes.
    Ignored,
}

/// Maps a move to the new value of a slider, dial, rating, resize handle or colour pad. `Forward`, `Up` and
/// `PageUp` increase; `Start` and `End` go to the ends. An arrow steps by `small`; with Shift (`extend`) by `large`,
/// with Alt (`fine`) by `fine`, and with both by `small`.
pub fn step_value(steps: &ValueSteps, axes: ValueAxes, mv: NavMove, mods: MoveMods) -> StepResult {
    let horizontal = !matches!(axes, ValueAxes::Vertical);
    let vertical = !matches!(axes, ValueAxes::Horizontal);
    let small = match (mods.extend, mods.fine) {
        (true, false) => steps.large,
        (false, true) => steps.fine,
        _ => steps.small,
    };
    let delta = match mv {
        NavMove::Forward if horizontal => small,
        NavMove::Back if horizontal => -small,
        NavMove::Up if vertical => small,
        NavMove::Down if vertical => -small,
        NavMove::PageUp => steps.large,
        NavMove::PageDown => -steps.large,
        NavMove::Start => return StepResult::Set(steps.min),
        NavMove::End => return StepResult::Set(steps.max),
        _ => return StepResult::Ignored,
    };
    StepResult::Set((steps.value + delta).clamp(steps.min, steps.max))
}

/// Maps a wheel event to the new value of a single-value control: one `notch` per event, up for positive scroll.
/// Vertical scroll counts first; horizontal scroll counts when there is none (a vertical-only control ignores it),
/// flipped under right-to-left text. Shift scales the notch by `large / small` and Alt by `fine / small`; both
/// together keep the notch, as the arrow keys do. Zero scroll is `Ignored`.
pub fn wheel_step(steps: &ValueSteps, axes: ValueAxes, wheel: &Wheel, rtl: bool) -> StepResult {
    let along = if wheel.scroll.y != 0.0 {
        wheel.scroll.y
    } else if matches!(axes, ValueAxes::Vertical) || wheel.scroll.x == 0.0 {
        return StepResult::Ignored;
    } else if rtl {
        -wheel.scroll.x
    } else {
        wheel.scroll.x
    };
    let scale = match (wheel.mods.shift, wheel.mods.alt) {
        (true, false) if steps.small != 0.0 => steps.large / steps.small,
        (false, true) if steps.small != 0.0 => steps.fine / steps.small,
        _ => 1.0,
    };
    let delta = steps.notch * scale * f64::from(along.signum());
    StepResult::Set((steps.value + delta).clamp(steps.min, steps.max))
}

/// How near a stop, in steps, a value counts as on it.
const ON_STOP: f64 = 1e-9;

/// `value` clamped to `range`, then on the nearest of the stops `origin + k * step` and the ends of `range`; a tie
/// goes to the larger stop. A value on a stop comes back unchanged. A value that is not finite, or a step that is not
/// finite and positive, comes back only clamped.
pub fn snap_to_step(value: f64, step: f64, origin: f64, range: RangeInclusive<f64>) -> f64 {
    let (low, high) = (range.start().min(*range.end()), range.start().max(*range.end()));
    let value = value.clamp(low, high);
    if !(value.is_finite() && step.is_finite() && step > 0.0) {
        return value;
    }
    let at = (value - origin) / step;
    if (at - at.round()).abs() < ON_STOP {
        return value;
    }
    let below = (origin + at.floor() * step).max(low);
    let above = (origin + at.ceil() * step).min(high);
    if value - below < above - value { below } else { above }
}
