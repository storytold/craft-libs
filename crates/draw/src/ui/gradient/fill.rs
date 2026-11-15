//! [`GradientFill`]: the gradient fill value of shapes and Gradient Fill layers.

use craft_behavior::IconId;
use craft_color::ui::gradient::GradientStops;
use craft_ui::runtime::env::Label;
use serde::{Deserialize, Serialize};

use super::fill_editor::GradientFillWords;

/// How a gradient runs from its line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum GradientStyle {
    /// Along the line.
    #[default]
    Linear,
    /// Out from the start in circles.
    Radial,
    /// Round the start.
    Angle,
    /// Along the line, mirrored about the start.
    Reflected,
    /// Out from the start in diamonds.
    Diamond,
}

impl GradientStyle {
    /// Every style, in the style select's order.
    pub const ALL: [GradientStyle; 5] =
        [GradientStyle::Linear, GradientStyle::Radial, GradientStyle::Angle, GradientStyle::Reflected, GradientStyle::Diamond];

    /// The style's name.
    pub fn label(self, words: &GradientFillWords) -> Label {
        match self {
            GradientStyle::Linear => words.linear.clone().into(),
            GradientStyle::Radial => words.radial.clone().into(),
            GradientStyle::Angle => words.angle_style.clone().into(),
            GradientStyle::Reflected => words.reflected.clone().into(),
            GradientStyle::Diamond => words.diamond.clone().into(),
        }
    }

    /// The style's icon: `gradient_linear`, `gradient_radial`, `gradient_angle`, `gradient_reflected`
    /// or `gradient_diamond`.
    pub fn icon(self) -> IconId {
        IconId::new(match self {
            GradientStyle::Linear => "gradient_linear",
            GradientStyle::Radial => "gradient_radial",
            GradientStyle::Angle => "gradient_angle",
            GradientStyle::Reflected => "gradient_reflected",
            GradientStyle::Diamond => "gradient_diamond",
        })
    }
}

/// The gradient fill value of shapes and Gradient Fill layers.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct GradientFill {
    /// The colour and opacity stops.
    pub stops: GradientStops,
    /// How the gradient runs.
    pub style: GradientStyle,
    /// The angle in degrees.
    pub angle: f64,
    /// The scale, 1 for 100 %.
    pub scale: f64,
    /// Whether the stops run backwards.
    pub reverse: bool,
    /// Whether the gradient is dithered.
    pub dither: bool,
    /// Whether the gradient aligns with its layer.
    pub align: bool,
    /// The centre offset as a fraction of the frame (PSD `Ofst`).
    pub offset: kurbo::Vec2,
}

impl GradientFill {
    /// A linear fill of `stops`: angle 90, scale 1, no reverse, no dither, aligned, no offset.
    pub fn new(stops: GradientStops) -> Self {
        GradientFill {
            stops,
            style: GradientStyle::Linear,
            angle: 90.0,
            scale: 1.0,
            reverse: false,
            dither: false,
            align: true,
            offset: kurbo::Vec2::ZERO,
        }
    }
}
