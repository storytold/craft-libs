//! One pattern fill editor ([`PatternFillEditor`]) for Photoshop-style bitmap patterns (picker, scale, angle, Link
//! with Layer, Snap to Origin) and PowerPoint-style two-colour presets (swatch grid, foreground, background), in
//! Photoshop's Pattern Fill dialog ([`PatternFillDialog`]) or inline (a paint chip's Pattern editor, a Format pane).

use craft_theme::Srgba;

mod dialog;
mod editor;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;
mod words;

pub use dialog::PatternFillDialog;
pub use editor::{PatternFillEditor, PatternFillOutput, SwatchPainter};
pub use words::PatternWords;

/// A pattern the editor offers: the app's pattern library (PhotoCraft's Patterns, a bitmap each)
/// or its preset patterns (PowerPoint's "pct25", "dkHorz", …). The app paints each swatch's tile
/// ([`PatternFillEditor::thumbnails`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PatternSwatch {
    /// The app's id, which [`PatternFillOptions::pattern`] holds.
    pub id: String,
    /// The name in the tooltip and for a screen reader ("Bubbles", "25 %").
    pub name: String,
}

impl PatternSwatch {
    /// A swatch of the app's pattern `id`, named `name`.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        PatternSwatch { id: id.into(), name: name.into() }
    }
}

/// A preset pattern's two colours (PowerPoint's Foreground and Background): present means the
/// editor is in preset mode.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PatternColors {
    /// The pattern's foreground colour.
    pub foreground: Srgba,
    /// The pattern's background colour.
    pub background: Srgba,
}

/// A pattern fill's values.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PatternFillOptions {
    /// The pattern's id ([`PatternSwatch::id`]).
    pub pattern: String,
    /// Bitmap mode: the scale in percent, [`SCALE`](Self::SCALE).
    pub scale: f64,
    /// Bitmap mode: the angle in degrees, −180 to 180.
    pub angle: f64,
    /// Bitmap mode: Link with Layer (the pattern moves with the layer).
    pub link: bool,
    /// Bitmap mode: the pattern's origin offset in pixels; Snap to Origin sets (0, 0).
    pub phase: [f64; 2],
    /// Preset mode: the two colours. `None` is bitmap mode.
    pub colors: Option<PatternColors>,
}

impl PatternColors {
    /// The two colours of a preset pattern.
    pub fn new(foreground: Srgba, background: Srgba) -> Self {
        PatternColors { foreground, background }
    }
}

impl PatternFillOptions {
    /// The scale range in percent (PhotoCraft's `pattern_fill_cmds::SCALE_RANGE`).
    pub const SCALE: std::ops::RangeInclusive<f64> = 1.0..=1000.0;

    /// A bitmap pattern at 100 %, 0°, linked, at the origin (Photoshop's New Fill Layer ›
    /// Pattern…).
    pub fn bitmap(pattern: impl Into<String>) -> Self {
        PatternFillOptions { pattern: pattern.into(), scale: 100.0, angle: 0.0, link: true, phase: [0.0, 0.0], colors: None }
    }

    /// A preset pattern with its two colours.
    pub fn preset(pattern: impl Into<String>, colors: PatternColors) -> Self {
        PatternFillOptions { colors: Some(colors), ..PatternFillOptions::bitmap(pattern) }
    }
}

impl Default for PatternFillOptions {
    /// A bitmap pattern with no pattern picked, for a form that starts from nothing.
    fn default() -> Self {
        PatternFillOptions::bitmap("")
    }
}
