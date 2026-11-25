//! The develop settings schema. Every field has a neutral default; serde uses `#[serde(default)]` so
//! older/newer files load (unknown fields are ignored, missing fields take defaults). The one
//! exception is `process`: missing (or unreadable), it is V1, the rendering those older files were
//! made with.

use craft_geom::{CropGeometry, Homography, Orientation, Point};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

/// The rendering process a photo's settings are interpreted by (what Lightroom calls the process
/// version). A change that would alter how existing settings render ships as a new process, so a
/// photo keeps its look until someone updates it (`develop.updateProcess`). See
/// `docs/process-versions.md`.
///
/// Stored as a plain number: one written by a newer LightCraft loads, is kept as it is, and
/// renders with the newest process this build knows ([`ProcessVersion::process`]). Reading is
/// lenient: a whole number from 0 to 2^32 - 1 (also written as `2.0`) is that process; anything
/// else (negative, fractional, too large, text, null, a list…) reads as V1, like a missing field,
/// so a damaged value never costs the rest of the settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ProcessVersion(pub u32);

impl<'de> Deserialize<'de> for ProcessVersion {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Whole(u64),
            Number(f64),
            Other(serde::de::IgnoredAny),
        }
        Ok(match Wire::deserialize(d)? {
            Wire::Whole(n) => u32::try_from(n).map_or_else(|_| ProcessVersion::legacy(), ProcessVersion),
            Wire::Number(x) if x.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&x) => ProcessVersion(x as u32),
            Wire::Number(_) | Wire::Other(_) => ProcessVersion::legacy(),
        })
    }
}

impl ProcessVersion {
    /// The first process: LightCraft's rendering from before process versions existed. Frozen.
    pub const V1: ProcessVersion = Process::V1.version();
    /// The process new photos and Reset get.
    pub const LATEST: ProcessVersion = Process::LATEST.version();

    /// The process of settings saved without one (catalogs, snapshots, XMP sidecars and browser
    /// libraries written before process versions existed): V1, whatever [`Self::LATEST`] becomes.
    pub fn legacy() -> ProcessVersion {
        ProcessVersion::V1
    }

    /// Saved settings leave the field out for V1, so files and preview-cache keys of settings made
    /// before process versions existed stay byte for byte what they were.
    pub fn is_legacy(&self) -> bool {
        *self == ProcessVersion::legacy()
    }

    /// The process this build renders with: the newest one it knows that isn't newer than this
    /// number (a number from a newer LightCraft renders with [`Process::LATEST`]; one below V1,
    /// which no LightCraft writes, with V1).
    pub fn process(self) -> Process {
        Process::ALL.into_iter().rev().find(|p| p.version() <= self).unwrap_or(Process::V1)
    }

    /// A process this build implements.
    pub fn is_known(self) -> bool {
        Process::ALL.iter().any(|p| p.version() == self)
    }

    /// Older than [`Self::LATEST`]: Update to Current Process applies.
    pub fn is_outdated(self) -> bool {
        self < ProcessVersion::LATEST
    }
}

/// A rendering process this build implements. Matches on it are exhaustive, so adding a process
/// makes the compiler point at every stage whose behaviour depends on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Process {
    /// The rendering of LightCraft before process versions existed (2026-10).
    V1,
    /// Highlights and Shadows keep the local contrast of the tones they move: their edge-aware base
    /// is taken of a fine median of log luminance (issue storytold/lightcraft#632).
    V2,
    /// Pixel-scale sharpening with working Radius and Detail controls.
    V3,
    /// V3, with Contrast and Blacks on a raw's camera look calibrated to Lightroom: the
    /// camera curve keeps V2 Whites and fitted pointwise operators follow it
    /// (`lightcraft_pipeline::light_v3`, `docs/light-v3.md`).
    V4,
    /// V4, with a raw's tone curve rolling off into white in stops past its last fitted knot, so
    /// highlights the sensor recorded keep their detail when Exposure lifts them (issue storytold/lightcraft#1632).
    V5,
    /// V5, with colour noise reduction on the linear colour differences R−Y and B−Y, leaving Y alone, with
    /// edge-aware weights from a denoised guide and a brake for isolated colour specks (issue storytold/lightcraft#815).
    V6,
    /// V6, with film grain from a stack of crystal layers (after Aurélien Pierre's model): resolution
    /// independent, fading out at black and white; V1 to V6 keep their value noise.
    V7,
    /// V7, with the Highlights/Shadows base taken of log luminance without its fine detail (clamped
    /// between a fine and a 3× wider median), so gaps in dark lettering aren't lifted into bright blocks.
    V8,
}

impl Process {
    /// Every process, oldest first.
    pub const ALL: [Process; 8] = [Process::V1, Process::V2, Process::V3, Process::V4, Process::V5, Process::V6, Process::V7, Process::V8];
    pub const LATEST: Process = Process::V8;

