//! Map PhotoCraft Camera Raw edit fields without changing the base processing version.
//!
//! Maps Camera Raw edit fields (Light, White Balance, Effects, Color, Tone Curve, Color
//! Mixer, Color Grading, Detail, Grain, Vignette, and Calibration) into `DevelopSettings`.
//! This is a settings mapping, not a promise that two renderers produce the same pixels.

use std::{error::Error, fmt};

use craft_geom::Point;
use serde_json::Value;

use crate::{DevelopSettings, VignetteStyle, WbMode, Wheel, controls, crs};

/// `(camera_raw_json_key, develop_model_control_id)` for 1-to-1 scalar controls.
const SCALAR_FIELDS: &[(&str, &str)] = &[
    ("exposure", "light.exposure"),
    ("contrast", "light.contrast"),
    ("highlights", "light.highlights"),
    ("shadows", "light.shadows"),
    ("whites", "light.whites"),
    ("blacks", "light.blacks"),
    ("texture", "effects.texture"),
    ("clarity", "effects.clarity"),
    ("dehaze", "effects.dehaze"),
    ("vibrance", "color.vibrance"),
    ("saturation", "color.saturation"),
    ("curveShadows", "curve.shadows"),
    ("curveDarks", "curve.darks"),
    ("curveLights", "curve.lights"),
    ("curveHighlights", "curve.highlights"),
    ("gradeBlending", "grading.blending"),
    ("gradeBalance", "grading.balance"),
    ("sharpenAmount", "detail.sharpenAmount"),
    ("sharpenRadius", "detail.sharpenRadius"),
    ("sharpenDetail", "detail.sharpenDetail"),
    ("sharpenMasking", "detail.sharpenMasking"),
    ("noiseLuminance", "detail.nrLuminance"),
    ("noiseLuminanceDetail", "detail.nrDetail"),
    ("noiseColor", "detail.nrColor"),
    ("noiseColorDetail", "detail.nrColorDetail"),
    ("grainAmount", "grain.amount"),
    ("grainSize", "grain.size"),
    ("grainRoughness", "grain.roughness"),
    ("vignetteAmount", "vignette.amount"),
    ("vignetteMidpoint", "vignette.midpoint"),
    ("vignetteFeather", "vignette.feather"),
    ("vignetteRoundness", "vignette.roundness"),
    ("vignetteHighlights", "vignette.highlights"),
    ("shadowTint", "calibration.shadowsTint"),
];

const EXTRA_MAPPED_FIELDS: &[&str] = &[
    "temperature",
    "tint",
    "curveSplits",
    "pointCurve",
    "pointCurveRed",
    "pointCurveGreen",
    "pointCurveBlue",
    "hslHue",
    "hslSat",
    "hslLum",
    "gradeShadows",
    "gradeMidtones",
    "gradeHighlights",
    "gradeGlobal",
    "vignetteStyle",
    "calibrationHue",
    "calibrationSat",
];

const MIXER_BAND_CONTROLS: [&str; 8] = ["red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta"];

/// The partially migrated settings and the supplied field names not yet mapped.
#[derive(Debug)]
pub struct Migration {
    /// A copy of the base with only the supported fields applied.
    pub settings: DevelopSettings,
    /// Sorted source keys, including recognized controls not yet mapped and unknown controls.
    /// Values are not included. The caller must preserve any original state it still needs.
    pub unmapped_fields: Vec<String>,
}

/// Why a PhotoCraft Camera Raw settings object could not be migrated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MigrationError {
    /// The supplied settings are not a JSON object.
    InvalidSettingsObject,
    /// The supplied exposure is not a finite number in EV.
    InvalidExposure,
    /// The supplied exposure is outside the develop model's control range.
    ExposureOutOfRange,
    /// The develop model does not define its exposure control.
    MissingExposureControl,
    /// A supplied scalar field is not a finite number or has an invalid representation.
    InvalidField(&'static str),
    /// A supplied field is outside the supported control range.
    FieldOutOfRange(&'static str),
    /// The develop model does not define the target control.
    MissingControl(&'static str),
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSettingsObject => f.write_str("Camera Raw edit fields must be an object"),
            Self::InvalidExposure => f.write_str("exposure must be a finite number in EV"),
            Self::ExposureOutOfRange => f.write_str("exposure is outside the develop control range"),
            Self::MissingExposureControl => f.write_str("the develop model has no exposure control"),
            Self::InvalidField(field) => write!(f, "{field} has an invalid value"),
            Self::FieldOutOfRange(field) => write!(f, "{field} is outside the develop control range"),
            Self::MissingControl(control) => write!(f, "the develop model has no {control} control"),
        }
    }
}

