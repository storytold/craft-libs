//! The non-destructive edit model of LightCraft.
//!
//! [`DevelopSettings`] is the complete description of a photo's look: pure, serializable data.
//! The pipeline evaluates it; the catalog stores it; presets, copy/paste, sync, versions and history
//! are operations on it. Numeric sliders are addressed by id through [`controls`].
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod controls;
pub mod crs;
pub mod crs_masks;
pub mod presets;
pub mod segmask;
pub mod settings;

pub use controls::{CONTROLS, ControlSpec, Section, Track};
pub use presets::{Preset, SettingsGroup, apply_partial, extract_groups};
pub use segmask::{SegMask, with_grid_interner};
pub use settings::*;

use serde_json::Value;

impl DevelopSettings {
    /// Settings for a freshly imported raw file (Lightroom applies capture sharpening and colour
    /// noise reduction by default to raw files).
    pub fn for_raw(as_shot_temp: f64, as_shot_tint: f64) -> DevelopSettings {
        let mut s = DevelopSettings::default();
        s.wb.temp = as_shot_temp;
        s.wb.tint = as_shot_tint;
        s.detail.sharpen_amount = 40.0;
        s.detail.nr_color = 25.0;
        s.detail.nr_outliers = true;
        s
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn from_json(v: &Value) -> Result<DevelopSettings, serde_json::Error> {
        serde_json::from_value(v.clone())
    }

    /// [`Self::to_json`] with Remove Hot Pixels (`detail.nr_outliers`) stated even when off, for JSON that is copied,
    /// merged, scaled or compared key by key (copy / paste, presets, Auto Sync). The stored JSON leaves it out when
    /// off, so settings saved before it keep their hash; a merge needs the `false`, or copying Detail would not turn
    /// the box off and a preset applied at an amount would not find its base value.
    pub fn to_json_for_merge(&self) -> Value {
        let mut v = self.to_json();
        if let Some(detail) = v.get_mut("detail").and_then(Value::as_object_mut) {
            detail.insert("nr_outliers".into(), Value::Bool(self.detail.nr_outliers));
        }
        v
    }

    /// [`Self::from_json`] for settings from outside the app, such as a sidecar's `lc:settings`
    /// (issue storytold/lightcraft#2555): kept in range as [`Self::merged`] keeps them.
    pub fn from_untrusted_json(v: &Value) -> Result<DevelopSettings, serde_json::Error> {
        let mut s = DevelopSettings::from_json(v)?;
        s.keep_in_range(&DevelopSettings::default());
        Ok(s)
    }

    /// Merge a partial JSON object into these settings (fields not mentioned are kept). A white balance that
    /// names no [`WbScale`] keeps the meaning it always had (`Legacy`), whatever scale these settings were on: a
    /// partial from before the scale existed, or one that never said, must not become Kelvin by landing on
    /// Kelvin settings. Out-of-range values are brought back into range ([`DevelopSettings::keep_in_range`]).
    pub fn merged(&self, partial: &Value) -> Result<DevelopSettings, serde_json::Error> {
        let mut v = self.to_json();
        presets::deep_merge(&mut v, partial);
        let mut out = DevelopSettings::from_json(&v)?;
        if let Some(wb) = partial.get("wb").and_then(Value::as_object)
            && (wb.contains_key("temp") || wb.contains_key("tint"))
            && !wb.contains_key("scale")
        {
            out.wb.scale = WbScale::Legacy;
        }
        out.keep_in_range(self);
        Ok(out)
    }

    /// What crs XMP import does to the values it reads, for settings that arrive as JSON (presets, `develop.merge`,
    /// `render --settings`; issue storytold/lightcraft#1946): every slider clamped through its control spec, point curves in 0..1, and a
    /// crop with no area left as it was in `before`. The angle is kept within ±45° (not finite: 0). Unrotated, the crop
    /// is clipped to the photo first; a turned crop may reach past 0..1 and is bounded where the photo's size is known
    /// (`CropGeometry::bounded`, applied by the pipeline).
    fn keep_in_range(&mut self, before: &DevelopSettings) {
        for c in controls::CONTROLS {
            if let Some(v) = controls::get(self, c.id) {
                controls::set(self, c.id, v);
            }
        }
        let c = &mut self.curve;
        for p in [&mut c.master, &mut c.red, &mut c.green, &mut c.blue].into_iter().flatten() {
            *p = craft_geom::Point::new(p.x.clamp(0.0, 1.0), p.y.clamp(0.0, 1.0));
        }
        let g = &mut self.crop.geometry;
        g.angle = if g.angle.is_finite() { g.angle.clamp(-45.0, 45.0) } else { 0.0 };
        if g.angle == 0.0 {
            g.rect = g.rect.intersect(&craft_geom::Rect::UNIT);
        }
        if g.rect.is_empty() {
            let old = before.crop.geometry.rect;
            g.rect = if old.is_empty() { craft_geom::Rect::UNIT } else { old };
        }
    }

    /// Stable 64-bit hash of the settings (FNV-1a over canonical JSON), for preview cache keys.
    pub fn hash64(&self) -> u64 {
        let s = serde_json::to_string(self).unwrap_or_default();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in s.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h
    }

    /// True if nothing differs from a fresh default (ignoring white balance "as shot" values, and
    /// the rendering process: that is not an edit).
    pub fn is_unedited(&self) -> bool {
        let mut a = self.clone();
        let d = DevelopSettings::default();
        a.wb = d.wb;
        a.process = d.process;
        a == d
    }

    pub fn section_enabled(&self, section: &str) -> bool {
        !self.disabled_sections.iter().any(|s| s == section)
    }

    /// The settings as rendered: an Edit-panel section whose eye is off contributes nothing (issue
    /// storytold/lightcraft#316). Light (with its tone curve), Color (white balance, presence, the mixers, colour grading
    /// and point colour) and Detail are reset to their defaults here; Effects, Optics, Geometry and
    /// Calibration the pipeline switches off where it applies them. Borrowed when every section is on.
    pub fn effective(&self) -> std::borrow::Cow<'_, DevelopSettings> {
        let off = |section: &str| !self.section_enabled(section);
        if !(off("light") || off("color") || off("detail")) {
            return std::borrow::Cow::Borrowed(self);
        }
        let mut d = self.clone();
        if off("light") {
            d.reset_section(Section::Light);
            d.reset_section(Section::Curve);
        }
        if off("color") {
            for section in [Section::Color, Section::Mixer, Section::BwMix, Section::Grading, Section::PointColor] {
                d.reset_section(section);
            }
            d.point_colors.clear();
        }
        if off("detail") {
            d.reset_section(Section::Detail);
        }
        std::borrow::Cow::Owned(d)
    }

