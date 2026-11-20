//! K-only black preservation for CMYK → CMYK transforms.
//!
//! A CMYK → CMYK transform goes through the three-component PCS, so black alone (C = M = Y = 0)
//! comes out as a four-ink mix. ISO 32000-2 §8.6.5.7 notes that this loses fidelity in the black
//! component. ICC defines no black preservation; this is a named policy the caller opts into.
//!
//! [`BlackPreserving`] converts like its [`Transform`], except that a colour with exactly zero
//! cyan, magenta and yellow becomes exactly `(0, 0, 0, K')`. `K'` is the destination black whose
//! K-only L* equals the source's, both measured to Lab with the transform's own intent and BPC
//! setting: both ramps are sampled once, at construction, and the destination ramp is inverted.
//! A source black darker than the destination's maximum black maps to K = 1. The test is exact zero, so the transform is discontinuous at C = M = Y = 0: an input
//! with any chromatic ink takes the ordinary path.

use crate::transform::{Transform, TransformOptions};
use crate::{Builtin, CmsError, ColorSpace, Intent, Profile, invalid_arg};

/// Samples along each K-only ramp. Linear interpolation between samples of a smooth ramp is far
/// below the 16-bit quantum.
const K_SAMPLES: usize = 1024;
/// Slack for the monotonicity checks (L* units, K units).
const MONOTONIC_EPS: f64 = 1e-6;

/// A CMYK → CMYK transform that keeps black-only colours black-only.
#[derive(Debug)]
pub struct BlackPreserving {
    transform: Transform,
    /// Destination K for source K at `i / (K_SAMPLES - 1)`; `K_SAMPLES` values in `[0, 1]`.
    curve: Vec<f32>,
    /// Source K from which the source black is at least as dark as the destination's darkest:
    /// mapped to exactly 1. Infinite when the destination's black is the darker.
    saturate: f32,
    /// The mapping at the 256 8-bit source codes, already quantised.
    curve8: [u8; 256],
}

impl BlackPreserving {
    /// Transform from `src` to `dst` with black preservation.
    ///
    /// Errors with [`CmsError::InvalidArgument`] when either profile is not CMYK or the intent is
    /// absolute colorimetric (which reproduces the source's paper and black, so there is no
    /// black to map), and with [`CmsError::Unsupported`] when a profile's K-only ramp is not
    /// monotonic at its 1024 samples (black that gets lighter as K rises has no equal-lightness
    /// inverse; a reversal between samples is not detected). Other errors
    /// are those of [`Transform::with_options`].
    pub fn new(src: &Profile, dst: &Profile, opts: TransformOptions) -> Result<BlackPreserving, CmsError> {
        if src.color_space != ColorSpace::Cmyk || dst.color_space != ColorSpace::Cmyk {
            return Err(invalid_arg("black preservation needs CMYK source and destination profiles"));
        }
        if opts.intent == Intent::AbsoluteColorimetric {
            return Err(invalid_arg("black preservation does not apply to absolute colorimetric"));
        }
        let transform = Transform::with_options(src, dst, opts)?;
        let src_l = k_ramp_lightness(src, opts)?;
        let dst_l = k_ramp_lightness(dst, opts)?;
        let (curve, saturate) = k_curve(&src_l, &dst_l)?;
        let mut curve8 = [0u8; 256];
        for (code, slot) in curve8.iter_mut().enumerate() {
            *slot = quantise(map(&curve, saturate, code as f32 / 255.0), 255.0) as u8;
        }
        Ok(BlackPreserving { transform, curve, saturate, curve8 })
    }

    /// The ordinary transform, which every colour with chromatic ink goes through. Converting with
    /// it directly does not preserve black.
    pub fn transform(&self) -> &Transform {
        &self.transform
    }

    /// Destination K for a black-only source colour of `k` (clamped to `[0, 1]`).
    pub fn map_k(&self, k: f32) -> f32 {
        map(&self.curve, self.saturate, k)
    }

    /// Exact evaluation of one colour; slices as in [`Transform::eval`].
    pub fn eval(&self, input: &[f32], out: &mut [f32]) -> Result<(), CmsError> {
        self.transform.eval(input, out)?;
        self.patch_one(input, out);
        Ok(())
    }

    /// Lookup-table evaluation of one colour; slices as in [`Transform::eval`].
    pub fn eval_fast(&self, input: &[f32], out: &mut [f32]) -> Result<(), CmsError> {
        self.transform.eval_fast(input, out)?;
        self.patch_one(input, out);
        Ok(())
    }

    /// Converts float pixels; layout and errors as in [`Transform::convert_f32`].
    pub fn convert_f32(&self, src: &[f32], src_stride: usize, dst: &mut [f32], dst_stride: usize, copy_extra: bool) -> Result<(), CmsError> {
        self.transform.convert_f32(src, src_stride, dst, dst_stride, copy_extra)?;
        for (s, d) in src.chunks_exact(src_stride).zip(dst.chunks_exact_mut(dst_stride)) {
            self.patch_one(s, d);
        }
        Ok(())
    }

