//! The gradient stop bar's widgets: [`GradientEditor`], the stop bar that edits the stops, and [`GradientRamp`], the paint.
//!
//! The model and the stop rules ([`GradientStops`], [`stops`]) need no egui and live in [`crate::gradient`]; they are
//! re-exported here so a widget user imports the editor and its model from one path.

mod editor;
mod ramp;
mod value;
mod words;

pub use crate::gradient::{ColorStop, GradientStops, OpacityStop, Sampler, StopRow, StopRows, StopRules, default_sampler, stops};
pub use editor::{GradientEditor, GradientEditorOutput, StopDrop, marker_texts};
pub use ramp::GradientRamp;
pub use value::{StopColor, StopRef};
pub use words::GradientWords;
