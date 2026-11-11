//! The annotation tool on the canvas, the overlay that draws annotations, and the toolbar.

mod overlay;
#[cfg(feature = "screen-testing")]
mod stories;
mod tool;
mod toolbar;
mod words;

pub use overlay::{AnnotationOverlay, AnnotationOverlayStyle};
pub use tool::{AnnotateTool, DEFAULT_COLOR, DEFAULT_STROKE_WIDTH, HIGHLIGHT_COLOR, NEW_ANNOTATION_FORM, TextQuads};
pub use toolbar::{AnnotationToolbar, AnnotationToolbarOutput};
pub use words::{AnnotateCommand, AnnotateWords, PALETTE};
