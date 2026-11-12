//! Raw file → what the pipeline renders: a linear Rec.2020 picture, upright, plus the [`SourceInfo`] that says how
//! its settings are read. LightCraft's `lightcraft-engine` `files::load` (the raw branch), without the catalog,
//! AI denoise or the lens database.
//!
//! [`load_raw`] decodes with [`craft_raw`] and develops; [`develop_raw`] starts from a [`RawImage`] already decoded.
//! The steps, in order: the decoder's leftovers ([`crate::sensor::prepare`]); Remove Hot Pixels on the mosaic when
//! asked; the embedded lens corrections taken out of `OpcodeList3` (the pipeline applies them, as "Enable Profile
//! Corrections", [`SourceInfo::lens`]); the starting colour ([`crate::look::starting_colour`]: the file's own
//! matrices, else a look fitted to its camera JPEG, else measured matrices, else the neutral fallback); the
//! white-balance scale ([`WbScaleOf`]); the mosaic binned straight to about `max_edge` when that is small enough
//! ([`bin_factor`]), else demosaiced (bilinear up to 600 px, AHD above); clipped highlights rebuilt; white balance,
//! colour matrix, profile tables and baseline exposure applied; fitted to `max_edge` (box filter) and oriented.
//!
//! What LightCraft adds from outside the file stays with the caller, through [`LoadOptions`]: a lens profile from
//! a lens database ([`LoadOptions::lens_fallback`]), an imported DCP ([`LoadOptions::dcp`]), the hot pixel limit.
//! Camera profiles for the look fit are the built-in ones unless the app registers its own
//! ([`crate::look::profiles::register`]).

mod colour;

pub use colour::{CameraColourModel, HIGHLIGHT_CLIP, bin_factor, clipped_white};

use crate::color::cct::xy_to_temp_tint;
use crate::raster::Rgb32f;
use crate::raster::resample::{Filter, fit};
use crate::sensor::dcp::DcpProfile;
use crate::sensor::prelude::*;
use crate::sensor::{Camera, ColorData, Method, RawData, RawError, RawFormat, RawImage, orientation};
use crate::{CameraColor, SourceInfo};
use craft_develop_model::EmbeddedLens;
use std::sync::Arc;

/// How [`load_raw`] / [`develop_raw`] develop a raw.
#[derive(Clone, Debug)]
pub struct LoadOptions<'a> {
    /// The picture's long edge at most (`usize::MAX`: full size). Small sizes bin the mosaic instead of
    /// demosaicing it: a preview, not the export.
    pub max_edge: usize,
    /// A DNG that keeps reduced-resolution copies of its raw image (Lightroom's merges) develops the smallest that
    /// covers `max_edge`. Thumbnails only: the result is not the full image ([`Loaded::reduced`]).
    pub allow_reduced: bool,
    /// An imported camera profile, in place of the file's own colour model (its matrices and look; the shot's white
    /// stays).
    pub dcp: Option<&'a DcpProfile>,
    /// Remove Hot Pixels (`detail.nr_outliers`): replace single outlier photosites before anything is developed,
    /// giving up when more than this share (0..1) would be replaced ([`crate::sensor::HOT_PIXEL_MAX_SHARE`] is
    /// LightCraft's default). `None`: off.
    pub hot_pixels: Option<f64>,
    /// Lens corrections for a file that embeds none (LightCraft looks the lens up in its lens database), in the
    /// developed picture's frame ([`SourceInfo::lens`]).
    pub lens_fallback: Option<EmbeddedLens>,
}

impl Default for LoadOptions<'_> {
    fn default() -> Self {
        LoadOptions { max_edge: usize::MAX, allow_reduced: false, dcp: None, hot_pixels: None, lens_fallback: None }
    }
}

impl LoadOptions<'_> {
    /// Full size, everything else at its default.
    pub fn full() -> Self {
        LoadOptions::default()
    }

    /// At most `max_edge` pixels on the long edge.
    pub fn fit(max_edge: usize) -> Self {
        LoadOptions { max_edge, ..LoadOptions::default() }
    }
}

