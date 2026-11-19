//! Panel and overlay patterns: `Panel`, `Inspector`, `HistoryPanel`, `ModifierKeysPanel`, `CommandPalette` and `Tour`.

mod words;

// Inspector.
pub(crate) mod inspector;

pub use inspector::Inspector;

// History and modifier keys.
pub(crate) mod history;
pub(crate) mod modifier_keys;

pub use history::{HistoryEvent, HistoryFooter, HistoryKey, HistoryPanel, HistoryPanelOutput, HistoryRow};
pub use modifier_keys::ModifierKeysPanel;

// CommandPalette and Tour.
pub(crate) mod palette;
pub(crate) mod tour;

pub use palette::{CommandPalette, PaletteOutput};
pub use tour::{Tour, TourEvent, TourOutput, TourStep};
pub use words::PanelWords;
