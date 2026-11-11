//! The data plane: pixels through `craft_process`.
//!
//! [`Generation::process_image`] is the one entry an app calls for an `effect` or `filter`
//! contribution. What happens behind it depends on what the generation is.
//!
//! ```text
//! craft-abi 1 module        tile into bands, one fresh instance per band, craft_process
//! PhotoCraft v1 (pc_)       the shim's own banding (apply), f32 samples, straight alpha
//! EffectCraft v1 (ec_)      the shim's pooled instances (render), f32 RGBA, premultiplied
//! VectorCraft v1 (vc_)      not pixels. Use [`Generation::run_objects`].
//! ```
//!
//! A native job, step by step.
//!
//! 1. The host checks the job against the contribution: the sample type and alpha mode must be
//!    ones `process.sample` and `process.alpha` list, and `params` must fit the params schema
//!    ([`crate::params::resolve`]). A wrong job is [`CallError::BadJob`]; no guest code runs and
//!    no undo scope opens.
//! 2. The image is cut into full width strips. A strip is sized so that its padded band is near
//!    `process.band_bytes` (default 4 MiB). The padded band has `process.overlap` extra rows and
//!    columns on every side. Outside the image those are edge pixels repeated, never zeros.
//! 3. Each band gets a fresh instance (one instance per band). The host copies
//!    the padded band and the params in, calls `craft_process` once, and copies the output band out.
//!    Bands run in parallel on native targets. Image bands never use a session instance.
//! 4. The host keeps the inner rectangle of each output band and cleans it (below). Nothing is
//!    written to the caller's buffer until every band succeeded, so a fault leaves the image as it
//!    was. The whole job is one logical call: one `begin_call`, one `end_call`, and one
//!    step toward the circuit.
//!
//! Output hygiene. Where the plugin wrote a value that cannot be a pixel, the host fixes it.
//!
//! - A float sample (f32, f16) that is NaN or infinite takes the input value at the same place.
//! - A float sample whose `depth` is below the sample's width (f32 below 32, f16 below 16) came from
//!   an integer image and is clamped to 0..=1.
//! - The alpha channel of a float image (the last channel, when `alpha` is not `None`) is clamped to 0..=1.
//! - An integer sample above `2^depth - 1` is clamped to that value (for example a 10 bit image in `u16`).
//!
//! Not wired: audio blocks (`audio-effect`), cancel through a v1 shim (a v1 plugin stops on its
//! fuel and wall limits only), and session instances for image bands.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Mutex, atomic::AtomicUsize};

use craft_plugin_abi::ProcessDescriptor;
use craft_plugin_abi::descriptor::{Alpha, Kind as DescKind, Sample};
use craft_plugin_abi::manifest::{AlphaMode, Contribution, Kind, SampleType};
use serde_json::Value;

use crate::engine::Trap;
use crate::invoke::{Backend, CallError, CallOpts, Generation};

/// Band size when neither the job nor the manifest names one.
pub const DEFAULT_BAND_BYTES: u64 = 4 << 20;
/// Most bytes one band may hold, so every size fits the guest's `i32` address space.
const MAX_BAND_BYTES: u64 = 1 << 30;

/// How the bytes of an image are laid out. Rows are tightly packed: `width * channels` samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageFormat {
    pub width: u32,
    pub height: u32,
    pub channels: u32,
    pub sample: Sample,
    pub alpha: Alpha,
    /// The document's color mode number, handed through (PhotoCraft uses PSD numbers).
    pub color_mode: u32,
    /// Bit depth of the source image: 8, 16, 32.
    pub depth: u32,
}

impl ImageFormat {
    pub fn bytes_per_sample(&self) -> usize {
        match self.sample {
            Sample::U8 => 1,
            Sample::U16 | Sample::F16 => 2,
            Sample::F32 => 4,
        }
    }

    pub fn bytes_per_pixel(&self) -> usize {
        self.bytes_per_sample() * self.channels as usize
    }

    pub fn row_bytes(&self) -> usize {
        self.bytes_per_pixel() * self.width as usize
    }

    /// Bytes of the whole image, or `None` when the product overflows.
    pub fn len(&self) -> Option<usize> {
        self.row_bytes().checked_mul(self.height as usize)
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0 || self.channels == 0
    }

