//! The per-pixel stage: everything after the spatial planes are ready, in one parallel pass.

use crate::color::luminance_2020;
use crate::color::spline::{Lut1, MonotoneCurve};
use crate::color::transfer::linear_to_srgb;
use crate::raster::Rgba8;
use craft_develop_model::{DevelopSettings, LocalAdjustments, Process, ToneCurve, VignetteStyle};
use craft_geom::Point;

use crate::colorops::ColorOps;
use crate::geometry::Frame;
use crate::local::log_lum;
use crate::output::{DeepImage, DeepSamples, OutputDepth, OutputSpace, OutputTrc};
use crate::tone::ToneMap;
use crate::{Prepared, SourceInfo, for_rows};

#[inline]
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of the sharpening a pixel gets (0..1) under Masking `masking` (0..1), from its fine
/// detail `det` (log luminance minus its texture blur): 1 everywhere at 0, only stronger edges
/// as it rises. The GPU kernel (`finish.wgsl`) computes the same; Alt-drag on Masking shows it
/// ([`crate::Overlay::SharpenMask`]).
#[inline]
pub fn sharpen_mask_weight(masking: f32, det: f32) -> f32 {
    if masking > 0.0 { smooth(masking * 0.25, masking * 0.25 + 0.15, det.abs()) } else { 1.0 }
}

/// Masking weight for the V3 pixel-scale sharpening band, also used by its Alt-drag preview.
#[inline]
pub fn pixel_sharpen_mask_weight(masking: f32, det: f32) -> f32 {
    if masking > 0.0 { smooth(masking * 0.1, masking * 0.1 + 0.06, det.abs()) } else { 1.0 }
}

/// Gain of the sharpening detail term (per unit of `Amount / 150`), in stops of luminance per stop of detail.
pub const SHARPEN_GAIN: f32 = 7.0;

/// Soft-clip limit (stops) of the sharpening detail term for Detail `0..=100`: 0.03 stops (large
/// edge detail is flattened so halos stay small) up to 1.5 stops (practically linear).
pub fn sharpen_limit(detail: f64) -> f32 {
    let d = (detail / 100.0).clamp(0.0, 1.0) as f32;
    0.03 * 50f32.powf(d)
}

/// The sharpening boost (stops) for pixel-scale detail `det` (log luminance minus its blur at the
/// sharpen radius): linear for small detail, soft-clipped at `limit`, and gated by the Masking edge
/// mask (`mask` = Masking / 100) so flat areas and noise can be left alone.
#[inline]
pub fn sharpen_term(det: f32, limit: f32, mask: f32) -> f32 {
    let m = pixel_sharpen_mask_weight(mask, det);
    SHARPEN_GAIN * m * det / (1.0 + det.abs() / limit)
}

/// Parametric region curve (encoded domain) composed with the master point curve.
fn curve_luts(c: &ToneCurve) -> Option<[Lut1; 3]> {
    let parametric = c.highlights != 0.0 || c.lights != 0.0 || c.darks != 0.0 || c.shadows != 0.0;
    let master = !ToneCurve::point_curve_is_identity(&c.master);
    let chans = [&c.red, &c.green, &c.blue].map(|p| !ToneCurve::point_curve_is_identity(p));
    if !parametric && !master && !chans.iter().any(|b| *b) {
        return None;
    }
    const N: usize = 1024;
    let mut base = crate::curve::ParametricCurve::new(c).to_lut();
    let to_pts = |p: &[Point]| p.iter().map(|q| (q.x, q.y)).collect::<Vec<_>>();
    if master {
        base = MonotoneCurve::new(&to_pts(&c.master)).to_lut(N).compose(&base);
    }
    let per = [&c.red, &c.green, &c.blue];
    Some(std::array::from_fn(|i| {
        if chans.get(i).copied().unwrap_or(false) {
            let pts = per.get(i).map(|&p| to_pts(p)).unwrap_or_default();
            MonotoneCurve::new(&pts).to_lut(N).compose(&base)
        } else {
            base.clone()
        }
    }))
}

/// Vignette (post-crop) parameters.
#[derive(Clone, Copy, Debug)]
pub struct Vig {
    pub amount: f32,
    pub start: f32,
    pub width: f32,
    pub aspect_mix: f32,
    pub power: f32,
    pub highlights: f32,
    pub style: VignetteStyle,
}

fn vignette(s: &DevelopSettings) -> Option<Vig> {
    let v = &s.vignette;
    (v.amount != 0.0).then(|| {
        let r = (v.roundness / 100.0) as f32;
        Vig {
            amount: (v.amount / 100.0) as f32,
            start: 0.15 + (v.midpoint / 100.0) as f32 * 0.95,
            width: 0.05 + (v.feather / 100.0) as f32 * 1.1,
            aspect_mix: ((r + 1.0) / 2.0).clamp(0.0, 1.0),
            power: if r >= 0.0 { 2.0 } else { 2.0 + (-r) * 6.0 },
            highlights: (v.highlights / 100.0) as f32,
            style: v.style,
        }
    })
}

/// Hash constants of [`grain_noise`] (shared with the GPU kernel).
pub const GRAIN_HASH: [u32; 4] = [0x8da6_b343, 0xd816_3841, 0xcb1a_b31f, 0x5bd1_e995];

/// The grain of processes V1 to V3: two octaves of value noise. Frozen: photos on those
/// processes keep rendering with it (`docs/process-versions.md`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValueGrain {
    /// Amplitude (display-encoded).
    pub amt: f32,
    /// Noise cell size (output pixels).
    pub cell: f32,
    /// Roughness 0..1: weight of the finer octave.
    pub rough: f32,
    pub seed: u32,
}

/// A render's film grain. The model follows the photo's process: changing it would change how
/// existing photos look, so the crystal grain came with process V4.
#[derive(Clone, Debug)]
pub enum GrainModel {
    /// V1 to V3: value noise.
    Value(ValueGrain),
    /// V4 and later: crystal layers ([`crate::crystal`]).
    Crystal(crate::crystal::CrystalGrain),
}

impl GrainModel {
    /// The grain `s` renders with at `px_per_long` output pixels per long edge (`None`: none);
    /// `draft`: for a drag ([`crate::crystal::CrystalGrain::draft`]).
    pub fn new(s: &DevelopSettings, px_per_long: f64, draft: bool) -> Option<GrainModel> {
        let g = &s.grain;
        match s.process.process() {
            Process::V1 | Process::V2 | Process::V3 | Process::V4 | Process::V5 | Process::V6 => (g.amount > 0.0).then(|| {
                let cell = (0.0006 + (g.size / 100.0) as f32 * 0.0024) * px_per_long as f32;
                GrainModel::Value(ValueGrain {
                    amt: (g.amount / 100.0) as f32 * 0.13,
                    cell: cell.max(0.6),
                    rough: (g.roughness / 100.0) as f32,
                    seed: g.seed,
                })
            }),
            Process::V7 | Process::V8 | Process::V9 if draft => crate::crystal::CrystalGrain::draft(g, px_per_long).map(GrainModel::Crystal),
            Process::V7 | Process::V8 | Process::V9 => crate::crystal::CrystalGrain::new(g, px_per_long).map(GrainModel::Crystal),
        }
    }
}

