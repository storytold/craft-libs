//! The EffectCraft v1 plugin ABI (`ec_`), from `effectcraft/crates/plugin/src/{lib,wasm}.rs` and
//! `effectcraft/crates/effects/src/plugin.rs`.
//!
//! ```text
//! memory                                              linear memory
//! ec_api_version() -> i32                             must be 1
//! ec_manifest_ptr() -> i32, ec_manifest_len() -> i32  UTF-8 JSON manifest, at most 1 MiB
//! ec_alloc(bytes: i32) -> i32                         8-byte aligned buffer, may reuse the last one
//! ec_render(px, w, h, params, nparams, time: f64, scale: f64) -> i32     0 is ok
//! ```
//!
//! For each render the host asks `ec_alloc` for one buffer, writes `nparams` little-endian `f64`
//! followed by `w * h * 4` little-endian `f32` (premultiplied RGBA, row-major), calls `ec_render`,
//! and reads the pixels back. The module imports nothing. Instances are pooled: a render takes one
//! (or makes one) and returns it only after a success, so a module that trapped is not reused.
//!
//! Limits as in v1: 50,000,000 fuel plus 20,000 per pixel for `ec_alloc` and `ec_render` together,
//! 1 GiB of linear memory. A v1 host had no wall clock and no table caps. This host adds both:
//! 60 s of wall time per render on the injected clock, and the strict wasmi module limits with one
//! memory and 4 tables of 100,000 elements, as PhotoCraft and VectorCraft have. A v1 host also
//! accepted `.wat` text. This one takes binary only.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft_plugin_abi::Manifest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wasmi::TypedFunc;

use super::runtime::{Deadline, Live, Machine, Profile};
use super::{Result, ShimError};
use crate::engine::Clock;

/// The `ec_api_version` this host implements.
pub const API_VERSION: u32 = 1;
/// Longest manifest read, in bytes.
pub const MAX_MANIFEST_BYTES: i32 = 1 << 20;
/// Most parameters a plugin may declare.
pub const MAX_PARAMS: usize = 256;

/// The v1 limits. `Default` is the v1 host: change a field only in a test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Fuel for any call, and the base of a render.
    pub fuel_base: u64,
    pub fuel_per_pixel: u64,
    /// Largest linear memory (a 4K float frame is about 130 MB).
    pub max_memory_bytes: usize,
    /// Wall-clock budget for one render, in milliseconds. v1 had none; this host adds it.
    pub wall_time_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { fuel_base: 50_000_000, fuel_per_pixel: 20_000, max_memory_bytes: 1 << 30, wall_time_ms: 60_000 }
    }
}

impl Limits {
    /// The limits in the words the plugin list shows.
    pub fn lines(&self) -> Vec<super::LimitLine> {
        use super::{LimitLine, grouped, size_text};
        vec![
            LimitLine::new("memory", size_text(self.max_memory_bytes as u64)),
            LimitLine::new("instructions per call", format!("{} plus {} per pixel", grouped(self.fuel_base), grouped(self.fuel_per_pixel))),
            LimitLine::new("wall time per run", format!("{} s", self.wall_time_ms / 1000)),
        ]
    }
}

/// A parameter's kind, default and range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ParamKind {
    Slider {
        default: f64,
        #[serde(default)]
        min: f64,
        #[serde(default = "hundred")]
        max: f64,
        /// Slider range (defaults to min..max).
        #[serde(default, rename = "sliderMin")]
        slider_min: Option<f64>,
        #[serde(default, rename = "sliderMax")]
        slider_max: Option<f64>,
        #[serde(default = "two")]
        decimals: u8,
    },
    Angle {
        #[serde(default)]
        default: f64,
    },
    Checkbox {
        #[serde(default)]
        default: bool,
    },
    Popup {
        options: Vec<String>,
        /// 0-based index.
        #[serde(default)]
        default: u32,
    },
    /// Default as fractions of the layer size (`[0.5, 0.5]` is the centre).
    Point {
        #[serde(default = "centre")]
        default: [f64; 2],
    },
    /// RGBA, 0 to 1.
    Color {
        #[serde(default = "white")]
        default: [f64; 4],
    },
}