    /// Bit width of one sample of this type.
    fn sample_bits(&self) -> u32 {
        (self.bytes_per_sample() * 8) as u32
    }
}

/// An image to process in place.
pub struct ImageJob<'a> {
    pub format: ImageFormat,
    /// Buffer pixels per layer pixel.
    pub scale: f64,
    /// Layer time in seconds.
    pub time: f64,
    /// Document position of pixel (0, 0), handed to plugins that care where they are.
    pub origin: (i32, i32),
    pub data: &'a mut [u8],
}

impl<'a> ImageJob<'a> {
    pub fn new(format: ImageFormat, data: &'a mut [u8]) -> ImageJob<'a> {
        ImageJob { format, scale: 1.0, time: 0.0, origin: (0, 0), data }
    }
}

/// Knobs of one job. The default is what an app wants.
#[derive(Clone, Debug, Default)]
pub struct ProcessOpts {
    /// Band size in bytes. Overrides `process.band_bytes`. Tests set it small to force many bands.
    pub band_bytes: Option<u64>,
    /// Bands that run at once. Default: the number of CPUs. Ignored on `wasm32`.
    pub threads: Option<usize>,
    /// Set from another thread to stop the job. Reaches every band's `host.cancelled` and engine budget.
    pub cancel: Option<Arc<AtomicBool>>,
}

/// What a finished job reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessReport {
    /// Bands the host ran. A v1 plugin counts as one: its shim bands the image itself.
    pub bands: usize,
}

// ---- band geometry -----------------------------------------------------------------------

/// Rows `y0..y0 + rows` of the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Band {
    pub y0: u32,
    pub rows: u32,
}

/// Cut `height` rows into strips whose padded size is near `band_bytes`. Every strip has at least
/// one row. The strips cover the image once, in order.
pub(crate) fn plan_bands(width: u32, height: u32, bytes_per_pixel: usize, overlap: u32, band_bytes: u64) -> Vec<Band> {
    let padded_row = (u64::from(width) + 2 * u64::from(overlap)) * bytes_per_pixel as u64;
    let padded_rows = (band_bytes / padded_row.max(1)).max(1);
    let rows = padded_rows.saturating_sub(2 * u64::from(overlap)).clamp(1, u64::from(height.max(1))) as u32;
    let mut bands = Vec::new();
    let mut y0 = 0u32;
    while y0 < height {
        let n = rows.min(height - y0);
        bands.push(Band { y0, rows: n });
        y0 += n;
    }
    bands
}

/// The input of one band: its rows plus `overlap` rows above and below and `overlap` pixels left
/// and right, with the image's edge pixels repeated where the context leaves the image.
pub(crate) fn pad_band(src: &[u8], f: &ImageFormat, overlap: u32, band: Band) -> Vec<u8> {
    let (bpp, o) = (f.bytes_per_pixel(), overlap as usize);
    let rb = f.row_bytes();
    let padded_rows = band.rows as usize + 2 * o;
    let last_row = f.height as i64 - 1;
    let mut out = Vec::with_capacity((rb + 2 * o * bpp) * padded_rows);
    for r in 0..padded_rows {
        let sy = (i64::from(band.y0) + r as i64 - o as i64).clamp(0, last_row) as usize;
        let row = &src[sy * rb..(sy + 1) * rb];
        let (first, last) = (&row[..bpp], &row[rb - bpp..]);
        for _ in 0..o {
            out.extend_from_slice(first);
        }
        out.extend_from_slice(row);
        for _ in 0..o {
            out.extend_from_slice(last);
        }
    }
    out
}

// ---- output hygiene ----------------------------------------------------------------------