#[inline]
fn grain_noise(x: f32, y: f32, seed: u32) -> f32 {
    // A degenerate frame (zero-width crop) maps pixels to NaN/inf coordinates: no grain there.
    if !x.is_finite() || !y.is_finite() {
        return 0.0;
    }
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let h = |i: i32, j: i32| {
        let mut v = (i as u32).wrapping_mul(GRAIN_HASH[0]) ^ (j as u32).wrapping_mul(GRAIN_HASH[1]) ^ seed.wrapping_mul(GRAIN_HASH[2]);
        v ^= v >> 13;
        v = v.wrapping_mul(GRAIN_HASH[3]);
        v ^= v >> 15;
        (v & 0xffff) as f32 / 32768.0 - 1.0
    };
    let (i, j) = (x0 as i32, y0 as i32);
    let (u, v) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = h(i, j) + (h(i + 1, j) - h(i, j)) * u;
    let b = h(i, j + 1) + (h(i + 1, j + 1) - h(i, j + 1)) * u;
    a + (b - a) * v
}

/// The crystal grain of output row `y` (`w` pixels) when its pixels are axis-aligned; empty
/// otherwise (and without crystal grain): the pixels then take it one by one.
fn crystal_row(fp: &FinishParams, y: usize, w: usize) -> Vec<f32> {
    let Some(GrainModel::Crystal(g)) = &fp.grain else { return Vec::new() };
    let long = fp.ow.max(fp.oh);
    let (a, b) = (fp.out_to_norm.apply(Point::new(0.5, y as f64 + 0.5)), fp.out_to_norm.apply(Point::new(1.5, y as f64 + 0.5)));
    let (gx, gy) = (a.x * fp.ow / long, a.y * fp.oh / long);
    let (dx, dy) = ((b.x - a.x) * fp.ow / long, (b.y - a.y) * fp.oh / long);
    let mut out = vec![0.0; w];
    if !g.row(gx, gy, dx, dy, &mut out) {
        out.clear();
    }
    out
}

/// Number of per-mask terms in [`mask_terms`].
pub const MASK_TERMS: usize = 21;

/// Number of alpha-weighted terms at the start of [`mask_terms`].
pub const MASK_SUMS: usize = 17;

/// A mask's local adjustments as the per-pixel stage uses them (alpha-weighted sums), in order:
/// exposure, temp, tint, contrast, highlights, shadows, whites, blacks, texture, clarity, dehaze,
/// saturation, hue, sharpness, noise, moiré, defringe, then the colour overlay: on (0/1), cos and
/// sin of its OkLCh hue, and its strength.
pub fn mask_terms(j: &LocalAdjustments) -> [f32; MASK_TERMS] {
    let (on, cos, sin, amt) = if j.color_sat > 0.0 {
        let hue = crate::colorops::oklch_hue_of_srgb_hue(j.color_hue);
        (1.0, hue.cos(), hue.sin(), (j.color_sat / 100.0) as f32)
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    [
        j.exposure as f32,
        (j.temp / 100.0) as f32,
        (j.tint / 100.0) as f32,
        (j.contrast / 100.0) as f32,
        (j.highlights / 100.0) as f32,
        (j.shadows / 100.0) as f32,
        (j.whites / 100.0) as f32,
        (j.blacks / 100.0) as f32,
        (j.texture / 100.0) as f32,
        (j.clarity / 100.0) as f32,
        (j.dehaze / 100.0) as f32,
        (j.saturation / 100.0) as f32,
        (j.hue / 100.0) as f32 * 0.6,
        (j.sharpness / 100.0) as f32,
        (j.noise / 100.0) as f32,
        (j.moire / 100.0) as f32,
        (j.defringe / 100.0) as f32,
        on,
        cos,
        sin,
        amt,
    ]
}

/// Everything the per-pixel stage computes once per render: the CPU loop below and the GPU kernel
/// (`lightcraft-gpu`) both read their parameters from here, so the two cannot drift apart.
pub struct FinishParams {
    /// A LUT profile and its amount (0..2), applied to the display-encoded colour.
    pub lut: Option<(std::sync::Arc<crate::lut::Lut3d>, f32)>,
    pub tone: ToneMap,
    /// Process V4's calibrated Contrast and Blacks on the camera-look path, applied after
    /// the tone map (`None`: another process or path, or both at 0; see [`crate::light_v3`]).
    pub light: Option<crate::light_v3::LightOps>,
    pub ops: ColorOps,
    /// Calibration: primaries matrix (row-major, linear Rec.2020) and shadows tint (−1..1).
    pub calib: Option<[[f32; 3]; 3]>,
    pub shadow_tint: f32,
    /// Tone curves (parametric ∘ point, per channel) on encoded values, 1024 entries each.
    pub curves: Option<[Lut1; 3]>,
    /// Refine Saturation as 0..1 (1 = the curves' own saturation).
    pub refine_sat: f32,
    pub vig: Option<Vig>,
    /// Linear Rec.2020 → linear output RGB, the output's luminance weights (gamut mapping) and its
    /// encoding curve (see [`crate::output`]).
    pub to_out: [[f32; 3]; 3],
    pub out_luma: [f32; 3],
    pub out_trc: OutputTrc,
    /// Soft proofing (the GPU kernel reads the same parameters).
    pub proof: Option<crate::output::ProofParams>,
    pub hl: f32,
    pub sh: f32,
    pub clar: f32,
    pub tex: f32,
    pub dehaze: f32,
    pub sharpen: f32,
    pub sharpen_mask: f32,
    /// V1/V2 retain their texture-band sharpening; V3 uses the pixel-scale plane.
    pub pixel_sharpen: bool,
    /// Soft-clip limit (stops) of the sharpening detail term: small damps halos (Detail 0), large keeps all (100).
    pub sharpen_limit: f32,
    /// Response compensation for a sharpening radius below one output pixel.
    pub sharpen_gain: f32,
    /// Airlight after and before exposure, exposure gain and EV (see [`crate::Prepared`]).
    pub air: f32,
    pub air_pre: f32,
    pub gain: f32,
    pub ev: f32,
    /// Film grain: the model of the photo's process ([`GrainModel`]).
    pub grain: Option<GrainModel>,
    /// Output px → normalized oriented coordinates.
    pub out_to_norm: craft_geom::Affine,
    pub ow: f64,
    pub oh: f64,
    pub w: usize,
    pub h: usize,
    pub px_per_long: f64,
    /// The window's offset and the whole output's size (pixels): `(0, 0, w, h)` unless windowed.
    pub view: [f32; 4],
    /// Peak linear output relative to SDR white: 1 for SDR renders; `2^max_ev` for an HDR render
    /// ([`OutputDepth::F32Hdr`] with HDR on), which keeps values above 1 instead of clipping them.
    pub peak: f32,
}

pub struct FinishParamsInput<'a> {
    pub settings: &'a DevelopSettings,
    pub frame: &'a Frame,
    pub info: &'a SourceInfo,
    pub image_size: (usize, usize),
    pub px_per_long: f64,
    pub air_pre: f32,
    pub space: OutputSpace,
    /// A draft render (a drag, [`crate::Quality::Draft`]): see [`GrainModel::new`].
    pub draft: bool,
}