fn hundred() -> f64 {
    100.0
}

fn two() -> u8 {
    2
}

fn centre() -> [f64; 2] {
    [0.5, 0.5]
}

fn white() -> [f64; 4] {
    [1.0; 4]
}

impl ParamKind {
    /// How many `f64` the parameter flattens to: a point is 2, a color is 4, the rest 1.
    pub fn width(&self) -> usize {
        match self {
            ParamKind::Point { .. } => 2,
            ParamKind::Color { .. } => 4,
            _ => 1,
        }
    }
}

/// One parameter of a plugin effect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Param {
    /// Stable id: letters, digits, `_`.
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub kind: ParamKind,
}

/// What a plugin declares about itself (`ec_manifest_ptr`, `ec_manifest_len`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectManifest {
    /// The plugin API version it was written for. Must be [`API_VERSION`].
    pub api: u32,
    /// Stable reverse-domain id (`org.example.posterize`). `ec.` is reserved.
    pub id: String,
    pub name: String,
    /// Effects and Presets category.
    pub category: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub params: Vec<Param>,
}

impl EffectManifest {
    /// Check the manifest against this API version.
    pub fn validate(&self) -> Result<()> {
        let bad = |m: String| Err(ShimError::Manifest(m));
        if self.api != API_VERSION {
            return bad(format!("plug-in `{}` targets plug-in API {} but this build implements API {API_VERSION}", self.id, self.api));
        }
        let ok_id = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if !ok_id(&self.id) || self.id.starts_with("ec.") {
            return bad(format!("bad plug-in id `{}` (reverse-domain, letters, digits, `.`, `_`, `-`; `ec.` is reserved)", self.id));
        }
        if self.name.trim().is_empty() || self.category.trim().is_empty() {
            return bad(format!("plug-in `{}` needs a name and a category", self.id));
        }
        if self.params.len() > MAX_PARAMS {
            return bad(format!("plug-in `{}` has {} parameters (at most {MAX_PARAMS})", self.id, self.params.len()));
        }
        let mut seen = HashSet::new();
        for p in &self.params {
            if p.id.is_empty() || !p.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || !seen.insert(p.id.as_str()) {
                return bad(format!("plug-in `{}`: bad or duplicate parameter id `{}`", self.id, p.id));
            }
            if let ParamKind::Popup { options, default } = &p.kind
                && (options.is_empty() || *default as usize >= options.len())
            {
                return bad(format!("plug-in `{}`: popup `{}` needs options and a default index in range", self.id, p.id));
            }
            if let ParamKind::Slider { default, min, max, slider_min, slider_max, .. } = &p.kind {
                let (lo, hi) = (slider_min.unwrap_or(*min), slider_max.unwrap_or(*max));
                if [*default, *min, *max, lo, hi].iter().any(|v| !v.is_finite()) || min > max || lo > hi || default < min || default > max {
                    return bad(format!("plug-in `{}`: slider `{}` needs min <= default <= max (and sliderMin <= sliderMax)", self.id, p.id));
                }
            }
        }
        Ok(())
    }

    /// Total flattened parameter count.
    pub fn flat_len(&self) -> usize {
        self.params.iter().map(|p| p.kind.width()).sum()
    }

