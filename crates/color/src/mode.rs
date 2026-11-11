//! [`AreaMode`]: what a colour area and its strip show.

#[cfg(feature = "ui")]
use craft_theme::ModelValues;
use craft_theme::{ColorModel, HsbModel, LabModel, OklchModel, RgbModel};

/// What the area and the strip show: the strip's component and the area's two, across (growing to
/// the right) and down (growing upwards). The nine modes of PhotoCraft's Color Picker, with
/// Photoshop's axes, and three for OKLCH laid out as the HSB ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AreaMode {
    /// Hue in the strip; saturation across, brightness down (HSB).
    #[default]
    #[serde(rename = "h")]
    Hue,
    /// Saturation in the strip; hue across, brightness down (HSB).
    #[serde(rename = "s")]
    Saturation,
    /// Brightness in the strip; hue across, saturation down (HSB).
    #[serde(rename = "v")]
    Brightness,
    /// Red in the strip; blue across, green down (RGB).
    #[serde(rename = "r")]
    Red,
    /// Green in the strip; blue across, red down (RGB).
    #[serde(rename = "g")]
    Green,
    /// Blue in the strip; red across, green down (RGB).
    #[serde(rename = "b")]
    Blue,
    /// Lab lightness in the strip; a across, b down.
    #[serde(rename = "lab_l")]
    LabL,
    /// Lab a in the strip; b across, lightness down.
    #[serde(rename = "lab_a")]
    LabA,
    /// Lab b in the strip; a across, lightness down.
    #[serde(rename = "lab_b")]
    LabB,
    /// OKLCH lightness in the strip; hue across, chroma down.
    #[serde(rename = "oklch_l")]
    OklchL,
    /// OKLCH chroma in the strip; hue across, lightness down.
    #[serde(rename = "oklch_c")]
    OklchC,
    /// OKLCH hue in the strip; chroma across, lightness down.
    #[serde(rename = "oklch_h")]
    OklchH,
}

impl AreaMode {
    /// PhotoCraft's nine modes, in its radio order: H, S, B, R, G, B, L, a, b.
    pub const PHOTOSHOP: [AreaMode; 9] = [
        AreaMode::Hue,
        AreaMode::Saturation,
        AreaMode::Brightness,
        AreaMode::Red,
        AreaMode::Green,
        AreaMode::Blue,
        AreaMode::LabL,
        AreaMode::LabA,
        AreaMode::LabB,
    ];

    /// The OKLCH modes: L, C, H.
    pub const OKLCH: [AreaMode; 3] = [AreaMode::OklchL, AreaMode::OklchC, AreaMode::OklchH];

    /// Every mode: PhotoCraft's nine, then OKLCH's three.
    pub const ALL: [AreaMode; 12] = [
        AreaMode::Hue,
        AreaMode::Saturation,
        AreaMode::Brightness,
        AreaMode::Red,
        AreaMode::Green,
        AreaMode::Blue,
        AreaMode::LabL,
        AreaMode::LabA,
        AreaMode::LabB,
        AreaMode::OklchL,
        AreaMode::OklchC,
        AreaMode::OklchH,
    ];

    /// The mode's key, as PhotoCraft stores it: `"h"`, `"s"`, `"v"` (brightness), `"r"`, `"g"`,
    /// `"b"`, `"lab_l"`, `"lab_a"`, `"lab_b"`, and `"oklch_l"`, `"oklch_c"`, `"oklch_h"`.
    pub fn key(self) -> &'static str {
        match self {
            AreaMode::Hue => "h",
            AreaMode::Saturation => "s",
            AreaMode::Brightness => "v",
            AreaMode::Red => "r",
            AreaMode::Green => "g",
            AreaMode::Blue => "b",
            AreaMode::LabL => "lab_l",
            AreaMode::LabA => "lab_a",
            AreaMode::LabB => "lab_b",
            AreaMode::OklchL => "oklch_l",
            AreaMode::OklchC => "oklch_c",
            AreaMode::OklchH => "oklch_h",
        }
    }

    /// The mode with `key` ([`key`](AreaMode::key)); `None` for any other text.
    pub fn from_key(key: &str) -> Option<AreaMode> {
        AreaMode::ALL.into_iter().find(|mode| mode.key() == key)
    }

    /// The area's accessible name, English, its two components across and down: the key a host
    /// translates ("Saturation and brightness").
    pub fn name(self) -> &'static str {
        match self {
            AreaMode::Hue => "Saturation and brightness",
            AreaMode::Saturation => "Hue and brightness",
            AreaMode::Brightness => "Hue and saturation",
            AreaMode::Red => "Blue and green",
            AreaMode::Green => "Blue and red",
            AreaMode::Blue => "Red and green",
            AreaMode::LabL => "Green-red and blue-yellow",
            AreaMode::LabA => "Blue-yellow and lightness",
            AreaMode::LabB => "Green-red and lightness",
            AreaMode::OklchL => "Hue and chroma",
            AreaMode::OklchC => "Hue and lightness",
            AreaMode::OklchH => "Chroma and lightness",
        }
    }

    /// The model whose values the area and strip edit.
    pub fn model(self) -> &'static dyn ColorModel {
        match self {
            AreaMode::Hue | AreaMode::Saturation | AreaMode::Brightness => &HsbModel,
            AreaMode::Red | AreaMode::Green | AreaMode::Blue => &RgbModel,
            AreaMode::LabL | AreaMode::LabA | AreaMode::LabB => &LabModel,
            AreaMode::OklchL | AreaMode::OklchC | AreaMode::OklchH => &OklchModel,
        }
    }

    /// The strip's channel, an index into the model's channels.
    pub fn strip(self) -> usize {
        match self {
            AreaMode::Hue | AreaMode::Red | AreaMode::LabL | AreaMode::OklchL => 0,
            AreaMode::Saturation | AreaMode::Green | AreaMode::LabA | AreaMode::OklchC => 1,
            AreaMode::Brightness | AreaMode::Blue | AreaMode::LabB | AreaMode::OklchH => 2,
        }
    }

    /// The area's channels, across and down.
    pub fn area(self) -> [usize; 2] {
        match self {
            AreaMode::Hue => [1, 2],
            AreaMode::Saturation => [0, 2],
            AreaMode::Brightness => [0, 1],
            AreaMode::Red => [2, 1],
            AreaMode::Green => [2, 0],
            AreaMode::Blue => [0, 1],
            AreaMode::LabL => [1, 2],
            AreaMode::LabA => [2, 0],
            AreaMode::LabB => [1, 0],
            AreaMode::OklchL => [2, 1],
            AreaMode::OklchC => [2, 0],
            AreaMode::OklchH => [1, 0],
        }
    }

    /// The values the strip's colours are drawn with: the colour's own, except that the HSB hue
    /// strip shows pure hues (full saturation and brightness), as Photoshop's.
    #[cfg(feature = "ui")]
    pub(crate) fn strip_base(self, values: &ModelValues) -> ModelValues {
        let mut base = *values;
        if self == AreaMode::Hue {
            base.values[1] = 100.0;
            base.values[2] = 100.0;
        }
        base
    }
}
