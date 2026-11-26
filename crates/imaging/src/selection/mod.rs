//! Selection Preview and Quick Mask colour rules, without egui.
//!
//! A mask's coverage turned into the colour a preview texture shows over the image: Photoshop's
//! Selection Preview modes (Color Range's None, Grayscale, Black Matte, White Matte, Quick Mask)
//! and the Quick Mask tint with its Channel Options (colour, opacity, Color Indicates). The
//! selection overlay (`craft_imaging::ui::selection::SelectionOverlay::preview`) builds its texture
//! with these rules; an engine that composites the preview itself uses them directly.

use craft_theme::{Rgba, Srgba};
use serde::{Deserialize, Serialize};

/// A 0..=1 value as a byte, rounded half up.
fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Channel Options › Color Indicates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorIndicates {
    /// The overlay colour shows the masked areas (Photoshop's default).
    #[default]
    MaskedAreas,
    /// The overlay colour shows the selected areas.
    SelectedAreas,
}

/// Photoshop's Selection Preview: how a selection being built (Color Range, Focus Area) shows on
/// the image. The eyedroppers always sample the original image below it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreviewMode {
    /// The image as it is: no texture.
    #[default]
    None,
    /// The coverage as grey: white selected, black unselected, grey partial. Opaque.
    Grayscale,
    /// Black over the unselected area: alpha `1 − c`.
    BlackMatte,
    /// White over the unselected area: alpha `1 − c`.
    WhiteMatte,
    /// The Quick Mask tint over the area its [`QuickMaskLook`] marks.
    QuickMask,
}

impl PreviewMode {
    /// Every mode, in the menu's order.
    pub const ALL: [PreviewMode; 5] =
        [PreviewMode::None, PreviewMode::Grayscale, PreviewMode::BlackMatte, PreviewMode::WhiteMatte, PreviewMode::QuickMask];

    /// The id PhotoCraft's dialogs and automation use: `none`, `grayscale`, `blackMatte`,
    /// `whiteMatte`, `quickMask`.
    // pc: crates/ui-egui/src/dialogs/color_range_ui.rs:49 `PREVIEWS`.
    pub fn id(self) -> &'static str {
        match self {
            PreviewMode::None => "none",
            PreviewMode::Grayscale => "grayscale",
            PreviewMode::BlackMatte => "blackMatte",
            PreviewMode::WhiteMatte => "whiteMatte",
            PreviewMode::QuickMask => "quickMask",
        }
    }

    /// The mode with `id` ([`id`](Self::id)), if any.
    pub fn from_id(id: &str) -> Option<PreviewMode> {
        PreviewMode::ALL.into_iter().find(|m| m.id() == id)
    }

    /// The colour (encoded sRGB, unmultiplied alpha) one pixel of coverage `c` (clamped to 0..=1)
    /// shows: `None` transparent; `Grayscale` grey `round(c · 255)`, opaque; `BlackMatte` black
    /// with alpha `round((1 − c) · 255)`; `WhiteMatte` white with that alpha; `QuickMask` what
    /// `quick_mask.pixel(c)` gives. Color Range's own Quick Mask passes
    /// [`QuickMaskLook::default()`]: red at 50 %, masked areas (PhotoCraft's fixed tint).
    pub fn pixel(self, coverage: f32, quick_mask: &QuickMaskLook) -> Rgba {
        let c = coverage.clamp(0.0, 1.0);
        match self {
            PreviewMode::None => Rgba([0, 0, 0, 0]),
            PreviewMode::Grayscale => {
                let g = byte(c);
                Rgba([g, g, g, 255])
            }
            PreviewMode::BlackMatte => Rgba([0, 0, 0, byte(1.0 - c)]),
            PreviewMode::WhiteMatte => Rgba([255, 255, 255, byte(1.0 - c)]),
            PreviewMode::QuickMask => quick_mask.pixel(c),
        }
    }
}

/// Quick Mask's look, from Quick Mask Options (and an alpha channel's Channel Options): the tint
/// colour, its opacity, and which area it marks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuickMaskLook {
    /// The tint, encoded sRGB; its alpha is ignored. Default red (`#ff0000`, Photoshop's and
    /// PhotoCraft's `AlphaChannel::DEFAULT_COLOR`).
    pub color: Srgba,
    /// 0..=1. Default 0.5.
    pub opacity: f32,
    /// Default [`ColorIndicates::MaskedAreas`].
    pub indicates: ColorIndicates,
}

impl Default for QuickMaskLook {
    // pc: crates/engine/src/cmd/channel_cmds/view.rs:85-89 `QuickMaskOptions::default`.
    fn default() -> Self {
        Self { color: Srgba::rgb(1.0, 0.0, 0.0), opacity: 0.5, indicates: ColorIndicates::MaskedAreas }
    }
}

impl QuickMaskLook {
    /// The tint one pixel of coverage `c` (clamped to 0..=1) shows: the colour with alpha
    /// `opacity · (1 − c)` for [`ColorIndicates::MaskedAreas`] and `opacity · c` for
    /// [`ColorIndicates::SelectedAreas`], rounded as `(a · 255 + 0.5) as u8`; unmultiplied. The
    /// Type Mask tools use it with `c` the glyph's coverage.
    pub fn pixel(&self, coverage: f32) -> Rgba {
        let c = coverage.clamp(0.0, 1.0);
        let covered = match self.indicates {
            ColorIndicates::MaskedAreas => 1.0 - c,
            ColorIndicates::SelectedAreas => c,
        };
        let alpha = (covered * self.opacity).clamp(0.0, 1.0);
        Rgba([byte(self.color.r), byte(self.color.g), byte(self.color.b), byte(alpha)])
    }
}