/// Fix one output row. `src` is the input row at the same place.
pub(crate) fn clean_row(dst: &mut [u8], src: &[u8], f: &ImageFormat) {
    let channels = f.channels.max(1) as usize;
    let has_alpha = f.alpha != Alpha::None;
    let integer_origin = f.depth < f.sample_bits();
    // Element `k` of the row is channel `k % channels`.
    let clamp_this = |k: usize| integer_origin || (has_alpha && k % channels == channels - 1);
    match f.sample {
        Sample::U8 | Sample::U16 => {
            if f.depth == 0 || f.depth >= f.sample_bits() {
                return;
            }
            let max = (1u32 << f.depth) - 1;
            if f.sample == Sample::U8 {
                for b in dst.iter_mut() {
                    *b = (u32::from(*b)).min(max) as u8;
                }
            } else {
                for pair in dst.chunks_exact_mut(2) {
                    let v = u32::from(u16::from_le_bytes([pair[0], pair[1]])).min(max) as u16;
                    pair.copy_from_slice(&v.to_le_bytes());
                }
            }
        }
        Sample::F32 => {
            for (k, (d, s)) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)).enumerate() {
                let mut v = f32::from_le_bytes([d[0], d[1], d[2], d[3]]);
                if !v.is_finite() {
                    d.copy_from_slice(s);
                    v = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                }
                if clamp_this(k) && v.is_finite() {
                    d.copy_from_slice(&v.clamp(0.0, 1.0).to_le_bytes());
                }
            }
        }
        Sample::F16 => {
            for (k, (d, s)) in dst.chunks_exact_mut(2).zip(src.chunks_exact(2)).enumerate() {
                let mut bits = u16::from_le_bytes([d[0], d[1]]);
                if bits & 0x7C00 == 0x7C00 {
                    d.copy_from_slice(s);
                    bits = u16::from_le_bytes([s[0], s[1]]);
                }
                if clamp_this(k) && bits & 0x7C00 != 0x7C00 {
                    // Sign bit set: below zero. At or above 0x3C00 (1.0): above one. Both land on a bound.
                    let fixed = if bits & 0x8000 != 0 {
                        0x0000
                    } else if bits > 0x3C00 {
                        0x3C00
                    } else {
                        bits
                    };
                    d.copy_from_slice(&fixed.to_le_bytes());
                }
            }
        }
    }
}

// ---- running bands -----------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Run `f(0..n)` and return the results in order. On native targets up to `threads` bands run at
/// once; the first error stops the bands that have not started and is the one reported.
#[cfg(not(target_arch = "wasm32"))]
fn run_bands<F>(n: usize, threads: usize, f: F) -> Result<Vec<Vec<u8>>, CallError>
where
    F: Fn(usize) -> Result<Vec<u8>, CallError> + Sync,
{
    let threads = threads.clamp(1, n.max(1));
    if threads == 1 {
        return (0..n).map(&f).collect();
    }
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let slots: Vec<Mutex<Option<Vec<u8>>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let first_error: Mutex<Option<CallError>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while !failed.load(Ordering::Relaxed) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= n {
                        break;
                    }
                    match f(i) {
                        Ok(piece) => *lock(&slots[i]) = Some(piece),
                        Err(e) => {
                            failed.store(true, Ordering::Relaxed);
                            lock(&first_error).get_or_insert(e);
                        }
                    }
                }
            });
        }
    });
    if let Some(e) = lock(&first_error).take() {
        return Err(e);
    }
    slots.into_iter().map(|slot| lock(&slot).take().ok_or_else(|| CallError::BadResponse("a band finished without output".into()))).collect()
}

/// `wasm32` has no threads: bands run one after another.
#[cfg(target_arch = "wasm32")]
fn run_bands<F>(n: usize, _threads: usize, f: F) -> Result<Vec<Vec<u8>>, CallError>
where
    F: Fn(usize) -> Result<Vec<u8>, CallError>,
{
    (0..n).map(&f).collect()
}

fn default_threads() -> usize {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::available_parallelism().map_or(1, usize::from)
    }
    #[cfg(target_arch = "wasm32")]
    {
        1
    }
}

// ---- the entry point ---------------------------------------------------------------------

fn sample_type(s: Sample) -> SampleType {
    match s {
        Sample::U8 => SampleType::U8,
        Sample::U16 => SampleType::U16,
        Sample::F16 => SampleType::F16,
        Sample::F32 => SampleType::F32,
    }
}

fn alpha_mode(a: Alpha) -> AlphaMode {
    match a {
        Alpha::None => AlphaMode::None,
        Alpha::Straight => AlphaMode::Straight,
        Alpha::Premultiplied => AlphaMode::Premultiplied,
    }
}

