//! The VectorCraft v1 plugin ABI (`vc_`), from `vectorcraft/crates/plugins/src/{runtime,manifest,effect}.rs`.
//!
//! ```text
//! memory
//! vc_abi_version() -> i32                       must be 1
//! vc_manifest() -> i64                          len << 32 | ptr, UTF-8 JSON, at most 64 KiB
//! vc_alloc(size: i32) -> i32                    0 is failure
//! vc_run(input, input_len, params, params_len) -> i64     len << 32 | ptr of the output JSON, negative is an error
//! ```
//!
//! A plugin sees the selection as a JSON document of objects and returns the objects to keep,
//! change, or add. The host calls `vc_alloc` for the input and for the params, writes both, calls
//! `vc_run`, and copies the output out (at most 64 MiB). Each run uses a fresh instance. The module
//! imports nothing. What the host does with the output (validate, apply as one edit, cache) stays in
//! VectorCraft. The shim passes the bytes through.
//!
//! Two kinds: a `filter` runs from Object, Plug-ins and gets 60 s of wall time, and an `effect`
//! rewrites one object's geometry each time it is drawn and gets [`LIVE_EFFECT_WALL_MS`]. An effect
//! is known to documents as `plugin.<id>`. Craft-abi 1 calls the kinds `object-filter` and
//! `live-effect`; the effect id does not change, so existing documents still resolve.
//!
//! Limits as in v1: 32 MiB module, 512 MiB memory (the input and params may use half of it), one
//! memory, 4 tables, 100,000 table elements, 50,000,000 fuel per call plus 4,000 per input byte for
//! `vc_run`, recursion depth 1024, strict wasmi module limits.

use std::sync::Arc;
use std::time::Duration;

use craft_plugin_abi::Manifest as CraftManifest;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::params::{ParamSpec, craft_params, ordered_params, params_as_object, valid_id};
use super::runtime::{Deadline, Machine, Profile};
use super::{Result, ShimError};
use crate::engine::Clock;

/// The `vc_abi_version` this host implements.
pub const ABI_VERSION: i32 = 1;
/// Longest manifest the host reads.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Most parameters a plugin may declare.
pub const MAX_PARAMS: usize = 64;
/// Wall time of one live-effect run, in milliseconds. Drawing waits for it.
pub const LIVE_EFFECT_WALL_MS: u64 = 5_000;
/// The effect id prefix of live-effect plugins: `plugin.<plugin id>`.
pub const EFFECT_PREFIX: &str = "plugin.";

/// The plugin id behind effect id `effect_id` (`plugin.<id>`).
pub fn plugin_id(effect_id: &str) -> Option<&str> {
    effect_id.strip_prefix(EFFECT_PREFIX)
}

/// The effect id of plugin `id`.
pub fn effect_id(plugin_id: &str) -> String {
    format!("{EFFECT_PREFIX}{plugin_id}")
}

/// The v1 limits. `Default` is the v1 host.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    /// Largest module accepted.
    pub max_module_bytes: usize,
    /// Cap on a plugin's linear memory. The input and params may use half of it.
    pub max_memory_bytes: usize,
    /// Largest output JSON read back.
    pub max_output_bytes: usize,
    /// Fuel any call may use, plus `fuel_per_byte` per byte of input for `vc_run`.
    pub fuel_base: u64,
    pub fuel_per_byte: u64,
    /// Deepest call nesting.
    pub max_recursion_depth: usize,
    /// Wall-clock budget for one run, in milliseconds.
    pub wall_time_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_module_bytes: 32 << 20,
            max_memory_bytes: 512 << 20,
            max_output_bytes: 64 << 20,
            fuel_base: 50_000_000,
            fuel_per_byte: 4_000,
            max_recursion_depth: 1024,
            wall_time_ms: 60_000,
        }
    }
}

impl Limits {
    /// The limits in the words the plugin list shows. A live effect runs with a shorter wall time.
    pub fn lines(&self) -> Vec<super::LimitLine> {
        use super::{LimitLine, grouped, size_text};
        vec![
            LimitLine::new("module size", size_text(self.max_module_bytes as u64)),
            LimitLine::new("memory", size_text(self.max_memory_bytes as u64)),
            LimitLine::new("output size", size_text(self.max_output_bytes as u64)),
            LimitLine::new("instructions per call", format!("{} plus {} per input byte", grouped(self.fuel_base), grouped(self.fuel_per_byte))),
            LimitLine::new("call depth", grouped(self.max_recursion_depth as u64)),
            LimitLine::new("wall time per run", format!("{} s ({} s for a live effect)", self.wall_time_ms / 1000, LIVE_EFFECT_WALL_MS / 1000)),
        ]
    }
}

