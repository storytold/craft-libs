//! The curve editor: the tone curve value and its point rules, the graph, the preview and the gradient ramp.

pub(crate) mod editor;
pub(crate) mod graph;
mod model;
pub(crate) mod preview;
pub(crate) mod ramp;
#[cfg(feature = "suite")]
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