    /// Turn named values, as the craft-abi 1 side of the plugin declares them, into the flat `f64`
    /// list `ec_render` takes. `values` is an object keyed by parameter id, or null for all
    /// defaults. The rules are those of the other two v1 shims (and of EffectCraft's own app, which
    /// clamped before it called): a missing id takes its default, a slider value is clamped into
    /// `min..=max`, an unknown id is dropped, and a value of the wrong type is an error. A popup is
    /// given by its option text and flattens to its index.
    pub fn flatten_params(&self, values: &Value) -> Result<Vec<f64>> {
        let empty = serde_json::Map::new();
        let given = match values {
            Value::Null => &empty,
            Value::Object(m) => m,
            other => return Err(ShimError::Params(format!("parameters must be an object, got {other}"))),
        };
        let number = |id: &str, v: &Value| -> Result<f64> {
            v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| ShimError::Params(format!("parameter `{id}` must be a finite number, got {v}")))
        };
        let list = |id: &str, v: &Value, n: usize| -> Result<Vec<f64>> {
            let items = v
                .as_array()
                .filter(|a| a.len() == n)
                .ok_or_else(|| ShimError::Params(format!("parameter `{id}` must be a list of {n} numbers, got {v}")))?;
            items.iter().map(|x| number(id, x)).collect()
        };
        let mut flat = Vec::with_capacity(self.flat_len());
        for p in &self.params {
            let id = p.id.as_str();
            match &p.kind {
                ParamKind::Slider { default, min, max, .. } => {
                    let v = given.get(id).map_or(Ok(*default), |v| number(id, v))?;
                    flat.push(v.clamp(*min, *max));
                }
                ParamKind::Angle { default } => flat.push(given.get(id).map_or(Ok(*default), |v| number(id, v))?),
                ParamKind::Checkbox { default } => {
                    let on = match given.get(id) {
                        None => *default,
                        Some(Value::Bool(b)) => *b,
                        Some(other) => return Err(ShimError::Params(format!("parameter `{id}` must be true or false, got {other}"))),
                    };
                    flat.push(f64::from(u8::from(on)));
                }
                ParamKind::Popup { options, default } => {
                    let index = match given.get(id) {
                        None => *default as usize,
                        Some(Value::String(s)) => options
                            .iter()
                            .position(|o| o == s)
                            .ok_or_else(|| ShimError::Params(format!("parameter `{id}` is {s:?}, not one of {options:?}")))?,
                        Some(other) => return Err(ShimError::Params(format!("parameter `{id}` must be one of {options:?}, got {other}"))),
                    };
                    flat.push(index as f64);
                }
                ParamKind::Point { default } => match given.get(id) {
                    None => flat.extend_from_slice(default),
                    Some(v) => flat.extend(list(id, v, 2)?),
                },
                ParamKind::Color { default } => match given.get(id) {
                    None => flat.extend_from_slice(default),
                    Some(v) => flat.extend(list(id, v, 4)?),
                },
            }
        }
        Ok(flat)
    }
}

type RenderFn = TypedFunc<(i32, i32, i32, i32, i32, f64, f64), i32>;

/// One instantiated module. Renders on several threads each take their own.
struct Inst {
    live: Live,
    alloc: TypedFunc<i32, i32>,
    render: RenderFn,
}

/// A loaded EffectCraft plugin.
pub struct EffectcraftPlugin {
    manifest: EffectManifest,
    machine: Machine,
    limits: Limits,
    clock: Arc<dyn Clock>,
    /// Idle instances.
    pool: Mutex<Vec<Inst>>,
}

fn profile(limits: &Limits) -> Profile {
    Profile { max_memory_bytes: limits.max_memory_bytes, strict: true, max_recursion_depth: Some(1024) }
}

