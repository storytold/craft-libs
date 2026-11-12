//! Presets and copy/paste subsets.
//!
//! A preset is a *partial* settings object (JSON) — only the settings it names. Applying it changes
//! exactly the settings it names (a leaf: a number, a flag, a name; an array is one leaf): each is
//! computed from the photo's *original* state (its import settings, when known) and the preset's
//! value, and replaces the photo's value; every other setting keeps the photo's value (issue
//! storytold/lightcraft#2159). `amount` (0..200 %) moves each named number between the original and the preset's
//! value. Copy/paste/sync use the same group selection ([`SettingsGroup`]) and merge into the
//! current settings as before.
//!
//! The rendering process ([`crate::ProcessVersion`]) is in no group: copy, paste, sync and presets
//! made from a photo never carry it, so the photos they change keep their own. A partial that names
//! `process` itself (Apply Settings JSON) sets it like any other field.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::borrow::Cow;

use crate::DevelopSettings;

/// Groups shown in "Choose settings to copy" / "Create preset" (Lightroom's checklist).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingsGroup {
    Profile,
    Treatment,
    WhiteBalance,
    Light,
    ToneCurve,
    Color,
    ColorMixer,
    ColorGrading,
    Effects,
    Vignette,
    Grain,
    Detail,
    Optics,
    Geometry,
    Crop,
    Masks,
    Spots,
    RedEye,
    LensBlur,
    Calibration,
}

impl SettingsGroup {
    pub const ALL: [SettingsGroup; 20] = [
        SettingsGroup::Profile,
        SettingsGroup::Treatment,
        SettingsGroup::WhiteBalance,
        SettingsGroup::Light,
        SettingsGroup::ToneCurve,
        SettingsGroup::Color,
        SettingsGroup::ColorMixer,
        SettingsGroup::ColorGrading,
        SettingsGroup::Effects,
        SettingsGroup::Vignette,
        SettingsGroup::Grain,
        SettingsGroup::Detail,
        SettingsGroup::Optics,
        SettingsGroup::Geometry,
        SettingsGroup::Crop,
        SettingsGroup::Masks,
        SettingsGroup::Spots,
        SettingsGroup::RedEye,
        SettingsGroup::LensBlur,
        SettingsGroup::Calibration,
    ];

    /// Groups included by default when copying (Lightroom excludes crop, masks and spots by default).
    pub fn default_copy() -> Vec<SettingsGroup> {
        SettingsGroup::ALL
            .into_iter()
            .filter(|g| !matches!(g, SettingsGroup::Crop | SettingsGroup::Masks | SettingsGroup::Spots | SettingsGroup::RedEye))
            .collect()
    }

    pub fn label(self) -> &'static str {
        match self {
            SettingsGroup::Profile => "Profile",
            SettingsGroup::Treatment => "Treatment",
            SettingsGroup::WhiteBalance => "White Balance",
            SettingsGroup::Light => "Light",
            SettingsGroup::ToneCurve => "Tone Curve",
            SettingsGroup::Color => "Color",
            SettingsGroup::ColorMixer => "Color Mixer",
            SettingsGroup::ColorGrading => "Color Grading",
            SettingsGroup::Effects => "Effects",
            SettingsGroup::Vignette => "Vignette",
            SettingsGroup::Grain => "Grain",
            SettingsGroup::Detail => "Detail",
            SettingsGroup::Optics => "Optics",
            SettingsGroup::Geometry => "Geometry",
            SettingsGroup::Crop => "Crop",
            SettingsGroup::Masks => "Masking",
            SettingsGroup::Spots => "Remove",
            SettingsGroup::RedEye => "Red Eye",
            SettingsGroup::LensBlur => "Lens Blur",
            SettingsGroup::Calibration => "Calibration",
        }
    }

    /// Top-level JSON keys of `DevelopSettings` that belong to the group.
    pub fn keys(self) -> &'static [&'static str] {
        match self {
            SettingsGroup::Profile => &["profile"],
            SettingsGroup::Treatment => &["treatment"],
            SettingsGroup::WhiteBalance => &["wb"],
            SettingsGroup::Light => &["light", "hdr"],
            SettingsGroup::ToneCurve => &["curve"],
            SettingsGroup::Color => &["color"],
            SettingsGroup::ColorMixer => &["mixer", "bw_mix", "point_colors"],
            SettingsGroup::ColorGrading => &["grading"],
            SettingsGroup::Effects => &["effects"],
            SettingsGroup::Vignette => &["vignette"],
            SettingsGroup::Grain => &["grain"],
            SettingsGroup::Detail => &["detail", "enhance"],
            SettingsGroup::Optics => &["optics"],
            SettingsGroup::Geometry => &["geometry"],
            SettingsGroup::Crop => &["crop", "orientation"],
            SettingsGroup::Masks => &["masks"],
            SettingsGroup::Spots => &["spots"],
            SettingsGroup::RedEye => &["red_eye"],
            SettingsGroup::LensBlur => &["lens_blur"],
            SettingsGroup::Calibration => &["calibration"],
        }
    }
}

