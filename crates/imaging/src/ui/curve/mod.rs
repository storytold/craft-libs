//! The curve editor: the tone curve value and its point rules, the graph, the preview and the gradient ramp.

mod editor;
mod graph;
mod model;
mod preview;
mod ramp;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;
mod words;

pub use editor::{CurveEditor, CurveEditorOutput, CurveRows};
pub(crate) use graph::PlotLook;
pub use graph::{CurveChannel, CurveSampler};
pub use model::{Curve, CurveRules};
pub use preview::CurvePreview;
pub(crate) use preview::paint_preview;
pub use ramp::ToneRamp;
pub use words::CurveWords;
