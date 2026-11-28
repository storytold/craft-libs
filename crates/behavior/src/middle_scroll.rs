//! The middle button's auto-scroll latch: the gesture state of one middle press.

use crate::frame_step;
use crate::press::PressPhase;
use emath::{Pos2, Vec2};

/// How far, in points, the pointer may sit above or below the anchor without scrolling.
const DEAD_ZONE: f32 = 15.0;
/// Held passes past the dead zone after which a release ends the latch: a shorter hold is a click, which keeps it.
const HELD_PASSES: u32 = 2;
/// The power of the distance the speed grows with.
const SPEED_EXPONENT: f32 = 2.2;
/// Points per second per unit of the distance raised to [`SPEED_EXPONENT`].
const SPEED_MULTIPLIER: f32 = 0.04;
/// The farthest distance the speed is computed from, so the power of a hostile pointer position stays finite. No
/// pointer on a real screen reaches it.
const MAX_DISPLACEMENT: f32 = 1_000_000.0;

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
pub struct MiddleScroll {
    anchor: Option<Pos2>,
    held_passes: u32,
    swallowing: bool,
}

impl MiddleScroll {
    /// A middle press: inside the scroll area it latches at `at`; a second press ends the latch.
    pub fn press(&mut self, at: Pos2, inside: bool) {
        if self.active() {
            self.cancel();
        } else if inside && at.is_finite() {
            self.anchor = Some(at);
            self.held_passes = 0;
        }
    }

    /// The button went up: after two held passes with the pointer past the dead zone the latch ends; an earlier
    /// release keeps it.
    pub fn release(&mut self) {
        if self.held_passes >= HELD_PASSES {
            self.cancel();
        }
    }

    /// Whether a press's `phase` belongs to the press that ended a latch and so reaches nothing else: true for the
    /// `Down` that cancels an active latch and for each later phase of that press through its release or cancel. A
    /// press that begins while nothing is latched is not swallowed.
    pub fn swallows(&mut self, phase: PressPhase) -> bool {
        match phase {
            PressPhase::Down { .. } => {
                self.swallowing = self.active();
                if self.swallowing {
                    self.cancel();
                }
                self.swallowing
            }
            PressPhase::Release { .. } | PressPhase::Cancel { .. } => std::mem::take(&mut self.swallowing),
            PressPhase::Idle | PressPhase::Hover => false,
            _ => self.swallowing,
        }
    }

    /// Ends the latch.
    pub fn cancel(&mut self) {
        self.anchor = None;
        self.held_passes = 0;
    }

    /// One pass: with this pass's `pointer` and `held` state it counts the held passes past the 15 pt dead zone and
    /// returns the vertical scroll for the step `frame_step(dt)`, at `|d|^2.2 × 0.04` points per second, reversed by
    /// `invert`. The result moves the content: with the pointer below the anchor it is negative, so the view scrolls
    /// down. Zero while nothing is latched.
    pub fn step(&mut self, pointer: Pos2, held: bool, dt: f64, invert: bool) -> Vec2 {
        let Some(anchor) = self.anchor else { return Vec2::ZERO };
        let displacement = pointer.y - anchor.y;
        if !displacement.is_finite() {
            return Vec2::ZERO;
        }
        let distance = displacement.abs();
        if distance <= DEAD_ZONE {
            return Vec2::ZERO;
        }
        let step = frame_step(dt);
        // A frame's later passes take no time: counting them would count one frame's hold twice and end a click.
        if held && step > 0.0 {
            self.held_passes = self.held_passes.saturating_add(1);
        }
        // The full distance sets the speed, not the part past the dead zone.
        let speed = distance.min(MAX_DISPLACEMENT).powf(SPEED_EXPONENT) * SPEED_MULTIPLIER;
        let delta = -displacement.signum() * speed * step as f32;
        Vec2::new(0.0, if invert { -delta } else { delta })
    }

    /// Where the latch is anchored, `None` when there is none.
    pub fn anchor(&self) -> Option<Pos2> {
        self.anchor
    }

    /// Whether a latch is active.
    pub fn active(&self) -> bool {
        self.anchor.is_some()
    }
}