impl Error for MigrationError {}

fn invalid_field_error(field: &'static str) -> MigrationError {
    if field == "exposure" { MigrationError::InvalidExposure } else { MigrationError::InvalidField(field) }
}

fn out_of_range_error(field: &'static str) -> MigrationError {
    if field == "exposure" { MigrationError::ExposureOutOfRange } else { MigrationError::FieldOutOfRange(field) }
}

fn missing_control_error(field: &'static str, control: &'static str) -> MigrationError {
    if field == "exposure" { MigrationError::MissingExposureControl } else { MigrationError::MissingControl(control) }
}

fn finite_in_range(value: &Value, field: &'static str, min: f64, max: f64) -> Result<f64, MigrationError> {
    let valid = value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| invalid_field_error(field))?;
    if !(min..=max).contains(&valid) {
        return Err(out_of_range_error(field));
    }
    Ok(valid)
}

fn apply_control(settings: &mut DevelopSettings, field: &'static str, control: &'static str, value: &Value) -> Result<(), MigrationError> {
    let spec = controls::find(control).ok_or_else(|| missing_control_error(field, control))?;
    let valid = finite_in_range(value, field, spec.min, spec.max)?;
    if !controls::set(settings, control, valid) {
        return Err(missing_control_error(field, control));
    }
    Ok(())
}

fn apply_scalars(settings: &mut DevelopSettings, fields: &serde_json::Map<String, Value>) -> Result<(), MigrationError> {
    for &(field, control) in SCALAR_FIELDS {
        if let Some(value) = fields.get(field) {
            apply_control(settings, field, control, value)?;
        }
    }
    Ok(())
}

fn apply_wb(settings: &mut DevelopSettings, fields: &serde_json::Map<String, Value>) -> Result<(), MigrationError> {
    if let Some(value) = fields.get("temperature") {
        let rel = finite_in_range(value, "temperature", -100.0, 100.0)?;
        settings.wb.mode = WbMode::Custom;
        settings.wb.temp = crs::rel_to_kelvin(rel);
    }
    if let Some(value) = fields.get("tint") {
        let tint = finite_in_range(value, "tint", -100.0, 100.0)?;
        settings.wb.mode = WbMode::Custom;
        settings.wb.tint = tint;
    }
    Ok(())
}

fn apply_curve_splits(settings: &mut DevelopSettings, fields: &serde_json::Map<String, Value>) -> Result<(), MigrationError> {
    let Some(value) = fields.get("curveSplits") else { return Ok(()) };
    let items = value.as_array().filter(|a| a.len() == 3).ok_or(MigrationError::InvalidField("curveSplits"))?;
    let controls = ["curve.splitShadows", "curve.splitMid", "curve.splitHighlights"];
    let mut parsed = [0.0; 3];
    for (i, (item, control)) in items.iter().zip(controls).enumerate() {
        let spec = controls::find(control).ok_or(MigrationError::MissingControl(control))?;
        parsed[i] = finite_in_range(item, "curveSplits", spec.min, spec.max)?;
    }
    if !(parsed[0] < parsed[1] && parsed[1] < parsed[2]) {
        return Err(MigrationError::InvalidField("curveSplits"));
    }
    settings.curve.split_shadows = parsed[0];
    settings.curve.split_mid = parsed[1];
    settings.curve.split_highlights = parsed[2];
    Ok(())
}

