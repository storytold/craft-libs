//! Interchange: camera-raw-settings (`crs:`) develop fields from XMP → our develop settings.
//!
//! Other raw developers store their edits as `crs:` properties in XMP sidecars, in DNG-embedded
//! XMP and in XMP preset files. This module maps the commonly used, documented-by-observation
//! fields to a *partial* [`DevelopSettings`](crate::DevelopSettings) JSON object
//! (only the fields present in the packet), which is then merged like a preset. Our pipeline is
//! not theirs, so the result is a best-effort approximation of the look, not a pixel match.
//! The full table is in `docs/xmp-interop.md`.
//!
//! Implemented from the public XMP specification (ISO 16684-1) and black-box observation of what
//! each field means; no third-party code or preset content was used.

use std::collections::BTreeMap;

use crate::{MIXER_BANDS, Preset};
use serde_json::{Map, Value, json};

/// Properties keyed `prefix:name` (as produced by [`craft_metadata::parse_xmp`]).
pub type Props = BTreeMap<String, Vec<String>>;

/// What the mapped settings will develop. It decides how a `crs:` white balance is read: a raw
/// with a Kelvin scale (colour matrices in the file, or measured for its camera) takes `Temperature`
/// as Kelvin; everything developed relative to its as-shot look (rendered files, and raws without
/// matrices anywhere) needs a shift from the other app's as-shot white instead, because on that scale 6500 K / 0 means *as shot* (issue storytold/lightcraft#510).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A preset: raw or rendered unknown. Absolute Kelvin first, else the relative scale.
    Any,
    /// A rendered photo (or a raw shown from its embedded preview): `Incremental*` fields on our
    /// relative scale.
    Rendered,
    /// A raw developed with an absolute white balance (DNG and other files with matrices, and raws
    /// whose camera has measured matrices): Kelvin as written.
    RawAbsolute,
    /// A raw developed relative to its as-shot look.
    RawRelative,
}

/// Band names as they appear in `crs:` field names, in our mixer order.
const CRS_BANDS: [&str; 8] = ["Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta"];

/// `crs:` fields that describe the packet rather than an adjustment.
const NON_ADJUSTMENT: &[&str] = &[
    "crs:Version",
    "crs:ProcessVersion",
    "crs:Name",
    "crs:ShortName",
    "crs:Group",
    "crs:UUID",
    "crs:PresetType",
    "crs:SupportsAmount",
    "crs:SupportsColor",
    "crs:SupportsMonochrome",
    "crs:SupportsHighDynamicRange",
    "crs:SupportsNormalDynamicRange",
    "crs:SupportsSceneReferred",
    "crs:SupportsOutputReferred",
    "crs:CameraModelRestriction",
    "crs:Copyright",
    "crs:ContactInfo",
    "crs:HasSettings",
    "crs:AlreadyApplied",
    "crs:RawFileName",
    "crs:HasCrop",
    // preset/library bookkeeping and tool options that don't change the rendering
    "crs:Cluster",
    "crs:SupportsAmount2",
    "crs:RequiresRGBTables",
    "crs:SortName",
    "crs:Description",
    "crs:CropConstrainToWarp",
    "crs:OverrideLookVignette",
    // the file's as-shot white, recorded next to the edit (ours comes from the file itself)
    "crs:AsShotTemperature",
    "crs:AsShotTint",
    // which Lightroom version may read the edit, and checksums of the profiles it names
    "crs:CompatibleVersion",
    "crs:CameraProfileDigest",
    "crs:LensProfileDigest",
    // the lens profile Lightroom picked: ours is the file's own, only `LensProfileEnable` carries over
    "crs:LensProfileName",
    "crs:LensProfileFilename",
    "crs:LensProfileIsEmbedded",
    "crs:LensProfileSetup",
    // a crop tool option (keep the crop inside the photo), not the crop itself
    "crs:CropConstrainToUnitSquare",
];

