//! The theme workspace: the panels that explain a theme and its appearance. The host holds one [`ThemeWorkspace`],
//! refreshes it for the theme on show and passes what it holds to the panels. The panels edit the workspace's pick
//! and app sizes through bindings, and the Problems view reports the pick it makes.

#![warn(missing_docs)]

pub(crate) mod browser;
mod chip;
pub(crate) mod detail;
pub(crate) mod layer;
pub(crate) mod problems;
mod rows;
#[cfg(feature = "suite")]
pub(crate) mod stories;
mod subject;
mod words;

pub use browser::TokenBrowser;
pub use detail::TokenDetail;
pub use layer::{AppLayerEditor, parse_sizes};
pub use problems::{Problems, ProblemsOutput};
pub use rows::Row;
pub use subject::{Pick, ThemeSubject, ThemeWorkspace};
pub use words::summary;

/// The stories this crate registers for the integration binary.
#[cfg(feature = "screen-testing")]
pub mod testing {

    #[cfg(feature = "suite")]
    craft_ui::registry!();
}

#[cfg(test)]
fn main() {
    craft_ui::registry!();
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::Registry::of(&[register]));
}
