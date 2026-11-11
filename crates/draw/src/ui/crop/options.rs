//! [`CropSettings`]: the Crop tool's options as one persisted section: the ratio or the
//! W × H × Resolution fields, the overlay and its show rule, the shield, Delete Cropped Pixels and
//! Fill. Typed lengths stay text until used, so a half-typed "4 i" survives a pass, and parse
//! through the scope's [`Units`] reader when the tool needs a number.

use craft_behavior::geometry::crop::{CropGuide, GuideShow};
use craft_ui::canvas::CropShield;
use craft_ui::runtime::env::{UnitKind, Units};

use super::bar_words::CropBarWords;
use super::session::CropDocument;

/// The largest number a ratio side keeps: 99,999 (PhotoCraft's ratio fields).
pub const RATIO_MAX: f64 = 99_999.0;
/// What one arrow press on a ratio field adds to its side.
pub const RATIO_STEP: f64 = 1.0;
/// Decimals a ratio side keeps and shows: 3, so 1.414 : 1 (ISO A sizes) survives (#6487).
pub const RATIO_DECIMALS: usize = 3;
/// The largest number a length or resolution field keeps: 1e9 (the crop command refuses what is
/// past a canvas or resolution limit with its own message).
pub const FIELD_MAX: f64 = 1e9;
/// The smallest length a W or H field keeps, in the field's own unit: a typed 0 or negative number lands here
/// (the field's two decimals), so a length is always positive.
pub const LENGTH_MIN: f64 = 0.01;

/// What the frame's proportions follow.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum CropRatio {
    /// No ratio: the bar's "Ratio" entry with empty fields. A ratio field with one side typed
    /// stays free until the other side is typed ([`CropSettings::ratio_fields`]).
    #[default]
    Free,
    /// The document's own proportions ("Original Ratio"); the fields show its size.
    Original,
    /// Width to height as typed or picked, both sides positive and at most [`RATIO_MAX`].
    #[non_exhaustive]
    Sides {
        /// The width part of the ratio.
        width: f64,
        /// The height part of the ratio.
        height: f64,
    },
    /// W × H × Resolution: the frame holds W : H while it is drawn, and the commit resamples to
    /// [`CropSettings::target`].
    Size,
}

impl CropRatio {
    /// Width to height `width` : `height`.
    pub fn sides(width: f64, height: f64) -> Self {
        CropRatio::Sides { width, height }
    }
}

/// The resolution field's unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ResolutionUnit {
    /// px/in.
    #[default]
    PixelsPerInch,
    /// px/cm; one px/cm is 2.54 px/in.
    PixelsPerCentimetre,
}

/// A length with the unit it was entered in: the W and H of W × H × Resolution mode keep what the user typed ("2 in"
/// stays in inches when the document resizes).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct CropLength {
    /// The length, in `unit`.
    pub value: f64,
    /// The unit the length is kept and shown in.
    pub unit: UnitKind,
}

impl CropLength {
    /// A length of `value` in `unit`.
    pub const fn new(value: f64, unit: UnitKind) -> Self {
        Self { value, unit }
    }

    /// The length when it is positive, finite and at most [`FIELD_MAX`].
    fn usable(length: Option<CropLength>) -> Option<CropLength> {
        length.filter(|l| l.value.is_finite() && l.value > 0.0 && l.value <= FIELD_MAX)
    }

    /// The value with its unit's decimals (at least 2), trailing zeros dropped.
    fn number(self) -> String {
        trim_number(self.value, (*self.unit.decimals().end()).max(2))
    }
}

/// The W, H and Resolution fields of W × H × Resolution mode. Unset W and H are `None`; the resolution is typed text
/// ("300"), empty when unset. Perspective Crop's options bar uses the same fields.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct CropSizeFields {
    /// The width, or `None` when unset.
    pub width: Option<CropLength>,
    /// The height, or `None` when unset.
    pub height: Option<CropLength>,
    /// A positive number, or empty for the document's resolution.
    pub resolution: String,
    /// The unit of `resolution`.
    pub resolution_unit: ResolutionUnit,
}

/// What the area a turned or extending crop uncovers is filled with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum CropFill {
    /// "Background (Default)": the background colour or transparency.
    #[default]
    Background,
    /// "Content-Aware Fill": filled from the surroundings (the engine's). Generative Expand is
    /// not offered.
    ContentAware,
}

