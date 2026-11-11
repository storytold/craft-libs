//! Writing a theme file: the parsed JSON updated with the theme's values, in a stable order.

use std::collections::BTreeMap;

use serde_json::{Map, Number, Value};

use super::{FormatErrors, SCHEMA, SCHEMA_VERSION, ThemeFile};
use crate::components::ComponentField;
use crate::scale::Radius;
use crate::theme::{AppOverrides, ContrastIntent, EdgeSeed, ModePins, ModeSeeds, Pins, ShapeSeed, ThemeDef, Variants};
use crate::tokens::RowStyle;

/// Top-level keys in the order a file lists them; other keys follow, sorted.
const TOP_ORDER: [&str; 14] = [
    "schema",
    "schema_version",
    "id",
    "name",
    "version",
    "variants",
    "contrast_intent",
    "seeds",
    "shape",
    "light",
    "dark",
    "high_contrast",
    "components",
    "apps",
];

/// The file as pretty-printed JSON ending in a newline. Known values come from `file.theme`;
/// unknown keys come from the file that was parsed. Keys are sorted below the top level. Fails
/// with [`ThemeDef::validate`]'s errors, so the text always parses.
pub fn write(file: &ThemeFile) -> Result<String, FormatErrors> {
    file.theme.validate()?;
    let mut raw = file.raw.clone();
    update(&mut raw, &file.theme);
    let mut out = String::new();
    emit(&Value::Object(raw), 0, Some(&TOP_ORDER), &mut out);
    out.push('\n');
    Ok(out)
}

fn update(raw: &mut Map<String, Value>, t: &ThemeDef) {
    raw.insert("schema".to_owned(), SCHEMA.into());
    raw.insert("schema_version".to_owned(), SCHEMA_VERSION.into());
    raw.insert("id".to_owned(), t.id.as_str().into());
    raw.insert("name".to_owned(), t.name.as_str().into());
    raw.insert("version".to_owned(), t.version.into());
    let variants = match t.variants {
        Variants::Both => "both",
        Variants::LightOnly => "light",
        Variants::DarkOnly => "dark",
    };
    raw.insert("variants".to_owned(), variants.into());
    let intent = match t.contrast_intent {
        ContrastIntent::Standard => "standard",
        ContrastIntent::Low => "low",
    };
    raw.insert("contrast_intent".to_owned(), intent.into());

    with_object(raw, "seeds", |seeds| {
        seeds.insert("neutral".to_owned(), t.seeds.neutral.to_hex().into());
        seeds.insert("accent".to_owned(), t.seeds.accent.to_hex().into());
        with_object(seeds, "status", |status| {
            let st = t.seeds.status;
            for (key, c) in [("good", st.good), ("warn", st.warn), ("bad", st.bad), ("info", st.info)] {
                status.insert(key.to_owned(), c.to_hex().into());
            }
        });
    });

    match &t.shape {
        None => {
            raw.insert("shape".to_owned(), Value::Null);
        }
        Some(shape) => with_object(raw, "shape", |o| update_shape(o, shape)),
    }

    mode_maps(raw, &t.pins);
    mode_seed_maps(raw, &t.mode_seeds);
    section(raw, "high_contrast", |hc| mode_maps(hc, &t.high_contrast));
    set_map(raw, "components", component_map(&t.components));

    if let Some(Value::Object(apps)) = raw.get_mut("apps") {
        apps.retain(|k, _| t.apps.keys().any(|id| id.as_str() == k));
    }
    if !t.apps.is_empty() || raw.contains_key("apps") {
        with_object(raw, "apps", |apps| {
            for (id, app) in &t.apps {
                with_object(apps, id.as_str(), |o| update_app(o, app));
            }
        });
    }
}

fn update_shape(shape: &mut Map<String, Value>, s: &ShapeSeed) {
    shape.remove("edge");
    if s.radius.is_none() {
        shape.remove("radius");
    }
    if let Some(r) = s.radius {
        with_object(shape, "radius", |radius| {
            for (key, v) in [("control", r.control), ("surface", r.surface), ("window", r.window), ("panel", r.panel), ("button", r.button)] {
                radius.insert(key.to_owned(), length(v));
            }
            // `pill` defaults to full, so only a set value is written.
            if r.pill == crate::scale::Radius::Full.px() {
                radius.remove("pill");
            } else {
                radius.insert("pill".to_owned(), length(r.pill));
            }
        });
    }
    match s.radius_offset {
        None => {
            shape.remove("radius_offset");
        }
        Some(o) => with_object(shape, "radius_offset", |m| {
            for (key, v) in [("control", o.control), ("surface", o.surface), ("window", o.window)] {
                m.insert(key.to_owned(), length(v));
            }
        }),
    }
    match s.button_radius {
        None => {
            shape.remove("button_radius");
        }
        Some(r) => {
            shape.insert("button_radius".to_owned(), length(r));
        }
    }
    if let Some(e) = s.edge {
        shape.insert("edge".to_owned(), edge(e));
    }
    match s.rows {
        None => {
            shape.remove("rows");
        }
        Some(r) => {
            shape.insert("rows".to_owned(), Value::String(if r == RowStyle::Divided { "divided" } else { "rounded" }.to_owned()));
        }
    }
}

fn update_app(app: &mut Map<String, Value>, o: &AppOverrides) {
    mode_maps(app, &o.pins);
    section(app, "high_contrast", |hc| mode_maps(hc, &o.high_contrast));
    set_map(app, "components", component_map(&o.components));
}

