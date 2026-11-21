//! Steps due while a press is held, as a spinner button repeats.

/// Seconds a press is held before the steps repeat.
pub const DELAY: f64 = 0.4;

/// Seconds between repeated steps.
pub const INTERVAL: f64 = 0.05;

/// Steps due while a press is held, from the pass's elapsed time (zero on later passes of a frame, so a rerun steps
/// nothing). A different concept from `HoldZoom`, which zooms continuously at a rate.
#[derive(Clone, Copy, Debug, Default)]
pub struct HoldRepeat {
    #[allow(dead_code)] // P3(widgets-w2): wg.W35
    held_for: f64,
    #[allow(dead_code)] // P3(widgets-w2): wg.W35
    fired: u32,
}

impl HoldRepeat {
    /// Starts a press: the one step due now.
    pub fn press(&mut self) -> u32 {
        todo!("wg.W35")
    }

    /// The steps due after `elapsed` more seconds of the press.
    pub fn hold(&mut self, elapsed: f64) -> u32 {
        let _ = elapsed;
        todo!("wg.W35")
    }

    /// Ends the press.
    pub fn release(&mut self) {
        todo!("wg.W35")
    }
}