/// The size the commit resamples to: W and H in pixels (unrounded; the app rounds, at least 1)
/// and the resolution in px/in it sets, when one was typed.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CropTarget {
    /// Target width in pixels.
    pub width: f64,
    /// Target height in pixels.
    pub height: f64,
    /// Target resolution in px/in, when one was typed.
    pub resolution: Option<f64>,
}

/// One built-in size preset: the W and H it fills in and the Resolution text, in px/in.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CropSizePreset {
    /// The W length.
    pub width: CropLength,
    /// The H length.
    pub height: CropLength,
    /// The Resolution text, in px/in.
    pub resolution: &'static str,
}

/// A crop preset the user saved (tool presets of the Crop tool, stored by the app). The bar lists
/// them after the built-in presets and fills the settings from the one picked.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct CropUserPreset {
    /// At most 255 characters (PhotoCraft's `MAX_NAME`); the app's New Crop Preset dialog
    /// suggests [`CropSettings::preset_name`].
    pub name: String,
    /// The ratio the preset sets.
    pub ratio: CropRatio,
    /// The fields of a [`CropRatio::Size`] preset; empty for a ratio preset.
    pub size: CropSizeFields,
}

impl CropUserPreset {
    /// A preset named `name` that sets `ratio`, with `size` as the fields of a [`CropRatio::Size`] preset.
    pub fn new(name: impl Into<String>, ratio: CropRatio, size: CropSizeFields) -> Self {
        Self { name: name.into(), ratio, size }
    }
}

/// An entry of the options bar's preset list, in the list's order: Ratio, W × H × Resolution, the
/// ratio presets, the size presets, the user's presets, Front Image, then New Crop Preset… and
/// Delete Crop Preset….
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CropPresetChoice {
    /// "Ratio": a free ratio with empty fields.
    Free,
    /// "W x H x Resolution".
    Size,
    /// [`CROP_RATIOS`]`[i]`.
    Ratio(usize),
    /// [`CROP_SIZES`]`[i]`.
    SizePreset(usize),
    /// The user's preset at this index of the list the app gave the bar.
    User(usize),
    /// "Front Image": W, H and the resolution from the document, at once (never kept as a mode).
    FrontImage,
    /// "New Crop Preset…": the app opens its dialog.
    NewPreset,
    /// "Delete Crop Preset…": the app opens its dialog.
    DeletePreset,
}

/// The built-in ratio presets after "Ratio" and "W x H x Resolution": Original Ratio, 1 : 1
/// (Square), 4 : 5 (8 : 10), 5 : 7, 2 : 3 (4 : 6), 16 : 9. Their names are
/// [`CropBarWords`]' `ratio_*` words, in this order.
// pc: crates/ui-egui/src/panels/chrome_ui.rs:278 `CROP_RATIOS` (its first entry, "Ratio", is
// `CropPresetChoice::Free`).
pub const CROP_RATIOS: [CropRatio; 6] = [
    CropRatio::Original,
    CropRatio::Sides { width: 1.0, height: 1.0 },
    CropRatio::Sides { width: 4.0, height: 5.0 },
    CropRatio::Sides { width: 5.0, height: 7.0 },
    CropRatio::Sides { width: 2.0, height: 3.0 },
    CropRatio::Sides { width: 16.0, height: 9.0 },
];

/// Photoshop's built-in size presets, all in px/in. The list shows each by
/// [`CropSettings::preset_name`] of its values ("4 x 5 in 300 ppi").
// pc: crates/ui-egui/src/tools/crop_size.rs:38 `PRESETS`.
pub const CROP_SIZES: [CropSizePreset; 8] = [
    CropSizePreset { width: CropLength::new(4.0, UnitKind::Inches), height: CropLength::new(5.0, UnitKind::Inches), resolution: "300" },
    CropSizePreset { width: CropLength::new(8.5, UnitKind::Inches), height: CropLength::new(11.0, UnitKind::Inches), resolution: "300" },
    CropSizePreset { width: CropLength::new(1024.0, UnitKind::Pixels), height: CropLength::new(768.0, UnitKind::Pixels), resolution: "92" },
    CropSizePreset { width: CropLength::new(1280.0, UnitKind::Pixels), height: CropLength::new(800.0, UnitKind::Pixels), resolution: "113" },
    CropSizePreset { width: CropLength::new(1366.0, UnitKind::Pixels), height: CropLength::new(768.0, UnitKind::Pixels), resolution: "135" },
    CropSizePreset { width: CropLength::new(4.0, UnitKind::Inches), height: CropLength::new(6.0, UnitKind::Inches), resolution: "300" },
    CropSizePreset { width: CropLength::new(5.0, UnitKind::Inches), height: CropLength::new(7.0, UnitKind::Inches), resolution: "300" },
    CropSizePreset { width: CropLength::new(8.0, UnitKind::Inches), height: CropLength::new(10.0, UnitKind::Inches), resolution: "300" },
];

