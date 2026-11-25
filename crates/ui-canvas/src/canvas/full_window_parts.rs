//! The parts of a full-window dialog's body: the side column, the zoom bar, the title text, the status line and the
//! canvas region, with the view they share. `FullWindowDialog` is the shell they fill.

pub(crate) mod canvas;
pub(crate) mod side;
pub(crate) mod status;
#[cfg(feature = "suite")]
pub(crate) mod stories;
pub(crate) mod title;
mod view;
pub(crate) mod zoom_bar;

pub use canvas::{FullWindowCanvas, FullWindowCanvasOutput, RasterOverlay};
pub use side::{FullWindowSide, FullWindowSideOutput, FullWindowSideStyle, SideFooter};
pub use status::{FullWindowStatus, FullWindowStatusOutput};
pub use title::{FullWindowTitle, FullWindowTitleOutput};
pub use view::FullWindowView;
pub use zoom_bar::{FullWindowZoomBar, FullWindowZoomBarOutput};
