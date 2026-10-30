//! The PhotoCraft v1 plugin ABI (`pc_`), from `photocraft/crates/plugins/src/{runtime,manifest,apply}.rs`.
//!
//! ```text
//! memory
//! pc_abi_version() -> i32                       must be 1
//! pc_manifest() -> i64                          len << 32 | ptr, UTF-8 JSON, at most 64 KiB
//! pc_alloc(size: i32) -> i32                    0 is failure. Never freed.
//! pc_filter(buf, buf_len, w, h, channels, format, params, params_len) -> i32     0 is ok
//! ```
//!
//! A filter gets one band of interleaved `f32` samples in the document's own color model, straight
//! alpha, and a params JSON object that holds the resolved user values plus `_image`. The host
//! calls `pc_alloc` twice (samples, params), writes both, calls `pc_filter` and reads the samples
//! back. Each band runs in a fresh instance. The module imports nothing.
//!
//! Limits as in v1: 32 MiB module, 512 MiB memory (one memory, 4 tables, 100,000 table elements),
//! 50,000,000 fuel per call plus 4,000 per sample for `pc_filter`, recursion depth 1024, strict
//! wasmi module limits, 60 s of wall time per run. The wall time is measured on the injected clock.
//!
//! [`PhotocraftPlugin::apply`] bands a plain interleaved image. PhotoCraft's own banding works on
//! tiles with a selection blend, and stays in the app. `apply` is the same band rules on a buffer:
//! `overlap` samples of edge-replicated context around each band, only the inner rectangle kept,
//! non-finite output replaced by the input, integer depths and alpha clamped to 0..1.

use std::sync::Arc;
use std::time::Duration;

use craft_plugin_abi::Manifest as CraftManifest;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::params::{ParamSpec, craft_params, ordered_params, params_as_object, valid_id};
use super::runtime::{Deadline, Machine, Profile};
use super::{Result, ShimError};
use crate::engine::Clock;

/// The `pc_abi_version` this host implements.
pub const ABI_VERSION: i32 = 1;
/// Longest manifest the host reads.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Most parameters a plugin may declare.
pub const MAX_PARAMS: usize = 64;
/// Largest neighbourhood (`overlap`) a plugin may ask for, in pixels.
pub const MAX_OVERLAP: u32 = 256;

/// The v1 limits. `Default` is the v1 host.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    /// Largest module accepted.
    pub max_module_bytes: usize,
    /// Cap on a plugin's linear memory.
    pub max_memory_bytes: usize,
    /// Target size of one band of `f32` samples handed to the plugin.
    pub band_bytes: usize,
    /// Fuel any call may use, plus `fuel_per_sample` per sample for `pc_filter`.
    pub fuel_base: u64,
    pub fuel_per_sample: u64,
    /// Deepest call nesting.
    pub max_recursion_depth: usize,
    /// Wall-clock budget for one filter run, all bands included, in milliseconds.
    pub wall_time_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_module_bytes: 32 << 20,
            max_memory_bytes: 512 << 20,
            band_bytes: 4 << 20,
            fuel_base: 50_000_000,
            fuel_per_sample: 4_000,
            max_recursion_depth: 1024,
            wall_time_ms: 60_000,
        }
    }
}

impl Limits {
    /// The limits in the words the plugin list shows.
    pub fn lines(&self) -> Vec<super::LimitLine> {
        use super::{LimitLine, grouped, size_text};
        vec![
            LimitLine::new("module size", size_text(self.max_module_bytes as u64)),
            LimitLine::new("memory", size_text(self.max_memory_bytes as u64)),
            LimitLine::new("band size", size_text(self.band_bytes as u64)),
            LimitLine::new("instructions per call", format!("{} plus {} per sample", grouped(self.fuel_base), grouped(self.fuel_per_sample))),
            LimitLine::new("call depth", grouped(self.max_recursion_depth as u64)),
            LimitLine::new("wall time per run", format!("{} s", self.wall_time_ms / 1000)),
        ]
    }
}

/// Which pixels a filter is given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Area {
    /// The layer's non-empty pixels.
    #[default]
    Content,
    /// The whole canvas, transparent pixels included.
    Canvas,
}

/// What a plugin is. ABI v1 has filters only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Filter,
}