    /// Converts 8-bit pixels; layout and errors as in [`Transform::convert_u8`].
    pub fn convert_u8(&self, src: &[u8], src_stride: usize, dst: &mut [u8], dst_stride: usize, copy_extra: bool) -> Result<(), CmsError> {
        self.transform.convert_u8(src, src_stride, dst, dst_stride, copy_extra)?;
        for (s, d) in src.chunks_exact(src_stride).zip(dst.chunks_exact_mut(dst_stride)) {
            if let ([0, 0, 0, k, ..], [c, m, y, kd, ..]) = (s, d) {
                (*c, *m, *y) = (0, 0, 0);
                *kd = self.curve8[usize::from(*k)];
            }
        }
        Ok(())
    }

    /// Converts 16-bit pixels; layout and errors as in [`Transform::convert_u16`].
    pub fn convert_u16(&self, src: &[u16], src_stride: usize, dst: &mut [u16], dst_stride: usize, copy_extra: bool) -> Result<(), CmsError> {
        self.transform.convert_u16(src, src_stride, dst, dst_stride, copy_extra)?;
        for (s, d) in src.chunks_exact(src_stride).zip(dst.chunks_exact_mut(dst_stride)) {
            if let ([0, 0, 0, k, ..], [c, m, y, kd, ..]) = (s, d) {
                (*c, *m, *y) = (0, 0, 0);
                *kd = quantise(self.map_k(f32::from(*k) / 65535.0), 65535.0) as u16;
            }
        }
        Ok(())
    }

    fn patch_one(&self, input: &[f32], out: &mut [f32]) {
        if let ([0.0, 0.0, 0.0, k, ..], [c, m, y, kd, ..]) = (input, out) {
            (*c, *m, *y) = (0.0, 0.0, 0.0);
            *kd = self.map_k(*k);
        }
    }
}

/// The sampled mapping from the source to the destination K-only ramp, and the source K at
/// which the destination's darkest black is reached (the `saturate` field).
fn k_curve(src_l: &[f64], dst_l: &[f64]) -> Result<(Vec<f32>, f32), CmsError> {
    let curve: Vec<f32> = src_l.iter().map(|&l| invert(dst_l, l) as f32).collect();
    if !curve.windows(2).all(|w| f64::from(w[0]) - f64::from(w[1]) <= MONOTONIC_EPS) {
        return Err(CmsError::Unsupported("black preservation: the K mapping is not monotonic".into()));
    }
    let saturate = match (src_l.last(), dst_l.last()) {
        (Some(&s), Some(&d)) if s < d => invert(src_l, d) as f32,
        _ => f32::INFINITY,
    };
    Ok((curve, saturate))
}

/// L* of `p`'s K-only ramp at `K_SAMPLES` points, light to dark; refused when it is not
/// non-increasing.
fn k_ramp_lightness(p: &Profile, opts: TransformOptions) -> Result<Vec<f64>, CmsError> {
    let to_lab = Transform::with_options(p, Builtin::LabD50.profile(), TransformOptions { precise_float: true, ..opts })?;
    let mut ramp = Vec::with_capacity(K_SAMPLES);
    let mut lab = [0.0f32; 3];
    for i in 0..K_SAMPLES {
        let k = i as f32 / (K_SAMPLES - 1) as f32;
        to_lab.eval(&[0.0, 0.0, 0.0, k], &mut lab)?;
        ramp.push(f64::from(lab[0]) * 100.0);
    }
    if !ramp.iter().all(|l| l.is_finite()) || !ramp.windows(2).all(|w| w[1] - w[0] <= MONOTONIC_EPS) {
        return Err(CmsError::Unsupported("black preservation: a K-only ramp is not monotonic".into()));
    }
    Ok(ramp)
}

/// The K in `[0, 1]` at which the non-increasing `ramp` reaches `target`, clamped to its ends.
fn invert(ramp: &[f64], target: f64) -> f64 {
    let (Some(&first), Some(&last)) = (ramp.first(), ramp.last()) else { return 0.0 };
    if ramp.len() < 2 || target >= first {
        return 0.0;
    }
    if target <= last {
        return 1.0;
    }
    // ramp[lo] > target >= ramp[hi]
    let (mut lo, mut hi) = (0usize, ramp.len() - 1);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if ramp.get(mid).is_some_and(|&v| v > target) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let (a, b) = (ramp.get(lo).copied().unwrap_or(first), ramp.get(hi).copied().unwrap_or(last));
    let t = if a - b > MONOTONIC_EPS { (a - target) / (a - b) } else { 0.0 };
    (lo as f64 + t) / (ramp.len() - 1) as f64
}

/// Linear interpolation in `curve` (samples over `[0, 1]`) at `k`, clamped; 1 from `saturate`.
fn map(curve: &[f32], saturate: f32, k: f32) -> f32 {
    let Some(&last) = curve.last() else { return 0.0 };
    let k = if k.is_nan() { 0.0 } else { k.clamp(0.0, 1.0) };
    if k >= saturate {
        return 1.0;
    }
    let x = k * (curve.len() - 1) as f32;
    let i = x.floor() as usize;
    match (curve.get(i), curve.get(i + 1)) {
        (Some(&a), Some(&b)) => a + (b - a) * (x - i as f32),
        _ => last,
    }
}

fn quantise(v: f32, max: f32) -> f32 {
    (v.clamp(0.0, 1.0) * max + 0.5).floor()
}

#[cfg(test)]
#[path = "black_tests.rs"]
mod tests;
