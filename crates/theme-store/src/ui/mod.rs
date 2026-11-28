//! The Preferences appearance page over the suite selection: [`AppearancePage`] and its [`ThemeChip`]s.

pub(crate) mod chip;
pub(crate) mod page;
#[cfg(feature = "suite")]
pub(crate) mod stories;

#[cfg(feature = "suite")]
craft_ui::registry!();

pub use chip::{ThemeChip, ThemeChipOutput, ThemeChipStyle};
pub use page::{AppearancePage, AppearancePageOutput, AppearanceWords};
