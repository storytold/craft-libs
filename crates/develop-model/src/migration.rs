//! Map PhotoCraft Camera Raw edit fields without changing the base processing version.
//!
//! Maps the 1-to-1 scalar Camera Raw controls (Light, Effects, Color, Parametric Tone Curve
//! amounts, Color Grading blending/balance, Detail, Grain, Vignette scalars, and Calibration
//! shadow tint) into `DevelopSettings`. This is a settings mapping, not a promise that two
//! renderers produce the same pixels.

use std::{error::Error, fmt};

use serde_json::Value;

use crate::{DevelopSettings, controls};

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
    /// A supplied scalar field is not a finite number.
    InvalidField(&'static str),
    /// A supplied scalar field is outside the develop model's control range.
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
            Self::InvalidField(field) => write!(f, "{field} must be a finite number"),
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

/// Apply supported fields from a PhotoCraft Camera Raw settings object to an explicit base.
///
/// Supply edit fields only, not a PSD descriptor or transport metadata. Missing fields
/// are not defaults: absent fields leave the baseline unchanged. Like a partial preset
/// this replaces only named controls, and does not normalize unrelated base fields.
///
/// # Errors
///
/// A non-object or an explicit mapped scalar that is not a finite number within the shared
/// control range is rejected rather than silently replaced or clipped.
pub fn migrate_camera_raw(base: &DevelopSettings, fields: &Value) -> Result<Migration, MigrationError> {
    let fields = fields.as_object().ok_or(MigrationError::InvalidSettingsObject)?;
    let mut settings = base.clone();
    for &(field, control) in SCALAR_FIELDS {
        let Some(value) = fields.get(field) else { continue };
        let valid = value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| invalid_field_error(field))?;
        let spec = controls::find(control).ok_or_else(|| missing_control_error(field, control))?;
        if !(spec.min..=spec.max).contains(&valid) {
            return Err(out_of_range_error(field));
        }
        if !controls::set(&mut settings, control, valid) {
            return Err(missing_control_error(field, control));
        }
    }
    let mut unmapped_fields: Vec<_> = fields.keys().filter(|key| !SCALAR_FIELDS.iter().any(|(field, _)| *field == key.as_str())).cloned().collect();
    unmapped_fields.sort_unstable();
    Ok(Migration { settings, unmapped_fields })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Process;
    use serde_json::json;

    #[test]
    fn camera_raw_scalars_replace_only_named_fields_and_preserve_process_and_unmapped() -> Result<(), MigrationError> {
        // V7 is not the default: detect an accidental restamp to the latest process.
        for process in [Process::V8, Process::V7] {
            let mut base = DevelopSettings { process: process.version(), ..DevelopSettings::default() };
            base.light.exposure = -0.75;
            base.light.contrast = 12.0;
            // Using a stored value outside today's control range detects whole-model normalization.
            base.optics.distortion = 123.0;
            let result = migrate_camera_raw(
                &base,
                &json!({
                    "unknownField": [1, 2],
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
                    "shadowTint": -6,
                    "temperature": -26
                }),
            )?;

            let mut expected = base.clone();
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
            expected.calibration.shadows_tint = -6.0;
            assert_eq!(result.settings, expected);
            assert_eq!(result.unmapped_fields, ["temperature", "unknownField"]);
        }
        Ok(())
    }

    #[test]
    fn camera_raw_absent_and_invalid_scalar_cases() -> Result<(), MigrationError> {
        let mut base = DevelopSettings::default();
        base.light.exposure = -0.75;
        let result = migrate_camera_raw(&base, &json!({ "temperature": -26 }))?;
        assert_eq!(result.settings, base);
        assert_eq!(result.unmapped_fields, ["temperature"]);

        assert_eq!(migrate_camera_raw(&base, &json!([])).err(), Some(MigrationError::InvalidSettingsObject));

        // JSON cannot represent a non-finite number; serde_json serializes it to null.
        for invalid in [json!({ "exposure": null }), json!({ "exposure": "NaN" }), json!({ "exposure": f64::INFINITY })] {
            assert_eq!(migrate_camera_raw(&base, &invalid).err(), Some(MigrationError::InvalidExposure));
        }
        assert_eq!(migrate_camera_raw(&base, &json!({ "exposure": 5.1 })).err(), Some(MigrationError::ExposureOutOfRange));
        assert_eq!(migrate_camera_raw(&base, &json!({ "contrast": "high" })).err(), Some(MigrationError::InvalidField("contrast")));
        assert_eq!(migrate_camera_raw(&base, &json!({ "contrast": 101 })).err(), Some(MigrationError::FieldOutOfRange("contrast")));
        assert_eq!(migrate_camera_raw(&base, &json!({ "sharpenRadius": 0.4 })).err(), Some(MigrationError::FieldOutOfRange("sharpenRadius")));
        Ok(())
    }
}
