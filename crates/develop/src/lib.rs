//! The raw develop pipeline, CPU path (LightCraft's `lightcraft-pipeline`, with the raw develop half, colour,
//! raster and inpainting modules it builds on).
//!
//! Input: a scene-referred, linear Rec.2020 source image (already EXIF-oriented) at any resolution
//! (full size or a proxy), plus [`DevelopSettings`]. Output: a display-encoded sRGB image at the
//! requested size, and its histogram.
//!
//! ```no_run
//! use craft_develop::load::{LoadOptions, load_raw};
//! use craft_develop::{OutputDepth, OutputSpace, RenderRequest, render};
//! use craft_develop_model::DevelopSettings;
//! # fn main() -> Result<(), craft_develop::load::LoadError> {
//! # let bytes: Vec<u8> = Vec::new();
//! // raw bytes → linear Rec.2020, upright, plus what the settings are read against
//! let source = load_raw(&bytes, &LoadOptions::full())?;
//! let settings = DevelopSettings::for_raw(source.info.as_shot_temp, source.info.as_shot_tint);
//! // 16-bit ProPhoto RGB, fitted to 4000 × 4000
//! let req = RenderRequest { space: OutputSpace::ProPhoto, depth: OutputDepth::U16, ..RenderRequest::fit(4000, 4000) };
//! let out = render(&source.image, &source.info, &settings, &req);
//! let deep = out.deep; // Some(DeepImage { samples: DeepSamples::U16(..), .. })
//! # let _ = deep; Ok(()) }
//! ```
//!
//! Raw sources come from [`load`]: [`load::load_raw`] (bytes) or [`load::develop_raw`] (a decoded
//! [`craft_raw::RawImage`]), with the sensor half of the develop in [`sensor`] and the starting look of raws without
//! colour matrices in [`look`]. A picture decoded some other way is rendered with [`SourceInfo::default`].
//!
//! Stage order (LightCraft's `docs/pipeline.md`):
//! 1. geometry — user orientation, lens corrections (distortion, CA, vignetting), perspective, crop +
//!    straighten, flips; one resample at output resolution; then defringe
//! 2. scene-linear — white balance, exposure, dehaze, local tone (highlights/shadows), texture,
//!    clarity, local adjustments (masks)
//! 3. tone map — contrast / whites / blacks filmic curve on luminance, highlight desaturation
//! 4. colour — vibrance, saturation, colour mixer, colour grading, B&W (OkLCh)
//! 5. display — gamut map to the output space (sRGB unless [`RenderRequest::space`] says otherwise), encode, tone curves (parametric + point), vignette, grain
//!
//! Spatial parameters are specified relative to the image's long edge, so a 400 px preview and a
//! 60 MP export look alike.
//!
//! Exposure is a gain, so the spatial stages run on the un-exposed image and the per-pixel stage
//! applies it (filters on log luminance are shift-equivariant: identical result). With
//! [`render_cached`] each stage's output is reused while its inputs are unchanged ([`StageCache`]):
//! dragging a tone, colour or exposure slider re-runs only the per-pixel stage.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod auto;
pub mod blemish;
pub mod color;
pub mod colorops;
pub mod crystal;
pub mod cull;
pub mod curve;
pub mod dust;
pub mod finish;
pub mod geometry;
pub mod inpaint;
pub mod lens_blur;
pub mod light_v3;
pub mod preview;
pub mod raster;
#[cfg(test)]
mod scenes;
pub mod sensor;
// generated (`light_v3_params.rs`), kept in its own compact layout
#[rustfmt::skip]
pub mod light_v3_params;
pub mod load;
pub mod local;
pub mod local_tone;
pub mod look;
pub mod lut;
pub mod mask_space;
pub mod masks;
pub mod nrc_joint;
#[cfg(test)]
mod nrc_joint_reference;
pub mod optics;
pub mod output;
pub mod profiles;
pub mod redeye;
pub mod spots;
pub mod straighten;
pub mod stroke_raster;
pub mod tone;
pub mod transform;
pub mod upright;
pub mod visualize;

pub use curve::{PARAM_AMOUNTS, PARAM_LUT, ParametricCurve};
pub use output::{DeepImage, DeepSamples, DisplaySpace, OutputDepth, OutputSpace, OutputTrc, Proof, ProofLut, proof_profile, register_proof_profile};
pub use visualize::{MaskView, Overlay};

use craft_develop_model::{DevelopSettings, Treatment};
use std::any::Any;
use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use crate::raster::{Histogram, Plane, Rgb32f, Rgba8, par_rows};

pub use tone::ToneMap;

/// A raw source's own colour model, for white balance: like Lightroom, a white balance
/// re-evaluates the camera's colour matrices at the chosen white (camera-space white balance, DNG
/// spec ch. 6) instead of adapting the as-shot rendering ([`local::wb_matrix_for`]).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CameraColor {
    /// The file's colour tags (its profile look left out: it is applied at load).
    pub tags: crate::sensor::ColorData,
    /// The white the source pixels were developed for.
    pub developed_for: crate::color::Xy,
}

/// Facts about the source the settings are interpreted against.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceInfo {
    /// Lens corrections embedded in the file (DNG opcodes), relative to the EXIF-oriented source.
    pub lens: Option<craft_develop_model::EmbeddedLens>,
    /// Raw sources use absolute Kelvin white balance; rendered sources use a relative scale around
    /// their as-shot white.
    pub raw: bool,
    pub as_shot_temp: f64,
    pub as_shot_tint: f64,
    /// No measured camera illuminant: WB adjustments are relative to the camera's rendered look.
    pub relative_wb: bool,
    /// The Kelvin scale comes from matrices measured for the camera model, not from the file: before those were
    /// used for white balance, this source was edited on the relative scale (6500 K / 0 = as shot), so settings
    /// without a [`craft_develop_model::WbScale`] are read that way ([`local::legacy_to_kelvin`]).
    pub legacy_relative_wb: bool,
    /// The camera's colour model (raw files with a colour matrix); `None`: white balance adapts
    /// the developed pixels (Bradford, linear Rec.2020).
    pub camera_color: Option<Arc<CameraColor>>,
    pub camera_tone: Option<tone::CameraTone>,
    /// Segmentation mattes stored in the file (DNG semantic masks): AI masks use them.
    pub mattes: Option<Arc<masks::Mattes>>,
    /// The raw's own local tone mapping (DNG `ProfileGainTableMap`), rendered only when the
    /// photo's Profile option asks for it ([`local_tone`]).
    pub local_tone: Option<Arc<local_tone::LocalTone>>,
    /// Long edge (px) of the original photo, when the source handed to the renderer is a smaller
    /// copy of it (the app's thumbnail and preview levels); `None`: the source is the original.
    /// Noise reduction sizes its radii in the original's pixels ([`Plan::detail_scale`]), so a
    /// preview looks like the full-size render scaled down (issue storytold/lightcraft#960).
    pub native_long: Option<usize>,
}

