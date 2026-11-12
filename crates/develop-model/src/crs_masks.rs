//! Interchange: local corrections (masks) stored as `crs:` structures in XMP sidecars, XMP
//! presets and `.lrtemplate` files → our [`Mask`](crate::Mask)s.
//!
//! Four containers hold them: `MaskGroupBasedCorrections` (current: each correction has a list
//! of mask components) and the older `GradientBasedCorrections`, `CircularGradientBasedCorrections`
//! and `PaintBasedCorrections`. Every correction carries its local sliders (`Local*`, stored as
//! −1..1 fractions; exposure ×4 EV) and its components (`crs:What` = `Mask/Gradient`,
//! `Mask/CircularGradient`, `Mask/Paint`, `Mask/Image`, `Mask/RangeMask`).
//!
//! Implemented from the public XMP data model and black-box observation; best effort — the
//! pipelines differ, and the component combine modes and AI mask kinds are mapped from what the
//! fields evidently mean. Unsupported components are reported, never guessed.

use std::collections::BTreeMap;

use craft_metadata::XmpValue;
use serde_json::{Value, json};

/// Top-level properties keyed `prefix:name`, with structure.
pub type Values = BTreeMap<String, XmpValue>;

/// The containers this module reads.
pub const CONTAINERS: [&str; 4] =
    ["crs:MaskGroupBasedCorrections", "crs:GradientBasedCorrections", "crs:CircularGradientBasedCorrections", "crs:PaintBasedCorrections"];

fn text<'a>(v: &'a XmpValue, k: &str) -> Option<&'a str> {
    v.field(k).and_then(XmpValue::text).filter(|s| !s.is_empty())
}

fn num(v: &XmpValue, k: &str) -> Option<f64> {
    let s = text(v, k)?;
    let s = s.strip_prefix('+').unwrap_or(s);
    s.parse::<f64>().ok().filter(|x| x.is_finite())
}

fn flag(v: &XmpValue, k: &str) -> Option<bool> {
    text(v, k).map(|s| s.eq_ignore_ascii_case("true") || s == "1")
}

/// Local slider fields → (our `adjust` field, scale from the stored fraction).
const LOCAL: &[(&str, &str, f64)] = &[
    ("LocalExposure2012", "exposure", 4.0),
    ("LocalContrast2012", "contrast", 100.0),
    ("LocalHighlights2012", "highlights", 100.0),
    ("LocalShadows2012", "shadows", 100.0),
    ("LocalWhites2012", "whites", 100.0),
    ("LocalBlacks2012", "blacks", 100.0),
    ("LocalClarity2012", "clarity", 100.0),
    ("LocalTexture", "texture", 100.0),
    ("LocalDehaze", "dehaze", 100.0),
    ("LocalTemperature", "temp", 100.0),
    ("LocalTint", "tint", 100.0),
    ("LocalSaturation", "saturation", 100.0),
    ("LocalHue", "hue", 100.0),
    ("LocalSharpness", "sharpness", 100.0),
    ("LocalLuminanceNoise", "noise", 100.0),
    ("LocalMoire", "moire", 100.0),
    ("LocalDefringe", "defringe", 100.0),
    ("LocalToningSaturation", "color_sat", 100.0),
    ("LocalToningHue", "color_hue", 1.0),
    // older process versions
    ("LocalExposure", "exposure", 4.0),
    ("LocalContrast", "contrast", 100.0),
    ("LocalClarity", "clarity", 100.0),
];

/// Is `crs` (without the `crs:` prefix) a local slider field we map?
pub fn is_local_field(crs: &str) -> bool {
    LOCAL.iter().any(|(k, _, _)| *k == crs)
}

/// Our local adjustment values from the `Local*` slider fields that `get` finds (the name without
/// the `crs:` prefix → the stored fraction); process-version 2012 fields win over the older ones.
pub fn local_values(get: impl Fn(&str) -> Option<f64>) -> serde_json::Map<String, Value> {
    let mut a = serde_json::Map::new();
    for (crs, ours, k) in LOCAL {
        if a.contains_key(*ours) {
            continue;
        }
        if let Some(v) = get(crs).filter(|v| v.is_finite()) {
            let lim = if *ours == "exposure" {
                4.0
            } else if *ours == "color_hue" {
                360.0
            } else {
                100.0
            };
            let v = (v * k).clamp(if *ours == "color_hue" { 0.0 } else { -lim }, lim);
            a.insert((*ours).into(), json!(v));
        }
    }
    a
}