impl EffectcraftPlugin {
    /// Compile the module, reject imports, check `ec_api_version`, and read and validate the manifest.
    pub fn load(bytes: &[u8], limits: Limits, clock: Arc<dyn Clock>) -> Result<EffectcraftPlugin> {
        let machine = Machine::new(bytes, profile(&limits))?;
        let none = Deadline::new(clock.clone(), Some(Duration::from_millis(limits.wall_time_ms)));
        let mut inst = Self::instantiate(&machine, &limits, &none)?;
        let mut pool = limits.fuel_base;
        let version = inst.live.func::<(), i32>("ec_api_version")?;
        let v = inst.live.call(&version, (), &mut pool, &none)?;
        if i64::from(v) != i64::from(API_VERSION) {
            return Err(ShimError::Abi(format!("the module implements plug-in API {v}, this build implements API {API_VERSION}")));
        }
        let ptr_fn = inst.live.func::<(), i32>("ec_manifest_ptr")?;
        let len_fn = inst.live.func::<(), i32>("ec_manifest_len")?;
        let ptr = inst.live.call(&ptr_fn, (), &mut pool, &none)?;
        let len = inst.live.call(&len_fn, (), &mut pool, &none)?;
        if !(0..=MAX_MANIFEST_BYTES).contains(&len) {
            return Err(ShimError::Manifest(format!("bad manifest length {len}")));
        }
        let start = ptr as u32 as usize;
        let text =
            inst.live.mem().get(start..start.saturating_add(len as usize)).ok_or_else(|| ShimError::Manifest("manifest out of bounds".into()))?;
        let manifest: EffectManifest = serde_json::from_slice(text).map_err(|e| ShimError::Manifest(format!("bad manifest JSON: {e}")))?;
        manifest.validate()?;
        Ok(EffectcraftPlugin { manifest, machine, limits, clock, pool: Mutex::new(vec![inst]) })
    }

    fn instantiate(machine: &Machine, limits: &Limits, deadline: &Deadline) -> Result<Inst> {
        let live = machine.instantiate(limits.fuel_base, deadline)?;
        let alloc = live.func::<i32, i32>("ec_alloc")?;
        let render = live.func::<(i32, i32, i32, i32, i32, f64, f64), i32>("ec_render")?;
        Ok(Inst { live, alloc, render })
    }

