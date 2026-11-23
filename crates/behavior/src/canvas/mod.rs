//! The transform a canvas body is shown through, and the canvas view with its zoom rules, history and ruler units.

use emath::{Pos2, Vec2};

mod history;
mod units;
mod view;
pub mod zoom;

pub use history::{ViewHistory, ViewPlace};
pub use units::{RulerUnits, Ticks};
pub use view::{CanvasView, resize_cursor};
pub use zoom::ZoomLadder;

/// A canvas body's mapping from canvas coordinates to its layer: `screen = canvas * scale + offset`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CanvasTransform {
    /// Layer points per canvas unit.
    pub scale: f32,
    /// Where the canvas origin lies in the layer.
    pub offset: Vec2,
}

impl CanvasTransform {
    /// The transform with `scale` layer points per canvas unit and the canvas origin at `offset`.
    pub const fn new(scale: f32, offset: Vec2) -> Self {
        Self { scale, offset }
    }

    /// The position in the layer of a canvas point.
    pub fn to_layer(&self, canvas: Pos2) -> Pos2 {
        Pos2::new(canvas.x * self.scale + self.offset.x, canvas.y * self.scale + self.offset.y)
    }
}
