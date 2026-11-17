//! The gradient parts that sit on the canvas and in panels: [`GradientPalette`], a picker of preset gradients;
//! [`GradientSwatch`], a well that shows a gradient; [`GradientMapEditor`], Photoshop's Gradient Map adjustment
//! ([`map`]); [`GradientOverlay`] and [`gradient_line`], the on-canvas gradient line; and [`GradientFillEditor`], the
//! fill and its style.
//!
//! The stop model, the stop rules, the stop bar `craft_color_ui::ui::gradient::GradientEditor` and the ramp paint
//! `craft_color_ui::ui::gradient::GradientRamp` are craft-color-ui's: the parts here build on them and import them from
//! there.

mod fill;
mod fill_editor;
pub mod gradient_line;
pub mod gradient_overlay;
pub mod map;
mod palette;
mod swatch;

pub use fill::{GradientFill, GradientStyle};
pub use fill_editor::{GradientFillEditor, GradientFillEditorOutput, GradientFillWords};
pub use gradient_line::{GradientLine, LineGradient, LinePart};
pub use gradient_overlay::{GradientLineWords, GradientOverlay, GradientOverlayOutput, GradientOverlayStyle};
pub use map::{GradientMap, GradientMapEditor, GradientMapOutput};
pub use palette::{GradientGroup, GradientItem, GradientPalette, GradientPaletteOutput, GradientRef};
pub use swatch::{GradientSwatch, GradientSwatchOutput};
