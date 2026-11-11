//! The theme workspace: the panels that explain a theme and its appearance. The host holds one [`ThemeWorkspace`],
//! refreshes it for the theme on show and passes what it holds to the panels. The panels edit the workspace's pick
//! and app sizes through bindings, and the Problems view reports the pick it makes.

mod browser;
mod chip;
mod detail;
mod layer;
mod problems;
mod rows;
mod stories;
mod subject;
mod words;

pub use browser::TokenBrowser;
pub use detail::TokenDetail;
pub use layer::{AppLayerEditor, parse_sizes};
pub use problems::{Problems, ProblemsOutput};
pub use rows::Row;
pub use subject::{Pick, ThemeSubject, ThemeWorkspace};
pub use words::summary;