fn parse_point_curve(value: &Value, field: &'static str) -> Result<Vec<Point>, MigrationError> {
    let items = value.as_array().ok_or(MigrationError::InvalidField(field))?;
    if items.is_empty() {
        return Ok(Vec::new());
    }
    if items.len() < 2 {
        return Err(MigrationError::InvalidField(field));
    }
    let mut points = Vec::with_capacity(items.len());
    let mut prev_x = -1.0;
    for item in items {
        let pair = item.as_array().filter(|p| p.len() == 2).ok_or(MigrationError::InvalidField(field))?;
        let x = finite_in_range(&pair[0], field, 0.0, 255.0)?;
        let y = finite_in_range(&pair[1], field, 0.0, 255.0)?;
        if x <= prev_x {
            return Err(MigrationError::InvalidField(field));
        }
        prev_x = x;
        points.push(Point::new(x / 255.0, y / 255.0));
    }
    Ok(points)
}

fn apply_point_curves(settings: &mut DevelopSettings, fields: &serde_json::Map<String, Value>) -> Result<(), MigrationError> {
    if let Some(v) = fields.get("pointCurve") {
        settings.curve.master = parse_point_curve(v, "pointCurve")?;
    }
    if let Some(v) = fields.get("pointCurveRed") {
        settings.curve.red = parse_point_curve(v, "pointCurveRed")?;
    }
    if let Some(v) = fields.get("pointCurveGreen") {
        settings.curve.green = parse_point_curve(v, "pointCurveGreen")?;
    }
    if let Some(v) = fields.get("pointCurveBlue") {
        settings.curve.blue = parse_point_curve(v, "pointCurveBlue")?;
    }
    Ok(())
}

fn apply_hsl(settings: &mut DevelopSettings, fields: &serde_json::Map<String, Value>) -> Result<(), MigrationError> {
    for (field, suffix) in [("hslHue", "hue"), ("hslSat", "sat"), ("hslLum", "lum")] {
        let Some(value) = fields.get(field) else { continue };
        let items = value.as_array().filter(|a| a.len() == 8).ok_or(MigrationError::InvalidField(field))?;
        for (band, item) in MIXER_BAND_CONTROLS.iter().zip(items) {
            let control = format!("mixer.{band}.{suffix}");
            let Some(spec) = controls::find(&control) else {
                return Err(MigrationError::MissingControl(suffix));
            };
            let valid = finite_in_range(item, field, spec.min, spec.max)?;
            controls::set(settings, &control, valid);
        }
    }
    Ok(())
}

fn apply_wheel(wheel: &mut Wheel, prefix: &'static str, field: &'static str, value: &Value) -> Result<(), MigrationError> {
    let obj = value.as_object().ok_or(MigrationError::InvalidField(field))?;
    for (key, val) in obj {
        let (target, control) = match key.as_str() {
            "hue" => (&mut wheel.hue, format!("grading.{prefix}.hue")),
            "sat" => (&mut wheel.sat, format!("grading.{prefix}.sat")),
            "lum" => (&mut wheel.lum, format!("grading.{prefix}.lum")),
            _ => return Err(MigrationError::InvalidField(field)),
        };
        let Some(spec) = controls::find(&control) else {
            return Err(MigrationError::MissingControl(prefix));
        };
        *target = finite_in_range(val, field, spec.min, spec.max)?;
    }
    Ok(())
}

fn apply_grading(settings: &mut DevelopSettings, fields: &serde_json::Map<String, Value>) -> Result<(), MigrationError> {
    if let Some(v) = fields.get("gradeShadows") {
        apply_wheel(&mut settings.grading.shadows, "shadows", "gradeShadows", v)?;
    }
    if let Some(v) = fields.get("gradeMidtones") {
        apply_wheel(&mut settings.grading.midtones, "midtones", "gradeMidtones", v)?;
    }
    if let Some(v) = fields.get("gradeHighlights") {
        apply_wheel(&mut settings.grading.highlights, "highlights", "gradeHighlights", v)?;
    }
    if let Some(v) = fields.get("gradeGlobal") {
        apply_wheel(&mut settings.grading.global, "global", "gradeGlobal", v)?;
    }
    Ok(())
}

fn apply_vignette_style(settings: &mut DevelopSettings, fields: &serde_json::Map<String, Value>) -> Result<(), MigrationError> {
    let Some(value) = fields.get("vignetteStyle") else { return Ok(()) };
    settings.vignette.style = match value.as_str() {
        Some("highlightPriority") => VignetteStyle::HighlightPriority,
        Some("colorPriority") => VignetteStyle::ColorPriority,
        Some("paintOverlay") => VignetteStyle::PaintOverlay,
        _ => return Err(MigrationError::InvalidField("vignetteStyle")),
    };
    Ok(())
}