/// The Crop tool's options. The app keeps it in its settings (it persists between runs); the
/// options bar edits it in place, the tool reads it each pass and reports key changes (O, ⇧O, H,
/// P, I, X) as [`CropEvent::Settings`](super::CropEvent::Settings). Every field defaults on its
/// own, so older settings load.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct CropSettings {
    /// The aspect constraint of the frame.
    pub ratio: CropRatio,
    /// The ratio fields as shown while one side is still empty (0): a pair with both sides
    /// positive becomes [`CropRatio::Sides`]; an incomplete pair is a free ratio that keeps the
    /// side typed first.
    pub ratio_fields: [f64; 2],
    /// W × H × Resolution's fields.
    pub size: CropSizeFields,
    /// Delete Cropped Pixels: on by default (Photoshop).
    pub delete_pixels: bool,
    /// What fills the area a crop grows beyond the canvas.
    pub fill: CropFill,
    /// The overlay inside the frame (Rule of Thirds by default), when it shows (Auto) and its
    /// orientation (Triangle and Golden Spiral have more than one).
    pub guide: CropGuide,
    /// When the guide shows.
    pub guide_show: GuideShow,
    /// Which orientation of the guide to draw, counting from 0.
    pub guide_orientation: u8,
    /// The gear menu's options: Classic Mode, Show Cropped Area, Auto Center Preview and the
    /// shield.
    pub shield: CropShield,
}

impl Default for CropSettings {
    fn default() -> Self {
        Self {
            ratio: CropRatio::Free,
            ratio_fields: [0.0, 0.0],
            size: CropSizeFields::default(),
            delete_pixels: true,
            fill: CropFill::Background,
            guide: CropGuide::default(),
            guide_show: GuideShow::default(),
            guide_orientation: 0,
            shield: CropShield::default(),
        }
    }
}

/// `value` with up to `decimals` decimals, trailing zeros and a bare point dropped.
fn trim_number(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_owned() } else { text }
}

/// A positive finite number as typed ("," reads as "."), without a unit.
fn parse_number(text: &str) -> Option<f64> {
    text.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0)
}

/// Both sides positive, finite and at most [`RATIO_MAX`].
fn valid_sides(width: f64, height: f64) -> bool {
    [width, height].iter().all(|v| v.is_finite() && *v > 0.0 && *v <= RATIO_MAX)
}

fn round_side(value: f64) -> f64 {
    let scale = 10f64.powi(RATIO_DECIMALS as i32);
    (value * scale).round() / scale
}

fn ratio_text(value: f64) -> String {
    trim_number(value, RATIO_DECIMALS)
}

impl CropSettings {
    /// The width-to-height ratio every frame gesture holds: the document's for
    /// [`CropRatio::Original`], the sides' for [`CropRatio::Sides`], W : H of the
    /// [`target`](Self::target) for [`CropRatio::Size`]; `None` for a free ratio and for any
    /// ratio that is not a positive finite number.
    pub fn ratio_value(&self, document: &CropDocument, units: &Units) -> Option<f64> {
        // pc: crates/ui-egui/src/tools/crop_ui.rs:217 `preset_ratio`; crop_size.rs:146 `ratio`.
        let (width, height) = match self.ratio {
            CropRatio::Free => return None,
            CropRatio::Original => (document.size.width, document.size.height),
            CropRatio::Sides { width, height } => (width, height),
            CropRatio::Size => {
                let target = self.target(document, units)?;
                (target.width, target.height)
            }
        };
        Some(width / height).filter(|k| k.is_finite() && *k > 0.0)
    }

