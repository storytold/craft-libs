//! The adjustment editors' image tones: colours that show what an adjustment does to the image, so they are
//! data and not theme tokens. The picker dots' greys (0, 128, 255), the two-colour slider tracks and the
//! hue spectrum.

use craft_ui::style::DataColor;
use egui::Color32;

/// An opaque data colour from 8-bit sRGB channels.
pub(crate) const fn rgb8(r: u8, g: u8, b: u8) -> DataColor {
    DataColor::new(Color32::from_rgb(r, g, b))
}

/// The image tones of the point pickers and the Levels thumb faces.
pub(crate) mod tones {
    use super::rgb8;
    use craft_ui::style::DataColor;

    /// The black point.
    pub const BLACK: DataColor = rgb8(0, 0, 0);
    /// The neutral gray point.
    pub const GRAY: DataColor = rgb8(128, 128, 128);
    /// The white point.
    pub const WHITE: DataColor = rgb8(255, 255, 255);
}

/// The ends of Color Balance's rows: the track runs from the first colour to the second.
pub(crate) mod balance {
    use super::{DataColor, rgb8};

    /// Cyan to Red.
    pub const CYAN_RED: [DataColor; 2] = [rgb8(0, 190, 210), rgb8(225, 40, 40)];
    /// Magenta to Green.
    pub const MAGENTA_GREEN: [DataColor; 2] = [rgb8(210, 40, 190), rgb8(40, 190, 60)];
    /// Yellow to Blue.
    pub const YELLOW_BLUE: [DataColor; 2] = [rgb8(230, 210, 30), rgb8(40, 80, 230)];
}

/// The colours the Black & White and Channel Mixer rows move towards from black.
pub(crate) mod mix {
    use super::{DataColor, rgb8};

    /// Black & White: Reds, Yellows, Greens, Cyans, Blues and Magentas.
    pub const RANGES: [DataColor; 6] =
        [rgb8(230, 50, 50), rgb8(235, 215, 40), rgb8(50, 200, 70), rgb8(40, 200, 220), rgb8(50, 90, 235), rgb8(220, 50, 210)];
    /// Channel Mixer: the Red, Green and Blue sources.
    pub const SOURCES: [DataColor; 3] = [rgb8(225, 50, 50), rgb8(50, 190, 70), rgb8(60, 100, 235)];
}
