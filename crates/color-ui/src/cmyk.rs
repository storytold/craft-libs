//! [`CmykModel`]: a colour as the inks of one CMYK profile.

use craft_cms::convert::CmykConverter;
use craft_theme::{Channel, ColorModel, DefaultOn, MAX_CHANNELS, ModelValues, Srgba};

/// CMYK through a CMYK profile, as Photoshop's and PhotoCraft's Color Picker show it: four
/// channels C, M, Y and K, ink in percent (0 no ink, 100 full). It converts through the
/// [`CmykConverter`] it holds (the document's CMYK profile, or the built-in coated CMYK), so the
/// same sRGB colour reads as different inks in different documents.
///
/// A host offers it by passing it to the picker
/// (`ColorPicker::models`, feature `ui`); the
/// `ColorPickerDialog` adds it when it is given a
/// `ColorPickerDialog::gamut`. Inks the user typed stay as typed while they
/// still give the colour (`PickedColor::model`): a CMYK document
/// fills with them, not with what its sRGB stand-in converts back to (storytold/photocraft#4196).
#[derive(Clone)]
pub struct CmykModel {
    converter: CmykConverter,
}

const CMYK_CHANNELS: [Channel; 4] = [
    Channel { name: "Cyan", qualified: "CMYK cyan", label: "C", min: 0.0, max: 100.0, unit: "%", decimals: 0, step: 1.0, track_stops: 9 },
    Channel { name: "Magenta", qualified: "CMYK magenta", label: "M", min: 0.0, max: 100.0, unit: "%", decimals: 0, step: 1.0, track_stops: 9 },
    Channel { name: "Yellow", qualified: "CMYK yellow", label: "Y", min: 0.0, max: 100.0, unit: "%", decimals: 0, step: 1.0, track_stops: 9 },
    Channel { name: "Black", qualified: "CMYK black", label: "K", min: 0.0, max: 100.0, unit: "%", decimals: 0, step: 1.0, track_stops: 9 },
];

impl CmykModel {
    /// The model on `converter` (cheap: the transforms are shared).
    pub fn new(converter: CmykConverter) -> CmykModel {
        CmykModel { converter }
    }

    /// The converter the model converts through.
    pub fn converter(&self) -> &CmykConverter {
        &self.converter
    }
}

impl std::fmt::Debug for CmykModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CmykModel").field("profile", &self.converter.profile().description).finish()
    }
}

impl ColorModel for CmykModel {
    fn id(&self) -> &'static str {
        "cmyk"
    }
    fn name(&self) -> &'static str {
        "CMYK"
    }
    fn channels(&self) -> &[Channel] {
        &CMYK_CHANNELS
    }
    // pc: crates/ui-egui/src/dialogs/color_picker_ui.rs:298 `Components` (cmyk from
    // `photocraft_color::convert::rgb_to_cmyk`); `self.converter.from_srgb([r, g, b])` scaled to
    // percent.
    fn from_srgb(&self, color: Srgba) -> ModelValues {
        let inks = self.converter.from_srgb([color.r, color.g, color.b]);
        ModelValues::new("cmyk", &inks.map(|v| v * 100.0))
    }
    // pc: crates/ui-egui/src/dialogs/color_picker_ui.rs:722 (typed inks to sRGB through
    // `cmyk_to_rgb`); `self.converter.to_srgb(values / 100)`, opaque.
    fn to_srgb(&self, values: &[f32; MAX_CHANNELS]) -> Srgba {
        let rgb = self.converter.to_srgb([values[0] / 100.0, values[1] / 100.0, values[2] / 100.0, values[3] / 100.0]);
        Srgba { r: rgb[0].clamp(0.0, 1.0), g: rgb[1].clamp(0.0, 1.0), b: rgb[2].clamp(0.0, 1.0), a: 1.0 }
    }
    fn default_on(&self) -> DefaultOn {
        DefaultOn { full: false, compact: false }
    }
}
