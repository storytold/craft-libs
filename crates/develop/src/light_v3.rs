//! Process V4's Contrast and Blacks on the camera-look path (a raw with its camera's tone
//! curve): pointwise operators fitted to how Lightroom's sliders move a photo's tones, applied
//! after the camera curve with V2 Whites. Method, operator family and the fitted constants' provenance:
//! `docs/light-v3.md`; the constants are in [`crate::light_v3_params`] (generated).
//!
//! Each slider is an operator `F_a` on encoded values `v` (linear ProPhoto RGB, D50, Bradford from
//! the D65 working space, through the sRGB transfer curve), anchored at `F_0` = identity:
//!
//! - `a > 0`: `F_a(v) = v + s(a)·D₊(v)`; `a < 0`: `F_a(v) = max(0, v − s(a)·D₋(v))`, with `D` piecewise
//!   linear on 12 fixed knots (`D(1) = 0`, and for Contrast `D(0) = 0`) and the amount law
//!   `s(t) = ((1 − e^(−t/τ)) / (1 − e^(−1/τ)))^q`, `t = |a|/100`, one `(τ, q)` per sign;
//! - applied per channel ([`Mode::PerChannel`]), to the encoded ProPhoto luminance with linear RGB
//!   scaled by the luminance ratio ([`Mode::Luminance`]), or a blend of the two in linear light
//!   ([`Mode::Mix`], one weight per sign).
//!
//! The functions here port `Family.curve`, `Family.apply`, `Family.law` of
//! `fit_pointwise.py` (the fitting tool) exactly; [`Fit::unpack`] is its `Family.unpack`. Each
//! active operator is tabulated once per render ([`LUT_N`] entries over `v` in 0..1, interpolated
//! linearly). High-bit-depth CPU exports and the GPU read these tables ([`LightOps`]); 8-bit CPU
//! renders compose per-channel operations over linear input ([`PreviewOps`]). Luminance operations
//! retain the original evaluation, including any preceding per-channel operation.

use crate::color::{Mat3, PROPHOTO, REC2020};
use craft_develop_model::{DevelopSettings, Process};

use crate::SourceInfo;
use crate::light_v3_params as fitted;

/// Entries of an operator's table over encoded `v` in 0..=1.
pub const LUT_N: usize = 1024;

/// Linear ProPhoto RGB luminance (the Y row of ProPhoto → XYZ D50), as the fit computes it.
pub const PRO_Y: [f32; 3] = [0.288_040_2, 0.711_874_1, 0.000_085_7];

/// How an operator is applied to a pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// To each encoded ProPhoto channel.
    PerChannel,
    /// To the encoded ProPhoto luminance; linear RGB is scaled by the luminance ratio.
    Luminance,
    /// `w·PerChannel + (1 − w)·Luminance` in linear light, with a weight per sign.
    Mix,
}

impl Mode {
    /// The GPU kernel's code (`LIGHT_MODE` in `crates/gpu/src/params.rs`).
    pub fn code(self) -> u32 {
        match self {
            Mode::PerChannel => 0,
            Mode::Luminance => 1,
            Mode::Mix => 2,
        }
    }
}

/// One slider's fitted parameters, laid out as `fit_pointwise.py`'s `Family`: 11 values of `D₊` at
/// the knots but the last (`D(1) = 0`), 11 of `D₋`, then `ln τ₊, ln q₊, ln τ₋, ln q₋`, then two mix
/// logits (positive, negative amounts) in [`Mode::Mix`].
#[derive(Clone, Copy, Debug)]
pub struct Fit {
    pub mode: Mode,
    /// Contrast moves tones both ways around a pivot: its shapes are anchored at black too (`D(0) = 0`).
    pub signed: bool,
    pub params: &'static [f64],
}

const KNOTS_N: usize = fitted::KNOTS.len();