/// A developed raw.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// Linear Rec.2020 (D65), scene-referred (1.0 = the sensor's white after baseline exposure, values above it
    /// kept), upright, at most [`LoadOptions::max_edge`] on the long edge.
    pub image: Rgb32f,
    pub info: SourceInfo,
    /// A reduced-resolution copy was developed ([`LoadOptions::allow_reduced`]).
    pub reduced: bool,
    /// The file is a raw variant [`craft_raw`] does not decode: `image` is its embedded camera JPEG (rendered, not
    /// raw; [`SourceInfo::default`]), and the string says why.
    pub from_preview: Option<String>,
}

/// Why a raw could not be developed.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Raw(#[from] RawError),
    #[error("unsupported raw ({0}) without an embedded preview")]
    NoPreview(String),
}

/// Decode `bytes` (a camera raw file) and develop it ([`develop_raw`]). A raw variant that is recognised but not
/// decoded comes back as its embedded camera JPEG ([`Loaded::from_preview`]) when it has one.
pub fn load_raw(bytes: &[u8], opts: &LoadOptions<'_>) -> Result<Loaded, LoadError> {
    let reduced = (opts.allow_reduced && opts.max_edge < usize::MAX)
        .then(|| craft_raw::decode_reduced(bytes, opts.max_edge, &craft_raw::Limits::default()).ok().flatten())
        .flatten();
    let used_reduced = reduced.is_some();
    let raw = match reduced {
        Some(r) => r,
        None => match craft_raw::decode(bytes) {
            Ok(r) => r,
            Err(RawError::Unsupported(why)) => return embedded_preview(bytes, opts.max_edge, why),
            Err(e) => return Err(e.into()),
        },
    };
    let mut out = develop_raw(raw, Some(bytes), opts)?;
    out.reduced = used_reduced;
    Ok(out)
}

/// The embedded camera JPEG of a raw that is not decoded, as linear Rec.2020, fitted and upright.
fn embedded_preview(bytes: &[u8], max_edge: usize, why: String) -> Result<Loaded, LoadError> {
    let Some((img, _)) = crate::preview::embedded_working(bytes, max_edge, crate::preview::MAX_PREVIEW_PIXELS) else {
        return Err(LoadError::NoPreview(why));
    };
    // (the raw's orientation: the camera JPEG is stored as the sensor is)
    let o = craft_raw::probe_info(bytes).map(|i| orientation(i.orientation)).unwrap_or_default();
    Ok(Loaded { image: img.into_oriented(o), info: SourceInfo::default(), reduced: false, from_preview: Some(why) })
}

