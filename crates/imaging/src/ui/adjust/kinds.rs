//! [`AdjustmentKind`]: the adjustments the Adjustments panel offers, with their command key, icon and title.

use craft_behavior::IconId;
use craft_ui::runtime::env::Label;

use super::AdjustWords;

/// One of the adjustments the Adjustments panel offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AdjustmentKind {
    /// Brightness/Contrast.
    BrightnessContrast,
    /// Levels.
    Levels,
    /// Curves.
    Curves,
    /// Exposure.
    Exposure,
    /// Vibrance.
    Vibrance,
    /// Hue/Saturation.
    HueSaturation,
    /// Color Balance.
    ColorBalance,
    /// Black & White.
    BlackWhite,
    /// Photo Filter.
    PhotoFilter,
    /// Channel Mixer.
    ChannelMixer,
    /// Invert.
    Invert,
    /// Posterize.
    Posterize,
    /// Threshold.
    Threshold,
    /// Gradient Map.
    GradientMap,
    /// Selective Color.
    SelectiveColor,
    /// Color Lookup.
    ColorLookup,
}

impl AdjustmentKind {
    /// Every kind, in the panel's order.
    pub const ALL: [AdjustmentKind; 16] = [
        AdjustmentKind::BrightnessContrast,
        AdjustmentKind::Levels,
        AdjustmentKind::Curves,
        AdjustmentKind::Exposure,
        AdjustmentKind::Vibrance,
        AdjustmentKind::HueSaturation,
        AdjustmentKind::ColorBalance,
        AdjustmentKind::BlackWhite,
        AdjustmentKind::PhotoFilter,
        AdjustmentKind::ChannelMixer,
        AdjustmentKind::Invert,
        AdjustmentKind::Posterize,
        AdjustmentKind::Threshold,
        AdjustmentKind::GradientMap,
        AdjustmentKind::SelectiveColor,
        AdjustmentKind::ColorLookup,
    ];

    /// The kind's key (`"brightnessContrast"`): the suffix of the app's `layer.newAdjustmentLayer.{key}` command.
    pub fn key(self) -> &'static str {
        match self {
            AdjustmentKind::BrightnessContrast => "brightnessContrast",
            AdjustmentKind::Levels => "levels",
            AdjustmentKind::Curves => "curves",
            AdjustmentKind::Exposure => "exposure",
            AdjustmentKind::Vibrance => "vibrance",
            AdjustmentKind::HueSaturation => "hueSaturation",
            AdjustmentKind::ColorBalance => "colorBalance",
            AdjustmentKind::BlackWhite => "blackWhite",
            AdjustmentKind::PhotoFilter => "photoFilter",
            AdjustmentKind::ChannelMixer => "channelMixer",
            AdjustmentKind::Invert => "invert",
            AdjustmentKind::Posterize => "posterize",
            AdjustmentKind::Threshold => "threshold",
            AdjustmentKind::GradientMap => "gradientMap",
            AdjustmentKind::SelectiveColor => "selectiveColor",
            AdjustmentKind::ColorLookup => "colorLookup",
        }
    }

    /// The kind's icon from the catalog.
    pub fn icon(self) -> IconId {
        IconId::new(match self {
            AdjustmentKind::BrightnessContrast => "sun",
            AdjustmentKind::Levels => "gauge",
            AdjustmentKind::Curves => "pen_tool",
            AdjustmentKind::Exposure => "scan",
            AdjustmentKind::Vibrance => "sparkles",
            AdjustmentKind::HueSaturation => "droplet",
            AdjustmentKind::ColorBalance => "blend",
            AdjustmentKind::BlackWhite => "contrast",
            AdjustmentKind::PhotoFilter => "target",
            AdjustmentKind::ChannelMixer => "adjustments",
            AdjustmentKind::Invert => "squares_subtract",
            AdjustmentKind::Posterize => "layers",
            AdjustmentKind::Threshold => "square",
            AdjustmentKind::GradientMap => "palette",
            AdjustmentKind::SelectiveColor => "swatch_book",
            AdjustmentKind::ColorLookup => "grid",
        })
    }

    /// The kind's title, from `words`.
    pub fn label(self, words: &AdjustWords) -> Label {
        match self {
            AdjustmentKind::BrightnessContrast => words.brightness_contrast.clone(),
            AdjustmentKind::Levels => words.levels.clone(),
            AdjustmentKind::Curves => words.curves.clone(),
            AdjustmentKind::Exposure => words.exposure.clone(),
            AdjustmentKind::Vibrance => words.vibrance.clone(),
            AdjustmentKind::HueSaturation => words.hue_saturation.clone(),
            AdjustmentKind::ColorBalance => words.color_balance.clone(),
            AdjustmentKind::BlackWhite => words.black_white.clone(),
            AdjustmentKind::PhotoFilter => words.photo_filter.clone(),
            AdjustmentKind::ChannelMixer => words.channel_mixer.clone(),
            AdjustmentKind::Invert => words.invert.clone(),
            AdjustmentKind::Posterize => words.posterize.clone(),
            AdjustmentKind::Threshold => words.threshold.clone(),
            AdjustmentKind::GradientMap => words.gradient_map.clone(),
            AdjustmentKind::SelectiveColor => words.selective_color.clone(),
            AdjustmentKind::ColorLookup => words.color_lookup.clone(),
        }
    }
}