/// The checks of step 1 that need no guest and no params.
fn check_job(c: &Contribution, f: &ImageFormat, data_len: usize) -> Result<(), CallError> {
    let bad = |m: String| Err(CallError::BadJob(m));
    match c.kind {
        Kind::Effect | Kind::Filter => {}
        Kind::ObjectFilter | Kind::LiveEffect => {
            return Err(CallError::NotCallable(format!("kind {} takes vector objects, not pixels: call run_objects", c.kind.name())));
        }
        Kind::AudioEffect => return Err(CallError::NotCallable("audio blocks are not wired in this release".into())),
        ref other => return Err(CallError::NotCallable(format!("kind {} does not process pixels", other.name()))),
    }
    if f.is_empty() {
        return bad(format!("image is {} x {} with {} channels", f.width, f.height, f.channels));
    }
    match f.len() {
        Some(n) if n == data_len => {}
        Some(n) => {
            return bad(format!(
                "a {} x {} image with {} bytes per pixel is {n} bytes, the buffer has {data_len}",
                f.width,
                f.height,
                f.bytes_per_pixel()
            ));
        }
        None => return bad("image size overflows".into()),
    }
    let process = c.process.as_ref();
    if let Some(allowed) = process.and_then(|p| p.sample.as_ref())
        && !allowed.contains(&sample_type(f.sample))
    {
        return bad(format!("{} takes {allowed:?} samples, the image has {:?}", c.id, sample_type(f.sample)));
    }
    if let Some(want) = process.and_then(|p| p.alpha)
        && want != alpha_mode(f.alpha)
    {
        return bad(format!("{} takes {want:?} alpha, the image has {:?}", c.id, alpha_mode(f.alpha)));
    }
    Ok(())
}

impl Generation {
    /// Run an `effect` or `filter` contribution over `job.data`, in place. On any error the buffer
    /// is as it was. See the module comment for the path each kind of generation takes.
    pub fn process_image(&self, c: &Contribution, job: &mut ImageJob<'_>, params: &Value, opts: &ProcessOpts) -> Result<ProcessReport, CallError> {
        check_job(c, &job.format, job.data.len())?;
        match self.backend() {
            Backend::Craft(_) => {
                let resolved = crate::params::resolve(c, params).map_err(|e| CallError::BadJob(e.to_string()))?;
                let params_json = serde_json::to_vec(&resolved).map_err(|e| CallError::BadJob(format!("params do not serialize: {e}")))?;
                self.guarded(c, CallOpts::default(), || self.native_bands(c, job, &params_json, opts))
            }
            #[cfg(feature = "legacy-v1")]
            Backend::Legacy(plugin) => {
                let plan = legacy::plan(plugin, &job.format, params)?;
                self.guarded(c, CallOpts::default(), || legacy::run(plugin, job, &plan).map(|()| ProcessReport { bands: 1 }))
            }
        }
    }