    /// The size the commit resamples to, in W × H × Resolution mode with both W and H holding
    /// lengths; `None` otherwise (W or H empty is a free crop: no resample). Physical lengths
    /// convert at the typed resolution, else at the document's; points and picas at the
    /// the environment's points per inch (`units`). A bare number is in pixels.
    pub fn target(&self, document: &CropDocument, units: &Units) -> Option<CropTarget> {
        // pc: crates/ui-egui/src/tools/crop_size.rs:131 `target`, :122 `resolution_ppi` (px/cm × 2.54),
        // :71 `parse_length` (positive, at most 1e9, no percent, ',' read as '.').
        if self.ratio != CropRatio::Size {
            return None;
        }
        let resolution = self.size.resolution_ppi();
        let dpi = resolution.unwrap_or(document.resolution);
        let pixels = |length: Option<CropLength>| {
            let length = CropLength::usable(length)?;
            let px = length.unit.to_pixels(length.value, 0.0, dpi, units.points_per_inch())?;
            (px.is_finite() && px > 0.0).then_some(px)
        };
        Some(CropTarget { width: pixels(self.size.width)?, height: pixels(self.size.height)?, resolution })
    }

    /// Applies a pick from the preset list: Free clears the ratio fields; Size turns W × H ×
    /// Resolution on; a ratio preset sets the ratio and its fields; a size preset fills W, H and
    /// the resolution in px/in and turns W × H × Resolution on; a user preset takes its ratio and
    /// fields; Front Image fills W, H in px and the resolution from `document`
    /// ([`front_image`](Self::front_image)). New and Delete change nothing. Returns whether the
    /// settings changed, so the tool refits the frame.
    pub fn choose(&mut self, choice: CropPresetChoice, document: &CropDocument, user: &[CropUserPreset]) -> bool {
        // pc: crates/ui-egui/src/tools/crop_size.rs:255 `chosen`, :169 `select_custom_ratio`;
        // crop_presets.rs:180 `apply_choice`.
        let before = self.clone();
        match choice {
            CropPresetChoice::Free => {
                self.ratio = CropRatio::Free;
                self.ratio_fields = [0.0, 0.0];
            }
            CropPresetChoice::Size => self.ratio = CropRatio::Size,
            CropPresetChoice::Ratio(i) => {
                if let Some(&ratio) = CROP_RATIOS.get(i) {
                    self.set_ratio_and_fields(ratio);
                }
            }
            CropPresetChoice::SizePreset(i) => {
                if let Some(preset) = CROP_SIZES.get(i) {
                    self.ratio = CropRatio::Size;
                    self.size = CropSizeFields {
                        width: Some(preset.width),
                        height: Some(preset.height),
                        resolution: preset.resolution.to_owned(),
                        resolution_unit: ResolutionUnit::PixelsPerInch,
                    };
                }
            }
            CropPresetChoice::User(i) => {
                if let Some(preset) = user.get(i) {
                    self.set_ratio_and_fields(preset.ratio);
                    if preset.ratio == CropRatio::Size {
                        self.size = preset.size.clone();
                    }
                }
            }
            CropPresetChoice::FrontImage => self.front_image(document),
            CropPresetChoice::NewPreset | CropPresetChoice::DeletePreset => {}
        }
        *self != before
    }

    /// Sets `ratio` and the ratio fields it shows (zero for a ratio without sides).
    fn set_ratio_and_fields(&mut self, ratio: CropRatio) {
        self.ratio = ratio;
        self.ratio_fields = match ratio {
            CropRatio::Sides { width, height } if valid_sides(width, height) => [width, height],
            _ => [0.0, 0.0],
        };
    }

    /// The entry the preset list shows as chosen: the latest user preset whose values match,
    /// else the built-in size preset whose fields match exactly, else the mode or ratio entry,
    /// else [`CropPresetChoice::Free`] for a typed ratio.
    pub fn choice(&self, user: &[CropUserPreset]) -> CropPresetChoice {
        // pc: crates/ui-egui/src/tools/crop_presets.rs:120 `button_label`.
        let matches = |preset: &CropUserPreset| preset.ratio == self.ratio && (self.ratio != CropRatio::Size || preset.size == self.size);
        if let Some(i) = user.iter().rposition(matches) {
            return CropPresetChoice::User(i);
        }
        if self.ratio == CropRatio::Size {
            let builtin = CROP_SIZES.iter().position(|p| {
                self.size.width == Some(p.width)
                    && self.size.height == Some(p.height)
                    && self.size.resolution == p.resolution
                    && self.size.resolution_unit == ResolutionUnit::PixelsPerInch
            });
            return builtin.map_or(CropPresetChoice::Size, CropPresetChoice::SizePreset);
        }
        CROP_RATIOS.iter().position(|r| *r == self.ratio).map_or(CropPresetChoice::Free, CropPresetChoice::Ratio)
    }

