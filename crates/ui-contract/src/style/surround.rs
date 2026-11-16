//! The canvas surround: the fill around a document and the theme's pattern over it.

use super::UiTokens;
use super::paint::color as rgba_color;
use egui::Color32;

/// How the surround around a canvas looks: its fill and the pattern drawn over it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SurroundLook {
    /// The surround's fill.
    pub fill: Color32,
    /// The pattern's colour; `None` draws no pattern.
    pub pattern: Option<Color32>,
}

impl SurroundLook {
    /// The surround of the theme `tokens` resolve from: the `surface.canvas` fill, and `surface.canvas_pattern` as
    /// the pattern where the theme sets one. A theme whose pattern equals its fill has none, so no texture is made
    /// that would paint nothing (#4542).
    pub fn of(tokens: &UiTokens) -> Self {
        let surface = &tokens.color.surface;
        let pattern = (surface.canvas_pattern != surface.canvas).then(|| rgba_color(surface.canvas_pattern));
        Self { fill: rgba_color(surface.canvas), pattern }
    }

    /// The same look with the pattern kept when `pattern_on`, else dropped. Callers pass the app's preference,
    /// `EnvView::canvas_pattern`, which a style cannot read.
    #[must_use]
    pub fn shown(self, pattern_on: bool) -> Self {
        Self { pattern: self.pattern.filter(|_| pattern_on), ..self }
    }
}