/// A [`Fit`] unpacked (`Family.unpack`).
#[derive(Clone, Copy, Debug)]
pub struct Unpacked {
    pub d_pos: [f64; KNOTS_N],
    pub d_neg: [f64; KNOTS_N],
    /// `(τ, q)` of the amount law, positive and negative amounts.
    pub law_pos: (f64, f64),
    pub law_neg: (f64, f64),
    /// Mix weights of the per-channel result for positive and negative amounts (1 outside [`Mode::Mix`]).
    pub weights: (f64, f64),
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

impl Fit {
    /// The parameters unpacked; `None` when their number doesn't fit the layout.
    pub fn unpack(&self) -> Option<Unpacked> {
        let n = KNOTS_N - 1;
        let p = self.params;
        let mix = self.mode == Mode::Mix;
        let len = 2 * n + 4 + if mix { 2 } else { 0 };
        if p.len() != len {
            return None;
        }
        // D(1) = 0 is appended (the last knot stays 0)
        let shape = |off: usize| -> Option<[f64; KNOTS_N]> {
            let mut d = [0.0; KNOTS_N];
            for (k, slot) in d.iter_mut().take(n).enumerate() {
                *slot = *p.get(off + k)?;
            }
            Some(d)
        };
        let (mut d_pos, mut d_neg) = (shape(0)?, shape(n)?);
        if self.signed {
            d_pos[0] = 0.0;
            d_neg[0] = 0.0;
        }
        let at = |i: usize| p.get(i).copied();
        let i = 2 * n;
        let law_pos = (at(i)?.exp(), at(i + 1)?.exp());
        let law_neg = (at(i + 2)?.exp(), at(i + 3)?.exp());
        let weights = if mix { (sigmoid(at(i + 4)?), sigmoid(at(i + 5)?)) } else { (1.0, 1.0) };
        Some(Unpacked { d_pos, d_neg, law_pos, law_neg, weights })
    }
}

/// The amount law `s(t)` (`Family.law`): 1 at `|a| = 100`, linear as `τ` grows, saturating as it
/// shrinks.
pub fn law((tau, q): (f64, f64), a: f64) -> f64 {
    let t = (a.abs() / 100.0).min(1.0);
    let tau = tau.clamp(1e-3, 1e3);
    ((1.0 - (-t / tau).exp()) / (1.0 - (-1.0 / tau).exp())).powf(q)
}

/// `numpy.interp(v, KNOTS, d)`: piecewise linear, constant beyond the ends.
fn interp(v: f64, d: &[f64; KNOTS_N]) -> f64 {
    let k = &fitted::KNOTS;
    if v <= k[0] {
        return d[0];
    }
    for i in 1..KNOTS_N {
        if v <= k[i] {
            let t = (v - k[i - 1]) / (k[i] - k[i - 1]);
            return d[i - 1] + t * (d[i] - d[i - 1]);
        }
    }
    d[KNOTS_N - 1]
}

impl Unpacked {
    /// `F_a(v)` on an encoded value (`Family.curve`).
    pub fn curve(&self, a: f64, v: f64) -> f64 {
        if a == 0.0 {
            return v;
        }
        if a > 0.0 {
            return v + law(self.law_pos, a) * interp(v, &self.d_pos);
        }
        (v - law(self.law_neg, a) * interp(v, &self.d_neg)).max(0.0)
    }
}

/// One slider's operator at one amount, tabulated: `lut[i]` is the *linear* value of
/// `F_a(i / (LUT_N − 1))` (decoded with the sRGB curve), made non-decreasing (see [`Operator::new`]).
#[derive(Clone, Debug)]
pub struct Operator {
    pub mode: Mode,
    /// Weight of the per-channel result ([`Mode::Mix`]; 1 for per channel, 0 for luminance).
    pub weight: f32,
    pub lut: Box<[f32; LUT_N]>,
}

impl Operator {
    /// `fit` at `amount` (-100..100); `None` at 0 (the identity) or for parameters that do not unpack.
    ///
    /// The table is the running maximum of `F_a`: where a fitted shape folds back (a `D` rising
    /// faster than 1), it is flat instead, so no two tones swap. None of the shipped fits folds.
    /// The fitting tool inverts `F` the same way.
    pub fn new(fit: &Fit, amount: f64) -> Option<Operator> {
        let a = if amount.is_finite() { amount.clamp(-100.0, 100.0) } else { 0.0 };
        if a == 0.0 {
            return None;
        }
        let u = fit.unpack()?;
        let weight = match fit.mode {
            Mode::PerChannel => 1.0,
            Mode::Luminance => 0.0,
            Mode::Mix => (if a > 0.0 { u.weights.0 } else { u.weights.1 }) as f32,
        };
        let mut lut = Box::new([0.0f32; LUT_N]);
        let mut top = 0.0f64;
        for (i, slot) in lut.iter_mut().enumerate() {
            let v = i as f64 / (LUT_N - 1) as f64;
            top = top.max(u.curve(a, v));
            *slot = srgb_decode(top) as f32;
        }
        // The negative Blacks fit rounds to exactly 1 in f32. Avoid calculating a luminance
        // result that contributes nothing, on both CPU and GPU.
        let mode = if weight == 1.0 {
            Mode::PerChannel
        } else if weight == 0.0 {
            Mode::Luminance
        } else {
            Mode::Mix
        };
        Some(Operator { mode, weight, lut })
    }