fn apply_calibration(settings: &mut DevelopSettings, fields: &serde_json::Map<String, Value>) -> Result<(), MigrationError> {
    if let Some(value) = fields.get("calibrationHue") {
        let items = value.as_array().filter(|a| a.len() == 3).ok_or(MigrationError::InvalidField("calibrationHue"))?;
        settings.calibration.red_hue = finite_in_range(&items[0], "calibrationHue", -100.0, 100.0)?;
        settings.calibration.green_hue = finite_in_range(&items[1], "calibrationHue", -100.0, 100.0)?;
        settings.calibration.blue_hue = finite_in_range(&items[2], "calibrationHue", -100.0, 100.0)?;
    }
    if let Some(value) = fields.get("calibrationSat") {
        let items = value.as_array().filter(|a| a.len() == 3).ok_or(MigrationError::InvalidField("calibrationSat"))?;
        settings.calibration.red_sat = finite_in_range(&items[0], "calibrationSat", -100.0, 100.0)?;
        settings.calibration.green_sat = finite_in_range(&items[1], "calibrationSat", -100.0, 100.0)?;
        settings.calibration.blue_sat = finite_in_range(&items[2], "calibrationSat", -100.0, 100.0)?;
    }
    Ok(())
}

fn is_mapped(key: &str) -> bool {
    SCALAR_FIELDS.iter().any(|(field, _)| *field == key) || EXTRA_MAPPED_FIELDS.contains(&key)
}

