//! Browsing for Craft apps: folders and volumes, linked files, file thumbnails and the widgets
//! that show them.
//!
//! - [`places`]: the roots of a folder tree and lazy, cached folder listings.
//! - [`links`]: link statuses kept current as files change.
//! - [`thumbs`]: file thumbnails decoded by the app, as jobs.
//! - `ui` (feature `ui`): the tile grid, media bin, places tree, compare tile and relink.
//!
//! Every listing, status check, search and decode runs as a job on the host's queue; nothing
//! here spawns a thread.

pub mod links;
pub mod places;
pub mod thumbs;
#[cfg(feature = "ui")]
pub mod ui;

#[cfg(all(test, feature = "screen-testing"))]
extern crate self as craft_browse;
#[cfg(all(test, feature = "screen-testing"))]
#[path = "../tests/it/main.rs"]
mod integration;

#[cfg(test)]
fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::registered_in(env!("CARGO_PKG_NAME")));
}