    /// The table at encoded `v` (clamped to 0..=1), interpolated: linear output.
    #[inline]
    pub fn eval(&self, v: f32) -> f32 {
        let f = v.clamp(0.0, 1.0) * (LUT_N - 1) as f32;
        let i = (f as usize).min(LUT_N - 2);
        let t = f - i as f32;
        self.lut[i] + (self.lut[i + 1] - self.lut[i]) * t
    }

    /// One linear value through the operator: values above 1 (HDR renders) pass unchanged, which
    /// is continuous because `F_a(1) = 1`; values below 0 (outside ProPhoto's gamut) keep their
    /// distance below the image of 0, `F_a(0) + x` (continuous and increasing). This is the
    /// per-channel domain: on luminance ([`Self::apply`]) a component above 1 is scaled with its
    /// pixel unless the pixel's luminance is above 1.
    #[inline]
    pub fn channel(&self, x: f32, encode: &impl Fn(f32) -> f32) -> f32 {
        if x > 1.0 {
            x
        } else if x < 0.0 {
            self.lut[0] + x
        } else {
            self.eval(encode(x))
        }
    }

    /// The luminance mode: linear ProPhoto `p` scaled by `F` of its luminance over its luminance
    /// (unchanged when the luminance is at most 1e-6, or above 1).
    #[inline]
    fn luminance(&self, p: [f32; 3], encode: &impl Fn(f32) -> f32) -> [f32; 3] {
        let y = PRO_Y[0] * p[0] + PRO_Y[1] * p[1] + PRO_Y[2] * p[2];
        if y.is_nan() || y <= 1e-6 || y > 1.0 {
            return p;
        }
        let r = self.channel(y, encode) / y;
        p.map(|c| c * r)
    }