/// Develop a decoded raw (see the module docs). `bytes`, the file it was decoded from, gives the camera JPEG the
/// look of a raw without colour matrices is fitted to, the white balance of an ERF / MEF, and a DNG's semantic
/// masks; without them those fall back (measured matrices or the neutral model; no mattes).
pub fn develop_raw(mut raw: RawImage, bytes: Option<&[u8]>, opts: &LoadOptions<'_>) -> Result<Loaded, LoadError> {
    let (dcp, max_edge) = (opts.dcp, opts.max_edge.max(1));
    let bytes = bytes.unwrap_or(&[]);
    crate::sensor::prepare(&mut raw, Some(bytes));
    // single outlier photosites, replaced in the mosaic before anything is developed from it (storytold/lightcraft#1345)
    if let Some(limit) = opts.hot_pixels {
        raw.suppress_hot_pixels_with(limit);
    }
    // Embedded lens corrections are applied by the pipeline ("Enable Profile Corrections"), not baked in
    // (removed before the camera look's binned sensor proxy too, which needs an empty `OpcodeList3`).
    let info = raw.info();
    let lens = embedded_lens(&info).or(opts.lens_fallback);
    // (the camera look still pairs the sensor with the camera JPEG through the warp the camera applied to it)
    let lens_geometry = crate::look::lens_geometry(&raw.opcodes.list3);
    // A DNG's preview already has its lens opcodes applied. Keep them for its single tone fit,
    // then remove them again: the rendered source leaves those corrections to the pipeline.
    let dng_lens_ops =
        (raw.format == RawFormat::Dng && raw.opcodes.list3.iter().any(|op| op.is_lens_correction())).then(|| raw.opcodes.list3.clone());
    raw.opcodes.list3.retain(|op| !op.is_lens_correction());
    // A camera profile without a tone curve of its own (most chart-made ones: a matrix, maybe a hue/saturation map)
    // keeps the contrast the photo has without it: the file's own DNG tone curve, else the tone of the look fitted
    // to its camera JPEG. (A chart pins colour, not a rendering: its curve would be a choice, and this one is the
    // camera's.)
    let kept_tone = match dcp {
        Some(p) if p.color.profile.tone_curve.is_none() => raw.color.profile.tone_curve.as_ref().and_then(dng_tone_curve).or_else(|| {
            let own = raw.color.clone();
            // the same proxy as the starting colour's fit below: the DNG's lens opcodes back in, no embedded lens
            let tone = with_dng_lens_ops(&mut raw, dng_lens_ops.as_ref(), lens.as_ref(), list3, |raw, embedded| {
                crate::look::starting_colour(raw, bytes, lens_geometry.as_ref(), embedded)
            })
            .2
            .map(|look| look.tone);
            raw.color = own;
            tone
        }),
        _ => None,
    };
    // An imported camera profile replaces the file's colour model (its matrices and look; the shot's white stays), so
    // the starting colour and the white balance scale below come from it, not from a fitted or built-in look.
    // (the white balance stays on the scale the file and its camera give: a profile's matrices must not turn a
    // relative white balance into Kelvin, which would read existing edits as absolute temperatures)
    let own_scale = dcp.map(|_| WbScaleOf::new(raw.format, &raw.color, raw.wb_multipliers, &raw.camera));
    if let Some(p) = dcp {
        raw.color = p.applied_to(&raw.color);
    }
    // Which scale white balance is edited on, and the as-shot white on it, from the file's own colour tags or
    // the camera's measured matrices
    let scale = WbScaleOf::new(raw.format, &raw.color, raw.wb_multipliers, &raw.camera);
    let scale = match &own_scale {
        Some(own) => scale.keeping_classification_of(own),
        None => scale,
    };
    // the source's segmentation mattes (DNG semantic masks), read while the starting colour is fitted
    let ((xy, t, camera_look), mattes) = rayon::join(
        || {
            with_dng_lens_ops(&mut raw, dng_lens_ops.as_ref(), lens.as_ref(), list3, |raw, embedded| {
                crate::look::starting_colour(raw, bytes, lens_geometry.as_ref(), embedded)
            })
        },
        || dng_mattes(bytes, &info),
    );
    // Previews and thumbnails bin the mosaic straight to (about) the size they need; only
    // larger levels (exports, 1:1) demosaic the whole sensor.
    let bin = bin_factor(&raw, max_edge);
    // (binned: block means, and which colours had a clipped sample in each block)
    let binned = match bin {
        Some(k) => raw.develop_binned_masked(k, HIGHLIGHT_CLIP)?,
        None => None,
    };
    let (img, clip_mask) = match binned {
        Some((img, mask)) => (img, Some(mask)),
        None => {
            let method = if max_edge <= 600 { Method::Bilinear } else { Method::Ahd };
            (raw.develop(method)?, None)
        }
    };
    // the samples aren't needed any more (the colour model below reads only the tags)
    raw.data = RawData::U16(Vec::new());
    let hue_sat = camera_look.as_ref().and_then(|p| p.hue_sat.as_ref()).and_then(crate::look::HueSat::new);
    // A DNG's own profile look (hue/saturation map, look table), DNG spec chapter 6.
    let tables = crate::sensor::profile::ProfileTables::new(&raw.color.profile, crate::sensor::color::illuminant_weight(&raw.color, xy));
    let colour = CameraColourModel {
        wb: t.wb,
        matrix: camera_look.as_ref().map(|p| p.matrix.mul(&t.matrix)).unwrap_or(t.matrix),
        gain: 2f32.powf(t.baseline_exposure as f32),
        tables: tables.as_ref(),
        hue_sat: hue_sat.as_ref(),
    };
    // camera RGB → what the pipeline takes: clipped highlights rebuilt, white balance and the colour model applied,
    // fitted to `max_edge` and upright
    let mut img = img;
    match &clip_mask {
        Some(mask) => colour.rebuild_highlights_masked(&mut img, mask),
        None => colour.rebuild_highlights(&mut img),
    }
    colour.apply(&mut img);
    // a full-size render keeps the picture (fitting it to its own size would copy it, ~390 MB at 33 MP)
    let img = if img.width.max(img.height) > max_edge { fit(&img, max_edge, max_edge, Filter::Box) } else { img };
    let img = img.into_oriented(orientation(raw.orientation));
    // White balance re-evaluates the file's own colour model (when it has one and no file-local look matrix sits
    // on top of it). A model's measured matrices never do: they only give the Kelvin scale its numbers, and the
    // picture is adapted from the as-shot white as every fitted look is (so settings from before that scale
    // existed render exactly as they did, `crate::local::legacy_white`).
    let camera_color = (scale.own_matrix && !t.matrix_is_fallback && (camera_look.is_none() || raw.format == RawFormat::Dng)).then(|| {
        let tags = ColorData { profile: Default::default(), ..raw.color.clone() };
        Arc::new(CameraColor { tags, developed_for: xy })
    });
    let camera_tone = camera_tone_of(
        dcp.is_some(),
        camera_look.as_ref().map(|p| p.tone),
        raw.color.profile.tone_curve.as_ref().and_then(dng_tone_curve),
        kept_tone,
    );
    let local_tone = local_tone(&raw);
    let (temp, tint) = scale.as_shot(xy);
    Ok(Loaded {
        image: img,
        info: SourceInfo {
            raw: true,
            as_shot_temp: temp,
            as_shot_tint: tint,
            lens,
            relative_wb: scale.relative,
            legacy_relative_wb: scale.legacy_relative,
            camera_color,
            camera_tone,
            mattes,
            local_tone,
            // (the caller knows the original's size when `raw` is a reduced copy)
            native_long: None,
        },
        reduced: false,
        from_preview: None,
    })
}

