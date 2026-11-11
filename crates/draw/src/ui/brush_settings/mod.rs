//! The Brush Settings panel.
//!
//! - [`BrushSettingsPanel`]: the frame: the section list, the shown section's pane (the app's
//!   content), the stroke strip and the New, Reset and Update buttons.
//! - [`BrushSectionList`] with [`BrushSection`] rows: Photoshop's list of sections with enable
//!   boxes, locks and switch rows, reporting [`BrushSectionEvent`]s.
//! - [`PreviewStroke`] and [`BrushStrokeStrip`]: the S-curve test stroke as data, and the strip
//!   that keeps the app's rendering of it as a texture.
//! - The rows the section bodies share: [`BrushSizeField`], [`TipEllipse`] and
//!   [`DynamicControlRow`].
//!
//! The brush settings and the section bodies stay in the app (its engine's types); the library
//! draws the frame and the shared rows, and every control reports edit events the app sends to its
//! tool options host.

mod ellipse;
mod panel;
mod preview;
mod rows;
mod sections;
mod size_field;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;
mod words;

pub use ellipse::{TipAngle, TipEllipse, TipEllipseGrab, TipEllipseStyle, ellipse_drag, ellipse_grab, ellipse_handles};
pub use panel::{BrushPanelAction, BrushSettingsPanel, BrushSettingsPanelOutput};
pub use preview::{BrushStrokeStrip, BrushStrokeStripStyle, PreviewSample, PreviewStroke, StripImage};
pub use rows::{DynamicControl, DynamicControlRow, DynamicControlRowOutput};
pub use sections::{BrushSection, BrushSectionEvent, BrushSectionKind, BrushSectionList, BrushSectionListOutput};
pub use size_field::{BrushSizeField, BrushSizeOutput};
pub use words::BrushSettingsWords;