    fn native_bands(&self, c: &Contribution, job: &mut ImageJob<'_>, params_json: &[u8], opts: &ProcessOpts) -> Result<ProcessReport, CallError> {
        let f = job.format;
        let process = c.process.as_ref();
        let overlap = process.and_then(|p| p.overlap).unwrap_or(0);
        let band_bytes = opts
            .band_bytes
            .or(process.and_then(|p| p.band_bytes))
            .unwrap_or(DEFAULT_BAND_BYTES)
            .min(self.limits().memory_bytes / 4)
            .clamp(1, MAX_BAND_BYTES);
        let bands = plan_bands(f.width, f.height, f.bytes_per_pixel(), overlap, band_bytes);
        let cancel = opts.cancel.clone().unwrap_or_default();
        let threads = opts.threads.unwrap_or_else(default_threads);
        let (rb, bpp, o) = (f.row_bytes(), f.bytes_per_pixel(), overlap as usize);
        let padded_w = f.width + 2 * overlap;
        let to_u32 = |n: usize| u32::try_from(n).map_err(|_| CallError::BadJob("band is too large to describe".into()));

        let src: &[u8] = job.data;
        let (scale, time, origin) = (job.scale, job.time, job.origin);
        let pieces = run_bands(bands.len(), threads, |i| {
            let band = bands[i];
            if cancel.load(Ordering::Relaxed) {
                return Err(CallError::Trap(Trap::Cancelled));
            }
            let input = pad_band(src, &f, overlap, band);
            let padded_h = band.rows + 2 * overlap;
            let desc = ProcessDescriptor {
                kind: DescKind::Image,
                entry: c.entry,
                w: padded_w,
                h: padded_h,
                stride_bytes: to_u32(padded_w as usize * bpp)?,
                channels: f.channels,
                sample: f.sample,
                alpha: f.alpha,
                color_mode: f.color_mode,
                depth: f.depth,
                origin_x: origin.0.saturating_sub(overlap as i32),
                origin_y: origin.1.saturating_add(band.y0 as i32).saturating_sub(overlap as i32),
                overlap,
                scale,
                time,
                sample_rate: 0,
                frames: 0,
                in_ptr: 0,
                out_ptr: 0,
                params_ptr: 0,
                params_len: 0,
            };
            let units = u64::from(padded_w) * u64::from(padded_h) * u64::from(f.channels);
            let out = self.run_process_band(c, &desc, &input, params_json, units, Arc::clone(&cancel))?;
            // Keep the inner rectangle, then clean it against the input.
            let padded_rb = padded_w as usize * bpp;
            let mut piece = Vec::with_capacity(rb * band.rows as usize);
            for r in 0..band.rows as usize {
                let start = (r + o) * padded_rb + o * bpp;
                let mut row = out[start..start + rb].to_vec();
                let sy = band.y0 as usize + r;
                clean_row(&mut row, &src[sy * rb..(sy + 1) * rb], &f);
                piece.extend_from_slice(&row);
            }
            Ok(piece)
        })?;
        for (band, piece) in bands.iter().zip(pieces) {
            let at = band.y0 as usize * rb;
            job.data[at..at + piece.len()].copy_from_slice(&piece);
        }
        Ok(ProcessReport { bands: bands.len() })
    }

    /// Run a VectorCraft v1 `object-filter` or `live-effect`: objects JSON in, objects JSON out.
    /// `params` is checked by the v1 rules. A live effect runs under its shorter wall limit.
    #[cfg(feature = "legacy-v1")]
    pub fn run_objects(&self, c: &Contribution, input: &[u8], params: &Value) -> Result<Vec<u8>, CallError> {
        use crate::shim::LegacyPlugin;
        let Some(LegacyPlugin::Vectorcraft(p)) = self.legacy() else {
            return Err(CallError::NotCallable("run_objects needs a VectorCraft v1 plugin".into()));
        };
        if !matches!(c.kind, Kind::ObjectFilter | Kind::LiveEffect) {
            return Err(CallError::NotCallable(format!("kind {} does not take vector objects", c.kind.name())));
        }
        let resolved = p.manifest().resolve_params(params).map_err(legacy::error)?;
        let params_json = serde_json::to_vec(&resolved).map_err(|e| CallError::BadJob(format!("params do not serialize: {e}")))?;
        self.guarded(c, CallOpts::default(), || {
            let run =
                if c.kind == Kind::LiveEffect { p.run_limited(input, &params_json, &p.live_effect_limits()) } else { p.run(input, &params_json) };
            run.map_err(legacy::error)
        })
    }
}

/// The v1 bridges: bytes to the `f32` buffers the shims take, and `ShimError` to `CallError`.
#[cfg(feature = "legacy-v1")]
mod legacy {
    use serde_json::Map;

    use super::{Alpha, CallError, ImageFormat, ImageJob, Sample, Value, sample_type};
    use crate::shim::photocraft::ImageInfo;
    use crate::shim::{LegacyPlugin, ShimError};

    /// A job the shim will take, checked before any undo scope opens.
    pub(super) enum Plan {
        /// PhotoCraft: the params after the v1 rules ran, as the object `pc_filter` gets.
        Photocraft(Map<String, Value>),
        /// EffectCraft: the flat `f64` list `ec_render` gets.
        Effectcraft(Vec<f64>),
    }