    pub const fn version(self) -> ProcessVersion {
        match self {
            Process::V1 => ProcessVersion(1),
            Process::V2 => ProcessVersion(2),
            Process::V3 => ProcessVersion(3),
            Process::V4 => ProcessVersion(4),
            Process::V5 => ProcessVersion(5),
            Process::V6 => ProcessVersion(6),
            Process::V7 => ProcessVersion(7),
            Process::V8 => ProcessVersion(8),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevelopSettings {
    /// Schema version of this structure (not the rendering process, see `process`).
    pub version: u32,
    /// The rendering process ([`ProcessVersion`]): the latest for new settings, V1 for settings
    /// saved without the field (the field-level default: not the struct's `Default`) or with a
    /// value that isn't a process number.
    #[serde(default = "ProcessVersion::legacy", skip_serializing_if = "ProcessVersion::is_legacy")]
    pub process: ProcessVersion,
    pub profile: Profile,
    pub treatment: Treatment,
    pub wb: WhiteBalance,
    pub light: Light,
    /// HDR editing: highlights above SDR white, and the SDR rendition derived from them. Not written while it is
    /// the default, so settings without HDR keep their JSON and `hash64` (and every cached preview stays valid).
    #[serde(skip_serializing_if = "Hdr::is_default")]
    pub hdr: Hdr,
    pub curve: ToneCurve,
    pub color: ColorAdj,
    pub mixer: Mixer,
    /// Point Color: up to [`MAX_POINT_COLORS`] sampled colours, each with its own adjustment.
    pub point_colors: Vec<PointColor>,
    pub bw_mix: BwMix,
    pub grading: ColorGrading,
    pub effects: Effects,
    pub vignette: Vignette,
    pub grain: Grain,
    pub detail: Detail,
    pub optics: Optics,
    pub geometry: Geometry,
    pub crop: Crop,
    pub orientation: Orientation,
    pub masks: Vec<Mask>,
    pub spots: Vec<Spot>,
    /// Red eye corrections; the first [`MAX_RED_EYES`] render.
    pub red_eye: Vec<RedEye>,
    pub lens_blur: LensBlur,
    pub enhance: Enhance,
    /// Camera calibration: shadows tint and primary hue/saturation (applied before tone mapping).
    pub calibration: Calibration,
    /// Section on/off toggles (the "eye" buttons on panel headers): section id → enabled.
    pub disabled_sections: Vec<String>,
}

impl Default for DevelopSettings {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            process: ProcessVersion::LATEST,
            profile: Profile::default(),
            treatment: Treatment::Color,
            wb: WhiteBalance::default(),
            light: Light::default(),
            hdr: Hdr::default(),
            curve: ToneCurve::default(),
            color: ColorAdj::default(),
            mixer: Mixer::default(),
            point_colors: Vec::new(),
            bw_mix: BwMix::default(),
            grading: ColorGrading::default(),
            effects: Effects::default(),
            vignette: Vignette::default(),
            grain: Grain::default(),
            detail: Detail::default(),
            optics: Optics::default(),
            geometry: Geometry::default(),
            crop: Crop::default(),
            orientation: Orientation::Normal,
            masks: Vec::new(),
            spots: Vec::new(),
            red_eye: Vec::new(),
            lens_blur: LensBlur::default(),
            enhance: Enhance::default(),
            calibration: Calibration::default(),
            disabled_sections: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Treatment {
    #[default]
    Color,
    Bw,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// Profile id from the profile registry (e.g. `lc.color`, `lc.vivid`, `lc.mono`).
    pub id: String,
    /// Creative profile amount 0..200 (%).
    pub amount: f64,
    /// Render the raw's own local tone mapping (DNG `ProfileGainTableMap`, Apple ProRAW), as the
    /// camera does. Off: rendered as Lightroom Classic does, without it. Omitted when off, so
    /// settings without it keep their JSON (and hash).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub camera_local_tone: bool,
}

impl Default for Profile {
    fn default() -> Self {
        Self { id: "lc.color".into(), amount: 100.0, camera_local_tone: false }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WbMode {
    #[default]
    AsShot,
    Auto,
    Daylight,
    Cloudy,
    Shade,
    Tungsten,
    Fluorescent,
    Flash,
    Custom,
}

impl WbMode {
    pub const ALL: [WbMode; 9] = [
        WbMode::AsShot,
        WbMode::Auto,
        WbMode::Daylight,
        WbMode::Cloudy,
        WbMode::Shade,
        WbMode::Tungsten,
        WbMode::Fluorescent,
        WbMode::Flash,
        WbMode::Custom,
    ];
    pub fn label(self) -> &'static str {
        match self {
            WbMode::AsShot => "As Shot",
            WbMode::Auto => "Auto",
            WbMode::Daylight => "Daylight",
            WbMode::Cloudy => "Cloudy",
            WbMode::Shade => "Shade",
            WbMode::Tungsten => "Tungsten",
            WbMode::Fluorescent => "Fluorescent",
            WbMode::Flash => "Flash",
            WbMode::Custom => "Custom",
        }
    }
    /// Preset temperature/tint for raw files with a Kelvin scale (K, tint): the CIE daylight series D55 / D65 /
    /// D75 (5503, 6504, 7504 K; CIE 15:2004) for Daylight / Cloudy / Shade, CIE Illuminant A (2856 K) for
    /// Tungsten, a typical cool-white fluorescent (CIE F2, 4230 K, is the lamp; 3800 K with a magenta correction
    /// is what a camera's preset neutralises) and a flash at daylight; the tints are the small magenta
    /// corrections daylight-series whites need above the Planckian locus, as Lightroom's presets of the same
    /// names show them in its panel (observed values, no Adobe data). On the relative scale (rendered files, raws
    /// without matrices) `develop.wb` scales them around 6500 K = as shot.
    pub fn preset(self) -> Option<(f64, f64)> {
        match self {
            WbMode::Daylight => Some((5500.0, 10.0)),
            WbMode::Cloudy => Some((6500.0, 10.0)),
            WbMode::Shade => Some((7500.0, 10.0)),
            WbMode::Tungsten => Some((2850.0, 0.0)),
            WbMode::Fluorescent => Some((3800.0, 21.0)),
            WbMode::Flash => Some((5500.0, 0.0)),
            _ => None,
        }
    }
}

/// Which scale a [`WhiteBalance`]'s `temp` / `tint` were written on.
///
/// Raws whose camera has colour matrices (in the file, or measured for the model) edit white balance in Kelvin;
/// before that, raws without matrices in the file were all edited relative to their as-shot look, where 6500 K / 0
/// means *as shot*. Settings written then carry no scale and keep that meaning (`Legacy`): the pipeline turns them
/// into the same colour change on the Kelvin scale (`lightcraft_pipeline::local::legacy_to_kelvin`), so no photo's
/// look changes. Settings written on the Kelvin scale say so (`Kelvin`). The scale is irrelevant on files the
/// Kelvin scale does not reach (rendered photos, raws without matrices): their values keep the relative meaning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WbScale {
    /// Written before the scale existed, or by a preset / partial JSON without one: relative (6500 K / 0 = as shot)
    /// on a raw developed from a fitted look, Kelvin on a raw whose file carries matrices. Not written.
    #[default]
    Legacy,
    /// Absolute Kelvin and tint, as Lightroom writes them for raws.
    Kelvin,
}

impl WbScale {
    fn is_legacy(&self) -> bool {
        *self == WbScale::Legacy
    }
}

/// White balance. For raw files with colour matrices `temp` is the scene illuminant in Kelvin (`scale` says the
/// values were written on that scale); for rendered files (JPEG etc.) and raws without matrices the image is
/// assumed D65-balanced and `temp`/`tint` are the same model with an "as shot" of 6500 K / 0 (the UI shows a
/// relative −100..100 scale for those, like Lightroom).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WhiteBalance {
    pub mode: WbMode,
    pub temp: f64,
    /// Correction direction: negative adds green, positive adds magenta.
    pub tint: f64,
    /// The scale `temp` / `tint` were written on; left out of the JSON when [`WbScale::Legacy`], so settings
    /// from before it existed read and write unchanged.
    #[serde(skip_serializing_if = "WbScale::is_legacy")]
    pub scale: WbScale,
}

impl Default for WhiteBalance {
    fn default() -> Self {
        // (`Legacy`, like an absent field: a default photo's settings JSON stays what it was; the engine marks the
        // scale when white balance is written on a raw that has one, `lightcraft_pipeline::local::normalize_wb`)
        Self { mode: WbMode::AsShot, temp: 6500.0, tint: 0.0, scale: WbScale::Legacy }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Light {
    pub exposure: f64,
    pub contrast: f64,
    pub highlights: f64,
    pub shadows: f64,
    pub whites: f64,
    pub blacks: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneCurve {
    pub highlights: f64,
    pub lights: f64,
    pub darks: f64,
    pub shadows: f64,
    /// Region split points (percent), Lightroom defaults 25/50/75.
    pub split_shadows: f64,
    pub split_mid: f64,
    pub split_highlights: f64,
    /// Point curves in 0..1 (identity = empty or [(0,0),(1,1)]).
    pub master: Vec<Point>,
    pub red: Vec<Point>,
    pub green: Vec<Point>,
    pub blue: Vec<Point>,
    /// Refine Saturation 0..100: 100 keeps the saturation a curve produces, lower values pull it
    /// back towards the saturation before the curve (strong contrast curves oversaturate).
    pub refine_saturation: f64,
}

impl Default for ToneCurve {
    fn default() -> Self {
        Self {
            highlights: 0.0,
            lights: 0.0,
            darks: 0.0,
            shadows: 0.0,
            split_shadows: 25.0,
            split_mid: 50.0,
            split_highlights: 75.0,
            master: Vec::new(),
            red: Vec::new(),
            green: Vec::new(),
            blue: Vec::new(),
            refine_saturation: 100.0,
        }
    }
}

impl ToneCurve {
    pub fn point_curve_is_identity(pts: &[Point]) -> bool {
        pts.is_empty() || pts.iter().all(|p| (p.x - p.y).abs() < 1e-9)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorAdj {
    pub vibrance: f64,
    pub saturation: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hsl {
    pub hue: f64,
    pub sat: f64,
    pub lum: f64,
}

/// The 8 colour-mixer bands in Lightroom order.
pub const MIXER_BANDS: [&str; 8] = ["red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta"];
/// Centre hue of each band in degrees (sRGB-ish HSV hue).
pub const MIXER_HUES: [f64; 8] = [0.0, 30.0, 60.0, 120.0, 180.0, 225.0, 270.0, 315.0];

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mixer {
    pub red: Hsl,
    pub orange: Hsl,
    pub yellow: Hsl,
    pub green: Hsl,
    pub aqua: Hsl,
    pub blue: Hsl,
    pub purple: Hsl,
    pub magenta: Hsl,
}

impl Mixer {
    pub fn bands(&self) -> [Hsl; 8] {
        [self.red, self.orange, self.yellow, self.green, self.aqua, self.blue, self.purple, self.magenta]
    }
    pub fn is_neutral(&self) -> bool {
        self.bands().iter().all(|b| *b == Hsl::default())
    }
}

/// Most Point Color samples per photo (Lightroom's limit).
pub const MAX_POINT_COLORS: usize = 8;

/// One Point Color sample: a colour picked on the image (OkLCh, at the stage where Point Color
/// applies: after the colour mixer) and the adjustment of the colours around it. The range is a
/// box in OkLCh around the sample, widened by `range` (overall) and the per-axis widths, with a
/// soft edge.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PointColor {
    /// Sampled OkLab lightness (0..1).
    pub lum: f64,
    /// Sampled OkLCh chroma (0..~0.37).
    pub chroma: f64,
    /// Sampled OkLCh hue, degrees.
    pub hue: f64,
    /// Hue shift −100..100 (±~29° at the sample).
    pub hue_shift: f64,
    /// Saturation shift −100..100 (chroma scale).
    pub sat_shift: f64,
    /// Luminance shift −100..100.
    pub lum_shift: f64,
    /// −100 compresses the colour spread within the range towards the sample (evens out e.g.
    /// skin), +100 expands it.
    pub variance: f64,
    /// Overall range 0..100 (scales all three widths).
    pub range: f64,
    pub hue_range: f64,
    pub sat_range: f64,
    pub lum_range: f64,
}

impl Default for PointColor {
    fn default() -> Self {
        Self {
            lum: 0.5,
            chroma: 0.1,
            hue: 0.0,
            hue_shift: 0.0,
            sat_shift: 0.0,
            lum_shift: 0.0,
            variance: 0.0,
            range: 50.0,
            hue_range: 50.0,
            sat_range: 50.0,
            lum_range: 50.0,
        }
    }
}

impl PointColor {
    /// True when the sample changes nothing.
    pub fn is_neutral(&self) -> bool {
        self.hue_shift == 0.0 && self.sat_shift == 0.0 && self.lum_shift == 0.0 && self.variance == 0.0
    }
}

/// B&W mix (used when treatment = Bw): per-band luminance contribution −100..100.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BwMix {
    pub red: f64,
    pub orange: f64,
    pub yellow: f64,
    pub green: f64,
    pub aqua: f64,
    pub blue: f64,
    pub purple: f64,
    pub magenta: f64,
}

impl BwMix {
    pub fn bands(&self) -> [f64; 8] {
        [self.red, self.orange, self.yellow, self.green, self.aqua, self.blue, self.purple, self.magenta]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Wheel {
    /// Hue in degrees 0..360.
    pub hue: f64,
    /// Saturation 0..100.
    pub sat: f64,
    /// Luminance −100..100.
    pub lum: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorGrading {
    pub shadows: Wheel,
    pub midtones: Wheel,
    pub highlights: Wheel,
    pub global: Wheel,
    /// 0..100, default 50.
    pub blending: f64,
    /// −100..100.
    pub balance: f64,
}

impl Default for ColorGrading {
    fn default() -> Self {
        Self {
            shadows: Wheel::default(),
            midtones: Wheel::default(),
            highlights: Wheel::default(),
            global: Wheel::default(),
            blending: 50.0,
            balance: 0.0,
        }
    }
}

impl ColorGrading {
    pub fn is_neutral(&self) -> bool {
        [self.shadows, self.midtones, self.highlights, self.global].iter().all(|w| w.sat == 0.0 && w.lum == 0.0)
    }
}

/// Camera calibration (Lightroom Classic's Calibration panel): a green/magenta tint of the shadows,
/// and a hue rotation / saturation scale of each working-space primary (−100..100), i.e. a
/// white-preserving 3×3 matrix applied in scene-linear light before tone mapping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Calibration {
    pub shadows_tint: f64,
    pub red_hue: f64,
    pub red_sat: f64,
    pub green_hue: f64,
    pub green_sat: f64,
    pub blue_hue: f64,
    pub blue_sat: f64,
}

impl Calibration {
    /// (hue, saturation) of the red, green and blue primaries.
    pub fn primaries(&self) -> [(f64, f64); 3] {
        [(self.red_hue, self.red_sat), (self.green_hue, self.green_sat), (self.blue_hue, self.blue_sat)]
    }
    pub fn is_neutral(&self) -> bool {
        *self == Calibration::default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Effects {
    pub texture: f64,
    pub clarity: f64,
    pub dehaze: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VignetteStyle {
    #[default]
    HighlightPriority,
    ColorPriority,
    PaintOverlay,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Vignette {
    pub amount: f64,
    pub midpoint: f64,
    pub roundness: f64,
    pub feather: f64,
    pub highlights: f64,
    pub style: VignetteStyle,
}

impl Default for Vignette {
    fn default() -> Self {
        Self { amount: 0.0, midpoint: 50.0, roundness: 0.0, feather: 50.0, highlights: 0.0, style: VignetteStyle::HighlightPriority }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grain {
    pub amount: f64,
    pub size: f64,
    pub roughness: f64,
    /// Seed for the procedural grain (stable per photo).
    pub seed: u32,
}

impl Default for Grain {
    fn default() -> Self {
        Self { amount: 0.0, size: 25.0, roughness: 50.0, seed: 0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Detail {
    pub sharpen_amount: f64,
    pub sharpen_radius: f64,
    pub sharpen_detail: f64,
    pub sharpen_masking: f64,
    pub nr_luminance: f64,
    pub nr_detail: f64,
    pub nr_contrast: f64,
    pub nr_color: f64,
    pub nr_color_detail: f64,
    pub nr_color_smoothness: f64,
    /// Replace single outlier photosites (noise spikes far above all their same-colour neighbours) in the
    /// raw mosaic before demosaicing, so they do not become coloured dots (storytold/lightcraft#1345). On for newly imported
    /// raws ([`crate::DevelopSettings::for_raw`]); off for edits saved before it existed, so they look the
    /// same. Raw files only. Omitted when off, so settings saved before it keep their JSON (and hash, and with it
    /// their cached thumbnails).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub nr_outliers: bool,
}

impl Default for Detail {
    /// Lightroom's raw defaults are sharpening 40/1.0/25/0 and colour NR 25/50/50; we use neutral
    /// values for rendered files and apply raw defaults at import time (`DevelopSettings::for_raw`).
    fn default() -> Self {
        Self {
            sharpen_amount: 0.0,
            sharpen_radius: 1.0,
            sharpen_detail: 25.0,
            sharpen_masking: 0.0,
            nr_luminance: 0.0,
            nr_detail: 50.0,
            nr_contrast: 0.0,
            nr_color: 0.0,
            nr_color_detail: 50.0,
            nr_color_smoothness: 50.0,
            nr_outliers: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Optics {
    pub remove_ca: bool,
    pub lens_profile: bool,
    pub profile_distortion: f64,
    pub profile_vignetting: f64,
    pub defringe_purple_amount: f64,
    pub defringe_purple_hue_lo: f64,
    pub defringe_purple_hue_hi: f64,
    pub defringe_green_amount: f64,
    pub defringe_green_hue_lo: f64,
    pub defringe_green_hue_hi: f64,
    pub distortion: f64,
    pub vignetting: f64,
    pub vignetting_midpoint: f64,
    /// Manual lateral chromatic aberration (red/cyan, blue/yellow fringes), −100..100, added to the
    /// automatic estimate when `remove_ca` is on.
    pub ca_red: f64,
    pub ca_blue: f64,
}

impl Default for Optics {
    fn default() -> Self {
        Self {
            remove_ca: false,
            lens_profile: false,
            profile_distortion: 100.0,
            profile_vignetting: 100.0,
            defringe_purple_amount: 0.0,
            defringe_purple_hue_lo: 30.0,
            defringe_purple_hue_hi: 70.0,
            defringe_green_amount: 0.0,
            defringe_green_hue_lo: 40.0,
            defringe_green_hue_hi: 60.0,
            distortion: 0.0,
            vignetting: 0.0,
            vignetting_midpoint: 50.0,
            ca_red: 0.0,
            ca_blue: 0.0,
        }
    }
}

/// Lens corrections embedded in the file, converted to the oriented, default-cropped image: a DNG's `OpcodeList3`
/// (`WarpRectilinear`, `FixVignetteRadial`), or a raw reader's equivalent of the camera's own correction (Panasonic /
/// Leica RW2 distortion, `lightcraft_raw`'s `vendor/rw2.rs`). This is camera/file data (stored on the photo record, not in the develop
/// settings); "Enable Profile Corrections" applies it, scaled by the profile distortion/vignetting amounts.
/// LightCraft never uses Adobe LCP lens profiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbeddedLens {
    pub warp: Option<EmbeddedWarp>,
    pub vignette: Option<EmbeddedVignette>,
    /// Where the correction came from: the file itself, or a lens database looked up by the
    /// lens named in the file (what the Optics panel says is loaded).
    #[serde(skip_serializing_if = "LensSource::is_file")]
    pub source: LensSource,
    /// A DNG 1.6 `WarpRectilinear2` chain (up to [`WARP_STAGES`] stages, in `OpcodeList3` order; each stage warps
    /// the result of the one before it). When any is set, `warp` is `None`: the chain holds every warp opcode.
    pub stages: [Option<EmbeddedWarpStage>; WARP_STAGES],
}

/// Where an [`EmbeddedLens`] came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LensSource {
    /// The file's own correction (DNG opcodes, a camera's correction a raw reader converts).
    #[default]
    File,
    /// A profile from a lensfun-format lens database, matched by the lens named in the file.
    Database,
}

impl LensSource {
    pub fn is_file(&self) -> bool {
        *self == LensSource::File
    }
}

/// Most warp stages an [`EmbeddedLens`] chain keeps.
pub const WARP_STAGES: usize = 3;

/// One DNG `WarpRectilinear2` (or a `WarpRectilinear` in a chain with one): per plane (R, G, B)
/// `[kr0 .. kr14, kt0, kt1, min_valid_radius, max_valid_radius]`. For an output point at offset
/// `d = (p − center) / radius` with `r = |d|` and `f = Σ krₚ·clamp(r, min, max)ᵖ`, the source point is
/// `center + radius · (d·f + tangential(d))`, or `d / f` for the radial part when `reciprocal`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EmbeddedWarpStage {
    pub planes: [[f64; 19]; 3],
    /// Optical centre, normalized to the oriented image (0..1).
    pub center: Point,
    /// Normalisation radius as a fraction of the oriented image's long edge.
    pub radius: f64,
    pub reciprocal: bool,
}

impl EmbeddedLens {
    /// Whether a `WarpRectilinear2` chain is set.
    pub fn has_stages(&self) -> bool {
        self.stages.iter().any(Option::is_some)
    }
}

/// DNG `WarpRectilinear`: per plane (R, G, B) `[kr0, kr1, kr2, kr3, kt0, kt1]`. For an output (corrected) point at
/// offset `d = (p − center) / radius`, the source point is `center + radius · (d·f(r²) + tangential(d))`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EmbeddedWarp {
    pub planes: [[f64; 6]; 3],
    /// Optical centre, normalized to the oriented image (0..1).
    pub center: Point,
    /// Normalisation radius as a fraction of the oriented image's long edge.
    pub radius: f64,
}

/// DNG `FixVignetteRadial`: gain `1 + k0 r² + k1 r⁴ + … + k4 r¹⁰` with `r = |p − center| / radius`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EmbeddedVignette {
    pub k: [f64; 5],
    pub center: Point,
    pub radius: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Upright {
    #[default]
    Off,
    Auto,
    Guided,
    Level,
    Vertical,
    Full,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Geometry {
    pub upright: Upright,
    /// Guided Upright guides, up to 4 line segments, in normalized coordinates of the lens-corrected
    /// (pre-perspective) oriented image.
    pub guides: Vec<(Point, Point)>,
    /// The automatic Upright correction found by analysis for `upright` (Auto/Level/Vertical/Full), as a
    /// homography lens-corrected → transformed in centred coordinates (`(p − centre) / (long edge / 2)`).
    /// Stored so previews and exports use the identical transform; recomputed by `geometry.upright`.
    pub upright_transform: Option<Homography>,
    pub vertical: f64,
    pub horizontal: f64,
    pub rotate: f64,
    pub aspect: f64,
    pub scale: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    pub constrain_crop: bool,
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            upright: Upright::Off,
            guides: Vec::new(),
            upright_transform: None,
            vertical: 0.0,
            horizontal: 0.0,
            rotate: 0.0,
            aspect: 0.0,
            scale: 100.0,
            offset_x: 0.0,
            offset_y: 0.0,
            constrain_crop: false,
        }
    }
}

impl Geometry {
    pub fn is_identity(&self) -> bool {
        self.upright == Upright::Off
            && self.vertical == 0.0
            && self.horizontal == 0.0
            && self.rotate == 0.0
            && self.aspect == 0.0
            && self.scale == 100.0
            && self.offset_x == 0.0
            && self.offset_y == 0.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Crop {
    pub geometry: CropGeometry,
    /// Locked aspect (w, h); None = free / original.
    pub aspect: Option<(u32, u32)>,
    pub flip_h: bool,
    pub flip_v: bool,
}

// ---------------------------------------------------------------------------------------------
// Masks and local adjustments

/// The sliders available inside a mask (Lightroom's local adjustments).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalAdjustments {
    pub temp: f64,
    pub tint: f64,
    pub exposure: f64,
    pub contrast: f64,
    pub highlights: f64,
    pub shadows: f64,
    pub whites: f64,
    pub blacks: f64,
    pub texture: f64,
    pub clarity: f64,
    pub dehaze: f64,
    pub hue: f64,
    pub saturation: f64,
    pub sharpness: f64,
    pub noise: f64,
    pub moire: f64,
    pub defringe: f64,
    /// Lens Blur inside the mask, 0..100 (`lightcraft_pipeline::lens_blur`): disc (bokeh) blur
    /// whose radius follows the mask's strength. Omitted while 0, so masks without it keep their
    /// JSON (and hash).
    #[serde(skip_serializing_if = "is_zero")]
    pub lens_blur: f64,
    /// The mask's Lens Blur block switched off (its values kept).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub lens_blur_off: bool,
    /// Blur what the mask leaves out instead of what it selects (the mask's other adjustments
    /// still go where it selects).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub lens_blur_outside: bool,
    /// Brighter bokeh highlights in the mask's blur, 0..100.
    #[serde(skip_serializing_if = "is_zero")]
    pub lens_blur_boost: f64,
    /// How gradually the mask's blur grows away from what stays sharp, 0..100 (0: it follows the
    /// mask's own edge).
    #[serde(skip_serializing_if = "is_zero")]
    pub lens_blur_transition: f64,
    /// Colour overlay (hue degrees, saturation 0..100); sat 0 = none.
    pub color_hue: f64,
    pub color_sat: f64,
    /// Local vignette/grain are not in Lightroom; we keep the struct extensible.
    pub amount: f64,
}

impl Default for LocalAdjustments {
    fn default() -> Self {
        Self {
            temp: 0.0,
            tint: 0.0,
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            texture: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            hue: 0.0,
            saturation: 0.0,
            sharpness: 0.0,
            noise: 0.0,
            moire: 0.0,
            defringe: 0.0,
            lens_blur: 0.0,
            lens_blur_off: false,
            lens_blur_outside: false,
            lens_blur_boost: 0.0,
            lens_blur_transition: 0.0,
            color_hue: 0.0,
            color_sat: 0.0,
            amount: 100.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MaskOp {
    #[default]
    Add,
    Subtract,
    Intersect,
}

/// One brush dab sequence. Coordinates normalized to the uncropped oriented image.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrushStroke {
    pub points: Vec<Point>,
    /// Radius as a fraction of the image's long edge.
    pub size: f64,
    /// 0..100.
    pub feather: f64,
    pub flow: f64,
    pub density: f64,
    pub erase: bool,
    pub auto_mask: bool,
    /// Pen pressure (0..1) at each of `points`, recorded when the brush's pressure switch is on: it scales each dab's
    /// radius and flow. Empty (the default, a mouse or the switch off) is full pressure everywhere; a point past the
    /// end of a shorter list takes the last value.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pressure: Vec<f32>,
}

impl BrushStroke {
    /// The pressure at point `i` (0..1; 1 without pressure data, a hostile value clamped, NaN as 1).
    pub fn pressure_at(&self, i: usize) -> f32 {
        let p = self.pressure.get(i).or(self.pressure.last()).copied().unwrap_or(1.0);
        if p.is_nan() { 1.0 } else { p.clamp(0.0, 1.0) }
    }
}

impl Default for BrushStroke {
    fn default() -> Self {
        Self { points: Vec::new(), size: 0.03, feather: 50.0, flow: 100.0, density: 100.0, erase: false, auto_mask: false, pressure: Vec::new() }
    }
}

/// Most samples a colour-range mask keeps. It compares every pixel against every sample, so render and export cost
/// is pixels x samples; the interactive sampler stops at 5, and a sidecar, preset or `mask.add` with thousands could
/// stall a render or lose the GPU device (issue storytold/lightcraft#2556).
pub const MAX_COLOR_RANGE_SAMPLES: usize = 64;

/// Deserialize a colour range's samples, keeping the first [`MAX_COLOR_RANGE_SAMPLES`] (the rest are skipped without
/// being stored), however the settings arrive: sidecar, catalog, history, preset or merge.
fn de_color_range_samples<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<[f64; 3]>, D::Error> {
    struct Capped;
    impl<'de> serde::de::Visitor<'de> for Capped {
        type Value = Vec<[f64; 3]>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a list of OkLab colours")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<[f64; 3]>, A::Error> {
            let mut out = Vec::new();
            while out.len() < MAX_COLOR_RANGE_SAMPLES {
                match seq.next_element::<[f64; 3]>()? {
                    Some(s) => out.push(s),
                    None => return Ok(out),
                }
            }
            while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
            Ok(out)
        }
    }
    d.deserialize_seq(Capped)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MaskShape {
    Brush {
        strokes: Vec<BrushStroke>,
    },
    /// Linear gradient: full effect at `start`, fading to none at `end` (normalized coords).
    Linear {
        start: Point,
        end: Point,
    },
    /// Radial gradient: ellipse centre, radii (normalized to the long edge), rotation (deg), feather 0..100.
    Radial {
        center: Point,
        rx: f64,
        ry: f64,
        angle: f64,
        feather: f64,
        invert: bool,
    },
    /// Colour range: sampled OkLab colours (at most [`MAX_COLOR_RANGE_SAMPLES`]) with a refine amount 0..100.
    ColorRange {
        #[serde(deserialize_with = "de_color_range_samples")]
        samples: Vec<[f64; 3]>,
        refine: f64,
    },
    /// Luminance range (0..1 bounds with feathered falloff 0..1).
    LuminanceRange {
        lo: f64,
        hi: f64,
        lo_feather: f64,
        hi_feather: f64,
    },
    DepthRange {
        lo: f64,
        hi: f64,
        feather: f64,
    },
    /// AI / automatic selections, evaluated by a segmenter.
    Subject,
    Sky,
    Background,
    /// One object picked by clicks (SAM 3 point prompts): `hint` holds the clicks that include,
    /// `exclude` the ones that exclude; `seg` the segmentation computed from them.
    Object {
        hint: Vec<Point>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        exclude: Vec<Point>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seg: Option<crate::SegMask>,
        /// Zoomed-in passes over parts of the image (higher resolution than `seg`), used
        /// inside their rectangles.
        #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "crate::segmask::de_detail")]
        detail: Vec<crate::SegMask>,
        /// Edge −100..100: below 0 harder (a steeper transition), above 0 softer (feathered).
        #[serde(default, skip_serializing_if = "is_zero")]
        edge: f64,
    },
    /// Everything a description names ("sky", "the red car": SAM 3 concept prompts); `seg` is
    /// the segmentation computed for it.
    Prompt {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seg: Option<crate::SegMask>,
        #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "crate::segmask::de_detail")]
        detail: Vec<crate::SegMask>,
        /// Edge −100..100, as for `Object`.
        #[serde(default, skip_serializing_if = "is_zero")]
        edge: f64,
    },
    People {
        person: u32,
        parts: Vec<String>,
    },
    Landscape {
        class: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskComponent {
    /// A name given in the Masking panel (Rename); `None` shows the shape's kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub op: MaskOp,
    #[serde(default)]
    pub invert: bool,
    pub shape: MaskShape,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mask {
    pub id: u32,
    pub name: String,
    pub visible: bool,
    pub invert: bool,
    pub components: Vec<MaskComponent>,
    pub adjust: LocalAdjustments,
    /// Refine Edges 0..100: the mask's edges snap to the photo's (guided filter).
    #[serde(skip_serializing_if = "is_zero")]
    pub refine: f64,
    /// Where its coordinates (gradients, brush strokes, AI selections) live: `true`, on the photo as
    /// shot (user orientation, before lens corrections, Upright and Transform), so the mask stays
    /// on the same content whatever those do; `false`, a mask from before that, on the corrected
    /// frame. Read as `false` when absent; [`Mask::default`] (every new mask) is `true`, and an
    /// edit converts an old one (`lightcraft_pipeline::mask_space::attach`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub on_photo: bool,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

fn is_false(v: &bool) -> bool {
    !*v
}

impl Default for Mask {
    fn default() -> Self {
        Self {
            id: 0,
            name: "Mask 1".into(),
            visible: true,
            invert: false,
            components: Vec::new(),
            adjust: LocalAdjustments::default(),
            refine: 0.0,
            on_photo: true,
        }
    }
}

impl DevelopSettings {
    /// Calls `f` for every segmentation grid stored in these settings (the `seg` and `detail`
    /// of Object and Prompt mask shapes).
    pub fn visit_seg_masks(&self, f: &mut impl FnMut(&crate::SegMask)) {
        for mask in &self.masks {
            for component in &mask.components {
                visit_shape(&component.shape, f);
            }
        }
    }

    /// [`DevelopSettings::visit_seg_masks`] with mutable access (resolving references into a
    /// snapshot's grid table).
    pub fn visit_seg_masks_mut(&mut self, f: &mut impl FnMut(&mut crate::SegMask)) {
        for mask in &mut self.masks {
            for component in &mut mask.components {
                visit_shape_mut(&mut component.shape, f);
            }
        }
    }
}

fn visit_shape(shape: &MaskShape, f: &mut impl FnMut(&crate::SegMask)) {
    match shape {
        MaskShape::Object { seg, detail, .. } => {
            if let Some(seg) = seg {
                f(seg);
            }
            for d in detail {
                f(d);
            }
        }
        MaskShape::Prompt { seg, detail, .. } => {
            if let Some(seg) = seg {
                f(seg);
            }
            for d in detail {
                f(d);
            }
        }
        _ => {}
    }
}

fn visit_shape_mut(shape: &mut MaskShape, f: &mut impl FnMut(&mut crate::SegMask)) {
    match shape {
        MaskShape::Object { seg, detail, .. } => {
            if let Some(seg) = seg.as_mut() {
                f(seg);
            }
            for d in detail {
                f(d);
            }
        }
        MaskShape::Prompt { seg, detail, .. } => {
            if let Some(seg) = seg.as_mut() {
                f(seg);
            }
            for d in detail {
                f(d);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------------
// Heal / remove / red eye

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpotMode {
    #[default]
    Remove,
    Heal,
    Clone,
    /// Content-aware: the spot is rebuilt from patches of its surroundings (no single source).
    Fill,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Spot {
    pub mode: SpotMode,
    /// Target: a single point (circle spot) or a brushed path. Normalized coords.
    pub points: Vec<Point>,
    /// Radius as a fraction of the long edge.
    pub size: f64,
    pub feather: f64,
    pub opacity: f64,
    /// Source offset (normalized) from the target; None = auto (content-aware remove).
    pub source_offset: Option<Point>,
}

impl Default for Spot {
    fn default() -> Self {
        Self { mode: SpotMode::Remove, points: Vec::new(), size: 0.02, feather: 50.0, opacity: 100.0, source_offset: None }
    }
}

/// Most red eye / pet eye corrections a photo renders. Every output pixel goes through every eye, so render cost is
/// pixels x eyes; a sidecar, preset or agent with thousands could stall a render for hours (issue storytold/lightcraft#2556).
pub const MAX_RED_EYES: usize = 64;

/// A red eye / pet eye correction: the user's ellipse (centre normalized, radii as fractions of
/// the long edge); the pupil inside it is found automatically.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RedEye {
    pub center: Point,
    pub rx: f64,
    pub ry: f64,
    /// 0..100 (50 = the detected pupil).
    pub pupil_size: f64,
    /// 0..100.
    pub darken: f64,
    pub pet: bool,
    /// Pet eye catchlight: offset of its centre from the pupil centre, in pupil radii.
    #[serde(default)]
    pub catchlight: Option<Point>,
}

impl Default for RedEye {
    fn default() -> Self {
        Self { center: Point::new(0.5, 0.5), rx: 0.02, ry: 0.015, pupil_size: 50.0, darken: 50.0, pet: false, catchlight: None }
    }
}

/// Lens Blur: a shallow depth of field added after the fact (`lightcraft_pipeline::lens_blur`).
/// What stays sharp is the photo's subject, or the selection of the mask [`LensBlur::focus_mask`]
/// names; everything else gets disc-shaped (bokeh) blur up to [`LensBlur::amount`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LensBlur {
    pub enabled: bool,
    /// Blur of what is out of focus, 0..100 (100: a disc of 3.5 % of the long edge).
    pub amount: f64,
    /// Reserved for a depth map (not rendered yet).
    pub focal_distance: f64,
    /// Reserved for a depth map (not rendered yet).
    pub focal_range: f64,
    /// How much brighter the out-of-focus highlights turn into bokeh discs, 0..100. Omitted while
    /// 0, so settings without it keep their JSON (and hash).
    #[serde(skip_serializing_if = "is_zero")]
    pub boost: f64,
    /// The id of the mask whose selection stays in focus (any mask, shown or hidden); `None`, or a
    /// mask that no longer exists: the photo's subject.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus_mask: Option<u32>,
    /// Brush Refinement, Focus brush: strokes painted in focus (their strength: how far; erase
    /// strokes take it back). Omitted while empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub focus_strokes: Vec<BrushStroke>,
    /// Brush Refinement, Blur brush: strokes painted out of focus, over the focus and the Focus
    /// brush. Omitted while empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blur_strokes: Vec<BrushStroke>,
    /// How gradually the blur grows away from what is in focus, 0..100: 0 a sharp cut at the
    /// focus's edge, higher a longer ramp (a stand-in for depth until there is a depth map).
    /// Omitted at its default, so settings without it keep their JSON (and hash).
    #[serde(skip_serializing_if = "LensBlur::is_default_transition")]
    pub transition: f64,
    /// What stays in focus without a focus mask: the subject estimate, or a fixed shape in the
    /// middle of the framing. Omitted while automatic.
    #[serde(skip_serializing_if = "FocusShape::is_auto")]
    pub shape: FocusShape,
    /// Where a fixed shape sits and how large it is, once moved or resized on the photo; `None`:
    /// the shape's preset, in the middle of the crop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<FocusArea>,
}

/// A fixed focus shape on the photo: an upright ellipse.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FocusArea {
    /// Normalized (uncropped, oriented) photo coordinates, as mask shapes.
    pub center: Point,
    /// Half-width and half-height, in units of the photo's long edge.
    pub rx: f64,
    pub ry: f64,
}

impl FocusArea {
    /// Smallest and largest radius (long-edge units).
    pub const MIN_RADIUS: f64 = 0.02;
    pub const MAX_RADIUS: f64 = 1.0;

    /// Usable values: finite, the radii within range, the centre no farther than a photo's width
    /// outside it. A hostile value falls back to the middle and the default size.
    pub fn sanitized(self) -> FocusArea {
        let r = |v: f64, d: f64| if v.is_finite() { v.clamp(Self::MIN_RADIUS, Self::MAX_RADIUS) } else { d };
        let c = |v: f64| if v.is_finite() { v.clamp(-1.0, 2.0) } else { 0.5 };
        FocusArea { center: Point::new(c(self.center.x), c(self.center.y)), rx: r(self.rx, 0.22), ry: r(self.ry, 0.22) }
    }
}

impl LensBlur {
    /// The fixed focus shape in effect, if any: the moved / resized [`LensBlur::area`], else the
    /// shape's preset centred on `crop_center`. `None` for [`FocusShape::Auto`].
    pub fn focus_area(&self, crop_center: Point) -> Option<FocusArea> {
        let (rx, ry) = self.shape.radii()?;
        Some(self.area.unwrap_or(FocusArea { center: crop_center, rx, ry }).sanitized())
    }
}

/// The focus area [`LensBlur`] uses when no mask says what is in focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FocusShape {
    /// A circle.
    Round,
    /// An oval lying down (landscape).
    Wide,
    /// An oval standing up (portrait).
    Tall,
    /// The photo's subject, estimated (person mattes, else a saliency heuristic). Also what a
    /// shape this build doesn't know reads as (so it is last: serde's `other`).
    #[default]
    #[serde(other)]
    Auto,
}

impl FocusShape {
    pub const ALL: [FocusShape; 4] = [FocusShape::Auto, FocusShape::Round, FocusShape::Wide, FocusShape::Tall];

    /// Whether this is the default (Auto: no fixed shape), for skipping it in the JSON.
    fn is_auto(&self) -> bool {
        *self == FocusShape::Auto
    }

    /// The fixed shape's half-width and half-height, in units of the photo's long edge; `None`
    /// for [`FocusShape::Auto`].
    pub fn radii(self) -> Option<(f64, f64)> {
        match self {
            FocusShape::Auto => None,
            FocusShape::Round => Some((0.22, 0.22)),
            FocusShape::Wide => Some((0.32, 0.17)),
            FocusShape::Tall => Some((0.17, 0.32)),
        }
    }

    /// The name shown on the shape's button (translated by the UI).
    pub fn label(self) -> &'static str {
        match self {
            FocusShape::Auto => "Auto",
            FocusShape::Round => "Round",
            FocusShape::Wide => "Horizontal",
            FocusShape::Tall => "Vertical",
        }
    }
}

impl LensBlur {
    /// [`LensBlur::transition`]'s default.
    pub const DEFAULT_TRANSITION: f64 = 50.0;

    /// Whether `v` is [`Self::DEFAULT_TRANSITION`], for skipping it in the JSON.
    fn is_default_transition(v: &f64) -> bool {
        *v == Self::DEFAULT_TRANSITION
    }
}

impl Default for LensBlur {
    /// Off, with the default Transition and everything else empty.
    fn default() -> Self {
        Self {
            enabled: false,
            amount: 0.0,
            focal_distance: 0.0,
            focal_range: 0.0,
            boost: 0.0,
            focus_mask: None,
            focus_strokes: Vec::new(),
            blur_strokes: Vec::new(),
            transition: Self::DEFAULT_TRANSITION,
            shape: FocusShape::Auto,
            area: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Enhance {
    pub denoise: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denoise_on: Option<bool>,
    pub raw_details: bool,
    pub super_resolution: bool,
}

impl Enhance {
    pub fn denoise_enabled(&self) -> bool {
        self.denoise_on.unwrap_or(self.denoise > 0.0)
    }
}

/// HDR editing. When enabled, an HDR render ([`Hdr::peak`]) keeps highlights above SDR white (1.0)
/// up to `2^max_ev`; every SDR render (previews on SDR displays, 8/16-bit exports and a gain map's
/// base image) uses the SDR rendition ([`DevelopSettings::sdr_rendition`]): the same edit with the
/// `sdr_*` offsets applied.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hdr {
    pub enabled: bool,
    /// Headroom limit: stops above SDR white the HDR render may reach.
    pub max_ev: f64,
    /// SDR rendition offsets (−100..100): brightness (±1 EV), then added to the matching sliders.
    pub sdr_brightness: f64,
    pub sdr_contrast: f64,
    pub sdr_highlights: f64,
    pub sdr_shadows: f64,
    pub sdr_whites: f64,
    pub sdr_clarity: f64,
}

impl Default for Hdr {
    fn default() -> Self {
        Self {
            enabled: false,
            max_ev: Hdr::DEFAULT_MAX_EV,
            sdr_brightness: 0.0,
            sdr_contrast: 0.0,
            sdr_highlights: 0.0,
            sdr_shadows: 0.0,
            sdr_whites: 0.0,
            sdr_clarity: 0.0,
        }
    }
}

impl Hdr {
    pub const DEFAULT_MAX_EV: f64 = 3.0;
    pub const MAX_EV_LIMIT: f64 = 5.0;

    /// Whether these are the default HDR settings (HDR off, nothing changed).
    pub fn is_default(&self) -> bool {
        *self == Hdr::default()
    }

    /// Peak linear value of an HDR render relative to SDR white (1.0 when HDR is off). A non-finite headroom
    /// (a damaged file or argument) falls back to the default, so the peak is always finite and at least 1.
    pub fn peak(&self) -> f32 {
        if !self.enabled {
            return 1.0;
        }
        let ev = if self.max_ev.is_finite() { self.max_ev } else { Hdr::DEFAULT_MAX_EV };
        (ev.clamp(0.0, Hdr::MAX_EV_LIMIT) as f32).exp2()
    }
}

impl DevelopSettings {
    /// The settings an SDR render uses: unchanged when HDR is off; otherwise HDR off and the
    /// SDR rendition offsets applied to exposure, contrast, highlights, shadows, whites, clarity.
    pub fn sdr_rendition(&self) -> std::borrow::Cow<'_, DevelopSettings> {
        if !self.hdr.enabled {
            return std::borrow::Cow::Borrowed(self);
        }
        let h = self.hdr;
        let mut s = self.clone();
        s.hdr.enabled = false;
        let add = |v: f64, d: f64| (v + d).clamp(-100.0, 100.0);
        s.light.exposure = (s.light.exposure + h.sdr_brightness / 100.0).clamp(-5.0, 5.0);
        s.light.contrast = add(s.light.contrast, h.sdr_contrast);
        s.light.highlights = add(s.light.highlights, h.sdr_highlights);
        s.light.shadows = add(s.light.shadows, h.sdr_shadows);
        s.light.whites = add(s.light.whites, h.sdr_whites);
        s.effects.clarity = add(s.effects.clarity, h.sdr_clarity);
        std::borrow::Cow::Owned(s)
    }
}