/// The subset of `s` covering `groups`, as a partial JSON object.
pub fn extract_groups(s: &DevelopSettings, groups: &[SettingsGroup]) -> Value {
    let full = s.to_json_for_merge();
    let mut out = Map::new();
    for g in groups {
        for k in g.keys() {
            if let Some(v) = full.get(*k) {
                out.insert((*k).to_string(), v.clone());
            }
        }
    }
    Value::Object(out)
}

/// Recursively merge `patch` into `base` (objects merge; everything else replaces).
pub fn deep_merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(k) {
                    Some(bv) if bv.is_object() && v.is_object() => deep_merge(bv, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, p) => *b = p.clone(),
    }
}

/// Interpolate numbers in `patch` towards/away from `base` by `t` (1 = patch as-is); non-numeric
/// values are taken from the patch when `t >= 0.5`.
fn scale_patch(base: &Value, patch: &Value, t: f64) -> Value {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            let mut out = Map::new();
            for (k, pv) in p {
                let bv = b.get(k).unwrap_or(&Value::Null);
                out.insert(k.clone(), scale_patch(bv, pv, t));
            }
            Value::Object(out)
        }
        // Integer fields (the grain seed, ids) are discrete like strings: an interpolated seed would
        // not deserialize as an integer, and the whole preset would silently not apply.
        (Value::Number(b), Value::Number(p)) if b.is_f64() || p.is_f64() => {
            let (b, p) = (b.as_f64().unwrap_or(0.0), p.as_f64().unwrap_or(0.0));
            serde_json::Number::from_f64(b + (p - b) * t).map(Value::Number).unwrap_or(Value::Null)
        }
        (b, p) => {
            if t >= 0.5 {
                p.clone()
            } else {
                b.clone()
            }
        }
    }
}

/// Apply a partial settings object with an amount (1.0 = 100 %). Out-of-range values are clamped
/// through the control specs, as every merge does ([`DevelopSettings::merged`]).
pub fn apply_partial(s: &DevelopSettings, partial: &Value, amount: f64) -> DevelopSettings {
    let patch = if (amount - 1.0).abs() < 1e-9 { Cow::Borrowed(partial) } else { Cow::Owned(scale_patch(&s.to_json_for_merge(), partial, amount)) };
    // (a white balance without a scale keeps its old meaning: `DevelopSettings::merged`)
    s.merged(&patch).unwrap_or_else(|_| s.clone())
}

/// Top-level settings keys that hold per-photo pixel repairs: a preset's named repairs compose on
/// the photo's own values instead of the original's (masks are added by [`Preset::apply_on`]), and
/// a preset without them leaves the photo's alone.
const REPAIR_KEYS: [&str; 3] = ["masks", "spots", "red_eye"];

