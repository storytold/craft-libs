//! Steps due while a press is held, as a spinner button repeats.

/// A press held on a spinner button, known by the pass time it began. It keeps nothing between passes: a pass builds it
/// from the press's own start time and asks how many repeats the pass owes, so a rerun pass (no time elapsed) owes none.
/// A different concept from `HoldZoom`, which zooms continuously at a rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoldRepeat {
    start: f64,
}

impl HoldRepeat {
    /// Seconds a press is held before the steps repeat.
    pub const DELAY: f64 = 0.45;

    /// Seconds between repeated steps.
    pub const INTERVAL: f64 = 0.06;

    /// A press that began at pass time `start`, in seconds.
    pub const fn new(start: f64) -> Self {
        HoldRepeat { start }
    }

    /// How many repeat instants (`start + DELAY + k * INTERVAL`, `k >= 0`) lie at or before `at`.
    fn reached(&self, at: f64) -> u32 {
        let since = at - self.start - Self::DELAY;
        if since < 0.0 { 0 } else { (since / Self::INTERVAL).floor() as u32 + 1 }
    }

    /// The repeats due in the pass at `now` that follows another `elapsed` seconds after: the repeat instants in
    /// `(now - elapsed, now]`. The step of the press itself is not counted.
    pub fn due(&self, now: f64, elapsed: f64) -> u32 {
        self.reached(now).saturating_sub(self.reached(now - elapsed))
    }

    /// The first repeat instant after `now`, the time to ask the next pass for while the press is held.
    pub fn next_repeat(&self, now: f64) -> f64 {
        self.start + Self::DELAY + f64::from(self.reached(now)) * Self::INTERVAL
    }
}