    /// The operator on a linear ProPhoto pixel (`Family.apply`). Per channel, each component goes
    /// through [`Self::channel`] (one above 1 passes unchanged). On luminance, the pixel is scaled
    /// by `F(Y) / Y` as a whole, components above 1 included, and passes unchanged only when `Y`
    /// itself is above 1 (or at most 1e-6). Mix blends the two results in linear light, so an HDR
    /// component of a pixel with `Y ≤ 1` moves by the luminance part's share.
    #[inline]
    pub fn apply(&self, p: [f32; 3], encode: &impl Fn(f32) -> f32) -> [f32; 3] {
        match self.mode {
            Mode::PerChannel => p.map(|c| self.channel(c, encode)),
            Mode::Luminance => self.luminance(p, encode),
            Mode::Mix => {
                let (a, b) = (p.map(|c| self.channel(c, encode)), self.luminance(p, encode));
                let w = self.weight;
                std::array::from_fn(|k| w * a[k] + (1.0 - w) * b[k])
            }
        }
    }
}

/// sRGB decoding in f64 (the tables are built in f64, like the fit).
fn srgb_decode(e: f64) -> f64 {
    if e <= 0.04045 { e / 12.92 } else { ((e + 0.055) / 1.055).powf(2.4) }
}

/// Linear Rec.2020 (the working space) → linear ProPhoto RGB (D50, Bradford), and back.
pub fn prophoto_matrices() -> ([[f32; 3]; 3], [[f32; 3]; 3]) {
    let to = REC2020.to_space(&PROPHOTO);
    let from = to.inverse().unwrap_or(Mat3::IDENTITY);
    (to.to_f32(), from.to_f32())
}

/// The active operators of a render, in order (Contrast, Blacks), and the conversions to
/// and from the space they work in. Applied after the tone map, chroma scaling and highlight
/// desaturation, before the colour stage (`finish::finish_with`, the GPU's `finish.wgsl`).
#[derive(Clone, Debug)]
pub struct LightOps {
    pub ops: Vec<Operator>,
    pub to_pro: [[f32; 3]; 3],
    pub from_pro: [[f32; 3]; 3],
}

/// The fits of Contrast and Blacks, in the order they apply.
pub const FITS: [&Fit; 2] = [&fitted::CONTRAST, &fitted::BLACKS];

/// Whether `s` renders `info` with the V4 operators: process V4 or later and a raw with its camera's tone
/// curve (other sources keep the tone map's own Contrast and Blacks).
pub fn applies(s: &DevelopSettings, info: &SourceInfo) -> bool {
    let camera_look = info.raw && info.camera_tone.is_some();
    match s.process.process() {
        Process::V1 | Process::V2 | Process::V3 => false,
        Process::V4 | Process::V5 | Process::V6 | Process::V7 | Process::V8 | Process::V9 => camera_look,
    }
}

impl LightOps {
    /// The operators of `s` for `info` (see [`applies`]); `None` when they don't apply or every
    /// slider is at 0.
    pub fn new(s: &DevelopSettings, info: &SourceInfo) -> Option<LightOps> {
        if !applies(s, info) {
            return None;
        }
        let l = &s.light;
        let ops: Vec<Operator> = FITS.iter().zip([l.contrast, l.blacks]).filter_map(|(f, a)| Operator::new(f, a)).collect();
        if ops.is_empty() {
            return None;
        }
        let (to_pro, from_pro) = prophoto_matrices();
        Some(LightOps { ops, to_pro, from_pro })
    }

    /// The operators on a linear Rec.2020 pixel; `encode`: linear → sRGB-encoded on 0..1 (the
    /// exact curve or the shared table, as the render encodes).
    #[inline]
    pub fn apply(&self, d: [f32; 3], encode: &impl Fn(f32) -> f32) -> [f32; 3] {
        let mut p = mul3(&self.to_pro, d);
        for op in &self.ops {
            p = op.apply(p, encode);
        }
        mul3(&self.from_pro, p)
    }
}

/// Number of intervals over linear input for 8-bit CPU renders. Deep exports retain [`LightOps`]
/// with the exact sRGB encoder; this approximation is not used for U16 or F32 samples.
const PREVIEW_INTERVALS: usize = 16_384;

/// An operator with its sRGB encoding and decoding included in one linear-input table.
struct LinearOperator {
    black: f32,
    white: f32,
    // Each linear interval stores [intercept, slope], so a lookup needs no integer-to-float
    // conversion or subtraction of neighbouring samples in the pixel loop.
    segments: Box<[[f32; 2]; PREVIEW_INTERVALS]>,
}

impl LinearOperator {
    fn new(channel: impl Fn(f32) -> f32) -> Self {
        let black = channel(0.0);
        let mut previous = black;
        let mut segments = Box::new([[0.0; 2]; PREVIEW_INTERVALS]);
        for (i, segment) in segments.iter_mut().enumerate() {
            let next = channel((i + 1) as f32 / PREVIEW_INTERVALS as f32);
            let slope = (next - previous) * PREVIEW_INTERVALS as f32;
            let intercept = (previous as f64 - slope as f64 * (i as f64 / PREVIEW_INTERVALS as f64)) as f32;
            *segment = [intercept, slope];
            previous = next;
        }
        Self { black, white: previous, segments }
    }