    pub fn manifest(&self) -> &EffectManifest {
        &self.manifest
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Idle instances in the pool.
    pub fn pooled(&self) -> usize {
        self.pool.lock().unwrap_or_else(std::sync::PoisonError::into_inner).len()
    }

    fn take(&self, deadline: &Deadline) -> Result<Inst> {
        let pooled = self.pool.lock().unwrap_or_else(std::sync::PoisonError::into_inner).pop();
        match pooled {
            Some(i) => Ok(i),
            None => Self::instantiate(&self.machine, &self.limits, deadline),
        }
    }

    fn give_back(&self, inst: Inst) {
        self.pool.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(inst);
    }

    /// Render `pixels` in place. `params` is the flattened `f64` list of [`EffectManifest::flat_len`]
    /// values (points in frame pixels, color as 4 floats, popup as an index, checkbox 0 or 1).
    /// `time` is layer time in seconds. `scale` is buffer pixels per layer pixel. On any error the
    /// pixels are left as they were and the instance is dropped.
    pub fn render(&self, pixels: &mut [[f32; 4]], width: u32, height: u32, params: &[f64], time: f64, scale: f64) -> Result<()> {
        let n = pixels.len();
        if n as u64 != u64::from(width) * u64::from(height) {
            return Err(ShimError::Params(format!("{n} pixels do not fill a {width} x {height} frame")));
        }
        if params.len() != self.manifest.flat_len() {
            return Err(ShimError::Params(format!("the plug-in takes {} parameter values, got {}", self.manifest.flat_len(), params.len())));
        }
        let params_bytes = params.len() * 8;
        let total = params_bytes + n * 16;
        if total > i32::MAX as usize {
            return Err(ShimError::Limit("frame size (too large for a 32-bit plug-in)".into()));
        }
        let none = Deadline::new(self.clock.clone(), Some(Duration::from_millis(self.limits.wall_time_ms)));
        let mut inst = self.take(&none)?;
        // One pool for `ec_alloc` and `ec_render` together, as in v1.
        let mut fuel = self.limits.fuel_base.saturating_add(self.limits.fuel_per_pixel.saturating_mul(n as u64));
        let alloc = inst.alloc;
        let base = inst.live.call(&alloc, total as i32, &mut fuel, &none)? as u32 as usize;
        // Parameters, then pixels, little-endian as WebAssembly memory is.
        let mut buf: Vec<u8> = Vec::with_capacity(total);
        for v in params {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        for px in pixels.iter() {
            for c in px {
                buf.extend_from_slice(&c.to_le_bytes());
            }
        }
        let end = base.checked_add(total).ok_or_else(|| ShimError::Abi("ec_alloc returned a buffer out of bounds".into()))?;
        inst.live
            .mem_mut()
            .get_mut(base..end)
            .ok_or_else(|| ShimError::Abi("ec_alloc returned a buffer out of bounds".into()))?
            .copy_from_slice(&buf);
        let px_ptr = (base + params_bytes) as i32;
        let render = inst.render;
        let code = inst.live.call(&render, (px_ptr, width as i32, height as i32, base as i32, params.len() as i32, time, scale), &mut fuel, &none);
        let code = code.map_err(|e| match e {
            ShimError::Trap(m) => ShimError::Trap(format!("ec_render trapped: {m}")),
            other => other,
        })?;
        if code != 0 {
            return Err(ShimError::Failed(format!("ec_render returned error {code}")));
        }
        let out = inst.live.mem().get(base + params_bytes..end).ok_or_else(|| ShimError::Abi("the pixel buffer left guest memory".into()))?;
        for (px, b) in pixels.iter_mut().zip(out.as_chunks::<16>().0) {
            for (c, v) in px.iter_mut().zip(b.as_chunks::<4>().0) {
                *c = f32::from_le_bytes(*v);
            }
        }
        self.give_back(inst);
        Ok(())
    }

    /// The plugin as a craft-abi 1 manifest. One `effect` contribution, `session` instance (pooled),
    /// `render` budget, `f32` premultiplied pixels. A popup becomes a `choice` and keeps its
    /// index meaning.
    pub fn craft_manifest(&self) -> Result<Manifest> {
        let m = &self.manifest;
        let params: serde_json::Map<String, Value> = m.params.iter().map(|p| (p.id.clone(), craft_param(p))).collect();
        let doc = json!({
            "schema": 1, "id": m.id, "name": m.name,
            "version": if m.version.is_empty() { "0" } else { m.version.as_str() },
            "description": if m.description.is_empty() { Value::Null } else { json!(m.description) },
            "author": if m.author.is_empty() { Value::Null } else { json!(m.author) },
            "abi": { "craft": 1 },
            "limits": { "memory_mb": self.limits.max_memory_bytes >> 20 },
            "legacy": "effectcraft-v1",
            "contributions": [{
                "kind": "effect", "id": "effect", "entry": 0, "label": m.name, "category": m.category,
                "instance": "session", "budget": "render",
                "process": { "sample": ["f32"], "alpha": "premultiplied" },
                "params": params
            }]
        });
        super::parse_craft_manifest(doc)
    }
}

fn craft_param(p: &Param) -> Value {
    let mut v = match &p.kind {
        ParamKind::Slider { default, min, max, decimals, .. } => {
            json!({ "type": "number", "min": min, "max": max, "default": default, "decimals": decimals })
        }
        ParamKind::Angle { default } => json!({ "type": "angle", "default": default }),
        ParamKind::Checkbox { default } => json!({ "type": "bool", "default": default }),
        ParamKind::Popup { options, default } => {
            json!({ "type": "choice", "options": options, "default": options.get(*default as usize).cloned().unwrap_or_default() })
        }
        ParamKind::Point { default } => json!({ "type": "point2", "default": default }),
        ParamKind::Color { default } => json!({ "type": "color", "default": default }),
    };
    v["label"] = json!(p.name);
    v
}