    /// Whether a raw is decoded with its single outlier photosites replaced: Remove Hot Pixels is on and the Detail
    /// section's eye is too (an Edit-panel section that is off contributes nothing, as in [`Self::effective`]).
    pub fn removes_hot_pixels(&self) -> bool {
        self.detail.nr_outliers && self.section_enabled("detail")
    }

    /// How much of the AI-denoised picture to mix in, 0 (none) to 1 (all of it): the Denoise amount, unless the Detail
    /// section is switched off or the value is not a number.
    pub fn denoise_amount(&self) -> f32 {
        let a = self.enhance.denoise;
        if a.is_finite() && self.enhance.denoise_enabled() && self.section_enabled("detail") { (a / 100.0).clamp(0.0, 1.0) as f32 } else { 0.0 }
    }

    pub fn set_section_enabled(&mut self, section: &str, on: bool) {
        self.disabled_sections.retain(|s| s != section);
        if !on {
            self.disabled_sections.push(section.to_string());
        }
    }

    /// Reset every control in `section` to its default, and the settings of the section that are not
    /// controls (the creative profile, the curve, the white balance mode).
    pub fn reset_section(&mut self, section: Section) {
        for c in controls::in_section(section) {
            controls::set(self, c.id, c.default);
        }
        match section {
            Section::Curve => {
                let d = ToneCurve::default();
                self.curve = d;
            }
            Section::Color => self.wb.mode = WbMode::AsShot,
            Section::Detail => {
                self.enhance.denoise_on = None;
                // not a slider: reset like them, to the general default (off)
                self.detail.nr_outliers = false;
            }
            // the creative profile is not a control: the chosen look goes back to the default one,
            // and so does the camera's own local tone mapping
            Section::Profile => {
                self.profile = Profile::default();
            }
            _ => {}
        }
    }