/// Writes each mode's seed overrides as `seeds` inside its pin map, after `mode_maps`.
fn mode_seed_maps(dst: &mut Map<String, Value>, seeds: &ModeSeeds) {
    for (key, o) in [("light", &seeds.light), ("dark", &seeds.dark)] {
        if o.is_empty() {
            continue;
        }
        with_object(dst, key, |map| {
            let mut s = Map::new();
            if let Some(c) = o.neutral {
                s.insert("neutral".to_owned(), c.to_hex().into());
            }
            if let Some(c) = o.accent {
                s.insert("accent".to_owned(), c.to_hex().into());
            }
            let st = o.status;
            let status: Map<String, Value> = [("good", st.good), ("warn", st.warn), ("bad", st.bad), ("info", st.info)]
                .into_iter()
                .filter_map(|(k, c)| Some((k.to_owned(), c?.to_hex().into())))
                .collect();
            if !status.is_empty() {
                s.insert("status".to_owned(), Value::Object(status));
            }
            map.insert("seeds".to_owned(), Value::Object(s));
        });
    }
}

fn mode_maps(dst: &mut Map<String, Value>, pins: &ModePins) {
    set_map(dst, "light", pin_map(&pins.light));
    set_map(dst, "dark", pin_map(&pins.dark));
}

/// Runs `f` on the object under `key`, creating it when absent or not an object.
fn with_object(dst: &mut Map<String, Value>, key: &str, f: impl FnOnce(&mut Map<String, Value>)) {
    let mut obj = match dst.remove(key) {
        Some(Value::Object(o)) => o,
        _ => Map::new(),
    };
    f(&mut obj);
    dst.insert(key.to_owned(), Value::Object(obj));
}

/// Runs `f` on the object under `key`; an object that was absent and stays empty is not added.
fn section(dst: &mut Map<String, Value>, key: &str, f: impl FnOnce(&mut Map<String, Value>)) {
    let existed = dst.contains_key(key);
    let mut obj = match dst.remove(key) {
        Some(Value::Object(o)) => o,
        _ => Map::new(),
    };
    f(&mut obj);
    if existed || !obj.is_empty() {
        dst.insert(key.to_owned(), Value::Object(obj));
    }
}

/// Replaces a token map; an empty one is left out unless the file had the key.
fn set_map(dst: &mut Map<String, Value>, key: &str, map: Map<String, Value>) {
    if !map.is_empty() || dst.contains_key(key) {
        dst.insert(key.to_owned(), Value::Object(map));
    }
}

fn pin_map(pins: &Pins) -> Map<String, Value> {
    let refs = pins.refs.iter().map(|(t, target)| (t.path(), Value::from(format!("{{{}}}", target.path()))));
    pins.colors.iter().filter(|(t, _)| !pins.refs.contains_key(t)).map(|(t, c)| (t.path(), Value::from(c.to_hex()))).chain(refs).collect()
}

fn component_map(components: &BTreeMap<ComponentField, f32>) -> Map<String, Value> {
    components.iter().map(|(f, &v)| (f.path(), length(v))).collect()
}

fn length(v: f32) -> Value {
    if v == Radius::Full.px() {
        return "full".into();
    }
    number(v)
}

/// `v` as a JSON number.
pub(super) fn number(v: f32) -> Value {
    // Through the shortest decimal form, so 0.1f32 is written 0.1, not 0.10000000149.
    v.to_string().parse::<f64>().ok().and_then(Number::from_f64).map_or(Value::Null, Value::Number)
}

fn edge(e: EdgeSeed) -> Value {
    match e {
        EdgeSeed::Flat => "flat".into(),
        EdgeSeed::Bevel { width } => {
            let mut w = Map::new();
            w.insert("width".to_owned(), length(width));
            let mut b = Map::new();
            b.insert("bevel".to_owned(), Value::Object(w));
            Value::Object(b)
        }
    }
}

/// Two-space pretty JSON with sorted keys, the top level in [`TOP_ORDER`].
/// Pretty-prints `v`; the keys of the top level are ordered by `top` when given, sorted otherwise.
pub(super) fn emit(v: &Value, depth: usize, top: Option<&[&str]>, out: &mut String) {
    let pad = |n: usize| "  ".repeat(n);
    match v {
        Value::Object(o) if !o.is_empty() => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort_by_key(|k| {
                let rank = top.map_or(0, |order| order.iter().position(|t| t == k).unwrap_or(order.len()));
                (rank, k.as_str().to_owned())
            });
            out.push_str("{\n");
            for (i, k) in keys.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                out.push_str(&Value::String((*k).clone()).to_string());
                out.push_str(": ");
                emit(&o[*k], depth + 1, None, out);
                out.push_str(if i + 1 < keys.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(depth));
            out.push('}');
        }
        Value::Array(a) if !a.is_empty() => {
            out.push_str("[\n");
            for (i, item) in a.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                emit(item, depth + 1, None, out);
                out.push_str(if i + 1 < a.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(depth));
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Sets the value at `path` below `dst`, replacing anything that is not an object on the way.
pub(super) fn set_path(dst: &mut Map<String, Value>, path: &[&str], value: Value) {
    match path {
        [] => {}
        [key] => {
            dst.insert((*key).to_owned(), value);
        }
        [key, rest @ ..] => with_object(dst, key, |o| set_path(o, rest, value)),
    }
}
