//! The Fonts panel: the installed fonts, the Google Fonts catalogue, and installing and removing through the font
//! store.

mod i18n;

mod app;
mod browse;
mod details;
pub(crate) mod faces;
#[cfg(feature = "screen-testing")]
pub(crate) mod fixtures;
pub(crate) mod jobs;
pub(crate) mod samples;
mod sources;
mod specimen;

pub use app::{
    FontsCommand, FontsPanel, FontsPanelOutput, FontsPanelSettings, FontsPanelView, FontsPanelWords, Options, Preset, Tab, install_interface_fonts,
};
pub use browse::FontsBrowseWords;
pub use sources::{FontSources, FontSourcesOutput};