fn ordered<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<(String, ParamSpec)>, D::Error> {
    ordered_params::<D, false, MAX_PARAMS>(d)
}

/// The manifest `pc_manifest` returns.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterManifest {
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
    /// Pixels of context the filter needs around each output pixel. 0 is a point filter.
    #[serde(default)]
    pub overlap: u32,
    #[serde(default)]
    pub area: Area,
}

impl FilterManifest {
    /// Parse and validate manifest JSON.
    pub fn parse(bytes: &[u8]) -> Result<FilterManifest> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ShimError::Manifest(format!("manifest is {} bytes (limit {MAX_MANIFEST_BYTES})", bytes.len())));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| ShimError::Manifest("manifest is not UTF-8".into()))?;
        let man: FilterManifest = serde_json::from_str(text).map_err(|e| ShimError::Manifest(format!("manifest: {e}")))?;
        man.validate()?;
        Ok(man)
    }

    fn validate(&self) -> Result<()> {
        let bad = |m: String| Err(ShimError::Manifest(m));
        if !valid_id(&self.id) {
            return bad(format!("invalid id {:?}: use 1-64 characters from A-Z a-z 0-9 . _ -", self.id));
        }
        let name = self.name.trim();
        let printable = name.chars().all(|c| !c.is_control() && !hidden_format(c)) && name.chars().any(visible);
        if !printable || name.chars().count() > 64 {
            return bad("`name` must be 1-64 printable characters".into());
        }
        if self.version.len() > 32 || self.description.len() > 1024 || self.author.len() > 128 {
            return bad("`version`, `description` or `author` is too long".into());
        }
        if self.overlap > MAX_OVERLAP {
            return bad(format!("`overlap` {} exceeds {MAX_OVERLAP}", self.overlap));
        }
        for (k, spec) in &self.params {
            if !valid_key(k) {
                return bad(format!("invalid parameter name {k:?}"));
            }
            match spec {
                ParamSpec::Number { min, max, default } => {
                    if !(min.is_finite() && max.is_finite() && default.is_finite()) || min > max {
                        return bad(format!("parameter `{k}`: bad range"));
                    }
                }
                ParamSpec::Int { min, max, .. } if min > max => return bad(format!("parameter `{k}`: bad range")),
                ParamSpec::Choice { options, default } => {
                    if options.is_empty() || options.len() > 64 || options.iter().any(|o| o.is_empty() || o.contains(['|', '"'])) {
                        return bad(format!("parameter `{k}`: choices must be 1-64 non-empty strings without | or \""));
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

    /// The parameter notation PhotoCraft's command registry and dialogs use
    /// (`{"amount":0..100=50,"mode":"a|b","mono":bool=false}`).
    pub fn params_notation(&self) -> String {
        let parts: Vec<String> = self
            .params
            .iter()
            .map(|(k, spec)| match spec {
                ParamSpec::Number { min, max, default } => format!("\"{k}\":{min}..{max}={default}"),
                ParamSpec::Int { default, .. } => format!("\"{k}\":int={default}"),
                ParamSpec::Bool { default } => format!("\"{k}\":bool={default}"),
                ParamSpec::Choice { options, default } => {
                    // The dialog's default is the first option, so list the default first.
                    let mut opts = options.clone();
                    if let Some(d) = default
                        && let Some(i) = opts.iter().position(|o| o == d)
                    {
                        let d = opts.remove(i);
                        opts.insert(0, d);
                    }
                    format!("\"{k}\":\"{}\"", opts.join("|"))
                }
            })
            .collect();
        format!("{{{}}}", parts.join(","))
    }

    /// Validate user `params`: a missing key takes its default, numbers (defaults included) are
    /// clamped into range, unknown keys are dropped, and a value of the wrong type is an error.
    pub fn resolve_params(&self, params: &Value) -> Result<Map<String, Value>> {
        let given = match params {
            Value::Null => &Map::new(),
            Value::Object(m) => m,
            _ => return Err(ShimError::Params("plug-in parameters must be a JSON object".into())),
        };
        let mut out = Map::new();
        for (k, spec) in &self.params {
            let v = given.get(k).filter(|v| !v.is_null());
            let wrong = |what: &str| ShimError::Params(format!("`{k}` must be {what}"));
            let resolved = match spec {
                ParamSpec::Number { min, max, default } => match v {
                    None => json!((*default).clamp(*min, *max)),
                    Some(v) => json!(v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| wrong("a number"))?.clamp(*min, *max)),
                },
                ParamSpec::Int { min, max, default } => match v {
                    None => json!((*default).clamp(*min, *max)),
                    Some(v) => {
                        let f = v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| wrong("a number"))?;
                        json!((f.round().clamp(*min as f64, *max as f64)) as i64)
                    }
                },
                ParamSpec::Bool { default } => match v {
                    None => json!(default),
                    Some(v) => json!(v.as_bool().ok_or_else(|| wrong("true or false"))?),
                },
                ParamSpec::Choice { options, default } => match v {
                    None => json!(default.clone().or_else(|| options.first().cloned()).unwrap_or_default()),
                    Some(v) => {
                        let s = v.as_str().ok_or_else(|| wrong("a string"))?;
                        if !options.iter().any(|o| o == s) {
                            return Err(ShimError::Params(format!("`{k}` must be one of {}", options.join(", "))));
                        }
                        json!(s)
                    }
                },
            };
            out.insert(k.clone(), resolved);
        }
        Ok(out)
    }
}

/// The Unicode format characters (general category Cf, Unicode 15.1). Most draw nothing, and the
/// bidi controls among them reorder the text around them.
const FORMAT: &[(char, char)] = &[
    ('\u{AD}', '\u{AD}'),
    ('\u{600}', '\u{605}'),
    ('\u{61C}', '\u{61C}'),
    ('\u{6DD}', '\u{6DD}'),
    ('\u{70F}', '\u{70F}'),
    ('\u{890}', '\u{891}'),
    ('\u{8E2}', '\u{8E2}'),
    ('\u{180E}', '\u{180E}'),
    ('\u{200B}', '\u{200F}'),
    ('\u{202A}', '\u{202E}'),
    ('\u{2060}', '\u{2064}'),
    ('\u{2066}', '\u{206F}'),
    ('\u{FEFF}', '\u{FEFF}'),
    ('\u{FFF9}', '\u{FFFB}'),
    ('\u{110BD}', '\u{110BD}'),
    ('\u{110CD}', '\u{110CD}'),
    ('\u{13430}', '\u{1343F}'),
    ('\u{1BCA0}', '\u{1BCA3}'),
    ('\u{1D173}', '\u{1D17A}'),
    ('\u{E0001}', '\u{E0001}'),
    ('\u{E0020}', '\u{E007F}'),
];

/// Default-ignorable characters outside Cf (fillers, variation selectors). A name may carry them,
/// but they draw nothing on their own.
const BLANK: &[(char, char)] = &[
    ('\u{34F}', '\u{34F}'),
    ('\u{115F}', '\u{1160}'),
    ('\u{17B4}', '\u{17B5}'),
    ('\u{180B}', '\u{180D}'),
    ('\u{180F}', '\u{180F}'),
    ('\u{3164}', '\u{3164}'),
    ('\u{FE00}', '\u{FE0F}'),
    ('\u{FFA0}', '\u{FFA0}'),
    ('\u{E0100}', '\u{E01EF}'),
];

fn in_ranges(c: char, ranges: &[(char, char)]) -> bool {
    ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&c))
}