impl Default for SourceInfo {
    fn default() -> Self {
        Self {
            raw: false,
            as_shot_temp: 6500.0,
            as_shot_tint: 0.0,
            lens: None,
            relative_wb: false,
            legacy_relative_wb: false,
            camera_color: None,
            camera_tone: None,
            mattes: None,
            local_tone: None,
            native_long: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Quality {
    /// Interactive drags: skips the most expensive refinements.
    Draft,
    #[default]
    Full,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderRequest {
    /// Output must fit in `max_w × max_h` (aspect preserved).
    pub max_w: usize,
    pub max_h: usize,
    /// The box is the final export size, already fitted against the full-resolution photo.
    /// Reduced decode proxies must not fit it a second time.
    pub exact_size: bool,
    pub quality: Quality,
    /// Show the crop frame's content only (true) or the whole uncropped image (crop tool active).
    pub apply_crop: bool,
    /// A diagnostic view drawn over the result (e.g. Point Color's "visualize range").
    pub overlay: Overlay,
    /// Colour space of the result (exports; previews stay sRGB).
    pub space: OutputSpace,
    /// Sample format: 8-bit only, or also a 16-bit / linear float [`Rendered::deep`] (exports).
    pub depth: OutputDepth,
    /// Soft proofing (a proof against an output-device profile renders on the CPU; see [`Proof`]).
    pub proof: Option<Proof>,
    /// Render only this window of the output that `max_w × max_h` describes (a zoomed view):
    /// the result is that window's pixels, as in the whole render. See [`PixelWindow`].
    pub window: Option<PixelWindow>,
    /// Previews for a monitor with a display profile: render 8-bit into its primaries instead of
    /// `space` (see [`DisplaySpace`]; ignored by deep renders).
    pub display: Option<DisplaySpace>,
}

/// A window of the (virtual) full output, in its pixels: what a zoomed view needs, rendered
/// without materializing the whole frame. The output frame is the one `max_w × max_h` fits, after
/// crop, straighten and flips, as drawn. Out-of-range windows are clamped into it. Spatial stages
/// (clarity, dehaze, sharpening…) read their neighbours, so ask for a margin around what is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelWindow {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl PixelWindow {
    /// This window inside a `full_w × full_h` frame: at least one pixel, wholly inside.
    pub fn clamped(self, full_w: usize, full_h: usize) -> PixelWindow {
        let (fw, fh) = (full_w.max(1), full_h.max(1));
        let x = self.x.min(fw - 1);
        let y = self.y.min(fh - 1);
        PixelWindow { x, y, w: self.w.clamp(1, fw - x), h: self.h.clamp(1, fh - y) }
    }
}

/// The longest output side a render may be asked for (TIFF/PNG/JPEG all fit within it).
pub const MAX_OUTPUT_SIDE: usize = 65_535;
/// The most output pixels a render may be asked for (the box it fits into, ≈ 1.07 Gpx).
pub const MAX_OUTPUT_PIXELS: usize = 1 << 30;

/// Rejects an output box past [`MAX_OUTPUT_SIDE`] or [`MAX_OUTPUT_PIXELS`] before anything is
/// sized from it.
pub fn check_output_size(max_w: usize, max_h: usize) -> Result<(), String> {
    let pixels = max_w.checked_mul(max_h);
    if max_w > MAX_OUTPUT_SIDE || max_h > MAX_OUTPUT_SIDE || pixels.is_none_or(|n| n > MAX_OUTPUT_PIXELS) {
        return Err(format!("output size {max_w}×{max_h} is too large (at most {MAX_OUTPUT_SIDE} px per side and {MAX_OUTPUT_PIXELS} pixels)"));
    }
    Ok(())
}

/// Fails when a visible brush mask holds more dabs than its budget ([`masks::MAX_STROKE_DABS`]) at this render's
/// output size: the render would otherwise skip its later strokes and show a wrong mask.
pub fn check_brush_budget(src: &Rgb32f, info: &SourceInfo, s: &DevelopSettings, req: &RenderRequest) -> Result<(), String> {
    let brush =
        |m: &craft_develop_model::Mask| m.visible && m.components.iter().any(|c| matches!(c.shape, craft_develop_model::MaskShape::Brush { .. }));
    if !s.masks.iter().any(brush) {
        return Ok(());
    }
    let p = plan(src, info, s, req);
    for m in s.masks.iter().filter(|m| brush(m)) {
        for c in &m.components {
            if let craft_develop_model::MaskShape::Brush { strokes } = &c.shape
                && masks::brush_over_budget(strokes, &p.frame.for_mask(m.on_photo), p.w, p.h)
            {
                return Err(format!("mask \"{}\": the brush strokes hold more than {} dabs at this size", m.name, masks::MAX_STROKE_DABS));
            }
        }
    }
    Ok(())
}

impl RenderRequest {
    pub fn fit(max_w: usize, max_h: usize) -> Self {
        Self {
            max_w,
            max_h,
            exact_size: false,
            quality: Quality::Full,
            apply_crop: true,
            overlay: Overlay::None,
            space: OutputSpace::Srgb,
            depth: OutputDepth::U8,
            proof: None,
            window: None,
            display: None,
        }
    }

    /// Render to a final size calculated from the original photo, even from a rounded proxy.
    pub fn exact(width: usize, height: usize) -> Self {
        Self { exact_size: true, ..Self::fit(width, height) }
    }

    /// The output size for `frame`. An exact box stands against a proxy that only rounds the photo's
    /// shape differently; a source of another shape (a raw's embedded preview) is fitted, not stretched.
    fn size(&self, frame: &geometry::Frame) -> (usize, usize) {
        let fitted = frame.fit(self.max_w, self.max_h);
        let exact = (self.max_w.max(1), self.max_h.max(1));
        let near = |a: usize, b: usize| a.abs_diff(b) <= b / 64 + 1;
        if self.exact_size && near(fitted.0, exact.0) && near(fitted.1, exact.1) { exact } else { fitted }
    }
}

pub struct Rendered {
    /// The result, 8-bit (for deep renders: `deep` reduced to 8 bits, display-encoded).
    pub image: Rgba8,
    pub histogram: Histogram,
    /// The high-bit-depth result when [`RenderRequest::depth`] asks for one.
    pub deep: Option<DeepImage>,
}

/// Everything the per-pixel stage needs, precomputed at output resolution.
///
/// The image and planes are computed *before exposure* (so they can be reused while exposure is
/// dragged): the per-pixel stage multiplies the image by `gain` and shifts log planes by `ev`.
/// Spatial filters on log luminance are shift-equivariant, so this equals filtering after exposure.
pub(crate) struct Prepared {
    pub img: Arc<Rgb32f>,
    /// Log2 luminance relative to middle grey (before exposure: add `ev`).
    pub log_l: Arc<Plane>,
    pub base: Arc<Plane>,
    pub clarity_blur: Option<Arc<Plane>>,
    pub texture_blur: Option<Arc<Plane>>,
    /// Log luminance blurred at the sharpen radius (the pixel-scale band sharpening boosts).
    pub sharpen_blur: Option<Arc<Plane>>,
    pub dark: Option<Arc<Plane>>,
    /// Blurred chromaticity (`rgb / Y`) for local Moiré / Noise.
    pub chroma_blur: Option<Arc<Rgb32f>>,
    /// Airlight of `dark` (before exposure).
    pub air: f32,
    pub masks: Vec<masks::Evaluated>,
    /// Output pixels per unit of the source long edge.
    pub px_per_long: f64,
}

/// Output size for a source of `src_w × src_h` under `s`, fitting `max_w × max_h`.
pub fn output_size(src_w: usize, src_h: usize, s: &DevelopSettings, req: &RenderRequest) -> (usize, usize) {
    req.size(&geometry::Frame::new(src_w, src_h, s, req.apply_crop))
}

/// Size of the cropped output of a `src_w × src_h` source under `s` at the source's own resolution
/// (what a full-size export produces).
pub fn native_output_size(src_w: usize, src_h: usize, s: &DevelopSettings) -> (f64, f64) {
    geometry::Frame::new(src_w, src_h, s, true).native_size()
}

/// The geometric frame `render` uses for `src` (lens data, automatic CA estimate included).
pub fn frame_for(src: &Rgb32f, info: &SourceInfo, s: &DevelopSettings, apply_crop: bool) -> geometry::Frame {
    let mut frame = geometry::Frame::with_lens(src.width, src.height, s, apply_crop, info.lens.as_ref());
    if s.optics.remove_ca && s.section_enabled("optics") {
        frame.add_lateral_ca(optics::estimate_lateral_ca(src));
        if apply_crop {
            // The estimated CA moves the colour planes too: keep the crop inside every one of them.
            frame.constrain_to_image(s.geometry.constrain_crop);
        }
    }
    frame
}

/// Intermediate results of recent renders of one view, reused by [`render_cached`].
///
/// Each stage is keyed by exactly the inputs it depends on: the resampled source by the source
/// buffer, framing and output size; the white-balanced, retouched, denoised image by those plus
/// white balance, spots and noise reduction; each spatial plane by that plus its own radius. So a
/// tone or colour slider drag (or exposure) re-runs only the per-pixel stage, a clarity drag skips
/// resampling and noise reduction, and so on. Holds the last few output sizes (a draft-size and a
/// full-size render of the same view both stay warm).
pub struct StageCache {
    entries: Mutex<Vec<CacheEntry>>,
    capacity: usize,
    /// Another renderer's per-view state (the GPU renderer keeps its device-resident stages here).
    ext: Mutex<Option<Arc<dyn Any + Send + Sync>>>,
    /// Brush strokes rasterized by recent renders ([`stroke_raster::BrushCache`]): a stroke that
    /// grew since (the next pointer move while it is painted) stamps only its new dabs.
    brush: stroke_raster::BrushCache,
}

impl Default for StageCache {
    fn default() -> Self {
        StageCache { entries: Mutex::new(Vec::new()), capacity: 2, ext: Mutex::new(None), brush: Default::default() }
    }
}

impl StageCache {
    /// Drop everything (e.g. when memory is needed).
    pub fn clear(&self) {
        self.lock().clear();
        self.brush.clear();
        *self.ext.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Per-view state of type `T` kept alongside this cache (created on first use). Lets another
    /// renderer of the same view (e.g. `lightcraft-gpu`) keep its own stage cache with this one.
    pub fn extension<T: Any + Send + Sync + Default>(&self) -> Arc<T> {
        let mut g = self.ext.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = g.as_ref()
            && let Ok(t) = e.clone().downcast::<T>()
        {
            return t;
        }
        let t = Arc::new(T::default());
        *g = Some(t.clone());
        t
    }

    /// The per-view state of type `T` if there is one (unlike [`Self::extension`], never creates it).
    pub fn peek_extension<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.ext.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|e| e.clone().downcast::<T>().ok())
    }

    /// Bytes of the intermediate images held (each buffer counted once; the sources they were
    /// made from belong to their owner and are not counted).
    pub fn bytes(&self) -> usize {
        fn size<T>(i: &crate::raster::Image<T>) -> usize {
            i.data.len() * std::mem::size_of::<T>()
        }
        let mut seen: Vec<usize> = Vec::new();
        let mut total = 0;
        let mut add = |p: usize, n: usize| {
            if !seen.contains(&p) {
                seen.push(p);
                total += n;
            }
        };
        for e in self.lock().iter() {
            add(Arc::as_ptr(&e.sampled) as usize, size(&e.sampled));
            if let Some((_, l)) = &e.lin {
                add(Arc::as_ptr(l) as usize, size(l));
            }
            let pl = &e.planes;
            let planes = pl.log_l.iter().chain(pl.base.iter().map(|x| &x.1)).chain(pl.clarity.iter().map(|x| &x.1));
            for p in planes.chain(pl.texture.iter().map(|x| &x.1)).chain(pl.sharpen.iter().map(|x| &x.1)).chain(pl.dark.iter().map(|x| &x.1)) {
                add(Arc::as_ptr(p) as usize, size(p));
            }
        }
        total
    }

    /// Number of cached output sizes.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<CacheEntry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn get(&self, src: &Arc<Rgb32f>, geo: u64) -> Option<CacheEntry> {
        self.lock().iter().find(|e| e.geo == geo && Arc::ptr_eq(&e.src, src)).cloned()
    }

    fn put(&self, e: CacheEntry) {
        let mut v = self.lock();
        v.retain(|o| !(o.geo == e.geo && Arc::ptr_eq(&o.src, &e.src)));
        v.push(e);
        while v.len() > self.capacity {
            v.remove(0);
        }
    }
}

#[derive(Clone)]
struct CacheEntry {
    src: Arc<Rgb32f>,
    geo: u64,
    sampled: Arc<Rgb32f>,
    lin: Option<(u64, Arc<Rgb32f>)>,
    /// The joint colour NR's noise model of the whole frame ([`colour_noise`]), keyed by [`Plan::noise_key`].
    colour_noise: Option<(u64, nrc_joint::JointNoise)>,
    planes: local::Planes,
}

fn hash_of(parts: impl std::hash::Hash) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
    BuildHasherDefault::<DefaultHasher>::default().hash_one(parts)
}