fn adjust(c: &XmpValue) -> Value {
    let mut a = local_values(|crs| num(c, &format!("crs:{crs}")));
    let amount = num(c, "crs:CorrectionAmount").unwrap_or(1.0);
    a.insert("amount".into(), json!((amount * 100.0).clamp(0.0, 200.0)));
    Value::Object(a)
}

/// The correction's local sliders we have no counterpart for (e.g. `LocalGrain`), when set:
/// reported, never dropped silently.
fn unmapped_local(c: &XmpValue) -> Vec<String> {
    let XmpValue::Struct(m) = c else { return Vec::new() };
    m.keys()
        .filter_map(|k| {
            let field = k.strip_prefix("crs:").filter(|f| f.starts_with("Local"))?;
            // 100 = "curve saturation unchanged", written with every correction
            let neutral = if field == "LocalCurveRefineSaturation" { 100.0 } else { 0.0 };
            let set = num(c, k).is_some_and(|v| v != neutral);
            (set && !LOCAL.iter().any(|(crs, ..)| *crs == field)).then(|| field.to_string())
        })
        .collect()
}

/// Perceptual lightness 0..1 (how range masks store luminance) → our range scale: log
/// luminance on −8..+4 EV mapped to 0..1.
fn lightness_to_range(l: f64) -> f64 {
    let y = l.clamp(1e-4, 1.0).powf(2.2);
    ((y.log2() + 8.0) / 12.0).clamp(0.0, 1.0)
}

fn pt(x: f64, y: f64) -> Value {
    json!({"x": x, "y": y})
}

/// Whether a `Mask/CircularGradient` component puts its effect inside the ellipse, given the
/// container it came from (`crs:Flipped` means different things in different formats).
///
/// What Lightroom Classic does, from its exports of synthetic DNGs whose masks we wrote (issue storytold/lightcraft#1815):
/// - **`MaskGroupBasedCorrections`, `crs:Version` ≥ 2** (what Lightroom writes today): an ordinary
///   radial — effect inside — is written `Flipped="true"`, so `Flipped` is the "inside" flag.
///   Inverting the mask is a separate field, `crs:MaskInverted` (see [`component`]). `Flipped="false"`
///   is read as outside, the same sense as the legacy filter's; Lightroom never wrote one for us.
/// - **`MaskGroupBasedCorrections` without `crs:Version`**: Lightroom renders the effect inside
///   whatever `Flipped` says, and rewrites both `Flipped="false"` and `"true"` as `Flipped="true"`
///   with `Version="2"`. So `Flipped` is ignored here.
/// - **`CircularGradientBasedCorrections`** (the process-2012 Radial Filter): its default effect was
///   outside, and "Invert Mask" — effect inside — was recorded as `Flipped="true"`. The same "inside"
///   flag as version 2, with outside as the default when it is missing.
fn radial_inside(m: &XmpValue, container: &str) -> bool {
    let flipped = flag(m, "crs:Flipped");
    if container == "crs:MaskGroupBasedCorrections" {
        if num(m, "crs:Version").is_some_and(|v| v >= 2.0) { flipped.unwrap_or(true) } else { true }
    } else {
        flipped.unwrap_or(false)
    }
}

/// A `Mask/CircularGradient` component's feather on our 0..100 scale.
///
/// Lightroom halves `crs:Feather` when it upgrades a `MaskGroupBasedCorrections` radial without
/// `crs:Version` to version 2 (50 → 25, 100 → 50) and renders both the same, so the version-less
/// value is on twice the scale. Version 2's value is used as it is: its 25 and 50 put the middle of
/// Lightroom's fall-off at about 0.87 and 0.76 of the radius, ours at about 0.86 and 0.73. (The
/// shape differs: Lightroom's fall-off straddles the ellipse edge, ours ends at it.) The legacy
/// Radial Filter's value is used as it is: no observation says otherwise.
fn radial_feather(m: &XmpValue, container: &str) -> f64 {
    let f = num(m, "crs:Feather").unwrap_or(50.0);
    let versionless_group = container == "crs:MaskGroupBasedCorrections" && !num(m, "crs:Version").is_some_and(|v| v >= 2.0);
    (if versionless_group { f / 2.0 } else { f }).clamp(0.0, 100.0)
}

