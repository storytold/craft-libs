//! The Color Range value and its sampling rules.

use craft_behavior::CursorBadge;
use craft_keymap::Mods;
use serde::{Deserialize, Serialize};

/// What the Select menu's Color Range picks colours by: a sampled colour, a colour family or a tonal range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ColorRangeSelect {
    /// Sampled colours.
    #[default]
    // P3(dialogs-w9): dl.K54 the engine's name
    #[serde(rename = "sampledColors")]
    Sampled,
    /// Reds.
    Reds,
    /// Yellows.
    Yellows,
    /// Greens.
    Greens,
    /// Cyans.
    Cyans,
    /// Blues.
    Blues,
    /// Magentas.
    Magentas,
    /// Highlights.
    Highlights,
    /// Midtones.
    Midtones,
    /// Shadows.
    Shadows,
    /// Skin tones.
    // P3(dialogs-w9): dl.K54 `SkinTones`
    SkinTones,
    /// Out-of-gamut colours.
    // P3(dialogs-w9): dl.K54 `OutOfGamut`
    OutOfGamut,
}

/// The sampling tool: the plain eyedropper, Add to Sample or Subtract from Sample.
// P3(dialogs-w9): dl.K54 serde for the `tool` json key
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SampleTool {
    /// The eyedropper: replaces the samples.
    #[default]
    Sample,
    /// Add to Sample.
    Add,
    /// Subtract from Sample.
    Subtract,
}

impl SampleTool {
    /// The tool as `mods` change it: Shift adds, else Alt subtracts.
    #[must_use]
    pub fn with_mods(self, mods: Mods) -> SampleTool {
        let _ = (self, mods);
        todo!("dl.K54")
    }

    /// The mark the canvas cursor shows for the tool.
    pub fn badge(self) -> Option<CursorBadge> {
        let _ = self;
        todo!("dl.K54")
    }
}

/// Whether a sample adds to the range or subtracts from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SampleSign {
    /// An added sample.
    Add,
    /// A subtracted sample.
    Subtract,
}

/// The points the user sampled, in document pixels, and the order they were made in.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ColorSamples {
    /// The added samples.
    pub add: Vec<[f64; 2]>,
    /// The subtracted samples.
    pub subtract: Vec<[f64; 2]>,
    /// The order the samples were made in.
    pub order: Vec<SampleSign>,
}

impl ColorSamples {
    /// Takes a sample at `at` in a document of `size`: floored and clamped to the document, by `tool` as `mods` change
    /// it. Also Replace Color's sampling rule.
    pub fn pick(&mut self, at: [f64; 2], size: [u32; 2], tool: SampleTool, mods: Mods) {
        let _ = (at, size, tool, mods);
        todo!("dl.K54")
    }

    /// The order of the samples, `None` while they are all of one sign.
    pub fn order(&self) -> Option<&[SampleSign]> {
        todo!("dl.K54")
    }

    /// Drops every sample.
    pub fn clear(&mut self) {
        todo!("dl.K54")
    }
}

/// The Color Range dialog's value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ColorRangeValue {
    /// What the range picks by.
    pub select: ColorRangeSelect,
    /// Fuzziness, 0..=200.
    pub fuzziness: f64,
    /// Range, 0..=100 percent.
    pub range: f64,
    /// The lower bound of Midtones.
    pub midtones_low: f64,
    /// The upper bound of Midtones.
    pub midtones_high: f64,
    /// The sampled points.
    pub samples: ColorSamples,
    /// Whether the selection is inverted.
    pub invert: bool,
    /// Localized Color Clusters.
    // P3(dialogs-w9): dl.K54 `localized`
    pub localized: bool,
    /// The fuzziness of the tonal ranges (Highlights, Midtones, Shadows, Skin Tones).
    // P3(dialogs-w9): dl.K54 `tone_fuzziness`
    pub tone_fuzziness: f64,
    /// The Shadows level.
    // P3(dialogs-w9): dl.K54 `shadows_level`
    pub shadows_level: f64,
    /// The Highlights level.
    // P3(dialogs-w9): dl.K54 `highlights_level`
    pub highlights_level: f64,
}

impl Default for ColorRangeValue {
    /// Production's opening values (pc color_range_ui.rs:94-110).
    // P3(dialogs-w9): dl.K54 `impl Default`
    fn default() -> Self {
        todo!("dl.K54")
    }
}

impl ColorRangeValue {
    /// Which controls the value's `select` shows.
    pub fn controls(&self) -> RangeControls {
        todo!("dl.K54")
    }
}

/// Which controls Color Range shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RangeControls {
    /// The Fuzziness slider.
    pub fuzziness: bool,
    /// The sampling tools.
    pub sampling: bool,
    /// The Range slider.
    pub range: bool,
    /// The Midtones bounds.
    pub tonal: bool,
}
