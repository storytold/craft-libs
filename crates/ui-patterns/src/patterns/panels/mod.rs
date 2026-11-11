//! Panel and overlay patterns: `Panel`, `Inspector`, `CommandPalette` and `Tour`.

mod words;

// Inspector.
mod inspector;

pub use inspector::Inspector;

// CommandPalette and Tour.
mod palette;
mod tour;

pub use palette::{CommandPalette, PaletteOutput};
pub use tour::{Tour, TourEvent, TourOutput, TourStep};
pub use words::PanelWords;
