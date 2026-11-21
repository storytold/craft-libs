//! Map PhotoCraft Camera Raw edit fields without changing the base processing version.
//!
//! The first supported field is `exposure` (EV) → `light.exposure` (EV). This is a
//! settings mapping, not a promise that two renderers produce the same pixels.

use serde_json::{Error, Value};

use crate::{DevelopSettings, controls};

/// The partially migrated settings and the supplied field names not yet mapped.
#[derive(Debug)]
pub struct Migration {
    /// A copy of the base with only the supported fields applied.
    pub settings: DevelopSettings,
    /// Sorted source keys, including recognized controls not yet mapped and unknown controls.
    /// Values are not included. The caller must preserve any original state it still needs.
    pub unmapped_fields: Vec<String>,
}

/// Apply supported fields from a PhotoCraft Camera Raw settings object to an explicit base.
///
/// Supply edit fields only, not a PSD descriptor or transport metadata. Missing fields
/// are not defaults: an absent exposure leaves the baseline unchanged. Like a partial
/// preset this replaces only named controls, but does not normalize unrelated base fields.
///
/// # Errors
///
/// A non-object or an explicit exposure that is not a finite number within the shared
/// control range is rejected rather than silently replaced or clipped.
pub fn migrate_camera_raw(base: &DevelopSettings, fields: &Value) -> Result<Migration, Error> {
    fn invalid(message: &str) -> Error {
        <Error as serde::de::Error>::custom(message)
    }

    let fields = fields.as_object().ok_or_else(|| invalid("Camera Raw edit fields must be an object"))?;
    let mut settings = base.clone();
    if let Some(value) = fields.get("exposure") {
        let valid = value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| invalid("exposure must be a finite number in EV"))?;
        let spec = controls::find("light.exposure").ok_or_else(|| invalid("the develop model has no exposure control"))?;
        if !(spec.min..=spec.max).contains(&valid) {
            return Err(invalid("exposure is outside the develop control range"));
        }
        settings.light.exposure = valid;
    }
    let mut unmapped_fields: Vec<_> = fields.keys().filter(|key| key.as_str() != "exposure").cloned().collect();
    unmapped_fields.sort_unstable();
    Ok(Migration { settings, unmapped_fields })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Process;
    use serde_json::json;

    #[test]
    fn camera_raw_exposure_replaces_only_one_field_and_reports_the_rest() -> Result<(), Error> {
        // V7 is not the default: detect an accidental restamp to the latest process.
        for process in [Process::V8, Process::V7] {
            let mut base = DevelopSettings { process: process.version(), ..DevelopSettings::default() };
            base.light.exposure = -0.75;
            // Using a stored value outside today's control range detects whole-model normalization.
            base.light.contrast = 123.0;
            let result = migrate_camera_raw(&base, &json!({ "unknownField": [1, 2], "contrast": -38, "exposure": 1.15, "temperature": -26 }))?;

            let mut expected = base.clone();
            expected.light.exposure = 1.15;
            assert_eq!(result.settings, expected);
            assert_eq!(result.unmapped_fields, ["contrast", "temperature", "unknownField"]);
        }
        Ok(())
    }

    #[test]
    fn camera_raw_exposure_absent_is_distinct_from_invalid() -> Result<(), Error> {
        let mut base = DevelopSettings::default();
        base.light.exposure = -0.75;
        let result = migrate_camera_raw(&base, &json!({ "contrast": -38 }))?;
        assert_eq!(result.settings, base);
        assert_eq!(result.unmapped_fields, ["contrast"]);

        // JSON cannot represent a non-finite number; serde_json serializes it to null.
        for invalid in [
            json!([]),
            json!({ "exposure": null }),
            json!({ "exposure": "NaN" }),
            json!({ "exposure": f64::INFINITY }),
            json!({ "exposure": 5.1 }),
        ] {
            assert!(migrate_camera_raw(&base, &invalid).is_err());
        }
        Ok(())
    }
}
