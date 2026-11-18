#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Craft Font Manager as a library: the bundled font indexes and searching them, the installed
//! fonts by family, and the font store interface installs and removals go through.
//!
//! With the `egui` feature, [`FontsPanel`] is the whole interface as one panel that fills the `Ui`
//! it is given, with no window, menu or eframe of its own, so ArtCraft Launcher can show it as its
//! Fonts tab and the standalone app as its window. The host passes in the catalog, the store and
//! the index ([`Options`]), saves [`Settings`], and reads or sets the download consent through the
//! panel.

pub mod filter;
pub mod index;
pub mod installed;
pub mod store;

#[cfg(feature = "egui")]
mod panel;
#[cfg(feature = "egui")]
mod script_autonyms;

pub use craft_font_catalog;
pub use craft_font_catalog::{Catalog, FaceInfo, FontEnv, Source};
#[cfg(feature = "egui")]
pub use panel::{FontsPanel, Look, Options, Palette, Preset, Runner, Settings, Tab, install_interface_fonts};