/// Lens corrections embedded in a raw's `OpcodeList3` (`WarpRectilinear`, `WarpRectilinear2`, `FixVignetteRadial`: a DNG's own, or the
/// raw reader's equivalent of the camera's correction, e.g. Panasonic / Leica RW2 distortion), re-expressed for the
/// default-cropped, EXIF-oriented image. These are the only "profile" corrections LightCraft applies.
pub fn embedded_lens(raw: &crate::sensor::RawInfo) -> Option<craft_develop_model::EmbeddedLens> {
    use crate::sensor::Opcode;
    use craft_develop_model::{EmbeddedLens, EmbeddedVignette, EmbeddedWarp, EmbeddedWarpStage, WARP_STAGES};
    use craft_geom::Point;
    let (aw, ah) = (raw.active_area.width as f64, raw.active_area.height as f64);
    if aw < 2.0 || ah < 2.0 {
        return None;
    }
    let c = raw.crop.clipped(raw.active_area.width, raw.active_area.height);
    let (cx0, cy0, cw, ch) =
        if c.width == 0 || c.height == 0 { (0.0, 0.0, aw, ah) } else { (c.x as f64, c.y as f64, c.width as f64, c.height as f64) };
    let long = cw.max(ch);
    // opcode centres are relative to the (uncropped) active area, in pixel-index units
    let centre = |rel: [f64; 2]| -> (Point, f64) {
        let (px, py) = (rel[0] * (aw - 1.0), rel[1] * (ah - 1.0));
        let m = [(0.0, 0.0), (aw - 1.0, 0.0), (0.0, ah - 1.0), (aw - 1.0, ah - 1.0)]
            .iter()
            .map(|&(x, y)| (x - px).hypot(y - py))
            .fold(0.0, f64::max)
            .max(1e-9);
        (Point::new((px + 0.5 - cx0) / cw, (py + 0.5 - cy0) / ch), m / long)
    };
    let mut lens = EmbeddedLens::default();
    // opcodes in list order, minus the ones the DNG 1.6 skip rule drops (a WarpRectilinear that follows an optional
    // WarpRectilinear2)
    let ops: Vec<&Opcode> = craft_raw::opcodes::applied_ops(&raw.opcodes.list3).collect();
    // A DNG with a WarpRectilinear2 keeps every warp opcode as a chain of stages (each warps the one before's output);
    // without one the first WarpRectilinear is the lens warp, as it always was.
    let chain = ops.iter().any(|op| matches!(op, Opcode::WarpRectilinear2 { .. }));
    let mut stages = Vec::new();
    for op in ops {
        match op {
            Opcode::WarpRectilinear2 { planes, center, reciprocal, .. } if !planes.is_empty() => {
                let (center, radius) = centre(*center);
                let p = |i: usize| planes[i.min(planes.len() - 1)];
                stages.push(EmbeddedWarpStage { planes: [p(0), p(1), p(2)], center, radius, reciprocal: *reciprocal });
            }
            Opcode::WarpRectilinear { planes, center } if chain && !planes.is_empty() => {
                // WarpRectilinear is the even-power case: kr0 + kr1 r² + kr2 r⁴ + kr3 r⁶, no radius limit
                let (center, radius) = centre(*center);
                let p = |i: usize| {
                    let k = planes[i.min(planes.len() - 1)];
                    let mut s = [0.0; craft_raw::opcodes::WR2_LEN];
                    s[0] = k[0];
                    s[2] = k[1];
                    s[4] = k[2];
                    s[6] = k[3];
                    s[15] = k[4];
                    s[16] = k[5];
                    s[18] = 1e6;
                    s
                };
                stages.push(EmbeddedWarpStage { planes: [p(0), p(1), p(2)], center, radius, reciprocal: false });
            }
            Opcode::WarpRectilinear { planes, center } if !chain && !planes.is_empty() && lens.warp.is_none() => {
                let (center, radius) = centre(*center);
                let p = |i: usize| planes[i.min(planes.len() - 1)];
                lens.warp = Some(EmbeddedWarp { planes: [p(0), p(1), p(2)], center, radius });
            }
            Opcode::FixVignetteRadial { k, center } if lens.vignette.is_none() => {
                let (center, radius) = centre(*center);
                lens.vignette = Some(EmbeddedVignette { k: *k, center, radius });
            }
            _ => {}
        }
    }
    // (more stages than a lens keeps: the first ones; real files carry two)
    for (slot, st) in lens.stages.iter_mut().zip(stages.into_iter().take(WARP_STAGES)) {
        *slot = Some(st);
    }
    if lens.warp.is_none() && lens.vignette.is_none() && !lens.has_stages() {
        return None;
    }
    Some(crate::optics::reorient_lens(&lens, orientation(raw.orientation), cw, ch))
}

