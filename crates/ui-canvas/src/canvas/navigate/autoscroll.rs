//! `MiddleAutoscroll`: the press, dead zone, mark and end of a middle-button autoscroll.

use egui::{Pos2, Vec2};

/// A middle click's autoscroll: the press, the dead zone, the mark and the end. The speed is the canvas's one autoscroll
/// rule, `Autoscroll::step`.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)] // P3(canvas-w1): cv.K4
pub(crate) struct MiddleAutoscroll {
    origin: Option<Pos2>,
    dragged: bool,
    last: Option<f64>,
}

#[allow(dead_code)] // P3(canvas-w1): cv.K4
impl MiddleAutoscroll {
    /// Starts the autoscroll with the mark at `at`.
    pub(crate) fn press(&mut self, at: Pos2) {
        let _ = (&mut *self, at);
        todo!("cv.K4")
    }

    /// The view's step this pass for the pointer at `pointer`, `clock` being the pass clock in seconds.
    pub(crate) fn step(&mut self, pointer: Pos2, clock: f64) -> Vec2 {
        let _ = (&mut *self, pointer, clock);
        todo!("cv.K4")
    }

    /// Ends the autoscroll.
    pub(crate) fn end(&mut self) {
        let _ = &mut *self;
        todo!("cv.K4")
    }

    /// The mark's position while the autoscroll runs.
    pub(crate) fn mark(&self) -> Option<Pos2> {
        let _ = self;
        todo!("cv.K4")
    }
}
