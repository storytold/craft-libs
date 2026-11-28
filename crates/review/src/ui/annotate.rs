//! The annotation tool on the canvas, the overlay that draws annotations, and the toolbar.

pub(crate) mod overlay;
#[cfg(feature = "suite")]
pub(crate) mod stories;
mod tool;
pub(crate) mod toolbar;
mod words;

pub use crate::annotate::ShiftLock;
pub use overlay::{AnnotationOverlay, AnnotationOverlayStyle, BodyPaint};
pub use tool::{
    AnnotateTool, DEFAULT_COLOR, DEFAULT_STROKE_WIDTH, DrawOptions, EmptyPress, EraseMode, FarNudge, HIGHLIGHT_COLOR, HandlesWhileDrawing,
    InkStrokes, MarkupPress, NEW_ANNOTATION_FORM, OpenOn, SelectOptions, TextQuads, ToggleMods,
};
pub use toolbar::{AnnotationToolbar, AnnotationToolbarOutput};
pub use words::{AnnotateCommand, AnnotateIcons, AnnotateWords, PALETTE};
