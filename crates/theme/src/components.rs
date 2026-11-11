//! The component tokens: the slider thumb's two lengths. Every other component computes its look
//! from the semantic tokens in `craft-ui`. A theme file may set the thumb lengths in its
//! `components` map ([`ComponentField`]).

use super::parts::BySize;
use super::sizes::Preset;
use super::values;

/// Names one component token. Theme files address it by [`ComponentField::path`], e.g.
/// `"slider.thumb_width"`. Each sets the standard size; the small size keeps its step under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ComponentField {
    /// `slider.thumb_width`: the width of a slider's thumb.
    SliderThumbWidth,
    /// `slider.thumb_height`: the height of a slider's thumb.
    SliderThumbHeight,
}

impl ComponentField {
    /// Every component token.
    pub const ALL: &'static [ComponentField] = &[ComponentField::SliderThumbWidth, ComponentField::SliderThumbHeight];

    /// The dotted path, e.g. `"slider.thumb_width"`.
    pub fn path(self) -> String {
        match self {
            ComponentField::SliderThumbWidth => "slider.thumb_width".to_owned(),
            ComponentField::SliderThumbHeight => "slider.thumb_height".to_owned(),
        }
    }

    /// The field whose [`path`](ComponentField::path) is `path`.
    pub fn from_path(path: &str) -> Option<ComponentField> {
        ComponentField::ALL.iter().copied().find(|f| f.path() == path)
    }
}

/// Slider tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderTokens {
    /// Width of the thumb: its height unless the theme sets another shape.
    pub thumb_width: BySize<f32>,
    /// Height of the thumb, and its diameter while round: the icon size less
    /// `values::SLIDER_THUMB_INSET`.
    pub thumb_height: BySize<f32>,
}

impl SliderTokens {
    /// `sizes` with its standard size set to `md`, the small size keeping its step under it.
    fn stepped(sizes: BySize<f32>, md: f32) -> BySize<f32> {
        BySize { sm: md - (sizes.md - sizes.sm), md }
    }
}

/// Every component token.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComponentTokens {
    /// Sliders.
    pub slider: SliderTokens,
}

impl ComponentTokens {
    /// The defaults from the Size preset.
    pub(crate) fn defaults(preset: &Preset) -> ComponentTokens {
        let icon = preset.size.icon;
        let thumb = BySize { sm: icon.sm - values::SLIDER_THUMB_INSET.0, md: icon.md - values::SLIDER_THUMB_INSET.1 };
        ComponentTokens { slider: SliderTokens { thumb_width: thumb, thumb_height: thumb } }
    }

    /// The value of one field: the standard size.
    pub fn get(&self, field: ComponentField) -> f32 {
        match field {
            ComponentField::SliderThumbWidth => self.slider.thumb_width.md,
            ComponentField::SliderThumbHeight => self.slider.thumb_height.md,
        }
    }

    /// Sets one field's standard size.
    pub fn set(&mut self, field: ComponentField, value: f32) {
        match field {
            ComponentField::SliderThumbWidth => self.slider.thumb_width = SliderTokens::stepped(self.slider.thumb_width, value),
            ComponentField::SliderThumbHeight => self.slider.thumb_height = SliderTokens::stepped(self.slider.thumb_height, value),
        }
    }
}