    /// The bar's Clear: W, H and the resolution in W × H × Resolution mode (the mode stays), else
    /// the ratio and its fields.
    pub fn clear(&mut self) {
        // pc: crates/ui-egui/src/tools/crop_size.rs:230 `clear_ratio`.
        if self.ratio == CropRatio::Size {
            self.size.width = None;
            self.size.height = None;
            self.size.resolution.clear();
        } else {
            self.ratio = CropRatio::Free;
            self.ratio_fields = [0.0, 0.0];
        }
    }

    /// The Crop tool's X: swaps W and H in W × H × Resolution mode, else swaps the ratio's sides
    /// (Original becomes the document's size turned on its side). Returns false when there is
    /// nothing to swap (a free ratio).
    pub fn swap(&mut self, document: &CropDocument) -> bool {
        // pc: crates/ui-egui/src/tools/crop_ui.rs:721 `swap_orientation` (the options half),
        // :713 `swapped_ratio`; crop_size.rs:239 `swap`.
        let (width, height) = match self.ratio {
            CropRatio::Size => {
                self.swap_fields();
                return true;
            }
            CropRatio::Free => return false,
            CropRatio::Original => (document.size.width, document.size.height),
            CropRatio::Sides { width, height } => (width, height),
        };
        if !valid_sides(width, height) {
            return false;
        }
        let (width, height) = (round_side(height), round_side(width));
        self.ratio = CropRatio::Sides { width, height };
        self.ratio_fields = [width, height];
        true
    }

    /// The bar's ⇄ between the two fields: always swaps them, in either mode (Perspective Crop
    /// shows the W and H fields without the mode).
    pub fn swap_fields(&mut self) {
        // pc: crates/ui-egui/src/tools/crop_size.rs:249 `swap_fields`; panels/options_bar.rs:483 (the
        // ratio pair).
        if self.ratio == CropRatio::Size {
            std::mem::swap(&mut self.size.width, &mut self.size.height);
            return;
        }
        // Ratio fields: Original has none of its own (the document's size is swapped by `swap`).
        self.ratio_fields.swap(0, 1);
        if let CropRatio::Sides { width, height } = self.ratio {
            self.ratio = CropRatio::Sides { width: height, height: width };
        }
    }

    /// Front Image (the list entry and the I key): W × H × Resolution with W and H the document's
    /// size in px and the resolution the document's in the field's unit.
    pub fn front_image(&mut self, document: &CropDocument) {
        // pc: crates/ui-egui/src/tools/crop_size.rs:211 `front_image`.
        self.ratio = CropRatio::Size;
        self.size.width = Some(CropLength::new(document.size.width, UnitKind::Pixels));
        self.size.height = Some(CropLength::new(document.size.height, UnitKind::Pixels));
        let resolution = match self.size.resolution_unit {
            ResolutionUnit::PixelsPerInch => document.resolution,
            ResolutionUnit::PixelsPerCentimetre => document.resolution / 2.54,
        };
        self.size.resolution = trim_number(resolution, 2);
    }

    /// Sets one ratio field as typed or scrubbed (`side` 0 is the width), keeping the other: a
    /// complete pair becomes [`CropRatio::Sides`] (rounded to [`RATIO_DECIMALS`]), an incomplete
    /// one a free ratio that keeps both fields.
    pub fn set_ratio_field(&mut self, side: usize, value: f64) {
        // pc: crates/ui-egui/src/tools/crop_size.rs:179 `set_custom_ratio_fields`, :161
        // `custom_ratio_fields` (what the fields show).
        let Some(field) = self.ratio_fields.get_mut(side) else { return };
        *field = if value.is_finite() { value.clamp(0.0, RATIO_MAX) } else { 0.0 };
        let [width, height] = self.ratio_fields.map(round_side);
        if width > 0.0 && height > 0.0 {
            self.ratio_fields = [width, height];
            self.ratio = CropRatio::Sides { width, height };
        } else {
            self.ratio = CropRatio::Free;
        }
    }