impl FinishParams {
    /// Parameters for a `w × h` render into `space`; `ev`/`air_pre` as in [`crate::Prepared`].
    pub fn new(input: FinishParamsInput<'_>) -> FinishParams {
        let FinishParamsInput { settings: s, frame, info, image_size: (w, h), px_per_long, air_pre, space, draft } = input;
        let effects = s.section_enabled("effects");
        let (clar, tex, dehaze) = if effects {
            ((s.effects.clarity / 100.0) as f32, (s.effects.texture / 100.0) as f32, (s.effects.dehaze / 100.0) as f32)
        } else {
            (0.0, 0.0, 0.0)
        };
        let ev = s.light.exposure as f32;
        let gain = 2f32.powf(ev);
        // A degenerate frame (zero-width crop, zero scale) has no finite grain mapping: skip grain (CPU and GPU).
        let out_to_norm = frame.out_to_norm(w, h);
        let finite_frame = px_per_long.is_finite()
            && px_per_long > 0.0
            && frame.ow.is_finite()
            && frame.oh.is_finite()
            && frame.ow.max(frame.oh) > 0.0
            && out_to_norm.0.iter().all(|v| v.is_finite());
        let grain = if effects && finite_frame { GrainModel::new(s, px_per_long, draft) } else { None };
        let calibration = s.section_enabled("calibration");
        let peak = s.hdr.peak();
        FinishParams {
            peak,
            calib: if calibration { crate::colorops::calibration_matrix(&s.calibration) } else { None },
            shadow_tint: if calibration { (s.calibration.shadows_tint / 100.0) as f32 } else { 0.0 },
            tone: base_tone(s, info, peak),
            light: crate::light_v3::LightOps::new(s, info),
            lut: crate::lut::get(&s.profile.id).map(|l| (l, (s.profile.amount / 100.0).clamp(0.0, 2.0) as f32)),
            ops: ColorOps::new(s),
            curves: curve_luts(&s.curve),
            refine_sat: (s.curve.refine_saturation / 100.0).clamp(0.0, 1.0) as f32,
            vig: if effects { vignette(s) } else { None },
            to_out: space.from_working(),
            out_luma: space.luma(),
            out_trc: space.trc(),
            proof: None,
            hl: (s.light.highlights / 100.0) as f32,
            sh: (s.light.shadows / 100.0) as f32,
            clar,
            tex,
            dehaze,
            sharpen: (s.detail.sharpen_amount / 150.0) as f32,
            sharpen_mask: (s.detail.sharpen_masking / 100.0) as f32,
            pixel_sharpen: crate::local::pixel_sharpen(s),
            sharpen_limit: sharpen_limit(s.detail.sharpen_detail),
            sharpen_gain: crate::local::sharpen_gain(crate::local::sharpen_sigma_raw(
                s,
                px_per_long,
                info.native_long.unwrap_or(0).max(frame.ow.max(frame.oh) as usize).max(1) as f64,
            )),
            air: air_pre * gain,
            air_pre,
            gain,
            ev,
            grain,
            out_to_norm,
            ow: frame.ow,
            oh: frame.oh,
            w,
            h,
            px_per_long,
            view: frame.view.map_or([0.0, 0.0, w as f32, h as f32], |v| [v.x as f32, v.y as f32, v.full_w as f32, v.full_h as f32]),
        }
    }

    /// Render into a display's primaries (sRGB curve) instead of the output space, soft proof
    /// included (see [`crate::DisplaySpace`]).
    pub fn for_display(&mut self, d: &crate::DisplaySpace, proof: Option<crate::Proof>) {
        self.to_out = d.from_working;
        self.out_luma = d.luma;
        self.out_trc = OutputTrc::Srgb;
        self.proof = proof.map(|pr| pr.params_display(d));
    }
}

/// The base tone map (scene → display luminance, with Contrast, Whites and Blacks) of `s`'s
/// rendering process. This is where a new tone model branches: a retuned default tone curve (issue
/// storytold/lightcraft#146) is a new [`Process`] with its own arm here, and V1's arm stays as it is, so photos edited
/// under V1 keep their look (`docs/process-versions.md`). CPU and GPU both take it from here.
/// From V4 the camera curve keeps V2 Whites; its Contrast and Blacks are the
/// calibrated operators that follow the tone map ([`crate::light_v3`], [`FinishParams::light`]).
/// `peak` is the HDR render's peak relative to SDR white ([`craft_develop_model::Hdr::peak`]); at 1.0 (SDR)
/// every arm is the SDR tone map. An HDR render keeps the camera's look through the midtones and
/// lifts its highlights past white ([`ToneMap::camera_hdr`]), so it matches the SDR render below them.
pub fn base_tone(s: &DevelopSettings, info: &SourceInfo, peak: f32) -> ToneMap {
    let l = &s.light;
    match s.process.process() {
        // V2 changes the Highlights/Shadows base (`local::tone_base`), not the tone map
        Process::V1 | Process::V2 | Process::V3 => match info.camera_tone.as_ref().filter(|_| info.raw) {
            Some(curve) => ToneMap::camera_hdr(curve, l.contrast, l.whites, l.blacks, peak),
            None if info.raw => ToneMap::hdr(l.contrast, l.whites, l.blacks, peak),
            None => ToneMap::display_hdr(l.contrast, l.whites, l.blacks, peak),
        },
        // Keep V2 Whites in the camera curve; calibrated Contrast and Blacks follow it.
        Process::V4 => match info.camera_tone.as_ref().filter(|_| info.raw) {
            Some(curve) => ToneMap::camera_hdr(curve, 0.0, l.whites, 0.0, peak),
            None if info.raw => ToneMap::hdr(l.contrast, l.whites, l.blacks, peak),
            None => ToneMap::display_hdr(l.contrast, l.whites, l.blacks, peak),
        },
        // V4, with a raw's tone curve rolling off into white in stops: a camera curve past its last
        // knot, the generic curve from half a stop below its white point instead of clipping (issue storytold/lightcraft#1632)
        Process::V5 | Process::V6 | Process::V7 | Process::V8 | Process::V9 => match info.camera_tone.as_ref().filter(|_| info.raw) {
            Some(curve) => ToneMap::camera_rolloff_hdr(curve, 0.0, l.whites, 0.0, peak),
            None if info.raw => ToneMap::rolloff_hdr(l.contrast, l.whites, l.blacks, peak),
            None => ToneMap::display_hdr(l.contrast, l.whites, l.blacks, peak),
        },
    }
}

