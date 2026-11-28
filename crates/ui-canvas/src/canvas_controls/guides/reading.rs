//! `GuideReading`: the guide a drag is moving or creating, read from the running gesture.

use crate::canvas::Guide;
use craft_behavior::Axis;
use egui::{Context, Id};

/// The position and orientation of the guide a drag is moving or creating.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct GuideReading {
    /// The axis the guide runs along.
    pub along: Axis,
    /// The guide's position across that axis, in document units.
    pub position: f64,
}

impl GuideReading {
    /// The reading of a guide running `along` at `position`.
    pub const fn new(along: Axis, position: f64) -> Self {
        Self { along, position }
    }
}

/// The guide a drag is moving or creating on the canvas `canvas`, read from the note the guides set on their running
/// gesture, so a panel drawn before or after the canvas reads the same value; `None` when no guide drag runs or the
/// dragged guide lies off the canvas.
pub fn running(ctx: &Context, canvas: Id) -> Option<GuideReading> {
    crate::runtime::edit::gesture_note::<Vec<Guide>, Option<GuideReading>>(ctx, super::guides_id(canvas)).flatten()
}