/// Which white-balance scale a raw is edited on, decided from the file alone, the way the catalog decides it from
/// the photo's format and camera (`lightcraft_catalog::relative_wb_camera`; a test keeps the two in step).
///
/// A file with colour matrices of its own (DNG) has an absolute scale from its tags. A file without
/// (`look::file_local_look`) has one when its camera's matrices are measured
/// (`crate::sensor::spectral`), else when its camera's white-balance presets give one
/// (`crate::sensor::kelvin_presets`): either turns its as-shot multipliers into Kelvin and tint, nothing else — the
/// look stays the file's fit. Any other raw is edited relative to its as-shot look (6500 K / 0 means as shot).
pub(crate) struct WbScaleOf {
    pub(super) own_matrix: bool,
    pub(super) relative: bool,
    /// Kelvin through the camera's measured matrices or presets: edited on the relative scale before those were used.
    pub(super) legacy_relative: bool,
    /// The as-shot white through the measured matrices or the presets, when that is where the scale comes from.
    pub(super) scale_white: Option<crate::color::Xy>,
    /// The camera's measured matrices, when they give the scale.
    camera: Option<&'static crate::sensor::spectral::Camera>,
}

/// The white balance a DNG of a raw's camera RGB (Photo Merge) declares so that it shows as its source does
/// ([`WbScaleOf::dng_white`]).
pub(crate) enum DngWhite {
    /// Colour tags with the camera's measured matrices: the DNG reads every white through them, as the source does.
    Matrices(Box<crate::sensor::ColorData>),
    /// The white the source's scale shows at As Shot.
    At(crate::color::Xy),
}

