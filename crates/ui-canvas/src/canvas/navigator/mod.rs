//! The Navigator panel (Photoshop's Window, Navigator): the pure geometry of the preview and its proxy rectangle, the
//! zoom row, Panel Options as data, and the panel menu as commands.
//!
//! | Part | Names |
//! |---|---|
//! | Geometry, for tests and for apps with their own preview | [`preview_rect`], [`proxy_quad`], [`zoom_to_document_rect`], [`view_center_for`] |
//! | The preview box | [`Navigator`], [`NavigatorPreview`], [`NavigatorPaint`], [`NavigatorOutput`], [`NavigatorStyle`] |
//! | The zoom row | [`NavigatorZoomRow`], [`NavigatorZoomRowOutput`] |
//! | Panel Options | [`NavigatorSettings`], [`NavigatorOptions`], [`ProxyColor`]; the dialog is `patterns::form_dialogs::NavigatorOptionsDialog` |
//! | The panel menu | [`NavigatorCommands`] |

mod menu;
mod preview;
mod proxy;
#[cfg(feature = "testing")]
mod stories;
mod zoom_row;

pub use menu::NavigatorCommands;
pub use preview::{Navigator, NavigatorOutput, NavigatorPaint, NavigatorPreview, NavigatorStyle};
pub use proxy::{preview_rect, proxy_quad, view_center_for, zoom_to_document_rect};
pub use zoom_row::{NavigatorZoomRow, NavigatorZoomRowOutput};
