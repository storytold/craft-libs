//! The Color Range value and its sampling rules.

use craft_behavior::CursorBadge;
use craft_keymap::Mods;
use serde::{Deserialize, Serialize};

/// What the Select menu's Color Range picks colours by: a sampled colour, a colour family or a tonal range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ColorRangeSelect {
    /// Sampled colours. Serialises as the engine's `sampledColors`.
    #[default]
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
    SkinTones,
    /// Out-of-gamut colours.
    OutOfGamut,
}

/// The sampling tool: the plain eyedropper, Add to Sample or Subtract from Sample.
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
    /// The tool as `mods` change it: Shift adds, else Alt subtracts. The one place this rule lives, for a pick and
    /// for the canvas cursor's mark.
    #[must_use]
    pub fn with_mods(self, mods: Mods) -> SampleTool {
        if mods.shift {
            SampleTool::Add
        } else if mods.alt {
            SampleTool::Subtract
        } else {
            self
        }
    }

    /// The mark the canvas cursor shows for the tool.
    pub fn badge(self) -> Option<CursorBadge> {
        match self {
            SampleTool::Sample => None,
            SampleTool::Add => Some(CursorBadge::Add),
            SampleTool::Subtract => Some(CursorBadge::Subtract),
        }
    }
}

/// Whether a sample adds to the range or subtracts from it. Serialises as the engine's `"+"` and `"-"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SampleSign {
    /// An added sample.
    #[serde(rename = "+")]
    Add,
    /// A subtracted sample.
    #[serde(rename = "-")]
    Subtract,
}

/// The points the user sampled, in document pixels, and the order they were made in.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
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
    ///
    /// The eyedropper replaces every sample with this one; Add and Subtract append, so the order of the clicks is kept
    /// (the engine applies additions and subtractions one after another).
    pub fn pick(&mut self, at: [f64; 2], size: [u32; 2], tool: SampleTool, mods: Mods) {
        let cell = |v: f64, extent: u32| {
            let v = if v.is_finite() { v } else { 0.0 };
            v.floor().clamp(0.0, f64::from(extent.max(1)) - 1.0)
        };
        let at = [cell(at[0], size[0]), cell(at[1], size[1])];
        match tool.with_mods(mods) {
            SampleTool::Add => {
                self.add.push(at);
                self.order.push(SampleSign::Add);
            }
            SampleTool::Subtract => {
                self.subtract.push(at);
                self.order.push(SampleSign::Subtract);
            }
            SampleTool::Sample => {
                self.add = vec![at];
                self.subtract.clear();
                self.order = vec![SampleSign::Add];
            }
        }
    }

    /// The order of the samples, `None` while it does not match the two lists (then the engine takes the lists in
    /// their own order).
    pub fn order(&self) -> Option<&[SampleSign]> {
        let added = self.order.iter().filter(|sign| **sign == SampleSign::Add).count();
        let subtracted = self.order.len() - added;
        (added == self.add.len() && subtracted == self.subtract.len()).then_some(self.order.as_slice())
    }

    /// Drops every sample.
    pub fn clear(&mut self) {
        self.add.clear();
        self.subtract.clear();
        self.order.clear();
    }
}

/// The Color Range dialog's value: the command's parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ColorRangeValue {
    /// What the range picks by.
    pub select: ColorRangeSelect,
    /// Fuzziness of Sampled Colors and Skin Tones, 0..=200.
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
    pub localized: bool,
    /// The fuzziness of the tonal ranges (Highlights, Midtones, Shadows), 0..=100 percent.
    pub tone_fuzziness: f64,
    /// The Shadows level, 0..=255.
    pub shadows_level: f64,
    /// The Highlights level, 0..=255.
    pub highlights_level: f64,
}

impl Default for ColorRangeValue {
    /// Photoshop's opening values: Sampled Colors, Fuzziness 40, Range 100, no samples.
    fn default() -> Self {
        ColorRangeValue {
            select: ColorRangeSelect::Sampled,
            fuzziness: 40.0,
            range: 100.0,
            midtones_low: 105.0,
            midtones_high: 150.0,
            samples: ColorSamples::default(),
            invert: false,
            localized: false,
            tone_fuzziness: 20.0,
            shadows_level: 65.0,
            highlights_level: 190.0,
        }
    }
}

impl ColorRangeValue {
    /// Which controls the value's `select` shows or enables.
    pub fn controls(&self) -> RangeControls {
        let sampled = self.select == ColorRangeSelect::Sampled;
        let tonal = matches!(self.select, ColorRangeSelect::Highlights | ColorRangeSelect::Midtones | ColorRangeSelect::Shadows);
        RangeControls {
            fuzziness: sampled || tonal || self.select == ColorRangeSelect::SkinTones,
            sampling: sampled,
            range: sampled && self.localized,
            tonal,
        }
    }
}

/// Which controls Color Range shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RangeControls {
    /// The Fuzziness slider is enabled.
    pub fuzziness: bool,
    /// The sampling tools are enabled.
    pub sampling: bool,
    /// The Range slider is shown.
    pub range: bool,
    /// The tonal sliders (Shadows, Highlights, Midtones bounds) are shown.
    pub tonal: bool,
}