/// Finish the prepared pipeline stages into an 8-bit RGBA image in `space` (or `display`).
pub(crate) fn finish(p: &Prepared, s: &DevelopSettings, frame: &Frame, info: &SourceInfo, req: &crate::RenderRequest) -> Rgba8 {
    let (space, display, proof) = (req.space, req.display.as_ref(), req.proof);
    let (w, h) = (p.img.width, p.img.height);
    let draft = req.quality == crate::Quality::Draft;
    let mut fp = FinishParams::new(FinishParamsInput {
        settings: s,
        frame,
        info,
        image_size: (w, h),
        px_per_long: p.px_per_long,
        air_pre: p.air,
        space,
        draft,
    });
    fp.proof = proof.map(|pr| pr.params(space));
    if let Some(d) = display {
        fp.for_display(d, proof);
    }
    let trc = fp.out_trc;
    let data = finish_with(p, &fp, false, |x, y, e| {
        let d = bayer_dither_offset(x, y);
        match trc {
            OutputTrc::Srgb => [enc(e[0], d), enc(e[1], d), enc(e[2], d), 255],
            t => {
                let c = e.map(|v| enc(t.encode(crate::color::transfer::srgb_to_linear(v.clamp(0.0, 1.0))), d));
                [c[0], c[1], c[2], 255]
            }
        }
    });
    Rgba8 { width: w, height: h, data }
}

/// [`finish`] into 16-bit display-encoded or 32-bit float linear samples (output primaries).
pub(crate) fn finish_deep(p: &Prepared, s: &DevelopSettings, frame: &Frame, info: &SourceInfo, req: &crate::RenderRequest) -> DeepImage {
    let all = crate::PixelWindow { x: 0, y: 0, w: p.img.width, h: p.img.height };
    finish_deep_region(p, s, frame, info, req, all)
}