/// True if the packet carries any `crs:` adjustment (not just bookkeeping fields).
pub fn has_adjustments(props: &Props) -> bool {
    if props.get("crs:AlreadyApplied").and_then(|v| v.first()).is_some_and(|s| s.eq_ignore_ascii_case("true")) {
        // The pixels already contain these settings (e.g. an exported/rendered file).
        return false;
    }
    props.keys().any(|k| k.starts_with("crs:") && !NON_ADJUSTMENT.contains(&k.split('/').next().unwrap_or(k)))
}

thread_local! {
    /// Keys read by [`to_partial`] while [`to_partial_report`] runs.
    static READ: std::cell::RefCell<Option<std::collections::BTreeSet<String>>> = const { std::cell::RefCell::new(None) };
    /// Adjustments [`to_partial`] read but deliberately left out, for the report.
    static SKIPPED: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

fn note(k: &str) {
    READ.with(|r| {
        if let Some(s) = r.borrow_mut().as_mut() {
            s.insert(k.to_string());
        }
    });
}

fn skipped(what: &str) {
    SKIPPED.with(|r| {
        if let Some(s) = r.borrow_mut().as_mut() {
            s.push(what.to_string());
        }
    });
}

fn first<'a>(props: &'a Props, k: &str) -> Option<&'a str> {
    note(k);
    props.get(k).and_then(|v| v.first()).map(|s| s.trim()).filter(|s| !s.is_empty())
}