    /// The two numbers the ratio fields show: the ratio's sides when one is set (Original: the
    /// document's size), else [`ratio_fields`](Self::ratio_fields).
    pub fn shown_ratio_fields(&self, document: &CropDocument) -> [f64; 2] {
        // pc: crates/ui-egui/src/tools/crop_size.rs:161 `custom_ratio_fields`.
        match self.ratio {
            CropRatio::Sides { width, height } if valid_sides(width, height) => [width, height],
            CropRatio::Original if valid_sides(document.size.width, document.size.height) => [document.size.width, document.size.height],
            _ => self.ratio_fields,
        }
    }

    /// O: the next overlay, at its first orientation.
    pub fn cycle_guide(&mut self) {
        // pc: crates/ui-egui/src/tools/crop_overlay.rs:247 `cycle`.
        self.guide = self.guide.next();
        self.guide_orientation = 0;
    }

    /// ⇧O: the overlay's next orientation (nothing for an overlay with one).
    pub fn cycle_orientation(&mut self) {
        // pc: crates/ui-egui/src/tools/crop_overlay.rs:261 `cycle_orientation`.
        self.guide_orientation = self.guide.next_orientation(self.guide_orientation);
    }

    /// Picks overlay `guide`; another overlay starts at its first orientation.
    pub fn set_guide(&mut self, guide: CropGuide) {
        // pc: crates/ui-egui/src/tools/crop_overlay.rs:253 `set_overlay`.
        if self.guide != guide {
            self.guide_orientation = 0;
        }
        self.guide = guide;
    }

    /// The name New Crop Preset… suggests: "4 x 5 in 300 ppi" (one unit for both sides), "4 in x
    /// 300 px" (different units), with " 300 px/cm" for a px/cm resolution, or "2 : 3" for a ratio;
    /// `None` when there is nothing to save (Original, a free ratio, W or H empty). The number
    /// words come from `words` (`ppi`, `px/cm`).
    pub fn preset_name(&self, words: &CropBarWords, units: &Units) -> Option<String> {
        // pc: crates/ui-egui/src/tools/crop_presets.rs:85 `savable`, :78 `number`; crop_size.rs:84
        // `format_length` (the unit's decimals, at least 2, trailing zeros dropped).
        match self.ratio {
            CropRatio::Free | CropRatio::Original => None,
            CropRatio::Sides { width, height } => valid_sides(width, height).then(|| format!("{} : {}", ratio_text(width), ratio_text(height))),
            CropRatio::Size => {
                let width = CropLength::usable(self.size.width)?;
                let height = CropLength::usable(self.size.height)?;
                let with_symbol = |l: CropLength| format!("{} {}", l.number(), units.symbol(l.unit));
                let mut name = if width.unit == height.unit {
                    format!("{} x {} {}", width.number(), height.number(), units.symbol(width.unit))
                } else {
                    format!("{} x {}", with_symbol(width), with_symbol(height))
                };
                if let Some(resolution) = parse_number(&self.size.resolution) {
                    let unit = match self.size.resolution_unit {
                        ResolutionUnit::PixelsPerInch => words.ppi.as_str(),
                        ResolutionUnit::PixelsPerCentimetre => words.pixels_per_centimetre.as_str(),
                    };
                    name = format!("{name} {} {unit}", trim_number(resolution, 2));
                }
                Some(name)
            }
        }
    }
}

impl CropSizeFields {
    /// The resolution in px/in, or `None` when the field is empty or not a positive number.
    pub fn resolution_ppi(&self) -> Option<f64> {
        // pc: crates/ui-egui/src/tools/crop_size.rs:122 `resolution_ppi`.
        let value = parse_number(&self.resolution)?;
        Some(match self.resolution_unit {
            ResolutionUnit::PixelsPerInch => value,
            ResolutionUnit::PixelsPerCentimetre => value * 2.54,
        })
    }

    /// What the resolution field keeps once its typing run ends: a positive number (two
    /// decimals at most), empty, or `previous`.
    pub fn normalize_resolution(typed: &str, previous: &str) -> String {
        // pc: crates/ui-egui/src/tools/crop_size.rs:111 `normalize_resolution`.
        if typed.trim().is_empty() {
            return String::new();
        }
        match parse_number(typed).filter(|v| *v <= FIELD_MAX) {
            Some(value) => trim_number(value, 2),
            None => previous.to_owned(),
        }
    }
}