/// [`finish_deep`] of `region` of the prepared image only (inside it): the same samples as the
/// same pixels of the whole, each pixel's finish reading only its own position.
pub(crate) fn finish_deep_region(
    p: &Prepared,
    s: &DevelopSettings,
    frame: &Frame,
    info: &SourceInfo,
    req: &crate::RenderRequest,
    region: crate::PixelWindow,
) -> DeepImage {
    use crate::color::transfer::srgb_to_linear;
    let (space, depth, proof) = (req.space, req.depth, req.proof);
    let (w, h) = (p.img.width, p.img.height);
    let region = region.clamped(w, h);
    let draft = req.quality == crate::Quality::Draft;
    let mut fp = FinishParams::new(FinishParamsInput {
        settings: s,
        frame,
        info,
        image_size: (w, h),
        px_per_long: p.px_per_long,
        air_pre: p.air,
        space,
        draft,
    });
    fp.proof = proof.map(|pr| pr.params(space));
    let trc = fp.out_trc;
    let samples = match depth {
        OutputDepth::F32Linear => {
            let v = finish_region(p, &fp, true, region, |_x, _y, e| e.map(|v| srgb_to_linear(v.clamp(0.0, 1.0))));
            DeepSamples::F32(v.into_flattened())
        }
        OutputDepth::F32Hdr => {
            // the sRGB curve extended past 1 (it is unbounded), so values up to `peak` survive
            let peak = fp.peak;
            let v = finish_region(p, &fp, true, region, |_x, _y, e| e.map(|v| srgb_to_linear(v.max(0.0)).clamp(0.0, peak)));
            DeepSamples::F32(v.into_flattened())
        }
        _ => {
            let q = |v: f32| (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
            let v = finish_region(p, &fp, true, region, |_x, _y, e| match trc {
                OutputTrc::Srgb => e.map(q),
                t => e.map(|v| q(t.encode(srgb_to_linear(v.clamp(0.0, 1.0))))),
            });
            DeepSamples::U16(v.into_flattened())
        }
    };
    DeepImage { width: region.w, height: region.h, space, samples }
}

/// The per-pixel stage: every output pixel's colour in the output primaries, encoded with the sRGB
/// curve (tone curves and grain applied; not clamped), handed to `store` for the final encoding.
/// `exact`: encode with the exact sRGB curve instead of the (8/10-bit accurate) table.
pub(crate) fn finish_with<T: Copy + Default + Send>(
    p: &Prepared,
    fp: &FinishParams,
    exact: bool,
    store: impl Fn(usize, usize, [f32; 3]) -> T + Sync + Send,
) -> Vec<T> {
    let all = crate::PixelWindow { x: 0, y: 0, w: p.img.width, h: p.img.height };
    finish_region(p, fp, exact, all, store)
}

/// [`finish_with`] of `region` of the prepared image (inside it), row by row: each pixel as the
/// whole image's finish makes it.
fn finish_region<T: Copy + Default + Send>(
    p: &Prepared,
    fp: &FinishParams,
    exact: bool,
    region: crate::PixelWindow,
    store: impl Fn(usize, usize, [f32; 3]) -> T + Sync + Send,
) -> Vec<T> {
    let w = p.img.width;
    let (rx0, ry0, rw) = (region.x, region.y, region.w);
    let p_lut = fp.lut.clone();
    let FinishParams { light, curves, vig, to_out, out_luma, grain, .. } = fp;
    let terms: Vec<[f32; MASK_TERMS]> = p.masks.iter().map(|m| mask_terms(&m.adjust)).collect();
    let out_to_norm = fp.out_to_norm;
    let long = fp.ow.max(fp.oh);
    let [vx, vy, vw, vh] = fp.view;
    let (vx_u, vy_u) = (vx.round() as usize, vy.round() as usize);
    // the vignette belongs to the whole output frame, not to the window being drawn
    let aspect = vw / vh;

    let srgb = srgb_lut();
    let preview_light = if exact { None } else { light.as_ref().and_then(|lo| crate::light_v3::PreviewOps::new(lo, &|v| encode_srgb(srgb, v))) };
    let peak = fp.peak;
    let hdr = peak > 1.0;
    let mut out = vec![T::default(); rw * region.h];
    for_rows(&mut out, rw, |ry, row| {
        let y = ry0 + ry;
        // the crystal grain of the whole row at once when its pixels are axis-aligned
        let crystal_row = crystal_row(fp, y, w);
        for (rx, px) in row.iter_mut().enumerate() {
            let x = rx0 + rx;
            let i = y * w + x;
            let raw = p.img.data[i];
            let local = pixel_local_terms(p, &terms, i);
            let l_sat = local.values[11];
            let l_hue = local.values[12];
            let l_pre = p.log_l.data[i];
            let l0 = l_pre + fp.ev;
            let (mut c, l1) = adjust_scene_pixel(p, fp, i, raw, &local);

            // --- local tone in log luminance
            let shift = l1 - l0;
            let base = p.base.data[i] + fp.ev + shift;
            let delta = local_tone_delta(p, fp, i, base, l1, &local);
            if delta != 0.0 {
                let g = delta.exp2();
                c = c.map(|v| v * g);
            }

            // --- calibration (scene linear, before the tone map)
            if fp.calib.is_some() || fp.shadow_tint != 0.0 {
                c = crate::colorops::calibrate(c, fp.calib.as_ref(), fp.shadow_tint);
            }

            // --- tone map on luminance, highlight desaturation
            let yl = luminance_2020(c);
            let o = fp.tone.apply(yl);
            let mut d = if yl > 1e-9 { c.map(|v| v * o / yl) } else { [0.0; 3] };
            let k = fp.tone.chroma_scale(o);
            if k != 1.0 {
                d = d.map(|v| o + (v - o) * k);
            }
            let mx = d[0].max(d[1]).max(d[2]);
            if mx > peak {
                let t = ((mx - peak) / (mx - o).max(1e-6)).clamp(0.0, 1.0);
                d = d.map(|v| v + (o - v) * t);
            }

            // --- process V4: calibrated Contrast, Blacks (ProPhoto with the sRGB transfer)
            d = apply_light(d, light.as_ref(), preview_light.as_ref(), exact, srgb);

            // --- colour
            d = fp.ops.apply(d, l_sat, l_hue);
            if let Some((dir, amt)) = local.tint {
                let lab = crate::color::perceptual::oklab_from_2020(d);
                d = crate::color::perceptual::oklab_to_2020([lab[0], lab[1] + dir[0] * 0.08 * amt, lab[2] + dir[1] * 0.08 * amt]);
            }

            // --- vignette (display linear, post-crop)
            if let Some(v) = vig {
                let u = (vx + x as f32 + 0.5) / vw * 2.0 - 1.0;
                let vv = (vy + y as f32 + 0.5) / vh * 2.0 - 1.0;
                let sx = 1.0 + (aspect - 1.0) * v.aspect_mix;
                let sy = 1.0 + (1.0 / aspect - 1.0) * v.aspect_mix;
                let (ax, ay) = ((u * sx.max(1.0) / sx.max(sy)).abs(), (vv * sy.max(1.0) / sx.max(sy)).abs());
                let dist = (ax.powf(v.power) + ay.powf(v.power)).powf(1.0 / v.power);
                let t = smooth(v.start, v.start + v.width, dist);
                if t > 0.0 {
                    let lum = luminance_2020(d).clamp(0.0, 1.0);
                    if v.amount < 0.0 {
                        let mut f = 1.0 + v.amount * t;
                        if v.style == VignetteStyle::HighlightPriority {
                            f += (1.0 - f) * v.highlights * smooth(0.4, 1.0, lum);
                        }
                        if v.style == VignetteStyle::PaintOverlay {
                            d = d.map(|c| c * (1.0 - (-v.amount) * t) + 0.0);
                        } else {
                            d = d.map(|c| c * f);
                        }
                    } else if hdr {
                        // lighten towards SDR white; never pull HDR highlights down to it
                        d = d.map(|c| c + (1.0 - c).max(0.0) * v.amount * t * 0.85);
                    } else {
                        d = d.map(|c| c + (1.0 - c) * v.amount * t * 0.85);
                    }
                }
            }

            // --- gamut map to the output space (desaturate towards luminance until in range);
            // soft proofing maps into the proof space first and shows that in the output space
            let mut warn = None;
            let mut r = match &fp.proof {
                Some(pp) if pp.table.is_some() => {
                    let (q, dist) = pp.table.as_ref().map_or((d, 0.0), |t| t.sample(d));
                    if pp.dest_warning && dist > crate::output::PROOF_GAMUT_TOLERANCE {
                        warn = Some(crate::output::PROOF_DEST_WARNING);
                    }
                    mul3(to_out, q)
                }
                Some(pp) => {
                    let q0 = mul3(&pp.to_proof, d);
                    let (q, t) = gamut_map_to(q0, pp.luma, peak);
                    if pp.dest_warning && out_of_gamut(q0, t) {
                        warn = Some(crate::output::PROOF_DEST_WARNING);
                    }
                    mul3(&pp.proof_to_out, q)
                }
                None => mul3(to_out, d),
            };
            let (mapped, t) = gamut_map_to(r, *out_luma, peak);
            if fp.proof.as_ref().is_some_and(|pp| pp.display_warning) && warn.is_none() && out_of_gamut(r, t) {
                warn = Some(crate::output::PROOF_DISPLAY_WARNING);
            }
            r = mapped;

            // --- encode, curves, grain
            let mut e = if hdr {
                // HDR: the (unbounded) sRGB curve, so values above SDR white encode above 1
                r.map(|v| {
                    if v > 1.0 {
                        linear_to_srgb(v)
                    } else if exact {
                        linear_to_srgb(v.max(0.0))
                    } else {
                        encode_srgb(srgb, v)
                    }
                })
            } else if exact {
                r.map(|v| linear_to_srgb(v.clamp(0.0, 1.0)))
            } else {
                r.map(|v| encode_srgb(srgb, v))
            };
            if let Some(l) = curves {
                let e0 = e;
                e = if hdr {
                    // above SDR white the curves continue with slope 1 from their end point
                    std::array::from_fn(|k| if e[k] > 1.0 { l[k].eval(1.0) + (e[k] - 1.0) } else { l[k].eval(e[k]) })
                } else {
                    [l[0].eval(e[0]), l[1].eval(e[1]), l[2].eval(e[2])]
                };
                if fp.refine_sat < 1.0 {
                    e = refine_saturation(e0, e, fp.refine_sat);
                }
            }
            match grain {
                Some(GrainModel::Value(g)) => {
                    // V1/V2 (frozen): exactly the arithmetic those processes always had
                    let n = out_to_norm.apply(Point::new(x as f64 + 0.5, y as f64 + 0.5));
                    let (gx, gy) = ((n.x * fp.ow) as f32 / long as f32, (n.y * fp.oh) as f32 / long as f32);
                    let sc = fp.px_per_long as f32 / g.cell;
                    let mut v = grain_noise(gx * sc, gy * sc, g.seed);
                    v = v * (1.0 - g.rough * 0.5) + grain_noise(gx * sc * 2.3, gy * sc * 2.3, g.seed ^ 0x55) * g.rough * 0.7;
                    let mut lum = 0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2];
                    if hdr {
                        lum = lum.clamp(0.0, 1.0);
                    }
                    let k = g.amt * v * (0.35 + 2.6 * lum * (1.0 - lum));
                    if k.is_finite() {
                        e = e.map(|v| v + k);
                    }
                }
                Some(GrainModel::Crystal(g)) => match crystal_row.get(x) {
                    Some(&f) => e = g.apply_field(e, f),
                    None => {
                        let n = out_to_norm.apply(Point::new(x as f64 + 0.5, y as f64 + 0.5));
                        e = g.apply(e, (n.x * fp.ow / long) as f32, (n.y * fp.oh / long) as f32);
                    }
                },
                None => {}
            }
            let e = match &p_lut {
                Some((l, k)) if hdr => {
                    // the LUT sees SDR values; the part above SDR white is carried over
                    let c = e.map(|v| v.clamp(0.0, 1.0));
                    let m = l.apply(c);
                    [e[0] + (m[0] - c[0]) * k, e[1] + (m[1] - c[1]) * k, e[2] + (m[2] - c[2]) * k]
                }
                Some((l, k)) => {
                    let m = l.apply(e.map(|v| v.clamp(0.0, 1.0)));
                    [e[0] + (m[0] - e[0]) * k, e[1] + (m[1] - e[1]) * k, e[2] + (m[2] - e[2]) * k]
                }
                None => e,
            };
            *px = store(vx_u + x, vy_u + y, warn.unwrap_or(e));
        }
    });
    out
}

