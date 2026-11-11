//! Transform presets, interpolation, the saved transform settings and the box menu's choices and words.

use craft_behavior::geometry::quad::{ScaleRule, TransformMode};
use craft_ui::runtime::env::UnitKind;
use serde::{Deserialize, Deserializer};

/// Edit › Transform's presets while a box is open (PhotoCraft `box_preset`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransformPreset {
    /// Rotate 180°.
    Rotate180,
    /// Rotate 90° clockwise.
    Rotate90Clockwise,
    /// Rotate 90° counter clockwise.
    Rotate90CounterClockwise,
    /// Flip horizontal.
    FlipHorizontal,
    /// Flip vertical.
    FlipVertical,
}

impl TransformPreset {
    /// Every preset, in the menu's order.
    pub const ALL: [TransformPreset; 5] = [
        TransformPreset::Rotate180,
        TransformPreset::Rotate90Clockwise,
        TransformPreset::Rotate90CounterClockwise,
        TransformPreset::FlipHorizontal,
        TransformPreset::FlipVertical,
    ];

    /// What the preset does to the box, as `(kx, ky, angle)`: a scale along the box's own axes,
    /// then a turn in radians, clockwise on screen, both about the reference point. Rotate 180° is
    /// `(1, 1, π)`, Rotate 90° Clockwise `(1, 1, π/2)`, Counter Clockwise `(1, 1, −π/2)`, Flip
    /// Horizontal `(−1, 1, 0)`, Flip Vertical `(1, −1, 0)`.
    pub fn scale_and_turn(self) -> (f64, f64, f64) {
        // pc: crates/ui-egui/src/tools/transform_tool/commit.rs:329 (`box_preset`).
        use std::f64::consts::{FRAC_PI_2, PI};
        match self {
            TransformPreset::Rotate180 => (1.0, 1.0, PI),
            TransformPreset::Rotate90Clockwise => (1.0, 1.0, FRAC_PI_2),
            TransformPreset::Rotate90CounterClockwise => (1.0, 1.0, -FRAC_PI_2),
            TransformPreset::FlipHorizontal => (-1.0, 1.0, 0.0),
            TransformPreset::FlipVertical => (1.0, -1.0, 0.0),
        }
    }
}

/// How the transform resamples the moving pixels: PhotoCraft's three choices (LC11). The app maps
/// them to its engine; [`id`](Self::id) gives PhotoCraft's ids.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Interpolation {
    /// Bicubic resampling (the default).
    #[default]
    Bicubic,
    /// Bilinear resampling.
    Bilinear,
    /// Nearest-neighbour resampling: no smoothing.
    NearestNeighbor,
}

impl Interpolation {
    /// Every choice, in the select's order.
    pub const ALL: [Interpolation; 3] = [Interpolation::Bicubic, Interpolation::Bilinear, Interpolation::NearestNeighbor];

    /// PhotoCraft's id: "bicubic", "bilinear", "nearest".
    pub fn id(self) -> &'static str {
        // pc: crates/ui-egui/src/tools/transform_tool/options_bar.rs:241 (the select's options).
        match self {
            Interpolation::Bicubic => "bicubic",
            Interpolation::Bilinear => "bilinear",
            Interpolation::NearestNeighbor => "nearest",
        }
    }

    /// The choice with PhotoCraft's `id`; `None` for any other text.
    pub fn from_id(id: &str) -> Option<Interpolation> {
        // pc: crates/ui-egui/src/tools/transform_tool/options_bar.rs:241.
        Interpolation::ALL.into_iter().find(|choice| choice.id() == id)
    }
}

/// A pick in the box's right-click menu ([`TransformSession::menu`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransformChoice {
    /// Free Transform, Scale (as Free), Rotate, Skew, Distort, Perspective: apply as
    /// [`TransformEdit::Mode`].
    Mode(TransformMode),
    /// Warp: apply as [`TransformEdit::WarpRequested`].
    Warp,
    /// Apply as [`TransformEdit::Preset`].
    Preset(TransformPreset),
}

/// Free Transform's persisted section: the app keeps it with its preferences. Unknown fields are
/// ignored, missing ones take their defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct TransformSettings {
    /// PhotoCraft's Use Legacy Free Transform: corners stretch freely and ⇧ keeps the proportions
    /// ([`ScaleRule::Free`]). Off by default: corners keep the proportions and ⇧ frees them
    /// (Photoshop CC 2019 and later, LC1).
    pub legacy: bool,
    /// The W and H fields' link (the chain button), kept between sessions.
    pub aspect_linked: bool,
    /// Show Reference Point: off by default (Photoshop CC 2019 and later, LC2). Hidden, the point
    /// is not drawn, hit or picked; its stored position still pivots ⌥ scaling and rotating.
    pub show_reference_point: bool,
    /// The interpolation a new session starts with.
    pub interpolation: Interpolation,
    /// The W and H fields' unit; percent when unset. Saved preferences may spell it as the legacy names
    /// (`"Pixels"`, `"Percent"`, `"Inches"`, `"Centimetres"`, `"Millimetres"`, `"Points"`, `"Picas"`).
    #[serde(deserialize_with = "unit_or_legacy")]
    pub size_unit: Option<UnitKind>,
}

/// Reads a unit by the spelling `UnitKind` serializes to or by the legacy spelling, whatever the case.
fn unit_or_legacy<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<UnitKind>, D::Error> {
    let Some(name) = Option::<String>::deserialize(deserializer)? else { return Ok(None) };
    let unit = match name.to_ascii_lowercase().as_str() {
        "pixels" => UnitKind::Pixels,
        "percent" => UnitKind::Percent,
        "degrees" => UnitKind::Degrees,
        "points" => UnitKind::Points,
        "times" => UnitKind::Times,
        "inches" => UnitKind::Inches,
        "centimetres" => UnitKind::Centimetres,
        "millimetres" => UnitKind::Millimetres,
        "picas" => UnitKind::Picas,
        "pixelsperinch" => UnitKind::PixelsPerInch,
        "pixelspercentimetre" => UnitKind::PixelsPerCentimetre,
        "minutes" => UnitKind::Minutes,
        _ => return Err(serde::de::Error::custom(format!("unknown unit `{name}`"))),
    };
    Ok(Some(unit))
}

impl TransformSettings {
    /// The scaling rule [`legacy`](Self::legacy) picks.
    pub fn scale_rule(&self) -> ScaleRule {
        if self.legacy { ScaleRule::Free } else { ScaleRule::Proportional }
    }
}

craft_ui::words! {
    /// The words of the box's right-click menu ([`TransformSession::menu`]), Photoshop's and
    /// PhotoCraft's order (`canvas_tool_menu.rs:75`).
    pub struct TransformMenuWords {
        free_transform: "transformMenu.freeTransform" = "Free Transform";
        scale: "transformMenu.scale" = "Scale";
        rotate: "transformMenu.rotate" = "Rotate";
        skew: "transformMenu.skew" = "Skew";
        distort: "transformMenu.distort" = "Distort";
        perspective: "transformMenu.perspective" = "Perspective";
        warp: "transformMenu.warp" = "Warp";
        rotate_180: "transformMenu.rotate180" = "Rotate 180°";
        rotate_90_clockwise: "transformMenu.rotate90Clockwise" = "Rotate 90° Clockwise";
        rotate_90_counter_clockwise: "transformMenu.rotate90CounterClockwise" = "Rotate 90° Counter Clockwise";
        flip_horizontal: "transformMenu.flipHorizontal" = "Flip Horizontal";
        flip_vertical: "transformMenu.flipVertical" = "Flip Vertical";
    }
}