impl WbScaleOf {
    pub(crate) fn new(format: crate::sensor::RawFormat, color: &crate::sensor::ColorData, wb_multipliers: Option<[f32; 3]>, meta: &Camera) -> Self {
        let own_matrix = crate::sensor::color::has_matrix(color);
        let fitted = crate::look::file_local_look(format) && !own_matrix;
        let model = meta.model.as_deref().filter(|_| fitted);
        let camera = model.and_then(|model| crate::sensor::spectral::find(meta.make.as_deref(), model));
        let presets = camera.is_none() && model.is_some_and(|model| crate::sensor::kelvin_presets::covers(meta.make.as_deref(), model));
        let scale_white = match camera {
            Some(c) => crate::sensor::spectral::as_shot_white_through(c, color, wb_multipliers),
            None if presets => crate::sensor::kelvin_presets::as_shot_white(color, wb_multipliers),
            None => None,
        };
        let covered = camera.is_some() || presets;
        WbScaleOf { own_matrix, relative: fitted && !covered, legacy_relative: fitted && covered, scale_white, camera }
    }

    /// This scale (a raw's, with an imported camera profile's colour model) on the white-balance scale `own` (the
    /// same raw's with its own colour) gives: relative, or Kelvin edited on the relative scale before. The matrices,
    /// and so the colour and the as-shot reading, stay this scale's.
    pub(super) fn keeping_classification_of(self, own: &WbScaleOf) -> Self {
        WbScaleOf { relative: own.relative, legacy_relative: own.legacy_relative, ..self }
    }

    /// What a DNG of this raw's camera RGB declares so that it shows the white balance as this scale does: the
    /// camera's measured matrices when they give the scale, else the white the scale shows at As Shot (the presets'
    /// reading; 6500 K / 0 on the relative scale; else `own`, the white the file's tags give).
    pub(crate) fn dng_white(&self, color: &crate::sensor::ColorData, own: crate::color::Xy) -> DngWhite {
        if let Some(camera) = self.camera {
            let mut c = crate::sensor::ColorData { profile: Default::default(), kelvin_presets: None, ..color.clone() };
            if camera.fill(&mut c) {
                return DngWhite::Matrices(Box::new(c));
            }
        }
        if self.relative {
            return DngWhite::At(crate::color::cct::temp_tint_to_xy(6500.0, 0.0));
        }
        DngWhite::At(self.scale_white.unwrap_or(own))
    }
    /// The as-shot temperature and tint on the file's scale: the relative scale's reference, the white through
    /// the measured matrices or the presets, else `own` (the white the file's own tags give). Whole numbers, as the
    /// catalog keeps them. A covered camera whose file names no white (no multipliers, or no presets) falls back to
    /// `own`, D65.
    pub(super) fn as_shot(&self, own: crate::color::Xy) -> (f64, f64) {
        if self.relative {
            return (6500.0, 0.0);
        }
        let (t, tint) = xy_to_temp_tint(self.scale_white.unwrap_or(own));
        (t.round(), tint.round())
    }
}

/// The raw's gain table map with where its developed picture (the default crop, oriented) sits in
/// the active area: rendered when the photo's "Camera local tone mapping" option is on.
fn local_tone(raw: &crate::sensor::RawImage) -> Option<Arc<crate::local_tone::LocalTone>> {
    let map = raw.color.profile.gain_table_map.clone()?;
    let a = raw.active_area;
    let (aw, ah) = (a.width.max(1) as f64, a.height.max(1) as f64);
    let c = raw.develop_crop(a.width, a.height);
    let rect = [c.x as f64 / aw, c.y as f64 / ah, c.width as f64 / aw, c.height as f64 / ah];
    let placement = crate::sensor::gaintable::SourcePlacement { rect, orientation: crate::sensor::orientation(raw.orientation) };
    Some(Arc::new(crate::local_tone::LocalTone { map, placement }))
}