/// One mask component of a correction in `container` → our component JSON; `Err(kind)` names an
/// unsupported one.
fn component(m: &XmpValue, container: &str, aspect: f64) -> Result<Value, String> {
    let what = text(m, "crs:What").unwrap_or("");
    let op = match num(m, "crs:MaskBlendMode").unwrap_or(0.0) as i64 {
        1 => "subtract",
        2 => "intersect",
        _ => "add",
    };
    let invert = flag(m, "crs:MaskInverted").unwrap_or(false);
    // positions are fractions of the image's width / height; our radii are in long-edge units
    let (sx, sy) = if aspect >= 1.0 { (1.0, 1.0 / aspect) } else { (aspect, 1.0) };
    let shape = match what {
        "Mask/Gradient" => {
            let g = |k: &str, d: f64| num(m, &format!("crs:{k}")).unwrap_or(d);
            json!({"kind": "linear", "start": pt(g("FullX", 0.5), g("FullY", 0.0)), "end": pt(g("ZeroX", 0.5), g("ZeroY", 1.0))})
        }
        "Mask/CircularGradient" => {
            let g = |k: &str, d: f64| num(m, &format!("crs:{k}")).unwrap_or(d);
            let (l, r, t, b) = (g("Left", 0.25), g("Right", 0.75), g("Top", 0.25), g("Bottom", 0.75));
            json!({
                "kind": "radial",
                "center": pt((l + r) / 2.0, (t + b) / 2.0),
                "rx": ((r - l).abs() / 2.0 * sx).max(1e-3),
                "ry": ((b - t).abs() / 2.0 * sy).max(1e-3),
                "angle": g("Angle", 0.0),
                "feather": radial_feather(m, container),
                // our radial's `invert` = effect outside; `crs:MaskInverted` stays the component's
                "invert": !radial_inside(m, container),
            })
        }
        "Mask/Paint" => {
            let pts: Vec<Value> = m
                .field("crs:Dabs")
                .map(XmpValue::items)
                .unwrap_or_default()
                .iter()
                .filter_map(XmpValue::text)
                .filter_map(|d| {
                    let mut it = d.split_whitespace().skip(1).filter_map(|v| v.parse::<f64>().ok());
                    Some(pt(it.next()?, it.next()?))
                })
                .collect();
            if pts.is_empty() {
                return Err("empty brush".into());
            }
            let erase = num(m, "crs:MaskValue").is_some_and(|v| v <= 0.0);
            json!({"kind": "brush", "strokes": [{
                "points": pts,
                "size": num(m, "crs:Radius").unwrap_or(0.03).clamp(0.001, 1.0),
                "feather": (num(m, "crs:CenterWeight").unwrap_or(0.5) * 100.0).clamp(0.0, 100.0),
                "flow": (num(m, "crs:Flow").unwrap_or(1.0) * 100.0).clamp(0.0, 100.0),
                "density": (num(m, "crs:MaskValue").unwrap_or(1.0).abs() * 100.0).clamp(0.0, 100.0),
                "erase": erase,
            }]})
        }
        "Mask/Image" => match text(m, "crs:MaskSubType") {
            Some("1") => json!({"kind": "subject"}),
            Some("2") => json!({"kind": "sky"}),
            other => return Err(format!("AI mask ({})", other.unwrap_or("?"))),
        },
        "Mask/RangeMask" => {
            let r = m.field("crs:CorrectionRangeMask").unwrap_or(m);
            match text(r, "crs:Type") {
                Some("2") => {
                    let q: Vec<f64> = text(r, "crs:LumRange").unwrap_or("0 0 1 1").split_whitespace().filter_map(|v| v.parse().ok()).collect();
                    let [a, b, c, d] = q[..] else { return Err("luminance range".into()) };
                    // full strength at white (1, the top of their scale) takes in everything brighter: ours
                    // goes on above scene white, so that end stays open instead of cutting off at 0 EV
                    let open_top = c >= 1.0;
                    let (a, b, c, d) = (lightness_to_range(a), lightness_to_range(b), lightness_to_range(c), lightness_to_range(d));
                    let (hi, hi_feather) = if open_top { (1.0, 0.0) } else { (c, (d - c).max(0.0)) };
                    json!({"kind": "luminanceRange", "lo": b, "hi": hi, "lo_feather": (b - a).max(0.0), "hi_feather": hi_feather})
                }
                Some("3") => {
                    let q: Vec<f64> = text(r, "crs:DepthRange").unwrap_or("0 0 1 1").split_whitespace().filter_map(|v| v.parse().ok()).collect();
                    let [a, b, c, _] = q[..] else { return Err("depth range".into()) };
                    json!({"kind": "depthRange", "lo": b, "hi": c, "feather": (b - a).max(0.0)})
                }
                _ => return Err("colour range".into()),
            }
        }
        other => return Err(if other.is_empty() { "unknown mask".into() } else { other.trim_start_matches("Mask/").to_string() }),
    };
    Ok(json!({"op": op, "invert": invert, "shape": shape}))
}