/// The settings `settings` (a partial JSON object) names, applied to `current` at `amount`: each
/// named setting is computed from `original` (the photo's import state) and the preset's value,
/// and replaces the photo's value (issue storytold/lightcraft#2159). Every setting the preset does not name keeps the
/// photo's current value, edits included. Pixel repairs compose on the photo's own values, as they
/// always have.
fn compose_named(current: &DevelopSettings, original: &DevelopSettings, settings: &Value, amount: f64) -> DevelopSettings {
    // At 100 % the preset's own values apply as they are, and `original` is not read. The presets
    // panel calls this for every visible row on every frame, so the scaling base is built below only.
    if (amount - 1.0).abs() < 1e-9 {
        return current.merged(settings).unwrap_or_else(|_| current.clone());
    }
    let cur = current.to_json();
    let mut base = original.to_json();
    if let (Some(b), Some(c)) = (base.as_object_mut(), cur.as_object()) {
        for k in REPAIR_KEYS {
            if let Some(v) = c.get(k) {
                b.insert(k.to_string(), v.clone());
            }
        }
    }
    let composed = match settings.as_object() {
        Some(patch) => Value::Object(interpolate_object(base.as_object(), patch, amount)),
        None => Value::Object(Map::new()),
    };
    // `merged` deep-merges: the named settings replace, everything else stays as `current` has it
    current.merged(&composed).unwrap_or_else(|_| current.clone())
}

/// The leaves of `patch` that apply at `t` (below 100 %), each computed from the same place in
/// `base`. A leaf the amount does not reach is left out, so the photo keeps its own value there.
fn interpolate_object(base: Option<&Map<String, Value>>, patch: &Map<String, Value>, t: f64) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, pv) in patch {
        let bv = base.and_then(|b| b.get(k));
        let v = match pv {
            Value::Object(p) => Some(Value::Object(interpolate_object(bv.and_then(Value::as_object), p, t))),
            _ => interpolate_leaf(bv, pv, t),
        };
        if let Some(v) = v {
            out.insert(k.clone(), v);
        }
    }
    out
}