/// The semantic masks of a DNG that AI masks understand, over the developed image (default crop,
/// oriented like it). Person mattes without a confidently selected pixel are left out (the
/// Subject mask then falls back to its heuristic); a sky matte counts even when empty (no sky).
fn dng_mattes(bytes: &[u8], raw: &crate::sensor::RawInfo) -> Option<Arc<crate::masks::Mattes>> {
    use crate::masks::{MatteKind, Mattes};
    if raw.format != crate::sensor::RawFormat::Dng {
        return None;
    }
    let mut mattes = Mattes::default();
    for m in craft_raw::semantic_masks(bytes) {
        let Some(kind) = matte_kind(&m.name) else { continue };
        let Some(img) = crate::sensor::semantic::developed(&m, raw.active_area, raw.crop) else { continue };
        if kind != MatteKind::Sky && !img.data.iter().any(|&v| v >= 128) {
            continue;
        }
        mattes.push(kind, img.into_oriented(crate::sensor::orientation(raw.orientation)));
    }
    (!mattes.is_empty()).then(|| Arc::new(mattes))
}

/// What a semantic mask selects, by its `SemanticName`: Apple's (iPhone ProRAW) are named after
/// their auxiliary image types, `urn:com:apple:photo:<year>:aux:<type>`. Other names: `None`.
fn matte_kind(name: &str) -> Option<crate::masks::MatteKind> {
    use crate::masks::MatteKind::*;
    let kind = name.strip_prefix("urn:com:apple:photo:")?.rsplit(':').next()?;
    Some(match kind {
        "semanticskymatte" => Sky,
        "semanticskinmatte" => Skin,
        "semantichairmatte" => Hair,
        "semanticteethmatte" => Teeth,
        "semanticglassesmatte" => Glasses,
        "portraiteffectsmatte" => Person,
        _ => return None,
    })
}

/// A DNG `ProfileToneCurve` (linear in, linear out, 1.0 = white after exposure compensation) as the
/// finish stage's camera tone curve: 32 knots, log-spaced over 12 stops below white; above white
/// the camera tone's shoulder continues it.
pub(crate) fn dng_tone_curve(curve: &crate::sensor::profile::ToneCurve) -> Option<crate::tone::CameraTone> {
    let knots: [[f32; 2]; 32] = std::array::from_fn(|i| {
        let x = 2f32.powf(-12.0 + 12.0 * i as f32 / 31.0);
        [x, curve.eval(x).min(0.9995)]
    });
    crate::tone::CameraTone::new(knots)
}

/// A raw's `OpcodeList3`.
fn list3(raw: &mut crate::sensor::RawImage) -> &mut Vec<crate::sensor::Opcode> {
    &mut raw.opcodes.list3
}

/// `fit(raw, embedded)` with a DNG's lens opcodes `ops` back in `raw`'s `OpcodeList3` (`list3`), as its preview has
/// them applied, and the list as it was afterwards. With the opcodes restored the proxy already includes the
/// vignetting, so `embedded` is `None` then (else `lens`): every tone fit of one photo sees the same proxy.
fn with_dng_lens_ops<R, L, T>(
    raw: &mut R,
    ops: Option<&Vec<crate::sensor::Opcode>>,
    lens: Option<&L>,
    list3: fn(&mut R) -> &mut Vec<crate::sensor::Opcode>,
    fit: impl FnOnce(&mut R, Option<&L>) -> T,
) -> T {
    let removed = ops.map(|ops| std::mem::replace(list3(raw), ops.clone()));
    let embedded = if removed.is_some() { None } else { lens };
    let out = fit(raw, embedded);
    if let Some(ops) = removed {
        *list3(raw) = ops;
    }
    out
}

/// The tone a raw's starting look gets from its camera: `look` (the tone of the look fitted to the camera JPEG),
/// `curve` (the colour model's own tone curve: the file's, or with an imported camera profile, the profile's) and
/// `kept` (the photo's own tone, kept for a profile without a curve). With a camera profile (`profile`), its curve
/// comes first, then the kept tone, and the fitted look's only last: the look was fitted without the profile, so it
/// must not replace the profile's curve or the contrast kept for it. Without one, the fitted look leads as before.
fn camera_tone_of<T>(profile: bool, look: Option<T>, curve: Option<T>, kept: Option<T>) -> Option<T> {
    if profile { curve.or(kept).or(look) } else { look.or(curve).or(kept) }
}

#[cfg(test)]
mod camera_tone_tests {
    use super::camera_tone_of;

