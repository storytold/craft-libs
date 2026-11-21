//! Steps due while a press is held, as a spinner button repeats.

/// Seconds a press is held before the steps repeat.
pub const DELAY: f64 = 0.4;

/// Seconds between repeated steps.
pub const INTERVAL: f64 = 0.05;

/// Steps due while a press is held, from the pass's elapsed time (zero on later passes of a frame, so a rerun steps
/// nothing). A different concept from `HoldZoom`, which zooms continuously at a rate.
#[derive(Clone, Copy, Debug, Default)]
pub struct HoldRepeat {
    held_for: f64,
    fired: u32,
}

impl HoldRepeat {
    /// Starts a press: the one step due now.
    pub fn press(&mut self) -> u32 {
        *self = Self::default();
        1
    }

    /// The steps due after `elapsed` more seconds of the press.
    pub fn hold(&mut self, elapsed: f64) -> u32 {
        self.held_for += elapsed.max(0.0);
        let due = if self.held_for >= DELAY {
            // Saturating float-to-int cast: a huge elapsed cannot panic.
            ((self.held_for - DELAY) / INTERVAL).floor() as u32 + 1
        } else {
            0
        };
        let new = due.saturating_sub(self.fired);
        self.fired = self.fired.max(due);
        new
    }

    /// Ends the press.
    pub fn release(&mut self) {
        *self = Self::default();
    }
}