#[inline]
fn apply_light(
    color: [f32; 3],
    light: Option<&crate::light_v3::LightOps>,
    preview: Option<&crate::light_v3::PreviewOps>,
    exact: bool,
    srgb: &[f32; SRGB_LUT_N + 1],
) -> [f32; 3] {
    if let Some(lo) = preview {
        lo.apply(color)
    } else if let Some(lo) = light {
        if exact { lo.apply(color, &linear_to_srgb) } else { lo.apply(color, &|v| encode_srgb(srgb, v)) }
    } else {
        color
    }
}

struct PixelLocalTerms {
    values: [f32; MASK_SUMS],
    tint: Option<([f32; 3], f32)>,
}

fn pixel_local_terms(p: &Prepared, terms: &[[f32; MASK_TERMS]], index: usize) -> PixelLocalTerms {
    let mut values = [0.0; MASK_SUMS];
    let mut tint = None;
    for (mask, terms) in p.masks.iter().zip(terms) {
        let alpha = mask.alpha.data[index];
        if alpha <= 0.0 {
            continue;
        }
        for (sum, term) in values.iter_mut().zip(terms.iter().take(MASK_SUMS)) {
            *sum += alpha * term;
        }
        if terms[MASK_SUMS] > 0.0 {
            tint = Some(([terms[MASK_SUMS + 1], terms[MASK_SUMS + 2], 0.0], alpha * terms[MASK_SUMS + 3]));
        }
    }
    PixelLocalTerms { values, tint }
}

fn adjust_scene_pixel(p: &Prepared, fp: &FinishParams, index: usize, raw: [f32; 3], local: &PixelLocalTerms) -> ([f32; 3], f32) {
    let [l_exp, l_temp, l_tint, _, _, _, _, _, _, _, l_dehaze, _, _, _, l_noise, l_moire, l_defringe] = local.values;
    let mut color = if fp.gain == 1.0 { raw } else { raw.map(|value| value * fp.gain) };

    let moire = (l_moire + 0.5 * l_noise.max(0.0)).clamp(-1.0, 1.0);
    if moire != 0.0
        && let Some(blur) = &p.chroma_blur
    {
        let (luminance, original, blurred) = (luminance_2020(color), crate::masks::chromaticity(raw), blur.data[index]);
        color = std::array::from_fn(|channel| ((original[channel] + (blurred[channel] - original[channel]) * moire) * luminance).max(0.0));
    }
    let defringe = l_defringe.clamp(0.0, 1.0);
    if defringe > 0.0
        && let Some(blur) = &p.texture_blur
    {
        let amount = defringe * defringe_weight(color, p.log_l.data[index] - blur.data[index]);
        if amount > 0.0 {
            let luminance = luminance_2020(color);
            color = color.map(|value| value + (luminance - value) * amount);
        }
    }

    let dehaze = fp.dehaze + l_dehaze;
    if dehaze != 0.0
        && let Some(dark) = &p.dark
    {
        // Limit the estimated veil to the pixel's own minimum channel. This avoids crushing bright
        // neighbours when the dark-channel blur spans the edge of a local window.
        let min_channel = color[0].min(color[1]).min(color[2]);
        let veil = (if fp.air_pre > 0.0 { dark.data[index] / fp.air_pre } else { 0.0 })
            .min(if fp.air > 0.0 { min_channel / fp.air } else { 0.0 })
            .clamp(0.0, 1.0);
        if dehaze > 0.0 {
            let transmission = (1.0 - 0.95 * dehaze.min(1.0) * veil).max(0.12);
            color = color.map(|value| ((value - fp.air * (1.0 - transmission)) / transmission).max(0.0));
        } else {
            let amount = (-dehaze).min(1.0) * 0.7 * (0.35 + 0.65 * veil);
            color = color.map(|value| value + (fp.air * 0.9 - value) * amount);
        }
    }

    if l_exp != 0.0 {
        let gain = l_exp.exp2();
        color = color.map(|value| value * gain);
    }
    if l_temp != 0.0 || l_tint != 0.0 {
        let luminance_before = luminance_2020(color);
        color = [color[0] * (1.0 + 0.3 * l_temp), color[1] * (1.0 - 0.22 * l_tint), color[2] * (1.0 - 0.3 * l_temp).max(0.0)];
        let luminance_after = luminance_2020(color).max(1e-9);
        color = color.map(|value| value * luminance_before / luminance_after);
    }

    let l0 = p.log_l.data[index] + fp.ev;
    let log_luminance = if dehaze != 0.0 || l_exp != 0.0 { log_lum(color) } else { l0 };
    (color, log_luminance)
}

