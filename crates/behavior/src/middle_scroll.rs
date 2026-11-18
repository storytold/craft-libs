//! The middle button's auto-scroll latch: the gesture state of one middle press.

use emath::{Pos2, Vec2};

/// What the middle button does over a scroll area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MiddleButton {
    /// Drags the view.
    #[default]
    Pan,
    /// Latches an auto-scroll anchor.
    AutoScroll,
}

/// The auto-scroll latch of one middle press.
#[derive(Clone, Debug, Default, PartialEq)]
#[allow(dead_code)] // P3(pdf-canvas-w2): CV-C8
pub struct MiddleScroll {
    anchor: Option<Pos2>,
    held_passes: u32,
}

impl MiddleScroll {
    /// A middle press: inside the scroll area it latches at `at`; a second press ends the latch.
    pub fn press(&mut self, at: Pos2, inside: bool) {
        let _ = (at, inside);
        todo!("CV-C8")
    }

    /// The button went up: after two held passes with the pointer past the dead zone the latch ends; an earlier
    /// release keeps it.
    pub fn release(&mut self) {
        todo!("CV-C8")
    }

    /// Ends the latch.
    pub fn cancel(&mut self) {
        todo!("CV-C8")
    }

    /// One pass: with this pass's `pointer` and `held` state it counts the held passes past the 15 pt dead zone and
    /// returns the vertical scroll for the step `frame_step(dt)`, at `|d|^2.2 × 0.04` points per second, reversed by
    /// `invert`.
    pub fn step(&mut self, pointer: Pos2, held: bool, dt: f64, invert: bool) -> Vec2 {
        let _ = (pointer, held, dt, invert);
        todo!("CV-C8")
    }

    /// Where the latch is anchored, `None` when there is none.
    pub fn anchor(&self) -> Option<Pos2> {
        todo!("CV-C8")
    }

    /// Whether a latch is active.
    pub fn active(&self) -> bool {
        todo!("CV-C8")
    }
}