/// What a render resolves to before any pixel work: the effective settings (profile and Upright
/// applied), the geometric frame, the output size and the stage-cache keys. Shared by the CPU
/// renderer and the GPU renderer (`lightcraft-gpu`), so both key their caches identically.
pub struct Plan<'a> {
    pub settings: Cow<'a, DevelopSettings>,
    pub frame: geometry::Frame,
    pub w: usize,
    pub h: usize,
    /// Output pixels per unit of the oriented source's long edge.
    pub px_per_long: f64,
    /// Long edge of the source buffer (px).
    pub src_long: usize,
    /// Long edge of the original photo, shared by sharpening and noise reduction.
    pub native_long: usize,
    /// Output pixels per pixel of the original photo ([`SourceInfo::native_long`], else the source
    /// buffer), at most 1: the scale of noise reduction, whose radii are in the original's pixels.
    pub detail_scale: f32,
    /// Key of the resampled source (together with the source buffer's identity).
    pub geo: u64,
    /// Key of the white-balanced, retouched, denoised image (and of the planes computed from it).
    pub lin_key: u64,
    /// [`Self::lin_key`] without the noise-reduction settings: draft renders reuse the smooth
    /// planes of an image that differs from this one only in its noise reduction.
    pub pre_nr_key: u64,
    /// Key of the whole frame's noise model for the joint colour NR ([`colour_noise`]): what it is
    /// measured after (everything before the colour NR), not the colour NR's own settings.
    pub noise_key: u64,
    /// Red eye / pet eye corrections with their detected pupils, in output pixels.
    pub eyes: Vec<redeye::EyeK>,
    /// The whole frame's dehaze airlight, for a windowed render (which can't see the whole frame).
    pub fixed_air: Option<f32>,
    /// A windowed render works on a larger window when spots reach into it (see
    /// [`spots::window_for_reads`]); this is the requested window inside the rendered one, which
    /// the result is cut to.
    pub keep: Option<PixelWindow>,
    /// The source's segmentation mattes ([`SourceInfo::mattes`]).
    pub mattes: Option<Arc<masks::Mattes>>,
    /// The whole photo's subject estimate a lens blur focuses on ([`lens_blur::subject_estimate`]).
    pub lens_subject: Option<Arc<lens_blur::SubjectEstimate>>,
    /// The whole frame and its output size (a windowed render's [`Self::frame`] is part of it).
    pub full: (geometry::Frame, usize, usize),
}

