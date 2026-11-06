//! Adapters for the three v1 plugin ABIs.
//!
//! A module whose exports start with `ec_`, `pc_` or `vc_` skips the craft-abi 1 gate
//! (`PolicyReport::legacy`) and loads here. Each adapter runs the module exactly as its own app did,
//! with that app's limits, and also describes the plugin as a craft-abi 1 [`Manifest`](craft_plugin_abi::Manifest)
//! so the permission prompt, the plugin list and the command layer see
//! one kind of plugin.
//!
//! | ABI | adapter | contribution kind | data |
//! |---|---|---|---|
//! | `ec_` EffectCraft | [`effectcraft`] | `effect` | premultiplied RGBA `f32` frame, flat `f64` params, pooled instances |
//! | `pc_` PhotoCraft | [`photocraft`] | `filter` | interleaved `f32` bands with overlap, params JSON with `_image`, fresh instance per band |
//! | `vc_` VectorCraft | [`vectorcraft`] | `object-filter`, `live-effect` | objects JSON in, objects JSON out, fresh instance per run |
//!
//! What these adapters are. They are the v1 hosts of the three apps, moved here with their limits
//! unchanged ([`effectcraft::Limits`], [`photocraft::Limits`], [`vectorcraft::Limits`]). Their wasmi
//! `Config` differs from the craft-abi engine because v1 modules import nothing and export
//! different names. They share [`runtime`] for fuel slicing, the wall clock check, and the guest
//! memory cap.
//!
//! What they are not. They do not touch documents. EffectCraft's registry, PhotoCraft's selection
//! blend and tile writes, and VectorCraft's object validation stay in the apps. The adapters take
//! and return plain buffers and JSON.
//!
//! Wall time comes from an injected [`crate::engine::Clock`], so these adapters build for `wasm32`.
//! PhotoCraft and VectorCraft used `std::time::Instant` on native only and fuel alone on the web.
//! With a web clock injected they now get the wall limit there too.
//!
//! Attribution. The behavior and the limits are those of EffectCraft `crates/plugin`, PhotoCraft
//! `crates/plugins` and VectorCraft `crates/plugins`, all `MIT OR Apache-2.0`, the license of this
//! repository. The tests build their modules from WAT text and carry no binaries.

use std::fmt;

use craft_plugin_abi::{HostPolicy, LegacyAbi, check_package};

use crate::engine::Clock;
use std::sync::Arc;

pub mod effectcraft;
mod params;
pub mod photocraft;
pub mod runtime;
pub mod vectorcraft;

pub use runtime::Deadline;

/// Validate a manifest document built from a v1 manifest.
pub(crate) fn parse_craft_manifest(doc: serde_json::Value) -> Result<craft_plugin_abi::Manifest> {
    // `null` fields would fail the typed parse, and the builders write them for absent text.
    let doc = strip_nulls(doc);
    let bytes = serde_json::to_vec(&doc).map_err(|e| ShimError::Manifest(e.to_string()))?;
    craft_plugin_abi::Manifest::parse(&bytes).map_err(|e| ShimError::Manifest(format!("as a craft-abi 1 manifest: {e}")))
}

fn strip_nulls(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) => {
            serde_json::Value::Object(m.into_iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k, strip_nulls(v))).collect())
        }
        serde_json::Value::Array(a) => serde_json::Value::Array(a.into_iter().map(strip_nulls).collect()),
        other => other,
    }
}

/// A v1 plugin failed to load or to run. Every way a module misbehaves ends here, never in a panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShimError {
    /// Not a WebAssembly binary, too large, or invalid.
    Module(String),
    /// The module breaks the v1 ABI: an import, a missing export, a bad pointer, a bad version.
    Abi(String),
    Manifest(String),
    Trap(String),
    /// A limit stopped the plugin: `instruction budget`, `time budget`, `memory budget`, `output size`.
    Limit(String),
    Params(String),
    /// The plugin ran and reported an error code, or could not allocate.
    Failed(String),
    Output(String),
}

impl fmt::Display for ShimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShimError::Module(m) => write!(f, "not a valid plug-in module: {m}"),
            ShimError::Abi(m) => write!(f, "plug-in ABI: {m}"),
            ShimError::Manifest(m) => write!(f, "plug-in manifest: {m}"),
            ShimError::Trap(m) => write!(f, "plug-in trapped: {m}"),
            ShimError::Limit(m) => write!(f, "plug-in exceeded its {m}"),
            ShimError::Params(m) => write!(f, "plug-in parameters: {m}"),
            ShimError::Failed(m) => write!(f, "plug-in failed: {m}"),
            ShimError::Output(m) => write!(f, "plug-in output: {m}"),
        }
    }
}