fn local_tone_delta(p: &Prepared, fp: &FinishParams, index: usize, base: f32, l1: f32, local: &PixelLocalTerms) -> f32 {
    let [_, _, _, l_con, l_hl, l_sh, l_wh, l_bl, l_tex, l_clar, _, _, _, l_sharp, l_noise, _, _] = local.values;
    let mut delta = 0.0;
    let (highlights, shadows) = (fp.hl + l_hl, fp.sh + l_sh);
    if highlights != 0.0 || shadows != 0.0 {
        let shadow_weight = 1.0 - smooth(-4.8, 0.3, base);
        let highlight_weight = smooth(-1.0, 2.8, base);
        delta += shadows * 1.7 * shadow_weight * shadow_weight.sqrt() + highlights * 1.7 * highlight_weight;
    }
    if l_wh != 0.0 {
        delta += l_wh * 0.8 * smooth(0.5, 3.0, l1);
    }
    if l_bl != 0.0 {
        delta += l_bl * 0.8 * (1.0 - smooth(-6.0, -1.5, l1));
    }
    if l_con != 0.0 {
        delta += l_con * 0.14 * l1.clamp(-6.0, 4.0);
    }
    let clarity = fp.clar + l_clar;
    if clarity != 0.0
        && let Some(blur) = &p.clarity_blur
    {
        let detail = (p.log_l.data[index] - blur.data[index]).clamp(-2.5, 2.5);
        let midtone = (-(base / 3.2).powi(2)).exp();
        delta += clarity * 0.85 * detail * (0.35 + 0.65 * midtone);
    }
    let texture = fp.tex + l_tex;
    if texture != 0.0
        && let Some(blur) = &p.texture_blur
    {
        let detail = p.log_l.data[index] - blur.data[index];
        let tame = 1.0 - 0.6 * smooth(0.4, 1.6, detail.abs());
        delta += texture * 1.1 * detail.clamp(-1.0, 1.0) * tame;
    }
    let sharpen = l_sharp * 0.6 + fp.sharpen;
    if fp.pixel_sharpen {
        if sharpen != 0.0
            && let Some(blur) = &p.sharpen_blur
        {
            delta += sharpen * fp.sharpen_gain * sharpen_term(p.log_l.data[index] - blur.data[index], fp.sharpen_limit, fp.sharpen_mask);
        }
    } else if sharpen != 0.0
        && let Some(blur) = &p.texture_blur
    {
        let detail = p.log_l.data[index] - blur.data[index];
        let mask = sharpen_mask_weight(fp.sharpen_mask, detail);
        delta += sharpen * 1.3 * detail.clamp(-0.8, 0.8) * mask;
    }
    if l_noise != 0.0
        && let Some(blur) = &p.texture_blur
    {
        let detail = p.log_l.data[index] - blur.data[index];
        delta -= l_noise.clamp(-1.0, 1.0) * 0.9 * detail * (1.0 - smooth(0.1, 0.5, detail.abs()));
    }
    delta
}

/// A colour needing more desaturation than this (scale < `GAMUT_WARN`) to fit is out of gamut for
/// the gamut warnings (tolerates rounding at the gamut boundary).
pub const GAMUT_WARN: f32 = 0.995;

/// For the gamut warnings: `r` needed desaturating by `t` to fit, and isn't simply a neutral
/// beyond white or below black (that's clipping, which the clipping warnings show).
#[inline]
fn out_of_gamut(r: [f32; 3], t: f32) -> bool {
    let (lo, hi) = (r[0].min(r[1]).min(r[2]), r[0].max(r[1]).max(r[2]));
    t < GAMUT_WARN && (hi - lo) > 0.02 * hi.abs().max(1e-3) && hi - lo > 1e-3 && (lo < -1e-3 || (hi > 1.0 && lo < 1.0))
}

#[inline]
fn mul3(m: &[[f32; 3]; 3], d: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * d[0] + m[0][1] * d[1] + m[0][2] * d[2],
        m[1][0] * d[0] + m[1][1] * d[1] + m[1][2] * d[2],
        m[2][0] * d[0] + m[2][1] * d[1] + m[2][2] * d[2],
    ]
}

/// Desaturate linear `r` towards its luminance (weights `luma`) until every channel is in 0..1;
/// returns the mapped colour and the chroma scale used (1 = already in gamut).
#[inline]
pub fn gamut_map(r: [f32; 3], luma: [f32; 3]) -> ([f32; 3], f32) {
    gamut_map_to(r, luma, 1.0)
}

/// [`gamut_map`] into `0..peak` (an HDR render's range; `peak` = 1 is [`gamut_map`]).
#[inline]
pub fn gamut_map_to(r: [f32; 3], luma: [f32; 3], peak: f32) -> ([f32; 3], f32) {
    let yy = (luma[0] * r[0] + luma[1] * r[1] + luma[2] * r[2]).clamp(0.0, peak);
    let mut t = 1.0f32;
    for c in r {
        if c < 0.0 {
            t = t.min(yy / (yy - c).max(1e-9));
        } else if c > peak {
            t = t.min((peak - yy) / (c - yy).max(1e-9));
        }
    }
    if t < 1.0 { (r.map(|c| yy + (c - yy) * t), t) } else { (r, 1.0) }
}

/// Local Defringe weight of a scene-linear colour `c` whose log luminance differs by `det` from
/// its fine blur: high on strong edges with a purple or green cast.
#[inline]
pub fn defringe_weight(c: [f32; 3], det: f32) -> f32 {
    let y = luminance_2020(c).max(1e-6);
    let purple = (c[0].min(c[2]) - c[1]) / y;
    let green = (c[1] - c[0].max(c[2])) / y;
    smooth(0.04, 0.3, det.abs()) * smooth(0.02, 0.2, purple).max(smooth(0.02, 0.2, green))
}

/// Refine Saturation: scale the curved colour's chroma (around its luma, encoded values) so its
/// saturation (chroma / luma) moves from the curve's towards the pre-curve one: the ratio is
/// `(s_curve / s_before)^refine`, so 1 keeps the curve, 0 restores the original saturation.
#[inline]
pub fn refine_saturation(before: [f32; 3], after: [f32; 3], refine: f32) -> [f32; 3] {
    let luma = |e: [f32; 3]| 0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2];
    let chroma = |e: [f32; 3]| e[0].max(e[1]).max(e[2]) - e[0].min(e[1]).min(e[2]);
    let (y0, y1) = (luma(before), luma(after));
    let s0 = chroma(before) / y0.max(1e-4);
    let s1 = chroma(after) / y1.max(1e-4);
    if s1 <= 1e-6 || s0 <= 1e-6 {
        return after;
    }
    let k = (s0 / s1).powf(1.0 - refine).clamp(0.0, 4.0);
    after.map(|v| y1 + (v - y1) * k)
}

