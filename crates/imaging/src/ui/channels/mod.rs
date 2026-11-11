//! The Channels panel over plain rows: thumbnails, eyes, selection, rename, drag, the footer and the menus. It
//! reports events and holds no document (`ChannelsPanel`, `ChannelRow`, `ChannelsEvent`).

mod footer;
mod labels;
mod menu;
mod model;
mod panel;
mod rows;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;

pub use footer::ChannelsFooterButton;
pub use labels::ChannelsWords;
pub use menu::{ChannelsCommands, panel_rows};
pub use model::{ChannelKind, ChannelRow, ChannelsEvent, ChannelsOutput, ColorIndicates, OVERLAY_OPACITIES, OverlayPreset, targeted_row};
pub use panel::ChannelsPanel;
