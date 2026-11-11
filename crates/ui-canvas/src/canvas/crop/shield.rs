//! The crop shield: the options that dim the area outside the frame.

use egui::Color32;

/// Default shield opacity, in percent.
const SHIELD_OPACITY: f32 = 75.0;
/// Share of the opacity kept during a gesture with Auto Adjust Opacity.
const SHIELD_EDITING: f32 = 0.5;

/// The shield's colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ShieldColor {
    /// The canvas colour.
    #[default]
    MatchCanvas,
    /// [`CropShield::custom_color`].
    Custom,
}

/// The crop gear menu's shield options. Photoshop's defaults: shown, enabled, Match Canvas, 75 %, Auto Adjust on.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct CropShield {
    /// Show Cropped Area: off hides everything outside the frame behind the canvas colour.
    pub show_cropped_area: bool,
    /// Enable Crop Shield: off shows the area outside the frame undimmed.
    pub enabled: bool,
    /// Where the shield's colour comes from.
    pub color: ShieldColor,
    /// sRGB, used when `color` is `Custom`.
    pub custom_color: [u8; 3],
    /// Auto Center Preview: in the default mode, drawing or resizing the frame re-pans the view so the frame sits in
    /// the middle of the canvas.
    pub auto_center_preview: bool,
    /// Percent, 0..=100; [`opacity`](Self::opacity) gives the fraction and clamps a stray value.
    pub opacity: f32,
    /// Auto Adjust Opacity: lighter while the frame is dragged.
    pub auto_adjust: bool,
    /// Classic mode, where Photoshop greys Show Cropped Area out: it then acts as on.
    pub classic_mode: bool,
}

impl Default for CropShield {
    fn default() -> Self {
        Self {
            show_cropped_area: true,
            enabled: true,
            color: ShieldColor::MatchCanvas,
            custom_color: [0, 0, 0],
            auto_center_preview: true,
            opacity: SHIELD_OPACITY,
            auto_adjust: true,
            classic_mode: false,
        }
    }
}

impl CropShield {
    /// Show Cropped Area as it acts: always on in Classic mode (the stored choice is kept for the default mode).
    pub fn shows_cropped_area(&self) -> bool {
        self.classic_mode || self.show_cropped_area
    }

    /// The opacity as a fraction in 0..=1: a non-finite value is the default, an out-of-range one is clamped.
    pub fn opacity(&self) -> f32 {
        let o = if self.opacity.is_finite() { self.opacity } else { SHIELD_OPACITY };
        o.clamp(0.0, 100.0) / 100.0
    }

    /// What to paint over the area outside the frame, `canvas` being the canvas colour and `editing` a crop drag in
    /// progress: the opaque canvas colour when Show Cropped Area is off, nothing when the shield is disabled or fully
    /// transparent, otherwise its colour at its opacity.
    pub fn fill(&self, canvas: Color32, editing: bool) -> Option<Color32> {
        let [r, g, b, _] = canvas.to_srgba_unmultiplied();
        if !self.shows_cropped_area() {
            return Some(Color32::from_rgb(r, g, b));
        }
        if !self.enabled {
            return None;
        }
        let [r, g, b] = match self.color {
            ShieldColor::MatchCanvas => [r, g, b],
            ShieldColor::Custom => self.custom_color,
        };
        let a = self.opacity() * if self.auto_adjust && editing { SHIELD_EDITING } else { 1.0 };
        let a = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        (a > 0).then(|| Color32::from_rgba_unmultiplied(r, g, b, a))
    }
}
