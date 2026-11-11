//! The canvas's data types that patterns read and write without the canvas: guides and the navigator's panel
//! options. `canvas` re-exports them.

mod guide;
mod navigator_options;

pub use guide::{Guide, GuideColor, GuideDrag, GuideKey, GuideLine};
pub use navigator_options::{NavigatorOptions, NavigatorSettings, ProxyColor};