    /// The middle of the crop, in normalized photo coordinates (the photo's middle when the crop
    /// isn't a usable rectangle).
    pub fn crop_center(&self) -> craft_geom::Point {
        let r = self.crop.geometry.rect;
        let (x, y) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
        if x.is_finite() && y.is_finite() { craft_geom::Point::new(x, y) } else { craft_geom::Point::new(0.5, 0.5) }
    }

    /// Next free mask id: one past the highest, or, when that is `u32::MAX` (ids come from files,
    /// pastes and agents), the lowest one no mask uses.
    pub fn next_mask_id(&self) -> u32 {
        match self.masks.iter().map(|m| m.id).max() {
            None => 1,
            // fewer masks than ids: one of the first `masks.len() + 1` is free
            Some(max) => max
                .checked_add(1)
                .or_else(|| {
                    let used: std::collections::HashSet<u32> = self.masks.iter().map(|m| m.id).collect();
                    (1..=u32::MAX).find(|id| !used.contains(id))
                })
                .unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod tests_process;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Remove Hot Pixels (storytold/lightcraft#1345) is on for a newly imported raw and off for everything else, and an edit saved
    /// before the box existed reads as off, so it renders as it did.
    #[test]
    fn hot_pixel_box_is_on_for_new_raws_only() {
        assert!(DevelopSettings::for_raw(5500.0, 10.0).detail.nr_outliers);
        assert!(!DevelopSettings::default().detail.nr_outliers);
        let mut saved = DevelopSettings::for_raw(5500.0, 10.0).to_json();
        let detail = saved.get_mut("detail").and_then(|d| d.as_object_mut()).unwrap();
        assert!(detail.remove("nr_outliers").is_some());
        let read = DevelopSettings::from_json(&saved).unwrap();
        assert!(!read.detail.nr_outliers);
        assert_eq!(read.detail.nr_color, 25.0, "the rest of the saved edit is kept");
        assert_ne!(read.hash64(), DevelopSettings::for_raw(5500.0, 10.0).hash64(), "cached renders are told apart");
        // off is omitted: an edit saved before the box keeps its JSON, and so its hash and cached thumbnails
        assert_eq!(read.to_json(), saved);
        // copying Detail still carries the box when it is off
        let copied = crate::extract_groups(&read, &[crate::SettingsGroup::Detail]);
        assert_eq!(copied["detail"]["nr_outliers"], false);
        let mut target = DevelopSettings::for_raw(5500.0, 10.0).to_json();
        crate::presets::deep_merge(&mut target, &copied);
        assert!(!DevelopSettings::from_json(&target).unwrap().detail.nr_outliers);
        // the Detail eye switches it off for the decode too, and Reset Detail resets it like the sliders
        let mut raw = DevelopSettings::for_raw(5500.0, 10.0);
        assert!(raw.removes_hot_pixels());
        raw.set_section_enabled("detail", false);
        assert!(!raw.removes_hot_pixels());
        raw.set_section_enabled("detail", true);
        raw.reset_section(Section::Detail);
        assert!(!raw.detail.nr_outliers && !raw.removes_hot_pixels());
    }

    #[test]
    fn serde_roundtrip_default_and_edited() {
        let mut s = DevelopSettings::default();
        controls::set(&mut s, "light.exposure", 1.25);
        s.masks.push(Mask {
            id: 1,
            components: vec![MaskComponent {
                name: None,
                op: MaskOp::Add,
                invert: false,
                shape: MaskShape::Radial { center: craft_geom::Point::new(0.5, 0.5), rx: 0.2, ry: 0.1, angle: 10.0, feather: 50.0, invert: false },
            }],
            ..Default::default()
        });
        let v = s.to_json();
        let back = DevelopSettings::from_json(&v).unwrap();
        assert_eq!(back, s);
        assert_ne!(back.hash64(), DevelopSettings::default().hash64());
    }

    #[test]
    fn white_balance_scale_reads_absent_as_legacy_and_writes_only_kelvin() {
        // settings from before the scale existed: no field, `Legacy`, and written back without it
        let old = json!({"wb": {"mode": "custom", "temp": 5800.0, "tint": 3.0}});
        let s = DevelopSettings::from_json(&old).unwrap();
        assert_eq!(s.wb.scale, WbScale::Legacy);
        assert_eq!(s.to_json()["wb"], old["wb"]);
        assert_eq!(DevelopSettings::default().wb.scale, WbScale::Legacy);
        assert!(DevelopSettings::default().to_json()["wb"].get("scale").is_none());
        // written on the Kelvin scale: said so, and read back
        let mut k = s.clone();
        k.wb.scale = WbScale::Kelvin;
        assert_eq!(k.to_json()["wb"]["scale"], json!("kelvin"));
        assert_eq!(DevelopSettings::from_json(&k.to_json()).unwrap(), k);
        assert_ne!(k.hash64(), s.hash64());
        // an unknown scale from a newer build is an error, not a panic
        assert!(DevelopSettings::from_json(&json!({"wb": {"scale": "mired"}})).is_err());
    }

    #[test]
    fn a_partial_white_balance_without_a_scale_is_legacy() {
        let mut base = DevelopSettings::default();
        base.wb.scale = WbScale::Kelvin;
        // an old preset's white balance lands on Kelvin settings: it keeps its relative meaning
        let out = apply_partial(&base, &json!({"wb": {"mode": "custom", "temp": 5800.0}}), 1.0);
        assert_eq!((out.wb.scale, out.wb.temp), (WbScale::Legacy, 5800.0));
        let out = apply_partial(&base, &json!({"wb": {"tint": 12.0}}), 1.0);
        assert_eq!(out.wb.scale, WbScale::Legacy);
        // one that says Kelvin stays Kelvin; a mode alone leaves the scale as it was
        let out = apply_partial(&base, &json!({"wb": {"mode": "custom", "temp": 5800.0, "scale": "kelvin"}}), 1.0);
        assert_eq!(out.wb.scale, WbScale::Kelvin);
        let out = apply_partial(&base, &json!({"wb": {"mode": "cloudy"}}), 1.0);
        assert_eq!(out.wb.scale, WbScale::Kelvin);
        let out = apply_partial(&base, &json!({"light": {"exposure": 1.0}}), 1.0);
        assert_eq!(out.wb.scale, WbScale::Kelvin);
        // the plain merge (`develop.merge`) follows the same rule
        assert_eq!(base.merged(&json!({"wb": {"temp": 5800.0}})).unwrap().wb.scale, WbScale::Legacy);
        assert_eq!(base.merged(&json!({"wb": {"temp": 5800.0, "scale": "kelvin"}})).unwrap().wb.scale, WbScale::Kelvin);
        assert_eq!(base.merged(&json!({"wb": {"mode": "auto"}})).unwrap().wb.scale, WbScale::Kelvin);
    }

    #[test]
    fn default_hdr_is_not_serialized_so_existing_hashes_stay() {
        let s = DevelopSettings::default();
        assert!(s.to_json().get("hdr").is_none(), "HDR off must not change the settings JSON or hash64");
        let mut on = s.clone();
        on.hdr.enabled = true;
        assert!(on.to_json().get("hdr").is_some());
        assert_ne!(on.hash64(), s.hash64());
        assert_eq!(DevelopSettings::from_json(&on.to_json()).unwrap(), on);
    }

    #[test]
    fn hdr_peak_is_finite_for_a_non_finite_headroom() {
        let mut h = Hdr { enabled: true, max_ev: f64::NAN, ..Hdr::default() };
        assert_eq!(h.peak(), (Hdr::DEFAULT_MAX_EV as f32).exp2());
        h.max_ev = f64::INFINITY;
        assert_eq!(h.peak(), (Hdr::DEFAULT_MAX_EV as f32).exp2());
        h.max_ev = -3.0;
        assert_eq!(h.peak(), 1.0);
    }

    #[test]
    fn missing_fields_take_defaults_and_unknown_ignored() {
        let s = DevelopSettings::from_json(&json!({"light": {"exposure": 0.5}, "futureThing": 3})).unwrap();
        assert_eq!(s.light.exposure, 0.5);
        assert_eq!(s.grading.blending, 50.0);
    }

    /// Settings with one colour-range mask of `n` distinct samples (built in code, so not capped).
    fn color_range_settings(n: usize) -> DevelopSettings {
        let samples = (0..n).map(|i| [i as f64 / 1000.0, 0.1, -0.1]).collect();
        let mut s = DevelopSettings::default();
        s.masks.push(Mask {
            id: 1,
            components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape: MaskShape::ColorRange { samples, refine: 50.0 } }],
            ..Default::default()
        });
        s
    }

    fn color_range_samples(s: &DevelopSettings) -> &[[f64; 3]] {
        match s.masks.first().and_then(|m| m.components.first()).map(|c| &c.shape) {
            Some(MaskShape::ColorRange { samples, .. }) => samples,
            other => panic!("expected a colour range, got {other:?}"),
        }
    }

    /// Issue storytold/lightcraft#2556: `mask.add`/`mask.update` capped a colour range's samples, but a sidecar `lc:settings`, a
    /// preset, `develop.merge`, `render --settings` or a catalog / history load took any number (render cost is
    /// pixels x samples). Every load now keeps the first [`MAX_COLOR_RANGE_SAMPLES`], in order.
    #[test]
    fn color_range_samples_are_capped_however_settings_load() {
        let big = color_range_settings(1000);
        let want = &color_range_samples(&big)[..MAX_COLOR_RANGE_SAMPLES];
        let v = big.to_json();
        let loaded = DevelopSettings::from_json(&v).unwrap();
        assert_eq!(color_range_samples(&loaded).len(), MAX_COLOR_RANGE_SAMPLES);
        assert_eq!(color_range_samples(&loaded), want);
        let from_str: DevelopSettings = serde_json::from_str(&v.to_string()).unwrap();
        assert_eq!(color_range_samples(&from_str), want);
        // `develop.merge`, `render --settings` and presets merge a partial
        let merged = DevelopSettings::default().merged(&json!({"masks": v["masks"]})).unwrap();
        assert_eq!(color_range_samples(&merged), want);
        let preset =
            Preset { id: "p".into(), name: "P".into(), group: "G".into(), settings: json!({"masks": v["masks"]}), favorite: false, builtin: false };
        assert_eq!(color_range_samples(&preset.apply(&DevelopSettings::default(), 1.0)), want);
        // a capped list round-trips, and a short one is unchanged
        assert_eq!(DevelopSettings::from_json(&loaded.to_json()).unwrap(), loaded);
        let few = color_range_settings(2);
        assert_eq!(DevelopSettings::from_json(&few.to_json()).unwrap(), few);
        // a list that isn't samples is still refused
        let mut bad = v.clone();
        bad["masks"][0]["components"][0]["shape"]["samples"] = json!([[0.1, 0.2]]);
        assert!(DevelopSettings::from_json(&bad).is_err());
    }

    #[test]
    fn merge_partial() {
        let s = DevelopSettings::default();
        let m = s.merged(&json!({"light": {"contrast": 20}, "effects": {"clarity": 10}})).unwrap();
        assert_eq!(m.light.contrast, 20.0);
        assert_eq!(m.effects.clarity, 10.0);
        assert_eq!(m.light.exposure, 0.0);
    }

    #[test]
    fn merged_settings_are_kept_in_range() {
        // issue storytold/lightcraft#1946: `develop.merge`, `render --settings` and presets stored whatever the JSON said
        let mut base = DevelopSettings::default();
        base.crop.geometry.rect = craft_geom::Rect::new(0.1, 0.2, 0.8, 0.9);
        let m = base
            .merged(&json!({"light": {"exposure": 1e308}, "vignette": {"amount": -1e308}, "geometry": {"scale": 0}, "optics": {"distortion": 1e308}}))
            .unwrap();
        assert_eq!((m.light.exposure, m.vignette.amount, m.geometry.scale, m.optics.distortion), (5.0, -100.0, 50.0, 100.0));
        // point curves stay in 0..1, as an imported XMP curve does
        let m = base.merged(&json!({"curve": {"master": [{"x": -3, "y": 0}, {"x": 1, "y": 1e308}]}})).unwrap();
        assert_eq!(m.curve.master, vec![craft_geom::Point::new(0.0, 0.0), craft_geom::Point::new(1.0, 1.0)]);
        // a crop with no area is left out, as XMP import leaves out one whose edges are inverted
        for rect in
            [json!({"x0": 0.9, "x1": 0.1, "y0": 0, "y1": 1}), json!({"x0": 1e308, "x1": -1e308, "y0": 0, "y1": 1}), json!({"x0": 0.5, "x1": 0.5})]
        {
            let m = base.merged(&json!({"crop": {"geometry": {"rect": rect}}})).unwrap();
            assert_eq!(m.crop.geometry.rect, base.crop.geometry.rect, "{rect}");
        }
        // unrotated, the crop stays inside the photo
        let m = base.merged(&json!({"crop": {"geometry": {"rect": {"x0": -1e308, "y0": 0.25, "x1": 1e308, "y1": 0.75}}}})).unwrap();
        assert_eq!(m.crop.geometry.rect, craft_geom::Rect::new(0.0, 0.25, 1.0, 0.75));
    }

    #[test]
    fn unedited_and_reset() {
        let mut s = DevelopSettings::default();
        s.wb.temp = 5000.0;
        assert!(s.is_unedited());
        s.light.exposure = 1.0;
        s.light.shadows = 30.0;
        assert!(!s.is_unedited());
        s.reset_section(Section::Light);
        assert!(s.is_unedited());
    }

    #[test]
    fn resetting_the_profile_section_restores_the_default_profile() {
        // the chosen creative profile is not a control, so resetting the section must restore it too
        let mut s = DevelopSettings::default();
        s.profile.id = "lc.muted.matte-soft".into();
        s.profile.amount = 40.0;
        s.reset_section(Section::Profile);
        assert_eq!(s.profile.id, "lc.color", "the creative profile is reset to the default one");
        assert_eq!(s.profile.amount, 100.0);
    }

    #[test]
    fn resetting_the_profile_section_clears_the_camera_local_tone() {
        // the camera's local tone mapping is not a control either: a Profile reset switches it off
        let mut s = DevelopSettings::default();
        s.profile.camera_local_tone = true;
        s.reset_section(Section::Profile);
        assert!(!s.profile.camera_local_tone);
    }

    #[test]
    fn section_toggles() {
        let mut s = DevelopSettings::default();
        assert!(s.section_enabled("effects"));
        s.set_section_enabled("effects", false);
        s.set_section_enabled("effects", false);
        assert!(!s.section_enabled("effects"));
        assert_eq!(s.disabled_sections.len(), 1);
        s.set_section_enabled("effects", true);
        assert!(s.section_enabled("effects"));
    }

    #[test]
    fn the_denoise_amount_is_a_slider_and_follows_the_detail_section() {
        let mut s = DevelopSettings::default();
        assert_eq!(s.denoise_amount(), 0.0);
        assert!(controls::set(&mut s, "enhance.denoise", 60.0));
        assert_eq!(controls::get(&s, "enhance.denoise"), Some(60.0));
        assert!((s.denoise_amount() - 0.6).abs() < 1e-6);
        // the slider's range is 0 to 100; values outside it are brought in
        controls::set(&mut s, "enhance.denoise", 500.0);
        assert_eq!(s.denoise_amount(), 1.0);
        controls::set(&mut s, "enhance.denoise", -5.0);
        assert_eq!(s.denoise_amount(), 0.0);
        // switching the Detail section off switches the denoise off with it; a settings file with a bad number does nothing
        controls::set(&mut s, "enhance.denoise", 80.0);
        s.set_section_enabled("detail", false);
        assert_eq!(s.denoise_amount(), 0.0);
        s.set_section_enabled("detail", true);
        s.enhance.denoise = f64::NAN;
        assert_eq!(s.denoise_amount(), 0.0);
        // it is part of Detail: a reset of the section clears it
        controls::set(&mut s, "enhance.denoise", 40.0);
        s.reset_section(Section::Detail);
        assert_eq!(s.enhance.denoise, 0.0);
        // The later independent switch preserves Amount when off, and can be on at zero.
        controls::set(&mut s, "enhance.denoise", 75.0);
        s.enhance.denoise_on = Some(false);
        assert_eq!(s.denoise_amount(), 0.0);
        assert_eq!(s.enhance.denoise, 75.0);
        s.enhance.denoise_on = Some(true);
        assert_eq!(s.denoise_amount(), 0.75);
        controls::set(&mut s, "enhance.denoise", 0.0);
        assert!(s.enhance.denoise_enabled());
        s.reset_section(Section::Detail);
        assert_eq!(s.enhance.denoise_on, None);
    }

    #[test]
    fn next_mask_id_after_the_highest_id_is_unused() {
        // ids come from files, pastes and agents: one at the limit must not overflow (a panic in
        // debug builds) or wrap to an id another mask has
        let mut s = DevelopSettings::default();
        assert_eq!(s.next_mask_id(), 1);
        s.masks = vec![Mask { id: u32::MAX, ..Default::default() }, Mask { id: 1, ..Default::default() }];
        let next = s.next_mask_id();
        assert!(!s.masks.iter().any(|m| m.id == next), "{next}");
        assert_eq!(next, 2);
    }
}