    #[inline(always)]
    fn channel(&self, x: f32) -> f32 {
        if x >= 1.0 {
            if x == 1.0 { self.white } else { x }
        } else if x < 0.0 {
            self.black + x
        } else {
            let f = x * PREVIEW_INTERVALS as f32;
            let i = (f as u32).min(PREVIEW_INTERVALS as u32 - 1) as usize;
            let [intercept, slope] = self.segments[i];
            intercept + slope * x
        }
    }
}

/// CPU preview tables for per-channel operations, built once per render. Luminance operations
/// use [`LightOps`] instead, so interpolation cannot change a nearly cancelling luminance sum.
pub(crate) struct PreviewOps {
    ops: PreviewOperators,
    to_pro: [[f32; 4]; 3],
    from_pro: [[f32; 4]; 3],
}

enum PreviewOperators {
    PerChannel(LinearOperator),
    Separate(Vec<LinearOperator>),
}

impl PreviewOps {
    pub(crate) fn new(exact: &LightOps, encode: &impl Fn(f32) -> f32) -> Option<Self> {
        let ops = &exact.ops;
        // Small per-channel errors can cross the luminance cutoff at 1e-6, or be amplified
        // by F(Y) / Y. Keep every preceding operation exact too, including Contrast.
        if ops.iter().any(|o| o.mode != Mode::PerChannel) {
            return None;
        }
        // A preceding black lift could carry a negative input across zero. Only fuse when
        // preceding operators fix zero, so the composed negative extension remains F(0) + x.
        // Contrast fixes zero and negative Blacks is per channel, so their usual pair fuses.
        let fuse = ops.iter().take(ops.len().saturating_sub(1)).all(|o| o.lut[0] == 0.0);
        let ops = if fuse {
            PreviewOperators::PerChannel(LinearOperator::new(|x| ops.iter().fold(x, |x, o| o.channel(x, encode))))
        } else {
            PreviewOperators::Separate(ops.iter().map(|o| LinearOperator::new(|x| o.channel(x, encode))).collect())
        };
        let columns = |m: [[f32; 3]; 3]| std::array::from_fn(|i| [m[0][i], m[1][i], m[2][i], 0.0]);
        Some(Self { ops, to_pro: columns(exact.to_pro), from_pro: columns(exact.from_pro) })
    }

    // Keep the single-table path in the finish loop. An out-of-line call per pixel saves
    // little code here but adds register spills to this otherwise small operation.
    #[inline(always)]
    pub(crate) fn apply(&self, d: [f32; 3]) -> [f32; 3] {
        let mut p = mul_preview(&self.to_pro, d);
        match &self.ops {
            PreviewOperators::PerChannel(op) => p = [op.channel(p[0]), op.channel(p[1]), op.channel(p[2])],
            PreviewOperators::Separate(ops) => {
                for op in ops {
                    p = p.map(|c| op.channel(c));
                }
            }
        }
        mul_preview(&self.from_pro, p)
    }
}

/// Padded columns let the compiler multiply the RGB rows together in vector registers.
/// The fourth lane is unused; each RGB row keeps the same arithmetic order as `mul3`.
#[inline(always)]
fn mul_preview(m: &[[f32; 4]; 3], d: [f32; 3]) -> [f32; 3] {
    let out: [f32; 4] = std::array::from_fn(|i| m[0][i] * d[0] + m[1][i] * d[1] + m[2][i] * d[2]);
    [out[0], out[1], out[2]]
}

#[inline]
fn mul3(m: &[[f32; 3]; 3], d: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * d[0] + m[0][1] * d[1] + m[0][2] * d[2],
        m[1][0] * d[0] + m[1][1] * d[1] + m[1][2] * d[2],
        m[2][0] * d[0] + m[2][1] * d[1] + m[2][2] * d[2],
    ]
}

/// `fit_pointwise.py`'s own values for one fit at one amount (generated into
/// [`crate::light_v3_params`]; the tests check the port against them).
#[cfg(test)]
pub struct Reference {
    pub amount: f64,
    /// `Family.curve` at `light_v3_params::REF_V`.
    pub curve: &'static [f64],
    /// `Family.apply` on `light_v3_params::REF_PIX` (encoded ProPhoto), where the curve increases.
    pub apply: Option<&'static [[f64; 3]]>,
}