/// Long edge of the small render a windowed render estimates the whole frame's airlight from.
const AIRLIGHT_PROXY_EDGE: usize = 384;

/// Resolve `s` against `src` for `req` (see [`Plan`]).
pub fn plan<'a>(src: &Rgb32f, info: &SourceInfo, s: &'a DevelopSettings, req: &RenderRequest) -> Plan<'a> {
    let mut settings: Cow<'a, DevelopSettings> = match profiles::effective(s) {
        Cow::Borrowed(b) => upright::resolve(src, info, b),
        Cow::Owned(o) => Cow::Owned(upright::resolve(src, info, &o).into_owned()),
    };
    visualize::adjust_settings(req.overlay, &mut settings);
    // a window must not choose a spot's source from the pixels it happens to hold: fix them
    // from a fixed-size render of the whole frame first
    if req.window.is_some() && settings.spots.iter().any(|sp| sp.source_offset.is_none()) {
        let picked: Vec<Option<craft_geom::Point>> =
            settings.spots.iter().map(|sp| sp.source_offset.or_else(|| spots::pick_source(src, info, &settings, sp, None))).collect();
        for (sp, o) in settings.to_mut().spots.iter_mut().zip(picked) {
            sp.source_offset = o;
        }
    }
    let s = &*settings;
    let frame = frame_for(src, info, s, req.apply_crop);
    let (full_w, full_h) = req.size(&frame);
    // sizes and scale are those of the whole output; a window only narrows what is drawn
    let px_per_long = frame.px_per_long(full_w);
    let src_long = src.width.max(src.height);
    // (an original reported smaller than the source is the source)
    let native_long = info.native_long.unwrap_or(0).max(src_long).max(1);
    let detail_scale = local::detail_scale(px_per_long, native_long);
    let mut fixed_air = None;
    let mut keep = None;
    let full = (frame.clone(), full_w, full_h);
    let (frame, w, h) = match req.window {
        Some(win) => {
            let win = win.clamped(full_w, full_h);
            if s.section_enabled("effects") && (s.effects.dehaze != 0.0 || s.masks.iter().any(|m| m.adjust.dehaze != 0.0)) {
                let k = (AIRLIGHT_PROXY_EDGE as f64 / full_w.max(full_h) as f64).min(1.0);
                let (pw, ph) = (((full_w as f64 * k).round() as usize).max(1), ((full_h as f64 * k).round() as usize).max(1));
                fixed_air = Some(local::frame_airlight(src, info, s, &frame, pw, ph));
            }
            // (spots grow the rendered window of 8-bit renders, the lens blur that of every
            // render; a grown window is cut back to the request)
            let work = if req.depth == OutputDepth::U8 { spots::window_for_reads(s, &frame, full_w, full_h, px_per_long, win) } else { win };
            let work = lens_blur::window_for_reads(s, full_w, full_h, px_per_long, work);
            let mut work = stroke_raster::window_for_reads(s, full_w, full_h, px_per_long, work);
            if local::joint_colour_nr(s)
                && let Some(params) = local::nrc_params(s, detail_scale)
            {
                // The joint diffusion reads a chain of neighbours across its scales. Keep
                // those reads outside the requested zoom window, just as spot reads are, at every depth.
                let halo = params.passes.last().map_or(0, |p| (6.0 * p.sigma).ceil() as usize).saturating_add(4);
                let x = work.x.saturating_sub(halo);
                let y = work.y.saturating_sub(halo);
                let right = work.x.saturating_add(work.w).saturating_add(halo).min(full_w);
                let bottom = work.y.saturating_add(work.h).saturating_add(halo).min(full_h);
                work = PixelWindow { x, y, w: right.saturating_sub(x), h: bottom.saturating_sub(y) }.clamped(full_w, full_h);
            }
            if work != win {
                keep = Some(PixelWindow { x: win.x - work.x, y: win.y - work.y, ..win });
            }
            (frame.window(full_w, full_h, work), work.w, work.h)
        }
        None => (frame, full_w, full_h),
    };
    let geo = hash_of((format!("{frame:?}"), w, h));
    let (wb_t, wb_tint) = local::effective_wb(info, s);
    let eyes = redeye::resolve(src, &s.red_eye, s.orientation, &frame, w, h, px_per_long);
    let d = &s.detail;
    let pre_nr_key = hash_of((
        geo,
        [wb_t, wb_tint, info.as_shot_temp, info.as_shot_tint].map(f64::to_bits),
        format!("{:?}", s.spots),
        format!("{eyes:?}"),
        // defringe runs in this stage
        format!("{:?}", s.optics),
        // and the local tone mapping
        local_tone::enabled(info, s),
        src_long,
        // and the lens blur (after noise reduction, but a draft must not reuse planes without it)
        lens_blur::key(s),
    ));
    let lin_key = hash_of((
        pre_nr_key,
        [d.nr_luminance, d.nr_detail, d.nr_color, d.nr_color_detail, d.nr_color_smoothness].map(f64::to_bits),
        detail_scale.to_bits(),
        // the process picks the colour filter (Update to Current Process, undo)
        local::joint_colour_nr(s),
    ));
    let lens_subject = lens_blur::subject_estimate(src, s, info.mattes.as_deref()).map(Arc::new);
    let noise_key = hash_of((pre_nr_key, [d.nr_luminance, d.nr_detail].map(f64::to_bits), detail_scale.to_bits()));
    Plan {
        settings,
        frame,
        w,
        h,
        px_per_long,
        src_long,
        native_long,
        detail_scale,
        geo,
        lin_key,
        pre_nr_key,
        noise_key,
        eyes,
        fixed_air,
        keep,
        mattes: info.mattes.clone(),
        lens_subject,
        full,
    }
}