impl std::error::Error for ShimError {}

pub type Result<T> = std::result::Result<T, ShimError>;

/// A v1 plugin loaded through the adapter its exports name.
pub enum LegacyPlugin {
    Effectcraft(effectcraft::EffectcraftPlugin),
    Photocraft(photocraft::PhotocraftPlugin),
    Vectorcraft(vectorcraft::VectorcraftPlugin),
}

impl LegacyPlugin {
    pub fn abi(&self) -> LegacyAbi {
        match self {
            LegacyPlugin::Effectcraft(_) => LegacyAbi::Effectcraft,
            LegacyPlugin::Photocraft(_) => LegacyAbi::Photocraft,
            LegacyPlugin::Vectorcraft(_) => LegacyAbi::Vectorcraft,
        }
    }

    /// The limits this plugin runs under, in the words the plugin list shows.
    pub fn limit_lines(&self) -> Vec<LimitLine> {
        match self {
            LegacyPlugin::Effectcraft(p) => p.limits().lines(),
            LegacyPlugin::Photocraft(p) => p.limits().lines(),
            LegacyPlugin::Vectorcraft(p) => p.limits().lines(),
        }
    }

    /// The plugin as a craft-abi 1 manifest with `legacy` set, no permissions and no host ops.
    pub fn craft_manifest(&self) -> Result<craft_plugin_abi::Manifest> {
        match self {
            LegacyPlugin::Effectcraft(p) => p.craft_manifest(),
            LegacyPlugin::Photocraft(p) => p.craft_manifest(),
            LegacyPlugin::Vectorcraft(p) => p.craft_manifest(),
        }
    }
}

/// One limit a v1 plugin still runs under, as the plugin list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LimitLine {
    pub name: &'static str,
    pub value: String,
}

impl LimitLine {
    pub(crate) fn new(name: &'static str, value: impl Into<String>) -> LimitLine {
        LimitLine { name, value: value.into() }
    }
}

/// `1,234,567`.
pub(crate) fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `512 MiB`, `64 KiB`, `12 bytes`.
pub(crate) fn size_text(bytes: u64) -> String {
    const MIB: u64 = 1 << 20;
    const KIB: u64 = 1 << 10;
    if bytes >= MIB && bytes.is_multiple_of(MIB) {
        format!("{} MiB", bytes / MIB)
    } else if bytes >= KIB && bytes.is_multiple_of(KIB) {
        format!("{} KiB", bytes / KIB)
    } else {
        format!("{bytes} bytes")
    }
}

/// The legacy ABI a package's exports name, from the policy gate's detection.
/// `None` for a craft-abi 1 package, and for one that mixes prefixes.
pub fn detect(wasm: &[u8]) -> Option<LegacyAbi> {
    check_package(wasm, &HostPolicy::native()).legacy
}

/// Detect the ABI of `wasm` and load it with that ABI's own default limits.
pub fn load_legacy(wasm: &[u8], clock: Arc<dyn Clock>) -> Result<LegacyPlugin> {
    let report = check_package(wasm, &HostPolicy::native());
    if !report.violations.is_empty() {
        let list = report.violations.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ");
        return Err(ShimError::Module(format!("the policy gate refuses this legacy package: {list}")));
    }
    match report.legacy {
        Some(LegacyAbi::Effectcraft) => {
            effectcraft::EffectcraftPlugin::load(wasm, effectcraft::Limits::default(), clock).map(LegacyPlugin::Effectcraft)
        }
        Some(LegacyAbi::Photocraft) => photocraft::PhotocraftPlugin::load(wasm, photocraft::Limits::default(), clock).map(LegacyPlugin::Photocraft),
        Some(LegacyAbi::Vectorcraft) => {
            vectorcraft::VectorcraftPlugin::load(wasm, vectorcraft::Limits::default(), clock).map(LegacyPlugin::Vectorcraft)
        }
        None => Err(ShimError::Abi("the exports carry no ec_, pc_ or vc_ prefix, or they mix prefixes".into())),
    }
}