/// Masks in `values` (JSON for `DevelopSettings::masks`), plus the kinds of component that
/// couldn't be carried over. `aspect` = width / height of the target (presets: assume 3:2).
pub fn masks(values: &Values, aspect: f64) -> (Vec<Value>, Vec<String>) {
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for container in CONTAINERS {
        let Some(list) = values.get(container) else { continue };
        for c in list.items() {
            if flag(c, "crs:CorrectionActive") == Some(false) {
                continue;
            }
            let comps = c.field("crs:CorrectionMasks").map(XmpValue::items).unwrap_or_default();
            let mut parts = Vec::new();
            for m in comps {
                match component(m, container, aspect) {
                    Ok(v) => parts.push(v),
                    Err(kind) => skipped.push(kind),
                }
            }
            if parts.is_empty() {
                continue;
            }
            skipped.extend(unmapped_local(c));
            // brush dabs of one correction are one brush component
            let mut merged: Vec<Value> = Vec::new();
            // where the brush of each op sits in `merged` (no scan of it per dab)
            let mut brush_at: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            for p in parts {
                let is_brush = p["shape"]["kind"] == "brush";
                if is_brush
                    && let Some(prev) = brush_at.get(&p["op"].to_string()).and_then(|&i| merged.get_mut(i))
                    && let (Some(a), Some(b)) = (prev["shape"]["strokes"].as_array_mut(), p["shape"]["strokes"].as_array())
                {
                    a.extend(b.iter().cloned());
                    continue;
                }
                if is_brush {
                    brush_at.insert(p["op"].to_string(), merged.len());
                }
                merged.push(p);
            }
            let n = out.len() + 1;
            let name = text(c, "crs:CorrectionName").map(str::to_string).unwrap_or_else(|| format!("Mask {n}"));
            out.push(json!({"id": n, "name": name, "visible": true, "invert": false, "components": merged, "adjust": adjust(c)}));
        }
    }
    skipped.sort();
    skipped.dedup();
    (out, skipped)
}

/// The containers spot removal is stored in: `RetouchAreas` (current, one struct per spot with a
/// mask component) and `RetouchInfo` (older, one `key = value, …` line per spot; still written
/// beside `RetouchAreas` for older readers and read on its own when that is all there is).
pub const SPOT_CONTAINERS: [&str; 2] = ["crs:RetouchAreas", "crs:RetouchInfo"];

/// One `RetouchAreas` item → a [`Spot`](crate::Spot) as JSON. `aspect` = width /
/// height of the target: horizontal sizes are stored as fractions of the width, ours of the long
/// edge. Returns the component kind it can't carry over.
fn spot(item: &XmpValue, aspect: f64) -> Result<Value, String> {
    let mode = match text(item, "crs:SpotType") {
        Some("clone") => "clone",
        _ => "heal",
    };
    let comps = item.field("crs:Masks").map(XmpValue::items).unwrap_or_default();
    let Some(m) = comps.first() else { return Err("spot without a mask".into()) };
    let width_to_long = aspect.min(1.0);
    let (points, size) = match text(m, "crs:What").unwrap_or("") {
        // a circle: centre + half-sizes in image fractions
        "Mask/Ellipse" => {
            let (Some(x), Some(y), Some(sx)) = (num(m, "crs:X"), num(m, "crs:Y"), num(m, "crs:SizeX")) else {
                return Err("ellipse spot".into());
            };
            (vec![pt(x, y)], sx * width_to_long)
        }
        // the shape exiftool's schema also allows for a spot
        "Mask/CircularGradient" => {
            let (Some(t), Some(l), Some(b), Some(r)) = (num(m, "crs:Top"), num(m, "crs:Left"), num(m, "crs:Bottom"), num(m, "crs:Right")) else {
                return Err("circular spot".into());
            };
            (vec![pt((l + r) / 2.0, (t + b) / 2.0)], (r - l).abs() / 2.0 * width_to_long)
        }
        // a brushed removal: the dabs are the path
        "Mask/Paint" => {
            let pts: Vec<Value> = m
                .field("crs:Dabs")
                .map(XmpValue::items)
                .unwrap_or_default()
                .iter()
                .filter_map(XmpValue::text)
                .filter_map(|d| {
                    let mut it = d.split_whitespace().skip(1).filter_map(|v| v.parse::<f64>().ok());
                    Some(pt(it.next()?, it.next()?))
                })
                .collect();
            if pts.is_empty() {
                return Err("empty brushed spot".into());
            }
            (pts, num(m, "crs:Radius").unwrap_or(0.02))
        }
        other => return Err(if other.is_empty() { "unknown spot".into() } else { other.trim_start_matches("Mask/").to_string() }),
    };
    let size = size.clamp(0.001, 0.25);
    // the source is stored as its absolute position (`SourceX`, and `OffsetY` despite the name);
    // ours is the offset from the target; anything but an explicit source leaves it automatic
    let explicit = text(item, "crs:SourceState").is_none_or(|s| s == "sourceSetExplicitly");
    let source_offset = match (explicit, num(item, "crs:SourceX"), num(item, "crs:OffsetY"), points.first()) {
        (true, Some(sx), Some(sy), Some(p)) => {
            let (Some(x), Some(y)) = (p["x"].as_f64(), p["y"].as_f64()) else { return Err("spot centre".into()) };
            let (dx, dy) = (sx - x, sy - y);
            (dx.is_finite() && dy.is_finite() && (dx != 0.0 || dy != 0.0)).then(|| pt(dx, dy))
        }
        _ => None,
    };
    let feather = (num(item, "crs:Feather").unwrap_or(0.0) * 100.0).clamp(0.0, 100.0);
    let opacity = (num(item, "crs:Opacity").unwrap_or(1.0) * 100.0).clamp(0.0, 100.0);
    Ok(json!({"mode": mode, "points": points, "size": size, "feather": feather, "opacity": opacity, "source_offset": source_offset}))
}

