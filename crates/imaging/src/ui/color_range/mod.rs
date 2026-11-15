//! Color Range: Photoshop's Select › Color Range, the dialog that selects by sampled colours, colour families or
//! tonal ranges.
//!
//! - [`ColorRangeDialog`]: the body. The select, the sliders that apply to it, the preview of the selection (or of the
//!   image) that the sample tools click and drag in, the sample count, Invert, and the Selection / Image and Selection
//!   Preview choices. It edits a [`ColorRangeValue`] and reports the dialog state it keeps beside it (the sample tool,
//!   the view, the Selection Preview).
//! - [`HostedColorRange`]: the body in a form dialog the dialog host owns. The app renders the previews and feeds them
//!   with `set_preview`; the canvas samples through `sample_canvas`.
//! - [`ColorRangeValue`]: the command's parameters. [`ColorSamples`] holds the sampled points and the order they were
//!   made in; its `pick` is also Replace Color's sampling rule.

mod dialog;
mod hosted;
mod labels;
#[cfg(feature = "screen-testing")]
mod stories;
mod value;

pub use dialog::{ColorRangeDialog, ColorRangeOutput};
pub use hosted::HostedColorRange;
pub use labels::ColorRangeWords;
pub use value::{ColorRangeSelect, ColorRangeValue, ColorSamples, RangeControls, SampleSign, SampleTool};