/// A format character a menu label must not carry. The zero-width non-joiner and joiner stay
/// allowed: Persian, Indic scripts and emoji sequences need them and they cannot reorder text.
fn hidden_format(c: char) -> bool {
    !matches!(c, '\u{200C}' | '\u{200D}') && in_ranges(c, FORMAT)
}

/// A character that draws something on its own.
fn visible(c: char) -> bool {
    !c.is_whitespace() && !c.is_control() && !in_ranges(c, FORMAT) && !in_ranges(c, BLANK)
}

fn valid_key(k: &str) -> bool {
    (1..=64).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') && !k.starts_with('_') && k != "id" && k != "layer"
}

/// PSD color mode numbers, as `pc_filter`'s `format` argument uses them.
pub mod mode {
    pub const BITMAP: u32 = 0;
    pub const GRAYSCALE: u32 = 1;
    pub const INDEXED: u32 = 2;
    pub const RGB: u32 = 3;
    pub const CMYK: u32 = 4;
    pub const MULTICHANNEL: u32 = 7;
    pub const DUOTONE: u32 = 8;
    pub const LAB: u32 = 9;

    /// The name the `_image.mode` field carries.
    pub fn name(code: u32) -> &'static str {
        match code {
            BITMAP => "bitmap",
            GRAYSCALE => "grayscale",
            INDEXED => "indexed",
            RGB => "rgb",
            CMYK => "cmyk",
            LAB => "lab",
            MULTICHANNEL => "multichannel",
            DUOTONE => "duotone",
            _ => "unknown",
        }
    }
}