/// What a plugin is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// An object filter: reads the selected paths and returns the objects to keep, change or add.
    Filter,
    /// A live effect: rewrites one object's geometry each time it is drawn.
    Effect,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Filter => "filter",
            Kind::Effect => "effect",
        }
    }
}

fn ordered<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<(String, ParamSpec)>, D::Error> {
    ordered_params::<D, true, MAX_PARAMS>(d)
}

/// The manifest `vc_manifest` returns.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    /// Stable id, `[A-Za-z0-9._-]{1,64}`.
    pub id: String,
    /// Menu label.
    pub name: String,
    #[serde(default)]
    pub version: String,
    pub kind: Kind,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// Parameters in dialog order.
    #[serde(default, deserialize_with = "ordered", serialize_with = "params_as_object")]
    pub params: Vec<(String, ParamSpec)>,
}

impl PluginManifest {
    /// Parse and validate manifest JSON.
    pub fn parse(bytes: &[u8]) -> Result<PluginManifest> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ShimError::Manifest(format!("manifest is {} bytes (limit {MAX_MANIFEST_BYTES})", bytes.len())));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| ShimError::Manifest("manifest is not UTF-8".into()))?;
        let man: PluginManifest = serde_json::from_str(text).map_err(|e| ShimError::Manifest(format!("manifest: {e}")))?;
        man.validate()?;
        Ok(man)
    }

    fn validate(&self) -> Result<()> {
        let bad = |m: String| Err(ShimError::Manifest(m));
        if !valid_id(&self.id) {
            return bad(format!("invalid id {:?}: use 1-64 characters from A-Z a-z 0-9 . _ -", self.id));
        }
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            return bad("`name` must be 1-64 printable characters".into());
        }
        if self.version.len() > 32 || self.description.len() > 1024 || self.author.len() > 128 {
            return bad("`version`, `description` or `author` is too long".into());
        }
        for (k, spec) in &self.params {
            if !valid_key(k) {
                return bad(format!("invalid parameter name {k:?}"));
            }
            match spec {
                ParamSpec::Number { min, max, default } if !(min.is_finite() && max.is_finite() && default.is_finite()) || min > max => {
                    return bad(format!("parameter `{k}`: bad range"));
                }
                ParamSpec::Int { min, max, .. } if min > max => return bad(format!("parameter `{k}`: bad range")),
                ParamSpec::Choice { options, default } => {
                    if options.is_empty()
                        || options.len() > 64
                        || options.iter().any(|o| o.is_empty() || o.len() > 64 || o.chars().any(char::is_control))
                    {
                        return bad(format!("parameter `{k}`: choices must be 1-64 non-empty, printable strings"));
                    }
                    if default.as_ref().is_some_and(|d| !options.contains(d)) {
                        return bad(format!("parameter `{k}`: default is not one of the options"));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// The parameters documented the way command and effect parameters are
    /// (`{amount: 0..100 (50), mode: "soft"|"hard" (soft), mono: bool (false)}`).
    pub fn params_doc(&self) -> String {
        let parts: Vec<String> = self
            .params
            .iter()
            .map(|(k, spec)| match spec {
                ParamSpec::Number { min, max, default } => format!("{k}: {min}..{max} ({default})"),
                ParamSpec::Int { min, max, default } => format!("{k}: int {min}..{max} ({default})"),
                ParamSpec::Bool { default } => format!("{k}: bool ({default})"),
                ParamSpec::Choice { options, .. } => {
                    let opts: Vec<String> = options.iter().map(|o| format!("{o:?}")).collect();
                    format!("{k}: {} ({})", opts.join("|"), spec.default_value().as_str().unwrap_or_default())
                }
            })
            .collect();
        format!("{{{}}}", parts.join(", "))
    }

    /// Every parameter at its default value.
    pub fn defaults(&self) -> Map<String, Value> {
        self.params.iter().map(|(k, spec)| (k.clone(), spec.default_value())).collect()
    }

    /// Validate user `params`: a missing key takes its default, numbers are clamped into range
    /// (whole numbers rounded, JSON integers kept exact), unknown keys are dropped, and a value of
    /// the wrong type is an error.
    pub fn resolve_params(&self, params: &Value) -> Result<Map<String, Value>> {
        let given = match params {
            Value::Null => &Map::new(),
            Value::Object(m) => m,
            _ => return Err(ShimError::Params("plug-in parameters must be a JSON object".into())),
        };
        let mut out = Map::new();
        for (k, spec) in &self.params {
            let Some(v) = given.get(k).filter(|v| !v.is_null()) else {
                out.insert(k.clone(), spec.default_value());
                continue;
            };
            let wrong = |what: &str| ShimError::Params(format!("`{k}` must be {what}"));
            let resolved = match spec {
                ParamSpec::Number { min, max, .. } => json!(v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| wrong("a number"))?.clamp(*min, *max)),
                ParamSpec::Int { min, max, .. } => {
                    // Keep JSON integers exact: f64 cannot represent every i64 or its bounds.
                    let value = if let Some(i) = v.as_i64() {
                        i.clamp(*min, *max)
                    } else if v.is_u64() {
                        // Past i64::MAX, so past any maximum.
                        *max
                    } else {
                        let f = v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| wrong("a number"))?;
                        (f.round() as i64).clamp(*min, *max)
                    };
                    json!(value)
                }
                ParamSpec::Bool { .. } => json!(v.as_bool().ok_or_else(|| wrong("true or false"))?),
                ParamSpec::Choice { options, .. } => {
                    let s = v.as_str().ok_or_else(|| wrong("a string"))?;
                    if !options.iter().any(|o| o == s) {
                        return Err(ShimError::Params(format!("`{k}` must be one of {}", options.join(", "))));
                    }
                    json!(s)
                }
            };
            out.insert(k.clone(), resolved);
        }
        Ok(out)
    }
}

/// `[A-Za-z0-9_]{1,64}`, not starting with `_` and not a key the commands and dialogs use themselves.
fn valid_key(k: &str) -> bool {
    (1..=64).contains(&k.len())
        && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !k.starts_with('_')
        && !matches!(k, "id" | "ids" | "params" | "preview")
}

type RunArgs = (i32, i32, i32, i32);

/// A loaded VectorCraft plugin. Cheap to share, safe to run from several threads: every run gets
/// its own store and instance.
pub struct VectorcraftPlugin {
    manifest: PluginManifest,
    machine: Machine,
    limits: Limits,
    size: usize,
    clock: Arc<dyn Clock>,
}

fn profile(limits: &Limits) -> Profile {
    Profile { max_memory_bytes: limits.max_memory_bytes, strict: true, max_recursion_depth: Some(limits.max_recursion_depth) }
}

impl VectorcraftPlugin {
    /// Validate and load a module: check the ABI version, the exports, and read the manifest.
    pub fn load(bytes: &[u8], limits: Limits, clock: Arc<dyn Clock>) -> Result<VectorcraftPlugin> {
        if bytes.len() > limits.max_module_bytes {
            return Err(ShimError::Module(format!("module is {} bytes (limit {})", bytes.len(), limits.max_module_bytes)));
        }
        if !bytes.starts_with(b"\0asm") {
            return Err(ShimError::Module("not a WebAssembly binary (missing \\0asm header)".into()));
        }
        let machine = Machine::new(bytes, profile(&limits))?;
        let deadline = Deadline::new(clock.clone(), Some(Duration::from_millis(limits.wall_time_ms)));
        let mut live = machine.instantiate(limits.fuel_base, &deadline)?;
        let version = live.func::<(), i32>("vc_abi_version")?;
        let v = live.call(&version, (), &mut limits.fuel_base.clone(), &deadline)?;
        if v != ABI_VERSION {
            return Err(ShimError::Abi(format!("vc_abi_version returned {v}; this host implements version {ABI_VERSION}")));
        }
        let manifest_fn = live.func::<(), i64>("vc_manifest")?;
        let packed = live.call(&manifest_fn, (), &mut limits.fuel_base.clone(), &deadline)?;
        let text = live.read_packed(packed, "vc_manifest", MAX_MANIFEST_BYTES).map_err(|e| match e {
            ShimError::Limit(_) => ShimError::Manifest(format!("manifest is too long (limit {MAX_MANIFEST_BYTES} bytes)")),
            e => e,
        })?;
        let manifest = PluginManifest::parse(&text)?;
        // The other exports must exist with the right types.
        live.func::<i32, i32>("vc_alloc")?;
        live.func::<RunArgs, i64>("vc_run")?;
        Ok(VectorcraftPlugin { manifest, machine, limits, size: bytes.len(), clock })
    }

    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Module size in bytes.
    pub fn size(&self) -> usize {
        self.size
    }

    /// The limits for a live-effect run: the shorter wall time.
    pub fn live_effect_limits(&self) -> Limits {
        Limits { wall_time_ms: LIVE_EFFECT_WALL_MS, ..self.limits.clone() }
    }

    /// Run `vc_run` on `input` (the objects JSON) with `params` (the params JSON) in a fresh instance
    /// and return the output JSON bytes.
    pub fn run(&self, input: &[u8], params: &[u8]) -> Result<Vec<u8>> {
        self.run_limited(input, params, &self.limits)
    }

    /// [`VectorcraftPlugin::run`] with other time and instruction budgets. A live effect passes
    /// [`VectorcraftPlugin::live_effect_limits`].
    pub fn run_limited(&self, input: &[u8], params: &[u8], limits: &Limits) -> Result<Vec<u8>> {
        // wasmi reports a misbehaving module as an error. A panic would be a bug in it, and must
        // not take the document down either.
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run_inner(input, params, limits)));
        run.map_err(|_| ShimError::Trap("the runtime panicked".into()))?
    }

    fn run_inner(&self, input: &[u8], params: &[u8], limits: &Limits) -> Result<Vec<u8>> {
        let total = input.len().checked_add(params.len()).filter(|b| *b <= i32::MAX as usize && *b <= self.limits.max_memory_bytes / 2);
        total.ok_or_else(|| ShimError::Limit("memory budget (the selection is too large to hand over)".into()))?;
        let deadline = Deadline::new(self.clock.clone(), Some(Duration::from_millis(limits.wall_time_ms)));
        let mut live = self.machine.instantiate(self.limits.fuel_base, &deadline)?;
        let alloc = live.func::<i32, i32>("vc_alloc")?;
        let run = live.func::<RunArgs, i64>("vc_run")?;
        let place = |live: &mut super::runtime::Live, bytes: &[u8]| -> Result<usize> {
            let ptr = live.call(&alloc, bytes.len().max(1) as i32, &mut limits.fuel_base.clone(), &deadline)? as u32 as usize;
            if ptr == 0 {
                return Err(ShimError::Failed("vc_alloc returned 0 (out of memory)".into()));
            }
            let outside = || ShimError::Abi("vc_alloc returned a block outside memory".into());
            let end = ptr.checked_add(bytes.len()).ok_or_else(outside)?;
            live.mem_mut().get_mut(ptr..end).ok_or_else(outside)?.copy_from_slice(bytes);
            Ok(ptr)
        };
        let iptr = place(&mut live, input)?;
        let pptr = place(&mut live, params)?;
        let mut fuel = limits.fuel_base.saturating_add((input.len() as u64).saturating_mul(limits.fuel_per_byte));
        let packed = live.call(&run, (iptr as i32, input.len() as i32, pptr as i32, params.len() as i32), &mut fuel, &deadline)?;
        if packed < 0 {
            return Err(ShimError::Failed(format!("vc_run returned error code {packed}")));
        }
        live.read_packed(packed, "the output", limits.max_output_bytes)
    }

    /// The plugin as a craft-abi 1 manifest: an `object-filter` (60 s, budget `batch`) or a
    /// `live-effect` (budget `interactive`), `fresh` instance. Craft manifests keep their `params` sorted by
    /// name, so the dialog order of the v1 manifest does not carry over.
    pub fn craft_manifest(&self) -> Result<CraftManifest> {
        let m = &self.manifest;
        let (kind, id, budget) = match m.kind {
            Kind::Filter => ("object-filter", "filter", "batch"),
            Kind::Effect => ("live-effect", "effect", "interactive"),
        };
        let doc = json!({
            "schema": 1, "id": m.id, "name": m.name,
            "version": if m.version.is_empty() { "0" } else { m.version.as_str() },
            "description": if m.description.is_empty() { Value::Null } else { json!(m.description) },
            "author": if m.author.is_empty() { Value::Null } else { json!(m.author) },
            "abi": { "craft": 1 },
            "limits": { "memory_mb": self.limits.max_memory_bytes >> 20 },
            "legacy": "vectorcraft-v1",
            "contributions": [{
                "kind": kind, "id": id, "entry": 0, "label": m.name,
                "instance": "fresh", "budget": budget,
                "params": craft_params(&m.params)
            }]
        });
        super::parse_craft_manifest(doc)
    }
}