/// Apply supported fields from a PhotoCraft Camera Raw settings object to an explicit base.
///
/// Supply edit fields only, not a PSD descriptor or transport metadata. Missing fields
/// are not defaults: absent fields leave the baseline unchanged. Like a partial preset
/// this replaces only named controls, and does not normalize unrelated base fields.
///
/// # Errors
///
/// A non-object or an explicit mapped field that is malformed or outside the shared
/// control range is rejected rather than silently replaced or clipped.
pub fn migrate_camera_raw(base: &DevelopSettings, fields: &Value) -> Result<Migration, MigrationError> {
    let fields = fields.as_object().ok_or(MigrationError::InvalidSettingsObject)?;
    let mut settings = base.clone();
    apply_scalars(&mut settings, fields)?;
    apply_wb(&mut settings, fields)?;
    apply_curve_splits(&mut settings, fields)?;
    apply_point_curves(&mut settings, fields)?;
    apply_hsl(&mut settings, fields)?;
    apply_grading(&mut settings, fields)?;
    apply_vignette_style(&mut settings, fields)?;
    apply_calibration(&mut settings, fields)?;
    let mut unmapped_fields: Vec<_> = fields.keys().filter(|key| !is_mapped(key)).cloned().collect();
    unmapped_fields.sort_unstable();
    Ok(Migration { settings, unmapped_fields })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Process;
    use serde_json::json;

    #[test]
    fn camera_raw_full_edit_replaces_named_fields_and_preserves_process_and_unmapped() -> Result<(), MigrationError> {
        // V7 is not the default: detect an accidental restamp to the latest process.
        for process in [Process::V8, Process::V7] {
            let mut base = DevelopSettings { process: process.version(), ..DevelopSettings::default() };
            base.light.exposure = -0.75;
            base.light.contrast = 12.0;
            // Using a stored value outside today's control range detects whole-model normalization.
            base.optics.distortion = 123.0;
            let fields: Value = serde_json::from_str(
                r#"{
                    "unknownField": [1, 2],
                    "encodingGamma": 1.8,
                    "temperature": 18,
                    "tint": -12,
                    "contrast": -38,
                    "exposure": 1.15,
                    "highlights": 35,
                    "shadows": -50,
                    "whites": 49,
                    "blacks": -34,
                    "texture": 12,
                    "clarity": 16,
                    "dehaze": 8,
                    "vibrance": 20,
                    "saturation": -5,
                    "curveShadows": 10,
                    "curveDarks": -8,
                    "curveLights": 12,
                    "curveHighlights": -6,
                    "curveSplits": [20, 50, 80],
                    "pointCurve": [[0, 0], [64, 52], [255, 255]],
                    "pointCurveRed": [[0, 0], [255, 200]],
                    "pointCurveGreen": [],
                    "hslHue": [5, -10, 0, 12, -4, 8, 0, -3],
                    "hslSat": [10, 5, -8, 0, 6, -12, 4, 0],
                    "hslLum": [-4, 8, 2, -6, 0, 10, -2, 3],
                    "gradeShadows": {"hue": 210, "sat": 25, "lum": -5},
                    "gradeMidtones": {"hue": 45, "sat": 10, "lum": 0},
                    "gradeHighlights": {"hue": 35, "sat": 20, "lum": 4},
                    "gradeGlobal": {"hue": 0, "sat": 0, "lum": 2},
                    "gradeBlending": 60,
                    "gradeBalance": -15,
                    "sharpenAmount": 40,
                    "sharpenRadius": 1.2,
                    "sharpenDetail": 30,
                    "sharpenMasking": 20,
                    "noiseLuminance": 15,
                    "noiseLuminanceDetail": 50,
                    "noiseColor": 25,
                    "noiseColorDetail": 50,
                    "grainAmount": 12,
                    "grainSize": 25,
                    "grainRoughness": 50,
                    "vignetteAmount": -20,
                    "vignetteMidpoint": 45,
                    "vignetteFeather": 55,
                    "vignetteRoundness": 10,
                    "vignetteHighlights": 15,
                    "vignetteStyle": "colorPriority",
                    "shadowTint": -6,
                    "calibrationHue": [4, -3, 2],
                    "calibrationSat": [8, -5, 6]
                }"#,
            )
            .map_err(|_| MigrationError::InvalidSettingsObject)?;
            let result = migrate_camera_raw(&base, &fields)?;

            let mut expected = base.clone();
            expected.wb.mode = WbMode::Custom;
            expected.wb.temp = crs::rel_to_kelvin(18.0);
            expected.wb.tint = -12.0;
            expected.light.exposure = 1.15;
            expected.light.contrast = -38.0;
            expected.light.highlights = 35.0;
            expected.light.shadows = -50.0;
            expected.light.whites = 49.0;
            expected.light.blacks = -34.0;
            expected.effects.texture = 12.0;
            expected.effects.clarity = 16.0;
            expected.effects.dehaze = 8.0;
            expected.color.vibrance = 20.0;
            expected.color.saturation = -5.0;
            expected.curve.shadows = 10.0;
            expected.curve.darks = -8.0;
            expected.curve.lights = 12.0;
            expected.curve.highlights = -6.0;
            expected.curve.split_shadows = 20.0;
            expected.curve.split_mid = 50.0;
            expected.curve.split_highlights = 80.0;
            expected.curve.master = vec![Point::new(0.0, 0.0), Point::new(64.0 / 255.0, 52.0 / 255.0), Point::new(1.0, 1.0)];
            expected.curve.red = vec![Point::new(0.0, 0.0), Point::new(1.0, 200.0 / 255.0)];
            expected.curve.green = Vec::new();
            expected.mixer.red = crate::Hsl { hue: 5.0, sat: 10.0, lum: -4.0 };
            expected.mixer.orange = crate::Hsl { hue: -10.0, sat: 5.0, lum: 8.0 };
            expected.mixer.yellow = crate::Hsl { hue: 0.0, sat: -8.0, lum: 2.0 };
            expected.mixer.green = crate::Hsl { hue: 12.0, sat: 0.0, lum: -6.0 };
            expected.mixer.aqua = crate::Hsl { hue: -4.0, sat: 6.0, lum: 0.0 };
            expected.mixer.blue = crate::Hsl { hue: 8.0, sat: -12.0, lum: 10.0 };
            expected.mixer.purple = crate::Hsl { hue: 0.0, sat: 4.0, lum: -2.0 };
            expected.mixer.magenta = crate::Hsl { hue: -3.0, sat: 0.0, lum: 3.0 };
            expected.grading.shadows = Wheel { hue: 210.0, sat: 25.0, lum: -5.0 };
            expected.grading.midtones = Wheel { hue: 45.0, sat: 10.0, lum: 0.0 };
            expected.grading.highlights = Wheel { hue: 35.0, sat: 20.0, lum: 4.0 };
            expected.grading.global = Wheel { hue: 0.0, sat: 0.0, lum: 2.0 };
            expected.grading.blending = 60.0;
            expected.grading.balance = -15.0;
            expected.detail.sharpen_amount = 40.0;
            expected.detail.sharpen_radius = 1.2;
            expected.detail.sharpen_detail = 30.0;
            expected.detail.sharpen_masking = 20.0;
            expected.detail.nr_luminance = 15.0;
            expected.detail.nr_detail = 50.0;
            expected.detail.nr_color = 25.0;
            expected.detail.nr_color_detail = 50.0;
            expected.grain.amount = 12.0;
            expected.grain.size = 25.0;
            expected.grain.roughness = 50.0;
            expected.vignette.amount = -20.0;
            expected.vignette.midpoint = 45.0;
            expected.vignette.feather = 55.0;
            expected.vignette.roundness = 10.0;
            expected.vignette.highlights = 15.0;
            expected.vignette.style = VignetteStyle::ColorPriority;
            expected.calibration.shadows_tint = -6.0;
            expected.calibration.red_hue = 4.0;
            expected.calibration.green_hue = -3.0;
            expected.calibration.blue_hue = 2.0;
            expected.calibration.red_sat = 8.0;
            expected.calibration.green_sat = -5.0;
            expected.calibration.blue_sat = 6.0;
            assert_eq!(result.settings, expected);
            assert_eq!(result.unmapped_fields, ["encodingGamma", "unknownField"]);
        }
        Ok(())
    }

    #[test]
    fn camera_raw_absent_and_invalid_field_cases() -> Result<(), MigrationError> {
        let mut base = DevelopSettings::default();
        base.light.exposure = -0.75;
        let result = migrate_camera_raw(&base, &json!({ "encodingGamma": 1.8 }))?;
        assert_eq!(result.settings, base);
        assert_eq!(result.unmapped_fields, ["encodingGamma"]);

        assert_eq!(migrate_camera_raw(&base, &json!([])).err(), Some(MigrationError::InvalidSettingsObject));

        // JSON cannot represent a non-finite number; serde_json serializes it to null.
        for invalid in [json!({ "exposure": null }), json!({ "exposure": "NaN" }), json!({ "exposure": f64::INFINITY })] {
            assert_eq!(migrate_camera_raw(&base, &invalid).err(), Some(MigrationError::InvalidExposure));
        }
        assert_eq!(migrate_camera_raw(&base, &json!({ "exposure": 5.1 })).err(), Some(MigrationError::ExposureOutOfRange));
        assert_eq!(migrate_camera_raw(&base, &json!({ "contrast": "high" })).err(), Some(MigrationError::InvalidField("contrast")));
        assert_eq!(migrate_camera_raw(&base, &json!({ "contrast": 101 })).err(), Some(MigrationError::FieldOutOfRange("contrast")));
        assert_eq!(migrate_camera_raw(&base, &json!({ "sharpenRadius": 0.4 })).err(), Some(MigrationError::FieldOutOfRange("sharpenRadius")));
        assert_eq!(migrate_camera_raw(&base, &json!({ "temperature": 101 })).err(), Some(MigrationError::FieldOutOfRange("temperature")));
        assert_eq!(migrate_camera_raw(&base, &json!({ "curveSplits": [50, 25, 75] })).err(), Some(MigrationError::InvalidField("curveSplits")));
        assert_eq!(migrate_camera_raw(&base, &json!({ "pointCurve": [[10, 10]] })).err(), Some(MigrationError::InvalidField("pointCurve")));
        assert_eq!(migrate_camera_raw(&base, &json!({ "hslHue": [0, 0] })).err(), Some(MigrationError::InvalidField("hslHue")));
        assert_eq!(migrate_camera_raw(&base, &json!({ "gradeShadows": {"bad": 1} })).err(), Some(MigrationError::InvalidField("gradeShadows")));
        assert_eq!(migrate_camera_raw(&base, &json!({ "vignetteStyle": "unknown" })).err(), Some(MigrationError::InvalidField("vignetteStyle")));
        assert_eq!(
            migrate_camera_raw(&base, &json!({ "calibrationHue": [0, 0, 101] })).err(),
            Some(MigrationError::FieldOutOfRange("calibrationHue"))
        );
        Ok(())
    }
}