/// One leaf at `t`. Numbers move from `base` towards `patch`. Other values switch to the preset's
/// at half amount and up, and below it take the base's value. A null in the base is the baseline
/// value the import state recorded (a cleared option, such as a free crop aspect): below half
/// amount it applies and clears the photo's value, as the amount contract promises. A null the
/// preset writes is a value like any other, so it applies at half amount and up (a preset can
/// clear an option field such as a crop aspect).
fn interpolate_leaf(base: Option<&Value>, patch: &Value, t: f64) -> Option<Value> {
    match (base, patch) {
        // integer fields (the grain seed, ids) are discrete like strings: see `scale_patch`
        (Some(Value::Number(b)), Value::Number(p)) if b.is_f64() || p.is_f64() => {
            let (b, p) = (b.as_f64().unwrap_or(0.0), p.as_f64().unwrap_or(0.0));
            serde_json::Number::from_f64(b + (p - b) * t).map(Value::Number)
        }
        _ if t >= 0.5 => Some(patch.clone()),
        _ => base.cloned(),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub group: String,
    /// Partial settings.
    pub settings: Value,
    #[serde(default)]
    pub favorite: bool,
    /// Built-in presets ship with the app and can't be deleted.
    #[serde(default)]
    pub builtin: bool,
}

impl Preset {
    pub fn from_settings(id: &str, name: &str, group: &str, s: &DevelopSettings, groups: &[SettingsGroup]) -> Preset {
        Preset { id: id.into(), name: name.into(), group: group.into(), settings: extract_groups(s, groups), favorite: false, builtin: false }
    }
    /// Apply at `amount` (1.0 = 100 %), with `s` as both the photo's current and its original
    /// settings (see [`Preset::apply_on`]): right for the import default preset and for scratch
    /// bases. Masks in the preset are added to the photo's own (ones it already has are skipped),
    /// their effect scaled by the amount.
    pub fn apply(&self, s: &DevelopSettings, amount: f64) -> DevelopSettings {
        self.apply_on(s, s, amount)
    }
    /// Apply at `amount` (1.0 = 100 %), composing on `original` (the photo's import state, or the
    /// photo's current settings when that is not known): each setting the preset names is computed
    /// from `original` and the preset's value, `amount` moving it between the two, and replaces the
    /// photo's value. Every setting the preset does not name keeps `current`'s value, and so do
    /// the photo's pixel repairs (masks, spots, red eye), which the preset patches; masks in the
    /// preset are added to the photo's own (ones it already has are skipped), their effect scaled
    /// by the amount. A second preset therefore restores what the first one changed in the
    /// settings it names (issue storytold/lightcraft#2159), and nothing it leaves out.
    pub fn apply_on(&self, current: &DevelopSettings, original: &DevelopSettings, amount: f64) -> DevelopSettings {
        let Some(masks) = self.settings.get("masks").and_then(Value::as_array).filter(|m| !m.is_empty()) else {
            return compose_named(current, original, &self.settings, amount);
        };
        let mut rest = self.settings.clone();
        if let Some(o) = rest.as_object_mut() {
            o.remove("masks");
        }
        let mut out = compose_named(current, original, &rest, amount);
        add_masks(&mut out, masks, amount);
        out
    }
}

/// The preset's masks, added to the photo's own (ones it already has are skipped), their effect
/// scaled by the amount.
fn add_masks(out: &mut DevelopSettings, masks: &[Value], amount: f64) {
    // the masks by name, kept up to date: a preset of many masks compares each with its
    // namesakes only, not with every mask added before it
    let mut by_name: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();
    for (i, q) in out.masks.iter().enumerate() {
        by_name.entry(q.name.clone()).or_default().push(i);
    }
    let mut next_id = out.next_mask_id();
    let mut used: std::collections::HashSet<u32> = out.masks.iter().map(|q| q.id).collect();
    for m in masks {
        let Ok(mut m) = serde_json::from_value::<crate::Mask>(m.clone()) else { continue };
        if by_name.get(&m.name).is_some_and(|ix| ix.iter().any(|&i| out.masks.get(i).is_some_and(|q| q.components == m.components))) {
            continue;
        }
        // ids run on from the first free one, past any a mask already has (a scan for the
        // next free id per mask would be another pass over them)
        while used.contains(&next_id) {
            next_id = next_id.checked_add(1).unwrap_or(1);
        }
        m.id = next_id;
        used.insert(next_id);
        m.adjust.amount = (m.adjust.amount * amount).clamp(0.0, 200.0);
        by_name.entry(m.name.clone()).or_default().push(out.masks.len());
        out.masks.push(m);
    }
}

/// Lower-case ASCII slug for ids (`"Warm & Soft"` → `warm-soft`).
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "preset".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_and_apply_roundtrip() {
        let mut a = DevelopSettings::default();
        a.light.exposure = 1.0;
        a.effects.clarity = 30.0;
        a.crop.geometry.angle = 5.0;
        let p = Preset::from_settings("p", "P", "User", &a, &SettingsGroup::default_copy());
        assert!(p.settings.get("crop").is_none());
        let b = p.apply(&DevelopSettings::default(), 1.0);
        assert_eq!(b.light.exposure, 1.0);
        assert_eq!(b.effects.clarity, 30.0);
        assert_eq!(b.crop.geometry.angle, 0.0);
    }

    #[test]
    fn amount_scales_numbers() {
        let base = DevelopSettings::default();
        let partial = json!({"light": {"exposure": 1.0, "contrast": 40}});
        let half = apply_partial(&base, &partial, 0.5);
        assert!((half.light.exposure - 0.5).abs() < 1e-9);
        assert!((half.light.contrast - 20.0).abs() < 1e-9);
        let more = apply_partial(&base, &partial, 2.0);
        assert!((more.light.contrast - 80.0).abs() < 1e-9);
        // clamped through specs
        let lots = apply_partial(&base, &json!({"light": {"exposure": 4.0}}), 2.0);
        assert_eq!(lots.light.exposure, 5.0);
    }

    /// A preset with the Grain group (its seed is an integer) used to do nothing at any amount
    /// other than 100 %: the interpolated seed failed to deserialize and the preset was dropped.
    #[test]
    fn amount_keeps_integer_fields_whole() {
        let mut a = DevelopSettings::default();
        a.light.exposure = 1.0;
        a.grain.amount = 40.0;
        a.grain.seed = 12345;
        let p = Preset::from_settings("p", "P", "User", &a, &SettingsGroup::default_copy());
        assert!(p.settings.pointer("/grain/seed").is_some_and(Value::is_u64), "the default copy groups include the grain seed");
        let base = DevelopSettings::default();
        for (amount, seed) in [(0.3, 0), (0.5, 12345), (1.5, 12345), (2.0, 12345)] {
            let out = p.apply(&base, amount);
            assert!((out.light.exposure - amount.min(5.0)).abs() < 1e-9, "{amount}: exposure {}", out.light.exposure);
            assert!((out.grain.amount - (40.0 * amount).min(100.0)).abs() < 1e-9, "{amount}: grain {}", out.grain.amount);
            assert_eq!(out.grain.seed, seed, "{amount}: seed");
        }
    }

    /// A preset changes only the settings it names: a setting in a group it styles but does not
    /// name keeps the photo's value, and so does a whole group it leaves out.
    #[test]
    fn apply_on_changes_only_the_settings_it_names() {
        let mut current = DevelopSettings::default();
        current.light.exposure = 2.0;
        current.light.blacks = -30.0;
        current.effects.clarity = 30.0;
        let original = DevelopSettings::default();
        let p = Preset {
            id: "p".into(),
            name: "P".into(),
            group: "G".into(),
            settings: json!({"light": {"contrast": 12.0}}),
            favorite: false,
            builtin: false,
        };
        let out = p.apply_on(&current, &original, 1.0);
        assert_eq!(out.light.contrast, 12.0);
        assert_eq!(out.light.exposure, 2.0, "a setting the preset doesn't name keeps the photo's value");
        assert_eq!(out.light.blacks, -30.0, "a setting in the same group, not named, keeps the photo's value");
        assert_eq!(out.effects.clarity, 30.0, "a group the preset doesn't name is untouched");
    }

    /// A partial preset that names one colour of the mixer leaves the other colours, and the
    /// unnamed settings of the colour it names, as the photo has them.
    #[test]
    fn apply_on_keeps_the_mixer_colours_it_does_not_name() {
        let mut current = DevelopSettings::default();
        current.mixer.red.sat = 20.0;
        current.mixer.blue.hue = 15.0;
        current.mixer.blue.sat = 5.0;
        let original = DevelopSettings::default();
        let p = Preset {
            id: "p".into(),
            name: "P".into(),
            group: "G".into(),
            settings: json!({"mixer": {"blue": {"sat": 15.0, "lum": -25.0}}}),
            favorite: false,
            builtin: false,
        };
        let out = p.apply_on(&current, &original, 1.0);
        assert_eq!(out.mixer.red.sat, 20.0, "a colour the preset doesn't name");
        assert_eq!(out.mixer.blue.hue, 15.0, "a setting of a named colour that the preset doesn't name");
        assert_eq!((out.mixer.blue.sat, out.mixer.blue.lum), (15.0, -25.0));
    }

    /// The white balance is not one setting: a preset that names only the temperature keeps the
    /// photo's tint.
    #[test]
    fn apply_on_keeps_the_white_balance_fields_it_does_not_name() {
        let mut current = DevelopSettings::default();
        current.wb.temp = 6000.0;
        current.wb.tint = 12.0;
        let original = DevelopSettings::default();
        let p = Preset {
            id: "p".into(),
            name: "P".into(),
            group: "G".into(),
            settings: json!({"wb": {"temp": 5000.0}}),
            favorite: false,
            builtin: false,
        };
        let out = p.apply_on(&current, &original, 1.0);
        assert_eq!((out.wb.temp, out.wb.tint), (5000.0, 12.0));
    }

    /// A preset's amount 0 changes nothing when the original is the photo's current settings (no
    /// import state is recorded for the photo), and any other amount moves the named settings
    /// from where the photo has them.
    #[test]
    fn apply_on_without_an_import_state_scales_from_the_current_settings() {
        let mut current = DevelopSettings::default();
        current.light.exposure = 2.0;
        current.light.blacks = -30.0;
        let p = Preset {
            id: "p".into(),
            name: "P".into(),
            group: "G".into(),
            settings: json!({"light": {"exposure": 0.45, "contrast": 12.0}}),
            favorite: false,
            builtin: false,
        };
        assert_eq!(p.apply_on(&current, &current, 0.0), current, "amount 0 applies nothing");
        let half = p.apply_on(&current, &current, 0.5);
        assert!((half.light.exposure - 1.225).abs() < 1e-9, "{}", half.light.exposure);
        assert_eq!(half.light.blacks, -30.0);
    }

    /// An unresolved `crs:Look` reference (its profile object has no id; the importer leaves
    /// `look` in it) changes nothing: the photo keeps its own profile.
    #[test]
    fn an_unresolved_profile_reference_keeps_the_photos_profile() {
        let mut current = DevelopSettings::default();
        current.profile.id = "lc.vivid".into();
        current.profile.amount = 60.0;
        let p = Preset {
            id: "p".into(),
            name: "P".into(),
            group: "G".into(),
            settings: json!({"profile": {"look": {"name": "Not imported", "amount": 100.0}}, "light": {"contrast": 12.0}}),
            favorite: false,
            builtin: false,
        };
        for amount in [1.0, 0.5] {
            let out = p.apply_on(&current, &current, amount);
            assert_eq!((out.profile.id.as_str(), out.profile.amount), ("lc.vivid", 60.0), "amount {amount}");
        }
    }

    /// The amount interpolates between the original and the preset, not between the visible
    /// values and the preset: a preset at 50 % looks the same however the photo was edited before.
    #[test]
    fn apply_on_scales_against_the_original() {
        let mut current = DevelopSettings::default();
        current.light.exposure = 2.0;
        let original = DevelopSettings::default();
        let p = Preset {
            id: "p".into(),
            name: "P".into(),
            group: "G".into(),
            settings: json!({"light": {"exposure": 0.45}}),
            favorite: false,
            builtin: false,
        };
        let out = p.apply_on(&current, &original, 0.5);
        assert!((out.light.exposure - 0.225).abs() < 1e-9, "{}", out.light.exposure);
    }

    /// Pixel repairs are the photo's own: a preset's are patched in, and one without them never
    /// restores the original's.
    #[test]
    fn apply_on_patches_pixel_repairs_into_the_current_ones() {
        let current = DevelopSettings {
            spots: serde_json::from_value(json!([{"mode": "heal", "points": [[0.5, 0.5]], "size": 0.05, "feather": 50.0, "opacity": 100.0}]))
                .unwrap(),
            ..Default::default()
        };
        let original = DevelopSettings::default();
        let none = Preset {
            id: "p".into(),
            name: "P".into(),
            group: "G".into(),
            settings: json!({"light": {"contrast": 10.0}}),
            favorite: false,
            builtin: false,
        };
        let out = none.apply_on(&current, &original, 1.0);
        assert_eq!(out.spots.len(), 1, "the photo's spot survives a preset without spots");
        let clearing = Preset {
            id: "q".into(),
            name: "Q".into(),
            group: "G".into(),
            settings: json!({"light": {"contrast": 10.0}, "spots": []}),
            favorite: false,
            builtin: false,
        };
        let out = clearing.apply_on(&current, &original, 1.0);
        assert_eq!(out.spots.len(), 0, "a preset that names spots replaces them, as before");
    }

    /// A null a preset writes into an option field (here a crop aspect: null is "free") is the
    /// preset's value, so it clears the photo's aspect from half amount up, as any value of the
    /// preset does; below half amount the import baseline applies, which here is null as well.
    #[test]
    fn a_preset_null_clears_an_option_field_from_half_amount() {
        let mut current = DevelopSettings::default();
        current.crop.aspect = Some((3, 2));
        let original = DevelopSettings::default(); // the import state has no aspect
        let p = Preset {
            id: "p".into(),
            name: "P".into(),
            group: "G".into(),
            settings: json!({"crop": {"aspect": null}}),
            favorite: false,
            builtin: false,
        };
        for amount in [1.0, 0.7, 0.5] {
            assert_eq!(p.apply_on(&current, &original, amount).crop.aspect, None, "amount {amount}: the preset's null clears the aspect");
        }
        assert_eq!(p.apply_on(&current, &original, 0.3).crop.aspect, None, "below half amount the import baseline (no aspect) applies");
    }

    /// The import state has no aspect, and that null is the baseline below half amount (the amount
    /// contract: a named setting moves between the import state and the preset's value); from half
    /// amount the preset's aspect applies.
    #[test]
    fn a_null_in_the_original_is_the_baseline_below_half_amount() {
        let mut current = DevelopSettings::default();
        current.crop.aspect = Some((3, 2));
        let original = DevelopSettings::default();
        let p = Preset {
            id: "p".into(),
            name: "P".into(),
            group: "G".into(),
            settings: json!({"crop": {"aspect": [16, 9]}}),
            favorite: false,
            builtin: false,
        };
        assert_eq!(p.apply_on(&current, &original, 0.4).crop.aspect, None, "40 %: the import baseline (no aspect) applies");
        assert_eq!(p.apply_on(&current, &original, 0.6).crop.aspect, Some((16, 9)), "60 %: the preset's aspect applies");
    }

    #[test]
    fn deep_merge_objects() {
        let mut a = json!({"x": {"a": 1, "b": 2}, "y": [1]});
        deep_merge(&mut a, &json!({"x": {"b": 3}, "y": [2, 3]}));
        assert_eq!(a, json!({"x": {"a": 1, "b": 3}, "y": [2, 3]}));
    }

    #[test]
    fn preset_masks_get_unused_ids_after_a_mask_id_at_the_limit() {
        let s = DevelopSettings { masks: vec![crate::Mask { id: u32::MAX, name: "Theirs".into(), ..Default::default() }], ..Default::default() };
        let p = Preset {
            id: "p".into(),
            name: "P".into(),
            group: "User".into(),
            settings: json!({"masks": [{"id": 7, "name": "A"}, {"id": 7, "name": "B"}]}),
            favorite: false,
            builtin: false,
        };
        let out = p.apply(&s, 1.0);
        let mut ids: Vec<u32> = out.masks.iter().map(|m| m.id).collect();
        assert_eq!(ids.len(), 3);
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 3, "{:?}", out.masks);
    }

    #[test]
    fn preset_masks_skip_ids_an_existing_mask_has_after_the_first_free_one() {
        // the first free id is 1, and 2 is taken: the second added mask must not get 2
        let theirs = |id| crate::Mask { id, name: format!("T{id}"), ..Default::default() };
        let s = DevelopSettings { masks: vec![theirs(u32::MAX), theirs(2)], ..Default::default() };
        let p = Preset {
            id: "p".into(),
            name: "P".into(),
            group: "User".into(),
            settings: json!({"masks": [{"name": "A"}, {"name": "B"}]}),
            favorite: false,
            builtin: false,
        };
        let mut ids: Vec<u32> = p.apply(&s, 1.0).masks.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, [1, 2, 3, u32::MAX]);
    }
}
