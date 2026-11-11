//! The gradient stop bar's parts: [`GradientStops`], the value, with the stop rules in [`stops`];
//! [`GradientRamp`], the paint; [`GradientSwatch`], a well that shows a gradient; and
//! [`GradientEditor`], the stop bar that edits the stops.
//!
//! Colour stops and opacity stops live in two rows ([`StopRows::Split`], PhotoCraft) or in one,
//! the opacity being the colour's alpha ([`StopRows::Combined`], DesignCraft and VectorCraft).
//! The rules are pure functions of the stops, so an on-canvas gradient line and the bar can't
//! disagree. A ramp and a swatch sample through a [`Sampler`]; the default mixes straight lines
//! in encoded sRGB, each segment bent so its midpoint gives the half mix.
//!
//! [`map`]: Photoshop's Gradient Map adjustment ([`GradientMapEditor`]) over the bar.

// draw-gradient-a: the stop bar, its stops, ramp and swatch.
mod editor;
pub mod gradient_line;
pub mod gradient_overlay;
pub mod map;
mod palette;
mod ramp;
pub mod stops;
mod swatch;
mod value;
mod words;

pub(crate) use editor::marker_texts;
pub use editor::{GradientEditor, GradientEditorOutput, StopDrop};
pub use palette::{GradientGroup, GradientItem, GradientPalette, GradientPaletteOutput, GradientRef};
pub use ramp::GradientRamp;
pub use stops::{Sampler, StopRules, default_sampler};
pub use swatch::{GradientSwatch, GradientSwatchOutput};
pub use value::{ColorStop, GradientStops, OpacityStop, StopColor, StopRef, StopRow, StopRows};
pub use words::GradientWords;

pub use map::{GradientMap, GradientMapEditor, GradientMapOutput};

pub use gradient_line::{GradientExtent, GradientLine, LineGradient, LinePart};
pub use gradient_overlay::{GradientLineWords, GradientOverlay, GradientOverlayOutput, GradientOverlayStyle};