/// The `format` argument of `pc_filter`: bits 0 to 7 the source bit depth (8, 16 or 32), bit 8 set
/// when the last channel is alpha, bits 16 to 23 the color mode ([`mode`]).
pub fn format_code(depth: u32, alpha: bool, color_mode: u32) -> u32 {
    (depth & 0xff) | (u32::from(alpha) << 8) | ((color_mode & 0xff) << 16)
}

/// What the shim needs to know about an interleaved image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    /// Samples per pixel.
    pub channels: u32,
    /// Source bit depth: 8, 16 or 32. Anything but 32 clamps the output to 0..1.
    pub depth: u32,
    /// The last channel is straight alpha.
    pub alpha: bool,
    /// PSD color mode number ([`mode`]).
    pub color_mode: u32,
}

type FilterArgs = (i32, i32, i32, i32, i32, i32, i32, i32);

/// A loaded PhotoCraft plugin. Cheap to share, safe to run from several threads: every run gets its
/// own store and instance.
pub struct PhotocraftPlugin {
    manifest: FilterManifest,
    machine: Machine,
    limits: Limits,
    size: usize,
    clock: Arc<dyn Clock>,
}

impl PhotocraftPlugin {
    /// Validate and load a module: check the ABI version, read the manifest, check the exports.
    pub fn load(bytes: &[u8], limits: Limits, clock: Arc<dyn Clock>) -> Result<PhotocraftPlugin> {
        if bytes.len() > limits.max_module_bytes {
            return Err(ShimError::Module(format!("module is {} bytes (limit {})", bytes.len(), limits.max_module_bytes)));
        }
        if !bytes.starts_with(b"\0asm") {
            return Err(ShimError::Module("not a WebAssembly binary (missing \\0asm header)".into()));
        }
        let machine = Machine::new(bytes, profile(&limits))?;
        let deadline = Deadline::new(clock.clone(), Some(Duration::from_millis(limits.wall_time_ms)));
        let mut live = machine.instantiate(limits.fuel_base, &deadline)?;
        let call_pool = || limits.fuel_base;
        let version = live.func::<(), i32>("pc_abi_version")?;
        let v = live.call(&version, (), &mut call_pool(), &deadline)?;
        if v != ABI_VERSION {
            return Err(ShimError::Abi(format!("pc_abi_version returned {v}; this host implements version {ABI_VERSION}")));
        }
        let manifest_fn = live.func::<(), i64>("pc_manifest")?;
        let packed = live.call(&manifest_fn, (), &mut call_pool(), &deadline)? as u64;
        let (ptr, len) = ((packed & 0xffff_ffff) as usize, (packed >> 32) as usize);
        if len > MAX_MANIFEST_BYTES {
            return Err(ShimError::Manifest(format!("manifest is {len} bytes (limit {MAX_MANIFEST_BYTES})")));
        }
        let text = live.mem().get(ptr..ptr.saturating_add(len)).ok_or_else(|| ShimError::Abi("pc_manifest points outside memory".into()))?;
        let manifest = FilterManifest::parse(text)?;
        // The other exports must exist with the right types.
        live.func::<i32, i32>("pc_alloc")?;
        live.func::<FilterArgs, i32>("pc_filter")?;
        Ok(PhotocraftPlugin { manifest, machine, limits, size: bytes.len(), clock })
    }

