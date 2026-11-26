//! The Fonts panel: the installed fonts, the Google Fonts catalogue, and installing and removing through the font
//! store.

mod i18n;

pub(crate) mod app;
pub(crate) mod browse;
pub(crate) mod details;
pub(crate) mod faces;
#[cfg(feature = "screen-testing")]
pub(crate) mod fixtures;
pub(crate) mod jobs;
pub(crate) mod legal;
pub(crate) mod samples;
pub(crate) mod sources;
pub(crate) mod specimen;
pub(crate) mod subsets;

pub use app::{
    FontsCommand, FontsPanel, FontsPanelOutput, FontsPanelSettings, FontsPanelView, FontsPanelWords, Options, Preset, Tab, install_interface_fonts,
};
pub use browse::FontsBrowseWords;
pub use i18n::CATALOGS;
pub use sources::{FontSources, FontSourcesOutput};