/// The joint colour NR's noise model of the whole frame, measured on fixed tiles of it
/// ([`nrc_joint::tiles`]) so that every window of it is filtered alike. The tiles are resampled,
/// tone-mapped, white-balanced and luminance-denoised like the frame; the retouching and defringing
/// are left out. `None` when the joint colour NR is off, or when the render is the whole frame and
/// that is a single tile: the noise is then measured on the image itself, as the CPU and the GPU do.
pub fn colour_noise(src: &Rgb32f, info: &SourceInfo, p: &Plan<'_>) -> Option<nrc_joint::JointNoise> {
    let s = &*p.settings;
    if !local::joint_colour_nr(s) || local::nrc_params(s, p.detail_scale).is_none() {
        return None;
    }
    let (full, fw, fh) = &p.full;
    let windows = nrc_joint::tiles(*fw, *fh);
    if (p.w, p.h) == (*fw, *fh) && windows.len() == 1 {
        return None;
    }
    let t = profiling().then(std::time::Instant::now);
    let (lum, _) = local::nr_params(s, p.detail_scale);
    // the source oriented and prefiltered once for all tiles
    let sampled = full.sample_windows(src, *fw, *fh, &windows);
    let tiles: Vec<Rgb32f> = windows
        .into_iter()
        .zip(sampled)
        .map(|(t, mut img)| {
            let frame = if (t.w, t.h) == (*fw, *fh) { full.clone() } else { full.window(*fw, *fh, t) };
            let tile = Plan {
                settings: Cow::Borrowed(s),
                frame,
                w: t.w,
                h: t.h,
                eyes: Vec::new(),
                fixed_air: None,
                keep: None,
                mattes: None,
                lens_subject: None,
                full: p.full.clone(),
                ..*p
            };
            local_tone::apply(&mut img, info, &tile);
            local::white_balance(&mut img, info, s);
            local::apply_nr(&mut img, lum, None);
            img
        })
        .collect();
    let noise = nrc_joint::noise_of_tiles(&tiles);
    if let Some(t) = t {
        eprintln!("    nr colour noise model ({} tiles): {:.0} ms", tiles.len(), t.elapsed().as_secs_f64() * 1e3);
    }
    Some(noise)
}

/// Whether the scene-linear stage needs work only the CPU does (defringe, spot removal).
pub fn lin_needs_cpu(s: &DevelopSettings, info: &SourceInfo) -> bool {
    let o = &s.optics;
    let defringe = s.section_enabled("optics") && (o.defringe_purple_amount > 0.0 || o.defringe_green_amount > 0.0);
    defringe || !s.spots.is_empty() || local_tone::enabled(info, s)
}

/// The white-balanced, defringed, retouched image (before noise reduction): the CPU part of the
/// scene-linear stage, in place.
pub fn lin_cpu(img: &mut Rgb32f, info: &SourceInfo, p: &Plan<'_>) {
    let s = &*p.settings;
    // (before the white balance: the gain's input is the as-shot development, as in the file)
    local_tone::apply(img, info, p);
    local::white_balance(img, info, s);
    optics::defringe(img, s, p.px_per_long / optics::DEFRINGE_REF_LONG);
    spots::apply(img, &s.spots, &p.frame, p.px_per_long);
    redeye::apply(img, &p.eyes);
}

/// The settings a render of `s` for `req` uses: the HDR edit for an [`OutputDepth::F32Hdr`]
/// render, otherwise its SDR rendition (see [`DevelopSettings::sdr_rendition`]). Every renderer
/// (CPU and GPU) goes through this, so an SDR render of an HDR edit looks the same everywhere.
pub fn settings_for<'a>(s: &'a DevelopSettings, req: &RenderRequest) -> std::borrow::Cow<'a, DevelopSettings> {
    if req.depth == OutputDepth::F32Hdr { std::borrow::Cow::Borrowed(s) } else { s.sdr_rendition() }
}

/// Long edge (pixels) of the small HDR render [`hdr_histogram`] measures.
pub const HDR_HISTOGRAM_EDGE: usize = 384;

/// Long edge of the render that gives a slider drag its live HDR histogram (drafts have none: a
/// second render per tick would slow them down), run beside the drafts by the UI.
pub const HDR_HISTOGRAM_DRAG_EDGE: usize = 192;