fn num(props: &Props, k: &str) -> Option<f64> {
    let s = first(props, k)?;
    let s = s.strip_prefix('+').unwrap_or(s);
    if let Some((n, d)) = s.split_once('/') {
        let (n, d) = (n.trim().parse::<f64>().ok()?, d.trim().parse::<f64>().ok()?);
        return (d != 0.0).then_some(n / d).filter(|v| v.is_finite());
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn boolean(props: &Props, k: &str) -> Option<bool> {
    first(props, k).map(|s| s.eq_ignore_ascii_case("true") || s == "1")
}

/// Insert `v` at the dotted `path` of a JSON object, creating intermediate objects.
fn put(out: &mut Value, path: &str, v: Value) {
    let mut cur = out;
    let mut parts = path.split('.').peekable();
    while let Some(p) = parts.next() {
        let Value::Object(m) = cur else { return };
        if parts.peek().is_none() {
            m.insert(p.to_string(), v);
            return;
        }
        cur = m.entry(p.to_string()).or_insert_with(|| Value::Object(Map::new()));
    }
}

/// Relative temperature (−100..100, used for rendered files) → Kelvin, matching the UI's scale
/// (a mired shift around 6500 K).
pub fn rel_to_kelvin(r: f64) -> f64 {
    1e6 / (1e6 / 6500.0 - r.clamp(-100.0, 100.0) * 0.8)
}

/// Parse a point curve stored as an `rdf:Seq` of `"x, y"` strings in 0..255.
fn curve(props: &Props, k: &str) -> Option<Value> {
    note(k);
    let items = props.get(k)?;
    let mut pts = Vec::new();
    for it in items {
        let (x, y) = it.split_once(',')?;
        // (`parse` reads "NaN" and "inf" too: a curve with such a point is not a curve)
        let (x, y) = (x.trim().parse::<f64>().ok().filter(|v| v.is_finite())?, y.trim().parse::<f64>().ok().filter(|v| v.is_finite())?);
        pts.push(json!({"x": (x / 255.0).clamp(0.0, 1.0), "y": (y / 255.0).clamp(0.0, 1.0)}));
    }
    if pts.len() < 2 {
        return None;
    }
    // A straight 0,0 → 255,255 line is the identity: store it as "no curve".
    let identity = pts.len() == 2 && items.iter().all(|it| it.split(',').map(|v| v.trim()).collect::<Vec<_>>().windows(2).all(|w| w[0] == w[1]));
    Some(if identity { json!([]) } else { Value::Array(pts) })
}

/// Map the `crs:` fields of an XMP packet to a partial develop-settings JSON object.
///
/// `target` decides how the white balance is read (see [`Target`]).
pub fn to_partial(props: &Props, target: Target) -> Value {
    let mut out = Value::Object(Map::new());
    let o = &mut out;
    let n = |o: &mut Value, crs: &str, path: &str| {
        if let Some(v) = num(props, &format!("crs:{crs}")) {
            put(o, path, json!(v));
        }
    };
    // ---- Light (process version 2012+ names)
    n(o, "Exposure2012", "light.exposure");
    n(o, "Contrast2012", "light.contrast");
    n(o, "Highlights2012", "light.highlights");
    n(o, "Shadows2012", "light.shadows");
    n(o, "Whites2012", "light.whites");
    n(o, "Blacks2012", "light.blacks");
    // Lightroom's Light panel "HDR" button
    if let Some(on) = boolean(props, "crs:HDREditMode") {
        put(o, "hdr.enabled", json!(on));
    }
    // older process versions (most `.lrtemplate` presets): approximate the 2012 sliders from the
    // earlier ones, relative to their defaults (contrast 25, blacks 5, brightness 50)
    let has = |o: &Value, path: &str| path.split('.').try_fold(o, |v, k| v.get(k)).is_some();
    // presets written by newer versions keep the old fields at their defaults: ignore them there
    let pv2012 = props.keys().any(|k| k.ends_with("2012"));
    let old = |o: &mut Value, crs: &str, path: &str, f: &dyn Fn(f64) -> f64| {
        let v = num(props, &format!("crs:{crs}"));
        if !pv2012
            && !has(o, path)
            && let Some(v) = v
        {
            put(o, path, json!(f(v).clamp(-100.0, 100.0)));
        }
    };
    old(o, "Exposure", "light.exposure", &|v| v);
    old(o, "Contrast", "light.contrast", &|v| v - 25.0);
    old(o, "HighlightRecovery", "light.highlights", &|v| -v);
    old(o, "FillLight", "light.shadows", &|v| v);
    old(o, "Shadows", "light.blacks", &|v| -(v - 5.0) * 4.0);
    old(o, "Brightness", "light.whites", &|v| (v - 50.0) * 0.6);
    // ---- Presence
    n(o, "Texture", "effects.texture");
    n(o, "Clarity2012", "effects.clarity");
    old(o, "Clarity", "effects.clarity", &|v| v);
    n(o, "Dehaze", "effects.dehaze");
    n(o, "Vibrance", "color.vibrance");
    n(o, "Saturation", "color.saturation");

    // ---- White balance
    let mode = first(props, "crs:WhiteBalance").map(|m| match m.to_ascii_lowercase().replace(' ', "").as_str() {
        "asshot" => "asShot",
        "auto" => "auto",
        "daylight" => "daylight",
        "cloudy" => "cloudy",
        "shade" => "shade",
        "tungsten" => "tungsten",
        "fluorescent" => "fluorescent",
        "flash" => "flash",
        _ => "custom",
    });
    let abs = num(props, "crs:Temperature").map(|t| (t, num(props, "crs:Tint")));
    let rel = num(props, "crs:IncrementalTemperature")
        .or(num(props, "crs:IncrementalTint").map(|_| 0.0))
        .map(|t| (rel_to_kelvin(t), num(props, "crs:IncrementalTint")));
    // A Kelvin value for something we develop relative to its as-shot look: the other app's
    // as-shot white (`AsShotTemperature` / `AsShotTint`, written next to the edit) is the
    // reference, so the edit is the same mired shift from ours (issue storytold/lightcraft#510).
    let shifted = || {
        let (t, tint) = abs?;
        let as_shot = num(props, "crs:AsShotTemperature").filter(|k| *k > 0.0)?;
        if t <= 0.0 {
            return None;
        }
        let r = (1e6 / as_shot - 1e6 / t) / 0.8;
        let tint = tint.map(|v| v - num(props, "crs:AsShotTint").unwrap_or(0.0));
        Some((rel_to_kelvin(r), tint))
    };
    let wb = match target {
        Target::RawAbsolute => abs,
        Target::Any => abs.or(rel),
        Target::Rendered => rel.or_else(shifted).or(abs),
        Target::RawRelative => rel.or_else(shifted),
    };
    match (mode, wb) {
        (Some(m), _) if m != "custom" => put(o, "wb.mode", json!(m)),
        (m, Some((t, tint))) => {
            put(o, "wb.mode", json!(m.unwrap_or("custom")));
            put(o, "wb.temp", json!(t));
            if let Some(tint) = tint {
                put(o, "wb.tint", json!(tint));
            }
            // Kelvin as the other app wrote it: say so, so the value stays Kelvin on a raw with that scale (a
            // preset's Kelvin is read as the relative scale on the files that have only that, as before)
            if wb == abs && matches!(target, Target::RawAbsolute | Target::Any) {
                put(o, "wb.scale", json!("kelvin"));
            }
        }
        // a custom Kelvin with nothing to shift it from: read on the relative scale it would be a
        // large colour shift (ΔE 22 against Lightroom's export on the photos of issue storytold/lightcraft#510), so
        // the photo stays As Shot (ΔE 12 there) and the report says so
        (_, None) if target == Target::RawRelative && abs.is_some() => {
            put(o, "wb.mode", json!("asShot"));
            skipped("Temperature, Tint (custom white balance without AsShotTemperature: kept As Shot)");
        }
        (Some(m), None) => put(o, "wb.mode", json!(m)),
        (None, None) => {}
    }

    // ---- Treatment + B&W mix
    if let Some(bw) = boolean(props, "crs:ConvertToGrayscale") {
        put(o, "treatment", json!(if bw { "bw" } else { "color" }));
    }
    for (band, crs) in MIXER_BANDS.iter().zip(CRS_BANDS) {
        n(o, &format!("HueAdjustment{crs}"), &format!("mixer.{band}.hue"));
        n(o, &format!("SaturationAdjustment{crs}"), &format!("mixer.{band}.sat"));
        n(o, &format!("LuminanceAdjustment{crs}"), &format!("mixer.{band}.lum"));
        n(o, &format!("GrayMixer{crs}"), &format!("bw_mix.{band}"));
    }

    // ---- Tone curve: parametric regions + point curves
    n(o, "ParametricShadows", "curve.shadows");
    n(o, "ParametricDarks", "curve.darks");
    n(o, "ParametricLights", "curve.lights");
    n(o, "ParametricHighlights", "curve.highlights");
    n(o, "ParametricShadowSplit", "curve.split_shadows");
    n(o, "ParametricMidtoneSplit", "curve.split_mid");
    n(o, "ParametricHighlightSplit", "curve.split_highlights");
    n(o, "CurveRefineSaturation", "curve.refine_saturation");
    if let Some(c) = curve(props, "crs:ToneCurve").filter(|c| c.as_array().is_some_and(|a| !a.is_empty())) {
        // the older single curve, used when no 2012 curve is present
        if !props.contains_key("crs:ToneCurvePV2012") {
            put(o, "curve.master", c);
        }
    }
    // Lightroom Classic reads the red / green / blue curves only when the packet holds the master
    // 2012 curve and all three channel curves (as its own packets always do): a packet missing any
    // of them imports without channel curves (master + red, master + red + green, or red / green /
    // blue without the master all leave the photo's channels unchanged there)
    if let Some(c) = curve(props, "crs:ToneCurvePV2012") {
        put(o, "curve.master", c);
        let channels = [("ToneCurvePV2012Red", "red"), ("ToneCurvePV2012Green", "green"), ("ToneCurvePV2012Blue", "blue")]
            .map(|(crs, ch)| (curve(props, &format!("crs:{crs}")), ch));
        if channels.iter().all(|(c, _)| c.is_some()) {
            for (c, ch) in channels {
                if let Some(c) = c {
                    put(o, &format!("curve.{ch}"), c);
                }
            }
        }
    }

    // ---- Color grading (split toning fields are shared with the older split-toning panel)
    n(o, "SplitToningShadowHue", "grading.shadows.hue");
    n(o, "SplitToningShadowSaturation", "grading.shadows.sat");
    n(o, "ColorGradeShadowLum", "grading.shadows.lum");
    n(o, "SplitToningHighlightHue", "grading.highlights.hue");
    n(o, "SplitToningHighlightSaturation", "grading.highlights.sat");
    n(o, "ColorGradeHighlightLum", "grading.highlights.lum");
    n(o, "ColorGradeMidtoneHue", "grading.midtones.hue");
    n(o, "ColorGradeMidtoneSat", "grading.midtones.sat");
    n(o, "ColorGradeMidtoneLum", "grading.midtones.lum");
    n(o, "ColorGradeGlobalHue", "grading.global.hue");
    n(o, "ColorGradeGlobalSat", "grading.global.sat");
    n(o, "ColorGradeGlobalLum", "grading.global.lum");
    n(o, "ColorGradeBlending", "grading.blending");
    n(o, "SplitToningBalance", "grading.balance");

    // ---- Detail
    n(o, "Sharpness", "detail.sharpen_amount");
    n(o, "SharpenRadius", "detail.sharpen_radius");
    n(o, "SharpenDetail", "detail.sharpen_detail");
    n(o, "SharpenEdgeMasking", "detail.sharpen_masking");
    n(o, "LuminanceSmoothing", "detail.nr_luminance");
    n(o, "LuminanceNoiseReductionDetail", "detail.nr_detail");
    n(o, "LuminanceNoiseReductionContrast", "detail.nr_contrast");
    n(o, "ColorNoiseReduction", "detail.nr_color");
    n(o, "ColorNoiseReductionDetail", "detail.nr_color_detail");
    n(o, "ColorNoiseReductionSmoothness", "detail.nr_color_smoothness");

    // ---- Effects: post-crop vignette + grain
    n(o, "PostCropVignetteAmount", "vignette.amount");
    n(o, "PostCropVignetteMidpoint", "vignette.midpoint");
    n(o, "PostCropVignetteRoundness", "vignette.roundness");
    n(o, "PostCropVignetteFeather", "vignette.feather");
    n(o, "PostCropVignetteHighlightContrast", "vignette.highlights");
    if let Some(st) = num(props, "crs:PostCropVignetteStyle") {
        let style = match st as i64 {
            2 => "colorPriority",
            3 => "paintOverlay",
            _ => "highlightPriority",
        };
        put(o, "vignette.style", json!(style));
    }
    n(o, "GrainAmount", "grain.amount");
    n(o, "GrainSize", "grain.size");
    n(o, "GrainFrequency", "grain.roughness");

    // ---- Optics (manual corrections; lens profiles are ours, only the switch carries over).
    // Our profile corrections are the file's own (DNG-embedded) ones, which Lightroom applies
    // whatever its "Enable Profile Corrections" box says [inferred]: so only an enabling switch
    // carries over, and `LensProfileEnable=0` (the default in most presets) leaves ours as it is.
    if boolean(props, "crs:LensProfileEnable") == Some(true) {
        put(o, "optics.lens_profile", json!(true));
    }
    if let Some(b) = boolean(props, "crs:AutoLateralCA") {
        put(o, "optics.remove_ca", json!(b));
    }
    n(o, "LensManualDistortionAmount", "optics.distortion");
    n(o, "VignetteAmount", "optics.vignetting");
    n(o, "VignetteMidpoint", "optics.vignetting_midpoint");
    n(o, "DefringePurpleAmount", "optics.defringe_purple_amount");
    n(o, "DefringePurpleHueLo", "optics.defringe_purple_hue_lo");
    n(o, "DefringePurpleHueHi", "optics.defringe_purple_hue_hi");
    n(o, "DefringeGreenAmount", "optics.defringe_green_amount");
    n(o, "DefringeGreenHueLo", "optics.defringe_green_hue_lo");
    n(o, "DefringeGreenHueHi", "optics.defringe_green_hue_hi");

    // ---- Calibration
    n(o, "ShadowTint", "calibration.shadows_tint");
    n(o, "RedHue", "calibration.red_hue");
    n(o, "RedSaturation", "calibration.red_sat");
    n(o, "GreenHue", "calibration.green_hue");
    n(o, "GreenSaturation", "calibration.green_sat");
    n(o, "BlueHue", "calibration.blue_hue");
    n(o, "BlueSaturation", "calibration.blue_sat");

    // ---- Geometry
    n(o, "PerspectiveVertical", "geometry.vertical");
    n(o, "PerspectiveHorizontal", "geometry.horizontal");
    n(o, "PerspectiveRotate", "geometry.rotate");
    n(o, "PerspectiveScale", "geometry.scale");
    n(o, "PerspectiveAspect", "geometry.aspect");
    n(o, "PerspectiveX", "geometry.offset_x");
    n(o, "PerspectiveY", "geometry.offset_y");
    if let Some(u) = num(props, "crs:PerspectiveUpright") {
        let mode = match u as i64 {
            1 => "auto",
            2 => "level",
            3 => "vertical",
            4 => "full",
            5 => "guided",
            _ => "off",
        };
        put(o, "geometry.upright", json!(mode));
    }

    // ---- Crop (normalized edges of the unrotated image + straighten angle)
    match boolean(props, "crs:HasCrop") {
        Some(true) => {
            let e = |k: &str, d: f64| num(props, &format!("crs:Crop{k}")).unwrap_or(d).clamp(0.0, 1.0);
            let (l, t, r, b) = (e("Left", 0.0), e("Top", 0.0), e("Right", 1.0), e("Bottom", 1.0));
            if r > l && b > t {
                put(
                    o,
                    "crop.geometry",
                    json!({"rect": {"x0": l, "y0": t, "x1": r, "y1": b}, "angle": num(props, "crs:CropAngle").unwrap_or(0.0).clamp(-45.0, 45.0)}),
                );
            }
        }
        Some(false) => put(o, "crop.geometry", json!({"rect": {"x0": 0.0, "y0": 0.0, "x1": 1.0, "y1": 1.0}, "angle": 0.0})),
        None => {}
    }
    out
}

/// [`to_partial`], plus the adjustments in the packet it could not carry over (field names
/// without the `crs:` prefix, grouped: `CameraProfile`, `Look`, `MaskGroupBasedCorrections`…).
///
/// With `values` (the packet's structured properties), local corrections become masks too
/// ([`crate::crs_masks`]; radial masks fitted to `aspect` = width / height); components that
/// can't be carried over are reported as `Mask: <kind>`.
pub fn to_partial_report(props: &Props, values: Option<&crate::crs_masks::Values>, target: Target, aspect: f64) -> (Value, Vec<String>) {
    READ.with(|r| *r.borrow_mut() = Some(Default::default()));
    SKIPPED.with(|r| *r.borrow_mut() = Some(Vec::new()));
    let mut out = to_partial(props, target);
    let mut read = READ.with(|r| r.borrow_mut().take()).unwrap_or_default();
    let mut mask_skips = SKIPPED.with(|r| r.borrow_mut().take()).unwrap_or_default();
    if let Some(values) = values {
        let (masks, skipped) = crate::crs_masks::masks(values, aspect);
        if !masks.is_empty() {
            put(&mut out, "masks", Value::Array(masks));
        }
        if values.keys().any(|k| crate::crs_masks::CONTAINERS.contains(&k.as_str())) {
            read.extend(crate::crs_masks::CONTAINERS.iter().map(|c| c.to_string()));
            mask_skips.extend(skipped.into_iter().map(|k| format!("Mask: {k}")));
        }
        let (spots, skipped) = crate::crs_masks::spots(values, aspect);
        if !spots.is_empty() {
            put(&mut out, "spots", Value::Array(spots));
        }
        if values.keys().any(|k| crate::crs_masks::SPOT_CONTAINERS.contains(&k.as_str())) {
            read.extend(crate::crs_masks::SPOT_CONTAINERS.iter().map(|c| c.to_string()));
            mask_skips.extend(skipped.into_iter().map(|k| format!("Spot: {k}")));
        }
    }
    // fields that only switch a panel on/off or name things: not adjustments by themselves; and
    // Point Color slots that are all empty (-1)
    let value_is = |k: &str, f: &dyn Fn(&[String]) -> bool| props.get(&format!("crs:{k}")).is_some_and(|v| f(v));
    let empty_points = |v: &[String]| v.iter().all(|s| s.split(',').all(|n| n.trim().parse::<f64>().is_ok_and(|x| x == -1.0) || n.trim().is_empty()));
    let quiet = |k: &str| {
        k.starts_with("Enable")
            || k.starts_with("ToneCurveName")
            || k == "AutoTone"
            || k == "AutoGrayscaleMix"
            || (k == "PointColors" && value_is(k, &empty_points))
            // the lens profile's strength, written at its default (100 %) next to every profile
            || (matches!(k, "LensProfileDistortionScale" | "LensProfileVignettingScale") && value_is(k, &|v| v.iter().all(|s| s.trim() == "100")))
    };
    let mut unmapped: Vec<String> = props
        .keys()
        .filter(|k| k.starts_with("crs:"))
        .map(|k| k.split('/').next().unwrap_or(k))
        .filter(|k| !NON_ADJUSTMENT.contains(k) && !read.contains(*k))
        .map(|k| k.trim_start_matches("crs:").to_string())
        .filter(|k| !quiet(k))
        .collect();
    unmapped.dedup();
    unmapped.extend(mask_skips);
    (out, unmapped)
}

/// The selected camera profile name from an XMP packet. The name is metadata only: it does not
/// identify or carry the profile's rendering data.
pub fn camera_profile_name(props: &Props) -> Option<&str> {
    first(props, "crs:CameraProfile")
}

/// Whether an XMP packet contains a saved Lightroom Upright matrix. Lightroom's matrix uses a
/// coordinate convention that LightCraft does not currently import.
pub fn has_saved_upright_transform(props: &Props) -> bool {
    props.keys().any(|key| {
        key.strip_prefix("crs:").is_some_and(|name| {
            name.starts_with("UprightTransform_") && props.get(key).is_some_and(|values| values.iter().any(|v| !v.trim().is_empty()))
        })
    })
}

/// Read an XMP preset (`crs:` fields + `crs:Name` / `crs:Group`) into one of our presets.
/// Returns `None` when the packet has no adjustments we understand.
pub fn preset_from_xmp(xmp: &str, fallback_name: &str) -> Option<Preset> {
    let d = craft_metadata::parse_xmp(xmp).ok()?;
    let props = &d.properties;
    let settings = to_partial(props, Target::Any);
    if settings.as_object().is_none_or(Map::is_empty) {
        return None;
    }
    let name = first(props, "crs:Name").unwrap_or(fallback_name).to_string();
    let group = first(props, "crs:Group").unwrap_or("Imported Presets").to_string();
    let uuid = first(props, "crs:UUID").map(|u| u.to_ascii_lowercase());
    let id = format!("user.xmp.{}", uuid.unwrap_or_else(|| crate::presets::slug(&format!("{group}-{name}"))));
    Some(Preset { id, name, group, settings, favorite: false, builtin: false })
}

#[cfg(test)]
mod tests;
