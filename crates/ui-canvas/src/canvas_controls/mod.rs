//! The canvas controls: leaves a `Canvas` body shows (`#[component(canvas_control)]`), and the contextual task bar
//! (`TaskBar`), a pattern a canvas body shows over its document.

mod words;

// Guides and smart guides.
pub(crate) mod guides;
pub(crate) mod smart_guides;
pub(crate) use smart_guides::paint_lines;

pub use guides::{GuideCommands, GuideStyle, Guides, GuidesOutput, reading};
pub use smart_guides::{Measure, SmartGuides, SmartGuidesStyle, SmartLine};
pub use words::CanvasControlWords;

// Selection outlines, grids and previews (canvas-controls-b).
mod bands;
pub(crate) mod grid;
pub(crate) mod line_preview;
pub(crate) mod marquee;
pub(crate) mod outline;
pub(crate) mod pixel_grid;
pub(crate) mod task_bar;

pub use bands::{range_bands, rect_around};
pub use grid::{CanvasGrid, GridAxis, GridCrowding, GridLine, GridLines, GridStyle, MAX_LINES};
pub use line_preview::{LineGuideStyle, StraightLinePreview};
pub use marquee::{Marquee, MarqueeLook, MarqueeShape, MarqueeStyle, Run};
pub use outline::{HoverMask, HoverOutline, HoverShape, OutlineStyle, SelectionOutline, SelectionOutlineLook, path_runs};
pub use pixel_grid::{GridSegment, PixelGrid, PixelGridStyle};
pub use task_bar::{TaskBar, TaskBarCx, TaskBarDismiss, TaskBarGrip, TaskBarHang, TaskBarKind, TaskBarOutput, TaskBarPlace};
// End of canvas-controls-b.

// Brush overlays.
pub mod brush;
