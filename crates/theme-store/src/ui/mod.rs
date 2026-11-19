//! The Preferences appearance page over the suite selection: [`AppearancePage`] and its [`ThemeChip`]s.

mod chip;
mod page;
#[cfg(feature = "suite")]
mod stories;

#[cfg(feature = "screen-testing")]
#[cfg(feature = "suite")]
craft_ui::testing_registered!();

pub use chip::{ThemeChip, ThemeChipOutput, ThemeChipStyle};
pub use page::{AppearancePage, AppearancePageOutput, AppearanceWords};