    /// The plugin's own error code is the plugin's doing. A bad value is the caller's. Every other
    /// failure is a limit or an ABI break, which counts toward the circuit.
    pub(super) fn error(e: ShimError) -> CallError {
        match e {
            ShimError::Failed(m) => CallError::Plugin(m),
            ShimError::Params(m) => CallError::BadJob(m),
            other => CallError::Legacy(other.to_string()),
        }
    }

    fn need_f32(f: &ImageFormat, who: &str) -> Result<(), CallError> {
        if f.sample != Sample::F32 {
            return Err(CallError::BadJob(format!("{who} works on f32 samples, the image has {:?}", sample_type(f.sample))));
        }
        Ok(())
    }

    fn floats(bytes: &[u8]) -> Vec<f32> {
        bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
    }

    fn store(bytes: &mut [u8], values: impl Iterator<Item = f32>) {
        for (chunk, v) in bytes.chunks_exact_mut(4).zip(values) {
            chunk.copy_from_slice(&v.to_le_bytes());
        }
    }

    /// Everything that can be wrong with the job itself: pixel format and parameters.
    pub(super) fn plan(plugin: &LegacyPlugin, f: &ImageFormat, params: &Value) -> Result<Plan, CallError> {
        match plugin {
            LegacyPlugin::Photocraft(p) => {
                need_f32(f, "a PhotoCraft v1 filter")?;
                p.manifest().resolve_params(params).map(Plan::Photocraft).map_err(error)
            }
            LegacyPlugin::Effectcraft(p) => {
                need_f32(f, "an EffectCraft v1 effect")?;
                if f.channels != 4 {
                    return Err(CallError::BadJob(format!("an EffectCraft v1 effect works on RGBA, the image has {} channels", f.channels)));
                }
                p.manifest().flatten_params(params).map(Plan::Effectcraft).map_err(error)
            }
            LegacyPlugin::Vectorcraft(_) => Err(CallError::NotCallable("a VectorCraft v1 plugin takes vector objects: call run_objects".into())),
        }
    }

