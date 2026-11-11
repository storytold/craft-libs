//! The parts of a full-window dialog's body: the side column, the zoom bar, the title text, the status line and the
//! canvas region, with the view they share. `FullWindowDialog` is the shell they fill.

mod canvas;
mod side;
mod status;
#[cfg(feature = "testing")]
mod stories;
mod title;
mod view;
mod zoom_bar;

pub use canvas::{FullWindowCanvas, FullWindowCanvasOutput, RasterOverlay};
pub use side::{FullWindowSide, FullWindowSideOutput, FullWindowSideStyle, SideFooter};
pub use status::{FullWindowStatus, FullWindowStatusOutput};
pub use title::{FullWindowTitle, FullWindowTitleOutput};
pub use view::FullWindowView;
pub use zoom_bar::{FullWindowZoomBar, FullWindowZoomBarOutput};