pub const SRGB_LUT_N: usize = 4096;

/// Linear → sRGB-encoded table (`SRGB_LUT_N + 1` entries over 0..1), interpolated linearly.
pub fn srgb_lut() -> &'static [f32; SRGB_LUT_N + 1] {
    static LUT: std::sync::OnceLock<Box<[f32; SRGB_LUT_N + 1]>> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        let mut t = Box::new([0.0f32; SRGB_LUT_N + 1]);
        for (i, v) in t.iter_mut().enumerate() {
            *v = linear_to_srgb(i as f32 / SRGB_LUT_N as f32);
        }
        t
    })
}

/// Linear → sRGB-encoded (clamped to 0..1) by an interpolated table: within 2e-5 of the exact
/// curve (≪ one 8-bit or 10-bit step), several times faster than `powf`.
#[inline]
pub(crate) fn encode_srgb(lut: &[f32; SRGB_LUT_N + 1], v: f32) -> f32 {
    let f = v.clamp(0.0, 1.0) * SRGB_LUT_N as f32;
    let i = (f as usize).min(SRGB_LUT_N - 1);
    let t = f - i as f32;
    lut[i] + (lut[i + 1] - lut[i]) * t
}

#[rustfmt::skip]
const BAYER_8X8: [u8; 64] = [
     0, 32,  8, 40,  2, 34, 10, 42,
    48, 16, 56, 24, 50, 18, 58, 26,
    12, 44,  4, 36, 14, 46,  6, 38,
    60, 28, 52, 20, 62, 30, 54, 22,
     3, 35, 11, 43,  1, 33,  9, 41,
    51, 19, 59, 27, 49, 17, 57, 25,
    15, 47,  7, 39, 13, 45,  5, 37,
    63, 31, 55, 23, 61, 29, 53, 21,
];

/// Returns the ordered Bayer 8×8 dither threshold offset in `0..1` for pixel `(x, y)`.
#[inline]
fn bayer_dither_offset(x: usize, y: usize) -> f32 {
    let idx = (y & 7) * 8 + (x & 7);
    (BAYER_8X8[idx] as f32 + 0.5) / 64.0
}

/// Quantizes display-curve-encoded value `v` (0..1) to 8-bit integer with 8×8 ordered Bayer dithering.
#[inline]
fn enc(v: f32, dither: f32) -> u8 {
    let y = v.clamp(0.0, 1.0) * 255.0;
    // Guard against floating point rounding error (e.g. sRGB <-> Rec.2020 matrix conversion)
    // where numerical white (1.0) evaluates to ~254.95.
    if y >= 254.9 {
        return 255;
    }
    (y + dither).clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grain_noise_is_finite_for_degenerate_coordinates() {
        for (x, y) in [(f32::NAN, 1.0), (1.0, f32::INFINITY), (f32::NEG_INFINITY, f32::NAN)] {
            assert_eq!(grain_noise(x, y, 7), 0.0);
        }
    }

    #[test]
    fn refine_saturation_restores_the_pre_curve_saturation() {
        let before = [0.5, 0.3, 0.2];
        let after = [0.7, 0.35, 0.15]; // a contrasty curve: more saturated
        assert_eq!(refine_saturation(before, after, 1.0), after);
        let r = refine_saturation(before, after, 0.0);
        let sat = |e: [f32; 3]| (e[0] - e[2]) / (0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2]);
        assert!((sat(r) - sat(before)).abs() < 1e-4, "{r:?}");
        let half = refine_saturation(before, after, 0.5);
        assert!(sat(half) > sat(before) && sat(half) < sat(after));
        // luma is kept
        let y = |e: [f32; 3]| 0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2];
        assert!((y(r) - y(after)).abs() < 1e-5);
    }

    #[test]
    fn srgb_table_matches_the_exact_curve() {
        let lut = srgb_lut();
        let mut worst = 0.0f32;
        for i in 0..=200_000 {
            // dense near black, where the curve bends most
            let v = (i as f32 / 200_000.0).powi(3);
            worst = worst.max((encode_srgb(lut, v) - linear_to_srgb(v)).abs());
        }
        assert!(worst < 2e-5, "{worst}");
        assert_eq!(encode_srgb(lut, -1.0), 0.0);
        assert!((encode_srgb(lut, 2.0) - 1.0).abs() < 1e-6);
    }

    /// Verifies 8x8 Bayer dithering preserves endpoints, exact 8-bit quantization steps, and fractional levels.
    #[test]
    fn bayer_dithering_preserves_extremes_and_dithers_smoothly() {
        for y in 0..8 {
            for x in 0..8 {
                let d = bayer_dither_offset(x, y);
                // Zero is strictly 0 across all dither offsets
                assert_eq!(enc(0.0, d), 0);
                // One is strictly 255 across all dither offsets
                assert_eq!(enc(1.0, d), 255);
                // Exact 8-bit quantization steps have zero spatial noise
                for k in 0..=255 {
                    assert_eq!(enc(k as f32 / 255.0, d), k);
                }
            }
        }

        // Fractional level: 128.5 / 255.0 should dither equally between 128 and 129
        let v = 128.5 / 255.0;
        let mut sum = 0u32;
        let mut count_128 = 0;
        let mut count_129 = 0;
        for y in 0..8 {
            for x in 0..8 {
                let val = enc(v, bayer_dither_offset(x, y));
                sum += val as u32;
                if val == 128 {
                    count_128 += 1;
                } else if val == 129 {
                    count_129 += 1;
                }
            }
        }
        assert_eq!(count_128, 32);
        assert_eq!(count_129, 32);
        assert_eq!(sum, 32 * 128 + 32 * 129);

        // Fractional highlight level: 254.5 / 255.0 should dither equally between 254 and 255
        let v = 254.5 / 255.0;
        let mut sum_hl = 0u32;
        let mut count_254 = 0;
        let mut count_255 = 0;
        for y in 0..8 {
            for x in 0..8 {
                let val = enc(v, bayer_dither_offset(x, y));
                sum_hl += val as u32;
                if val == 254 {
                    count_254 += 1;
                } else if val == 255 {
                    count_255 += 1;
                }
            }
        }
        assert_eq!(count_254, 32);
        assert_eq!(count_255, 32);
        assert_eq!(sum_hl, 32 * 254 + 32 * 255);
    }
}