/// The histogram of an HDR edit's HDR rendition for a preview request `req`: a small float HDR
/// render of the same framing ([`HDR_HISTOGRAM_EDGE`]), binned with SDR on the left half and the
/// whole HDR range ([`Hdr::MAX_EV_LIMIT`](craft_develop_model::Hdr::MAX_EV_LIMIT) stops above SDR
/// white, whatever the headroom limit, so the scale stays put) on the right ([`Histogram::of_hdr`]). A preview of
/// an HDR edit shows its SDR rendition; this says how far its highlights go. `None` for SDR edits,
/// high-bit-depth, windowed and proofed requests (their histogram stays the image's).
pub fn hdr_histogram(src: &Rgb32f, info: &SourceInfo, s: &DevelopSettings, req: &RenderRequest) -> Option<Histogram> {
    // a drag's drafts keep their own histogram: a second render per tick would slow them down
    if !s.hdr.enabled || req.depth != OutputDepth::U8 || req.window.is_some() || req.proof.is_some() || req.quality == Quality::Draft {
        return None;
    }
    let k = (HDR_HISTOGRAM_EDGE as f64 / req.max_w.max(req.max_h).max(1) as f64).min(1.0);
    let size = |v: usize| ((v as f64 * k).round() as usize).max(1);
    let hreq = RenderRequest {
        max_w: size(req.max_w),
        max_h: size(req.max_h),
        depth: OutputDepth::F32Hdr,
        overlay: Overlay::None,
        proof: None,
        window: None,
        display: None,
        ..*req
    };
    let deep = render(src, info, s, &hreq).deep?;
    let DeepSamples::F32(v) = &deep.samples else { return None };
    Some(Histogram::of_hdr(v.as_chunks::<3>().0, craft_develop_model::Hdr::MAX_EV_LIMIT as f32))
}

/// Render `src` with settings `s`.
pub fn render(src: &Rgb32f, info: &SourceInfo, s: &DevelopSettings, req: &RenderRequest) -> Rendered {
    render_impl(Src::Borrowed(src), info, s, req, None)
}

/// [`render`], reusing (and refreshing) the intermediate results in `cache`. The output is
/// identical to [`render`]'s, except that an Auto Mask brush stroke continued from a cached raster
/// ([`stroke_raster`]) refines only its new part: its alpha matches up to float rounding (under
/// 1e-4).
pub fn render_cached(src: &Arc<Rgb32f>, info: &SourceInfo, s: &DevelopSettings, req: &RenderRequest, cache: &StageCache) -> Rendered {
    render_impl(Src::Shared(src), info, s, req, Some(cache))
}

/// `region` of the deep render [`render_cached`] makes of `req` (in that render's pixels: a part
/// of `req.window` when it has one), with the stages before the per-pixel finish taken from (and
/// kept in) `cache` and only `region` finished: the same samples as those pixels of the whole
/// render, for the part of a shown render a brush stroke changed. `None` for an 8-bit request
/// (its dither and display conversion belong to the whole render) or a region outside the render.
pub fn render_region_cached(
    src: &Arc<Rgb32f>,
    info: &SourceInfo,
    s: &DevelopSettings,
    req: &RenderRequest,
    cache: &StageCache,
    region: PixelWindow,
) -> Option<DeepImage> {
    if req.depth == OutputDepth::U8 || region.w == 0 || region.h == 0 {
        return None;
    }
    let effective = s.effective();
    let rendition = settings_for(&effective, req);
    let plan = plan(src, info, &rendition, req);
    // the render kept is `plan.keep` of what was rendered (spots and Auto Mask strokes read past
    // a window): the region in the rendered pixels
    let (kx, ky, kw, kh) = match plan.keep {
        Some(k) => (k.x, k.y, k.w, k.h),
        None => (0, 0, plan.w, plan.h),
    };
    if region.x.checked_add(region.w)? > kw || region.y.checked_add(region.h)? > kh {
        return None;
    }
    let prep = prepare_stages(&Src::Shared(src), info, &plan, req, Some(cache), &mut |_| {});
    let at = PixelWindow { x: kx + region.x, y: ky + region.y, ..region };
    Some(finish::finish_deep_region(&prep, &plan.settings, &plan.frame, info, req, at))
}