    pub(super) fn run(plugin: &LegacyPlugin, job: &mut ImageJob<'_>, plan: &Plan) -> Result<(), CallError> {
        let f = job.format;
        match (plugin, plan) {
            (LegacyPlugin::Photocraft(p), Plan::Photocraft(params)) => {
                let info = ImageInfo {
                    width: f.width,
                    height: f.height,
                    channels: f.channels,
                    depth: f.depth,
                    alpha: f.alpha != Alpha::None,
                    color_mode: f.color_mode,
                };
                let mut samples = floats(job.data);
                p.apply(&mut samples, &info, &Value::Object(params.clone())).map_err(error)?;
                store(job.data, samples.into_iter());
                Ok(())
            }
            (LegacyPlugin::Effectcraft(p), Plan::Effectcraft(flat)) => {
                let mut pixels: Vec<[f32; 4]> = floats(job.data).chunks_exact(4).map(|q| [q[0], q[1], q[2], q[3]]).collect();
                p.render(&mut pixels, f.width, f.height, flat, job.time, job.scale).map_err(error)?;
                store(job.data, pixels.into_iter().flatten());
                Ok(())
            }
            _ => Err(CallError::NotCallable("the plan was made for another kind of plugin".into())),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn fmt(sample: Sample, depth: u32, alpha: Alpha, channels: u32) -> ImageFormat {
        ImageFormat { width: 2, height: 1, channels, sample, alpha, color_mode: 0, depth }
    }

    #[test]
    fn bands_cover_the_image_once_and_in_order() {
        for (h, o, budget) in [(1, 0, 1u64), (10, 0, 7), (10, 2, 1), (37, 3, 400), (37, 0, 1 << 30), (5, 256, 100)] {
            let bands = plan_bands(4, h, 3, o, budget);
            let mut next = 0;
            for b in &bands {
                assert_eq!(b.y0, next, "h {h} overlap {o} budget {budget}");
                assert!(b.rows >= 1);
                next += b.rows;
            }
            assert_eq!(next, h);
        }
    }

    #[test]
    fn a_band_holds_about_the_asked_bytes_with_its_padding() {
        // 10 wide, 1 byte per pixel, overlap 1: padded rows are 12 bytes. 120 bytes is 10 padded rows, so 8 inner rows.
        let bands = plan_bands(10, 100, 1, 1, 120);
        assert_eq!(bands[0].rows, 8);
        assert_eq!(bands.len(), 13);
    }

    #[test]
    fn padding_repeats_the_edge_pixels_and_never_invents_zeros() {
        // 3 x 2 image, one byte per pixel. Rows: 1 2 3 / 4 5 6.
        let f = ImageFormat { width: 3, height: 2, channels: 1, sample: Sample::U8, alpha: Alpha::None, color_mode: 0, depth: 8 };
        let src = [1, 2, 3, 4, 5, 6];
        let all = pad_band(&src, &f, 1, Band { y0: 0, rows: 2 });
        assert_eq!(all, vec![1, 1, 2, 3, 3, 1, 1, 2, 3, 3, 4, 4, 5, 6, 6, 4, 4, 5, 6, 6]);
        // The second row alone: context above is row 0, context below is row 1 again.
        let low = pad_band(&src, &f, 1, Band { y0: 1, rows: 1 });
        assert_eq!(low, vec![1, 1, 2, 3, 3, 4, 4, 5, 6, 6, 4, 4, 5, 6, 6]);
    }

    #[test]
    fn float_nan_and_infinity_take_the_input_value() {
        let f = fmt(Sample::F32, 32, Alpha::None, 1);
        let src: Vec<u8> = [0.25f32, 0.5].iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut dst: Vec<u8> = [f32::NAN, f32::INFINITY].iter().flat_map(|v| v.to_le_bytes()).collect();
        clean_row(&mut dst, &src, &f);
        assert_eq!(dst, src);
    }

    #[test]
    fn integer_origin_floats_and_alpha_are_clamped_to_unit_range() {
        // depth 8 in f32: every channel clamps.
        let f = fmt(Sample::F32, 8, Alpha::None, 1);
        let src = vec![0u8; 8];
        let mut dst: Vec<u8> = [1.5f32, -0.5].iter().flat_map(|v| v.to_le_bytes()).collect();
        clean_row(&mut dst, &src, &f);
        assert_eq!(dst, [1.0f32, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
        // depth 32 with alpha: color may exceed 1 (HDR), alpha may not.
        let f = ImageFormat { channels: 2, ..fmt(Sample::F32, 32, Alpha::Straight, 2) };
        let mut dst: Vec<u8> = [4.0f32, 3.0].iter().flat_map(|v| v.to_le_bytes()).collect();
        clean_row(&mut dst, &[0u8; 8], &f);
        assert_eq!(dst, [4.0f32, 1.0].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
    }

    #[test]
    fn half_floats_are_fixed_by_their_bits() {
        // 0x7E00 NaN, 0x7C00 infinity, 0xBC00 -1.0, 0x4000 2.0, 0x3800 0.5. depth 8 so they clamp.
        let f = ImageFormat { width: 5, ..fmt(Sample::F16, 8, Alpha::None, 1) };
        let src: Vec<u8> = [0x3400u16; 5].iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut dst: Vec<u8> = [0x7E00u16, 0x7C00, 0xBC00, 0x4000, 0x3800].iter().flat_map(|v| v.to_le_bytes()).collect();
        clean_row(&mut dst, &src, &f);
        let got: Vec<u16> = dst.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(got, vec![0x3400, 0x3400, 0x0000, 0x3C00, 0x3800]);
    }

    #[test]
    fn integers_above_the_declared_depth_are_clamped() {
        let f = fmt(Sample::U16, 10, Alpha::None, 1);
        let mut dst: Vec<u8> = [1023u16, 1024].iter().flat_map(|v| v.to_le_bytes()).collect();
        clean_row(&mut dst, &[0u8; 4], &f);
        let got: Vec<u16> = dst.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(got, vec![1023, 1023]);
        // A full width sample is left alone.
        let f = fmt(Sample::U8, 8, Alpha::None, 1);
        let mut dst = vec![255u8, 7];
        clean_row(&mut dst, &[0u8; 2], &f);
        assert_eq!(dst, vec![255, 7]);
    }
}