/// One `RetouchInfo` line (`centerX = 0.5, centerY = 0.5, radius = 0.01, sourceState = …,
/// sourceX = …, sourceY = …, spotType = heal`) → a spot as JSON.
fn legacy_spot(line: &str, aspect: f64) -> Option<Value> {
    let mut kv = BTreeMap::new();
    for part in line.split(',') {
        if let Some((k, v)) = part.split_once('=') {
            kv.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    let f = |k: &str| kv.get(k).and_then(|s| s.parse::<f64>().ok()).filter(|x| x.is_finite());
    let (x, y, r) = (f("centerX")?, f("centerY")?, f("radius")?);
    let mode = if kv.get("spotType").is_some_and(|s| s == "clone") { "clone" } else { "heal" };
    let explicit = kv.get("sourceState").is_none_or(|s| s == "sourceSetExplicitly");
    let source_offset = match (explicit, f("sourceX"), f("sourceY")) {
        (true, Some(sx), Some(sy)) if sx != x || sy != y => Some(pt(sx - x, sy - y)),
        _ => None,
    };
    let opacity = (f("opacity").unwrap_or(1.0) * 100.0).clamp(0.0, 100.0);
    Some(
        json!({"mode": mode, "points": [pt(x, y)], "size": (r * aspect.min(1.0)).clamp(0.001, 0.25), "feather": 0.0, "opacity": opacity, "source_offset": source_offset}),
    )
}

/// Spot removal in `values` (JSON for `DevelopSettings::spots`), plus the kinds of spot that
/// couldn't be carried over. `aspect` = width / height of the target (presets: assume 3:2).
pub fn spots(values: &Values, aspect: f64) -> (Vec<Value>, Vec<String>) {
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    if let Some(list) = values.get("crs:RetouchAreas") {
        for item in list.items() {
            match spot(item, aspect) {
                Ok(v) => out.push(v),
                Err(kind) => skipped.push(kind),
            }
        }
    }
    if out.is_empty()
        && let Some(list) = values.get("crs:RetouchInfo")
    {
        out.extend(list.items().iter().filter_map(XmpValue::text).filter_map(|l| legacy_spot(l, aspect)));
    }
    skipped.sort();
    skipped.dedup();
    (out, skipped)
}

/// The aspect radial masks are computed for when the target's shape isn't known yet.
pub const DEFAULT_ASPECT: f64 = 1.5;

/// Re-fit radial masks in `partial` (computed for aspect `from`) to a target of aspect `to`.
pub fn refit_radials(partial: &mut Value, from: f64, to: f64) {
    let scale = |a: f64| if a >= 1.0 { (1.0, 1.0 / a) } else { (a, 1.0) };
    let ((fx, fy), (tx, ty)) = (scale(from), scale(to));
    let Some(masks) = partial.get_mut("masks").and_then(Value::as_array_mut) else { return };
    for m in masks {
        for c in m["components"].as_array_mut().into_iter().flatten() {
            let s = &mut c["shape"];
            if s["kind"] == "radial" {
                if let Some(rx) = s["rx"].as_f64() {
                    s["rx"] = json!(rx / fx * tx);
                }
                if let Some(ry) = s["ry"].as_f64() {
                    s["ry"] = json!(ry / fy * ty);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