enum Src<'a> {
    Borrowed(&'a Rgb32f),
    Shared(&'a Arc<Rgb32f>),
}

fn render_impl(src: Src<'_>, info: &SourceInfo, s: &DevelopSettings, req: &RenderRequest, cache: Option<&StageCache>) -> Rendered {
    // sections switched off with their eye render as if at their defaults (issue storytold/lightcraft#316)
    let effective = s.effective();
    let edit: &DevelopSettings = &effective;
    let rendition = settings_for(edit, req);
    let s: &DevelopSettings = &rendition;
    // `Instant::now()` panics on wasm32-unknown-unknown: only read the clock when profiling.
    let lap = |what: &str, t: &mut Option<std::time::Instant>| {
        if let Some(t) = t {
            eprintln!("  {what}: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
            *t = std::time::Instant::now();
        }
    };
    let mut t = profiling().then(std::time::Instant::now);
    let src_img: &Rgb32f = match &src {
        Src::Borrowed(r) => r,
        Src::Shared(a) => a,
    };
    let plan = plan(src_img, info, s, req);
    let Plan { ref frame, .. } = plan;
    let s = &*plan.settings;

    let prep = prepare_stages(&src, info, &plan, req, cache, &mut |what| lap(what, &mut t));
    if req.depth != OutputDepth::U8 {
        let deep = finish::finish_deep(&prep, s, frame, info, req);
        let deep = match plan.keep {
            Some(k) => deep.crop(k.x, k.y, k.w, k.h),
            None => deep,
        };
        let image = deep.to_rgba8();
        let histogram = Histogram::of_srgb8(&image);
        lap("finish (deep)", &mut t);
        return Rendered { image, histogram, deep: Some(deep) };
    }
    let image = finish::finish(&prep, s, frame, info, req);
    lap("finish", &mut t);
    let cut = |i: &Rgba8| match plan.keep {
        Some(k) => i.crop(k.x, k.y, k.w, k.h),
        None => i.clone(),
    };
    let histogram = match plan.keep {
        Some(_) => Histogram::of_srgb8(&cut(&image)),
        None => Histogram::of_srgb8(&image),
    };
    lap("histogram", &mut t);
    let mut image = image;
    let mask = overlay_alpha(req.overlay, &plan, &prep);
    let masks = overlay_alphas_all(req.overlay, &plan, &prep);
    visualize::apply(&mut image, req.overlay, &plan, mask.as_ref(), masks.as_deref());
    let mut image = if plan.keep.is_some() { cut(&image) } else { image };
    if req.overlay == Overlay::HdrRange && edit.hdr.enabled {
        // the HDR rendition of the same request says how far above SDR white each pixel goes
        let hreq = RenderRequest { depth: OutputDepth::F32Hdr, overlay: Overlay::None, proof: None, ..*req };
        if let Some(hdr) = render(src_img, info, edit, &hreq).deep {
            visualize::hdr_range(&mut image, &hdr, req.space.luma());
        }
        lap("visualize hdr", &mut t);
    }
    Rendered { image, histogram, deep: None }
}

/// The stages of a render before its per-pixel finish, for `plan` (of `src`, `info` and `req`):
/// the resampled, white-balanced and denoised image, its spatial planes and the masks evaluated,
/// reusing and refreshing `cache`. `lap` marks the end of each stage (profiling).
fn prepare_stages(
    src: &Src<'_>,
    info: &SourceInfo,
    plan: &Plan<'_>,
    req: &RenderRequest,
    cache: Option<&StageCache>,
    lap: &mut dyn FnMut(&str),
) -> Prepared {
    let Plan { ref frame, w, h, px_per_long, detail_scale, geo, lin_key, .. } = *plan;
    let s = &*plan.settings;
    let src_img: &Rgb32f = match src {
        Src::Borrowed(r) => r,
        Src::Shared(a) => a,
    };
    let shared = match (src, cache) {
        (Src::Shared(a), Some(c)) => Some((*a, c)),
        _ => None,
    };
    let cached = shared.and_then(|(a, c)| c.get(a, geo));
    let sampled = match &cached {
        Some(e) => e.sampled.clone(),
        None => Arc::new(frame.sample(src_img, w, h)),
    };
    lap("sample");

    let mut colour_noise = cached.as_ref().and_then(|e| e.colour_noise).filter(|(k, _)| *k == plan.noise_key);
    let lin = match cached.as_ref().and_then(|e| e.lin.clone()).filter(|(k, _)| *k == lin_key) {
        Some((_, img)) => img,
        None => {
            // Without a cache the resampled buffer is ours: work on it in place.
            let mut img = if shared.is_some() { (*sampled).clone() } else { Arc::unwrap_or_clone(sampled.clone()) };
            lin_cpu(&mut img, info, plan);
            if colour_noise.is_none() {
                colour_noise = self::colour_noise(src_img, info, plan).map(|n| (plan.noise_key, n));
            }
            local::denoise(&mut img, s, detail_scale, colour_noise.as_ref().map(|c| &c.1));
            lens_blur::apply_with(&mut img, s, frame, px_per_long, plan.mattes.as_deref(), plan.lens_subject.as_deref());
            Arc::new(img)
        }
    };
    lap("wb/spots/nr");
    let mut planes = match cached.map(|e| e.planes) {
        Some(p) if p.key == lin_key => p,
        // A draft while noise reduction changes (a slider drag): the wide, edge-aware planes of the
        // image before the change stand in (noise reduction barely moves a blur that wide); the
        // per-pixel log luminance and the fine texture band are redone. Keyed so a full-quality
        // render never takes them.
        Some(p) if req.quality == Quality::Draft && p.pre_nr == plan.pre_nr_key => p.for_other_nr(),
        _ => local::Planes { key: lin_key, pre_nr: plan.pre_nr_key, ..Default::default() },
    };
    // (the brush rasters are keyed by the image they compared against: `lin_key` covers it and the frame)
    let brush = shared.map(|(a, c)| stroke_raster::BrushCacheRef { cache: &c.brush, src: a, key: lin_key });
    let mut prep =
        local::prepare(lin.clone(), s, frame, px_per_long, plan.native_long as f64, req.quality, &mut planes, plan.mattes.as_deref(), brush);
    if let Some(a) = plan.fixed_air {
        prep.air = a;
    }
    lap("prepare");
    if let Some((a, c)) = shared {
        c.put(CacheEntry { src: a.clone(), geo, sampled, lin: Some((lin_key, lin)), colour_noise, planes });
    }
    prep
}

/// The alpha (0..1) of mask `id` of `s` where [`render_cached`] renders `req`: what an overlay of
/// that mask shows, without rendering the picture (the stages before the per-pixel finish come
/// from `cache`, and a brush stroke that grew since the last render stamps only its new dabs, see
/// [`stroke_raster`]). `None` when `s` has no mask `id` with components. Matches the alpha
/// [`render_cached`] evaluates for the same mask.
pub fn mask_alpha_cached(
    src: &Arc<Rgb32f>,
    info: &SourceInfo,
    s: &DevelopSettings,
    req: &RenderRequest,
    cache: &StageCache,
    id: u32,
) -> Option<Plane> {
    let effective = s.effective();
    let rendition = settings_for(&effective, req);
    let plan = plan(src, info, &rendition, req);
    let m = plan.settings.masks.iter().find(|m| m.id == id && !m.components.is_empty())?;
    let prep = prepare_stages(&Src::Shared(src), info, &plan, req, Some(cache), &mut |_| {});
    let alpha = match prep.masks.iter().find(|e| e.id == id) {
        Some(e) => e.alpha.clone(),
        // (a hidden mask isn't evaluated with the others)
        None => masks::evaluate_one_cached(
            m,
            &plan.frame,
            plan.w,
            plan.h,
            &prep.img,
            &prep.log_l,
            plan.settings.light.exposure as f32,
            plan.mattes.as_deref(),
            auto_model(&plan),
            None,
        ),
    };
    Some(match plan.keep {
        Some(k) => alpha.crop(k.x, k.y, k.w, k.h),
        None => alpha,
    })
}

/// How `plan`'s settings paint Auto Mask strokes.
fn auto_model(plan: &Plan<'_>) -> stroke_raster::AutoMaskModel {
    stroke_raster::AutoMaskModel::for_process(plan.settings.process.process())
}

/// The colour a colour-range mask would sample at normalized image point `p` (OkLab of the
/// exposed scene colours, as [`MaskShape::ColorRange`](craft_develop_model::MaskShape) compares
/// them), averaged over 3 × 3 pixels of a render fitting `req`. `None` outside the image.
pub fn color_range_sample(src: &Rgb32f, info: &SourceInfo, s: &DevelopSettings, req: &RenderRequest, p: craft_geom::Point) -> Option<[f64; 3]> {
    // Pick from the same settings as the render: disabled panels contribute nothing, and an
    // SDR preview of an HDR edit includes the SDR rendition's exposure offset.
    let effective = s.effective();
    let rendition = settings_for(&effective, req);
    let plan = plan(src, info, &rendition, req);
    let mut img = plan.frame.sample(src, plan.w, plan.h);
    lin_cpu(&mut img, info, &plan);
    let o = plan.frame.norm_to_out(plan.w, plan.h).apply(p);
    let (cx, cy) = (o.x.floor() as i64, o.y.floor() as i64);
    if cx < 0 || cy < 0 || cx >= plan.w as i64 || cy >= plan.h as i64 {
        return None;
    }
    let gain = (plan.settings.light.exposure as f32).exp2();
    let mut acc = [0f64; 3];
    let mut n = 0.0;
    for y in (cy - 1).max(0)..=(cy + 1).min(plan.h as i64 - 1) {
        for x in (cx - 1).max(0)..=(cx + 1).min(plan.w as i64 - 1) {
            let c = img.data[y as usize * plan.w + x as usize].map(|v| v * gain);
            let lab = crate::color::perceptual::oklab_from_2020(masks::tonemap_for_select(c));
            for k in 0..3 {
                acc[k] += lab[k] as f64;
            }
            n += 1.0;
        }
    }
    Some(acc.map(|v| v / n))
}

/// The plane an overlay shows: a sharpening view's detail plane, or the alpha of a mask overlay
/// (the one the render evaluated, or for a hidden mask a fresh evaluation).
fn overlay_alpha(o: Overlay, plan: &Plan<'_>, prep: &Prepared) -> Option<Plane> {
    if o.reads_detail() {
        let band = if local::pixel_sharpen(&plan.settings) { &prep.sharpen_blur } else { &prep.texture_blur };
        return visualize::sharpen_plane(o, &plan.settings, &prep.log_l, band.as_deref(), plan.px_per_long, plan.native_long as f64);
    }
    let m = o.mask(&plan.settings)?;
    if let Some(e) = prep.masks.iter().find(|e| e.id == m.id) {
        return Some(e.alpha.clone());
    }
    let ev = plan.settings.light.exposure as f32;
    Some(masks::evaluate_one_cached(m, &plan.frame, plan.w, plan.h, &prep.img, &prep.log_l, ev, plan.mattes.as_deref(), auto_model(plan), None))
}

/// The alpha planes an `Overlay::AllMasks` shows: one per visible mask that has components,
/// reusing the render's own evaluation where available (a fresh evaluation otherwise).
fn overlay_alphas_all(o: Overlay, plan: &Plan<'_>, prep: &Prepared) -> Option<Vec<Plane>> {
    if !matches!(o, Overlay::AllMasks { .. }) {
        return None;
    }
    let ev = plan.settings.light.exposure as f32;
    let mut out = Vec::new();
    for m in plan.settings.masks.iter().filter(|m| m.visible && !m.components.is_empty()) {
        if let Some(e) = prep.masks.iter().find(|e| e.id == m.id) {
            out.push(e.alpha.clone());
        } else {
            out.push(masks::evaluate_one_cached(
                m,
                &plan.frame,
                plan.w,
                plan.h,
                &prep.img,
                &prep.log_l,
                ev,
                plan.mattes.as_deref(),
                auto_model(plan),
                None,
            ));
        }
    }
    Some(out)
}

/// Convenience: render a before/after pair side by side is up to the UI; this renders "before"
/// (default look, keeping the crop so framing matches, under the photo's own rendering process).
pub fn before_settings(s: &DevelopSettings) -> DevelopSettings {
    let mut b = DevelopSettings { crop: s.crop, orientation: s.orientation, process: s.process, ..DevelopSettings::default() };
    b.wb = craft_develop_model::WhiteBalance { mode: craft_develop_model::WbMode::AsShot, ..b.wb };
    b
}

pub(crate) fn is_bw(s: &DevelopSettings) -> bool {
    s.treatment == Treatment::Bw || s.profile.id == "lc.mono" || s.profile.id.starts_with("lc.bw.")
}

/// `LIGHTCRAFT_PROFILE` is set: print per-stage timings to stderr.
pub fn profiling() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LIGHTCRAFT_PROFILE").is_some())
}

/// Run `f`, printing its duration under `LIGHTCRAFT_PROFILE`.
pub(crate) fn timed<R>(what: &str, f: impl FnOnce() -> R) -> R {
    if !profiling() {
        return f();
    }
    let t = std::time::Instant::now();
    let r = f();
    eprintln!("    {what}: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    r
}

pub(crate) fn for_rows<T: Send>(data: &mut [T], w: usize, f: impl Fn(usize, &mut [T]) + Sync + Send) {
    par_rows(data, w, f)
}

/// `plan` must keep the output box the caller (export) already decided instead of
/// re-fitting it: `Frame::fit` passes a box through unchanged only when it matches
/// the *decoded* frame's aspect within rounding slack (geometry.rs:189-191), so a
/// reduced source whose aspect rounds differently (a 149 x 100 proxy of a 240 x 160
/// photo) makes `plan` return a size a pixel off from the box (issue storytold/lightcraft#1042).
#[cfg(test)]
#[test]
fn plan_preserves_matching_box_size() {
    let src = Rgb32f::from_fn(149, 100, |_, _| [0.5; 3]);
    let info = SourceInfo { native_long: Some(240), ..SourceInfo::default() };
    let s = DevelopSettings::default();
    let fitted = plan(&src, &info, &s, &RenderRequest::fit(240, 160));
    assert_eq!((fitted.w, fitted.h), (238, 160));
    let req = RenderRequest::exact(240, 160);
    let p = plan(&src, &info, &s, &req);
    assert_eq!((p.w, p.h), (240, 160));
    // an embedded preview of another shape keeps its shape: an exact box must not stretch it
    let preview = Rgb32f::from_fn(120, 100, |_, _| [0.5; 3]);
    let p = plan(&preview, &info, &s, &req);
    assert_eq!((p.w, p.h), (192, 160));
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_crs;
#[cfg(test)]
mod tests_geometry;
#[cfg(test)]
mod tests_grain;
#[cfg(test)]
mod tests_hdr;
#[cfg(test)]
mod tests_legacy;
#[cfg(test)]
mod tests_light;
#[cfg(test)]
mod tests_local;
#[cfg(test)]
mod tests_mask_geometry;
#[cfg(test)]
mod tests_nr_scale;
#[cfg(test)]
mod tests_sharpen;
#[cfg(test)]
mod tests_tone;
#[cfg(test)]
mod tests_window;