    /// CodeRabbit on storytold/lightcraft#1063: with an imported camera profile, a fitted camera look's tone replaced the profile's own
    /// tone curve and the tone kept for a profile without one. Without a profile the order is unchanged.
    #[test]
    fn a_camera_profiles_curve_and_kept_tone_come_before_the_fitted_look() {
        assert_eq!(camera_tone_of(true, Some("look"), Some("profile"), Some("kept")), Some("profile"));
        assert_eq!(camera_tone_of(true, Some("look"), None, Some("kept")), Some("kept"));
        assert_eq!(camera_tone_of(true, Some("look"), None, None), Some("look"));
        assert_eq!(camera_tone_of(false, Some("look"), Some("file"), Some("kept")), Some("look"));
        assert_eq!(camera_tone_of(false, None, Some("file"), Some("kept")), Some("file"));
        assert_eq!(camera_tone_of::<&str>(false, None, None, None), None);
    }
}

#[cfg(test)]
mod wb_scale_tests {
    use super::WbScaleOf;

    /// CodeRabbit on storytold/lightcraft#1063: a raw edited on the relative white-balance scale (no colour matrices, none measured for
    /// its camera) turned to Kelvin once an imported camera profile's matrices replaced its colour, so its existing
    /// edits (6500 K / 0 = as shot) were read as absolute temperatures, and the catalog and quick previews still
    /// used the relative scale. The profile's matrices stay; the scale is the file's.
    #[test]
    fn a_camera_profile_keeps_a_relative_white_balance_relative() {
        let format = crate::sensor::RawFormat::Cr3;
        let meta = crate::sensor::Camera::default();
        let own_color = crate::sensor::ColorData::default();
        let own = WbScaleOf::new(format, &own_color, Some([2.0, 1.0, 1.5]), &meta);
        assert!(own.relative && !own.own_matrix, "a matrix-less raw of an unmeasured camera is relative");
        let m = crate::color::Mat3([[0.9, 0.1, 0.0], [0.1, 0.8, 0.1], [0.0, 0.2, 0.8]]);
        let profiled = crate::sensor::ColorData { illuminant: [21, 0], color_matrix: [Some((m).0), None], ..Default::default() };
        let with = WbScaleOf::new(format, &profiled, Some([2.0, 1.0, 1.5]), &meta);
        assert!(!with.relative, "the profile's matrices alone would make it Kelvin");
        let kept = with.keeping_classification_of(&own);
        assert!(kept.relative && !kept.legacy_relative && kept.own_matrix, "relative, with the profile's matrices");
        assert_eq!(kept.as_shot(crate::color::Xy { x: 0.3127, y: 0.329 }), (6500.0, 0.0));
    }
}

#[cfg(test)]
mod lens_ops_tests {
    use crate::sensor::Opcode;

    use super::with_dng_lens_ops;

    fn same(v: &mut Vec<Opcode>) -> &mut Vec<Opcode> {
        v
    }

    /// Both tone fits of a DNG (the kept tone for a camera profile without a curve, and the starting colour) see its
    /// lens opcodes restored and no embedded lens; the list is as it was afterwards (CodeRabbit on storytold/lightcraft#1063: the
    /// kept-tone fit ran without the opcodes and with the embedded lens).
    #[test]
    fn a_dngs_tone_fit_sees_its_lens_opcodes_and_no_embedded_lens() {
        let vignette = Opcode::FixVignetteRadial { k: [0.1, 0.0, 0.0, 0.0, 0.0], center: [0.5, 0.5] };
        let ops = vec![vignette.clone()];
        let mut list: Vec<Opcode> = Vec::new();
        let lens = 7;
        let seen = with_dng_lens_ops(&mut list, Some(&ops), Some(&lens), same, |l, embedded| (l.clone(), embedded.copied()));
        assert_eq!(seen, (vec![vignette], None), "the opcodes are in, the embedded lens is not");
        assert!(list.is_empty(), "the list is as it was");
        let seen = with_dng_lens_ops(&mut list, None, Some(&lens), same, |l, embedded| (l.len(), embedded.copied()));
        assert_eq!(seen, (0, Some(7)), "without DNG lens opcodes the embedded lens is used");
    }
}
