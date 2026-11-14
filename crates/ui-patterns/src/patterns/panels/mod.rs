//! Panel and overlay patterns: `Panel`, `Inspector`, `HistoryPanel`, `ModifierKeysPanel`, `CommandPalette` and `Tour`.

mod words;

// Inspector.
mod inspector;

pub use inspector::Inspector;

// History and modifier keys.
mod history;
mod modifier_keys;

pub use history::{HistoryEvent, HistoryFooter, HistoryKey, HistoryPanel, HistoryPanelOutput, HistoryRow};
pub use modifier_keys::ModifierKeysPanel;

// CommandPalette and Tour.
mod palette;
mod tour;

pub use palette::{CommandPalette, PaletteOutput};
pub use tour::{Tour, TourEvent, TourOutput, TourStep};
pub use words::PanelWords;
