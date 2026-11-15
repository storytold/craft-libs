//! The canvas controls: leaves a `Canvas` body shows (`#[component(canvas_control)]`), and the contextual task bar
//! (`TaskBar`), a pattern a canvas body shows over its document.

mod words;

// Guides and smart guides.
mod guides;
mod smart_guides;
pub(crate) use smart_guides::paint_lines;

pub use guides::{GuideCommands, GuideStyle, Guides, GuidesOutput, reading};
pub use smart_guides::{Measure, SmartGuides, SmartGuidesStyle, SmartLine};
pub use words::CanvasControlWords;

// Selection outlines, grids and previews (canvas-controls-b).
mod bands;
mod grid;
mod line_preview;
mod marquee;
mod outline;
mod pixel_grid;
mod task_bar;

pub use bands::{range_bands, rect_around};
pub use grid::{CanvasGrid, GridAxis, GridLine, GridLines, GridStyle, MAX_LINES};
pub use line_preview::{LineGuideStyle, StraightLinePreview};
pub use marquee::{Marquee, MarqueeLook, MarqueeShape, MarqueeStyle, Run};
pub use outline::{HoverMask, HoverOutline, HoverShape, OutlineStyle, SelectionOutline, SelectionOutlineLook, path_runs};
pub use pixel_grid::{GridSegment, PixelGrid, PixelGridStyle};
pub use task_bar::{TaskBar, TaskBarCx, TaskBarOutput, TaskBarPlace};
// End of canvas-controls-b.

// Brush overlays.
pub mod brush;
