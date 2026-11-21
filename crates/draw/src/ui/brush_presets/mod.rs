//! The Brush Preset picker.
//!
//! - [`BrushPresetPicker`]: a closed field that opens a popover with Size, Hardness, the Brush Settings and New
//!   Brush Preset buttons, the gear menu and the preset browser; it reports [`BrushPick`]s and the size and hardness
//!   edits.
//! - [`BrushesPanel`]: the Brushes panel, a Size field over the preset browser with Import in its footer.
//! - [`brush_row`] and [`BrushRowPaint`]: the preset rows it shows, picture chosen by [`BrushCardParts`].
//! - [`BrushPickerSettings`]: the picker's persisted view (parts and scale).
//!
//! The app paints the tip and stroke images from its engine; the library never sees an engine type. Presets are
//! keyed by the app's key type (PhotoCraft: `PresetKey<String>`, by name).

pub(crate) mod panel;
pub(crate) mod picker;
mod rows;
mod settings;
#[cfg(feature = "suite")]
pub(crate) mod stories;
mod words;

pub use panel::{BrushesPanel, BrushesPanelOutput};
pub use picker::{BrushPick, BrushPickerState, BrushPickerValues, BrushPresetPicker, BrushPresetPickerOutput, GearRows, GearSection};
pub use rows::{BrushRowPaint, brush_row, tip_thumb_scale};
pub use settings::{BrushCardPart, BrushCardParts, BrushPickerSettings};
pub use words::BrushPresetWords;
