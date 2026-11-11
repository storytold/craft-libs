//! The brush tip on the canvas: its cursor, its falloff preview and the resize gesture.
//!
//! Every app with a brush describes its tip once and gets Photoshop's pointers and on-canvas resize. The engine
//! computes the numbers (the 50 % contour, a traced outline, the falloff); the library never sees an engine type.
//!
//! - [`BrushTip`], [`BrushTipOutline`], [`TipRings`]: the tip as the cursor draws it: a round tip's two radii, a
//!   traced shape's rings, or an aliased tip's whole-pixel cell.
//! - [`TipPose`]: the pen's tilt and rotation as the tip's axes in document space.
//! - [`BrushCursor`]: the cursor for the tip under the user's [`BrushCursorOptions`] (Standard, Precise, Normal or
//!   Full Size Brush Tip, Caps Lock, Show Only Crosshair While Painting), through zoom, rotation, flip and pixel
//!   aspect. [`CursorBitmap`] rasterises the painted outline for a window's own cursor image.
//! - [`TipCell`]: an aliased tip's pixel cell, a canvas control the tool shows beside the hidden pointer.
//! - [`BrushResize`]: Photoshop's resize drag (Alt + right drag, or Control + Alt + drag): left and right set the
//!   diameter, up and down the hardness ([`BrushLevel`]), reported as [`ResizeStep`]s.
//! - [`TipFalloff`] and [`FalloffPreview`]: the tip's falloff and the canvas control that shows it with the brush
//!   circle round it.
//!
//! A brush tool returns [`BrushCursor::cursor`] from `CanvasTool::cursor`. The app shows [`BrushResize::preview`] or
//! [`BrushCursor::tip_cell`] at the canvas rect in the canvas body, and the tool hands
//! [`BrushResize::readout_rows`] to `ToolContext::readout`.

mod cursor;
mod falloff;
mod resize;
#[cfg(feature = "testing")]
mod stories;
mod tip;

pub use cursor::{BrushCursor, BrushCursorOptions, CursorBitmap, CursorBitmapSpec, PaintingCursor, TipCell, TipCellStyle};
pub use falloff::{FalloffPreview, FalloffPreviewStyle, TipFalloff};
pub use resize::{BrushDimensions, BrushLevel, BrushResize, BrushResizeOptions, RESIZE_TRAVEL, ResizeStep};
pub use tip::{BrushTip, BrushTipOutline, TipPose, TipRings};