    pub fn manifest(&self) -> &FilterManifest {
        &self.manifest
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Module size in bytes.
    pub fn size(&self) -> usize {
        self.size
    }

    /// A deadline for one filter run: the wall budget starts now.
    pub fn deadline(&self) -> Deadline {
        Deadline::new(self.clock.clone(), Some(Duration::from_millis(self.limits.wall_time_ms)))
    }

    /// Run `pc_filter` on `data` (interleaved `f32`, `w x h x channels`) in a fresh instance.
    /// `params` is the JSON the plugin receives. One band; banding is the caller's.
    #[allow(clippy::too_many_arguments)]
    pub fn filter_buffer(&self, data: &mut [f32], w: u32, h: u32, channels: u32, format: u32, params: &[u8], deadline: &Deadline) -> Result<()> {
        let bytes = data.len().checked_mul(4).filter(|b| *b <= i32::MAX as usize && *b + params.len() <= self.limits.max_memory_bytes);
        let bytes = bytes.ok_or_else(|| ShimError::Limit("memory budget (band too large)".into()))?;
        let mut live = self.machine.instantiate(self.limits.fuel_base, deadline)?;
        let alloc = live.func::<i32, i32>("pc_alloc")?;
        let filter = live.func::<FilterArgs, i32>("pc_filter")?;
        let ptr = live.call(&alloc, bytes as i32, &mut self.limits.fuel_base.clone(), deadline)? as u32 as usize;
        let pptr = live.call(&alloc, params.len().max(1) as i32, &mut self.limits.fuel_base.clone(), deadline)? as u32 as usize;
        if ptr == 0 || pptr == 0 {
            return Err(ShimError::Failed("pc_alloc returned 0 (out of memory)".into()));
        }
        let outside = || ShimError::Abi("pc_alloc returned a block outside memory".into());
        let end = ptr.checked_add(bytes).ok_or_else(outside)?;
        let pend = pptr.checked_add(params.len()).ok_or_else(outside)?;
        {
            let mem = live.mem_mut();
            let dst = mem.get_mut(ptr..end).ok_or_else(outside)?;
            for (d, v) in dst.as_chunks_mut::<4>().0.iter_mut().zip(data.iter()) {
                *d = v.to_le_bytes();
            }
            mem.get_mut(pptr..pend).ok_or_else(outside)?.copy_from_slice(params);
        }
        let mut fuel = self.limits.fuel_base.saturating_add((data.len() as u64).saturating_mul(self.limits.fuel_per_sample));
        let args = (ptr as i32, bytes as i32, w as i32, h as i32, channels as i32, format as i32, pptr as i32, params.len() as i32);
        let rc = live.call(&filter, args, &mut fuel, deadline)?;
        if rc != 0 {
            return Err(ShimError::Failed(format!("pc_filter returned error code {rc}")));
        }
        let src = live.mem().get(ptr..end).ok_or_else(|| ShimError::Abi("pixel buffer no longer in memory".into()))?;
        for (v, s) in data.iter_mut().zip(src.as_chunks::<4>().0) {
            *v = f32::from_le_bytes(*s);
        }
        Ok(())
    }

    /// Filter a whole interleaved image in place, band by band, with the user `params` resolved
    /// against the manifest. Bands are full-width strips of about `band_bytes`, each with
    /// `overlap` rows and columns of context (edge pixels repeated outside the image). A failure
    /// in any band leaves `data` as it was.
    pub fn apply(&self, data: &mut [f32], info: &ImageInfo, params: &Value) -> Result<()> {
        let n = info.channels as usize;
        let (w, h) = (info.width as usize, info.height as usize);
        if n == 0 || data.len() != w * h * n {
            return Err(ShimError::Params(format!("{} samples do not fill a {w} x {h} image of {n} channels", data.len())));
        }
        let resolved = Value::Object(self.manifest.resolve_params(params)?);
        let ov = self.manifest.overlap as usize;
        let row_bytes = w * n * 4;
        // Half the plugin's memory is for the band; the module needs room of its own.
        let room = self.limits.max_memory_bytes / 2;
        let in_bytes = (w + 2 * ov) * (1 + 2 * ov) * n * 4;
        if in_bytes > room {
            return Err(ShimError::Limit("memory budget (a band does not fit in the plug-in's memory)".into()));
        }
        let rows = (self.limits.band_bytes.min(room) / row_bytes.max(1)).clamp(1, h.max(1));
        let deadline = self.deadline();
        let mut out = data.to_vec();
        let mut y0 = 0;
        while y0 < h {
            let y1 = (y0 + rows).min(h);
            if let Err(e) = self.run_band(data, &mut out, info, &resolved, (y0, y1), &deadline) {
                deadline.abort(e.clone());
                return Err(e);
            }
            y0 = y1;
        }
        data.copy_from_slice(&out);
        Ok(())
    }

    /// One band: rows `y0..y1` of `src`, written into `dst`.
    fn run_band(&self, src: &[f32], dst: &mut [f32], info: &ImageInfo, params: &Value, rows: (usize, usize), deadline: &Deadline) -> Result<()> {
        let (y0, y1) = rows;
        let n = info.channels as usize;
        let (w, h) = (info.width as usize, info.height as usize);
        let ov = self.manifest.overlap as usize;
        let (iw, ih) = (w + 2 * ov, y1 - y0 + 2 * ov);
        // Read the band with its context, repeating the edge pixels.
        let mut band = vec![0.0f32; iw * ih * n];
        for iy in 0..ih {
            let sy = (y0 + iy).saturating_sub(ov).min(h - 1);
            for ix in 0..iw {
                let sx = ix.saturating_sub(ov).min(w - 1);
                let s = (sy * w + sx) * n;
                let d = (iy * iw + ix) * n;
                if let (Some(to), Some(from)) = (band.get_mut(d..d + n), src.get(s..s + n)) {
                    to.copy_from_slice(from);
                }
            }
        }
        let original = band.clone();
        let mut params = params.clone();
        if let Value::Object(m) = &mut params {
            m.insert(
                "_image".into(),
                json!({
                    "x": -(ov as i64), "y": y0 as i64 - ov as i64, "width": iw, "height": ih, "overlap": ov,
                    "canvasWidth": w, "canvasHeight": h,
                    "mode": mode::name(info.color_mode), "depth": info.depth, "alpha": info.alpha, "channels": n,
                }),
            );
        }
        let pbytes = serde_json::to_vec(&params).map_err(|e| ShimError::Params(e.to_string()))?;
        let format = format_code(info.depth, info.alpha, info.color_mode);
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.filter_buffer(&mut band, iw as u32, ih as u32, info.channels, format, &pbytes, deadline)
        }));
        run.map_err(|_| ShimError::Trap("the runtime panicked".into()))??;
        // Keep the band, drop the margins, sanitise.
        let int_depth = info.depth != 32;
        for row in 0..(y1 - y0) {
            for col in 0..w {
                let si = ((row + ov) * iw + col + ov) * n;
                let di = ((y0 + row) * w + col) * n;
                for ch in 0..n {
                    let o = original.get(si + ch).copied().unwrap_or(0.0);
                    let mut v = band.get(si + ch).copied().unwrap_or(o);
                    if !v.is_finite() {
                        v = o;
                    }
                    if int_depth || (info.alpha && ch == n - 1) {
                        v = v.clamp(0.0, 1.0);
                    }
                    if let Some(d) = dst.get_mut(di + ch) {
                        *d = v;
                    }
                }
            }
        }
        Ok(())
    }

    /// The plugin as a craft-abi 1 manifest. One `filter` contribution, `fresh` instance, `render`
    /// budget, `f32` straight alpha. `overlap` and `area` carry over to `process`.
    pub fn craft_manifest(&self) -> Result<CraftManifest> {
        let m = &self.manifest;
        let area = match m.area {
            Area::Content => "content",
            Area::Canvas => "canvas",
        };
        let doc = json!({
            "schema": 1, "id": m.id, "name": m.name,
            "version": if m.version.is_empty() { "0" } else { m.version.as_str() },
            "description": if m.description.is_empty() { Value::Null } else { json!(m.description) },
            "author": if m.author.is_empty() { Value::Null } else { json!(m.author) },
            "abi": { "craft": 1 },
            "limits": { "memory_mb": self.limits.max_memory_bytes >> 20 },
            "legacy": "photocraft-v1",
            "contributions": [{
                "kind": "filter", "id": "filter", "entry": 0, "label": m.name,
                "instance": "fresh", "budget": "render",
                "process": { "overlap": m.overlap, "band_bytes": self.limits.band_bytes, "sample": ["f32"], "alpha": "straight", "area": area },
                "params": craft_params(&m.params)
            }]
        });
        super::parse_craft_manifest(doc)
    }
}

fn profile(limits: &Limits) -> Profile {
    Profile { max_memory_bytes: limits.max_memory_bytes, strict: true, max_recursion_depth: Some(limits.max_recursion_depth) }
}
