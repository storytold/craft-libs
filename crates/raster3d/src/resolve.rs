//! The box resolve of a supersampled target and the sRGB encoding of its pixels.

use rayon::prelude::*;

use crate::raster::{AuxPlanes, AuxTarget, Target, try_render, try_render_with_aux};
use crate::scene::{NO_PART, Scene};
use crate::shade::linear_to_srgb;
use crate::{Limits, RenderError};

/// An image of premultiplied RGBA pixels, row-major from the top row.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<[f32; 4]>,
}

/// A resolved image with its auxiliary outputs at the image's size.
#[derive(Clone, Debug, PartialEq)]
pub struct AuxImage {
    pub image: Image,
    pub aux: AuxPlanes,
}

/// Alpha above 1 that [`try_encode`] accepts as rounding from blending.
const ALPHA_SLACK: f32 = 1e-4;

/// `w × h` as a length, when it fits `usize`.
fn area(w: u32, h: u32) -> Option<usize> {
    usize::try_from(u64::from(w) * u64::from(h)).ok()
}

/// Box-filter a supersampled target down by `k` per side: each output pixel is the mean of its
/// `k × k` premultiplied colors, and its depth is the mean of the covered subsamples' depths
/// (infinity where none is covered). A pixel whose `f32` sums overflow, and every pixel when a
/// block holds more than 2^24 subsamples, is averaged in `f64`.
///
/// Returns [`RenderError::BadFactor`] when `k` is zero or does not divide both sides, and
/// [`RenderError::BadDataLength`] when the color or depth buffer does not hold width × height
/// values.
pub fn try_resolve(t: &Target, k: u32) -> Result<(Image, Vec<f32>), RenderError> {
    if k == 0 || !t.width.is_multiple_of(k) || !t.height.is_multiple_of(k) {
        return Err(RenderError::BadFactor { k, width: t.width, height: t.height });
    }
    let expected = area(t.width, t.height).ok_or(RenderError::TooLarge(t.width, t.height))?;
    for found in [t.color.len(), t.depth.len()] {
        if found != expected {
            return Err(RenderError::BadDataLength { expected, found });
        }
    }
    let (w, h) = (t.width / k, t.height / k);
    // Fits: at most `expected`.
    let out = area(w, h).unwrap_or(0);
    let mut img = Image { width: w, height: h, data: vec![[0.0; 4]; out] };
    let mut depth = vec![f32::INFINITY; out];
    if out == 0 {
        return Ok((img, depth));
    }
    let n = (u64::from(k) * u64::from(k)) as f32;
    // An f32 count stops at 2^24, so a larger block is averaged in f64.
    let wide = u64::from(k) * u64::from(k) > 1 << 24;
    img.data.par_chunks_mut(w as usize).zip(depth.par_chunks_mut(w as usize)).enumerate().for_each(|(y, (row, drow))| {
        for x in 0..w as usize {
            if wide {
                (row[x], drow[x]) = mean_f64(t, k as usize, x, y);
                continue;
            }
            let mut acc = [0.0f32; 4];
            let (mut dz, mut dn) = (0.0f32, 0.0f32);
            for j in 0..k as usize {
                for i in 0..k as usize {
                    let s = (y * k as usize + j) * t.width as usize + x * k as usize + i;
                    let c = t.color[s];
                    for q in 0..4 {
                        acc[q] += c[q];
                    }
                    if t.depth[s].is_finite() {
                        dz += t.depth[s];
                        dn += 1.0;
                    }
                }
            }
            row[x] = acc.map(|v| v / n);
            if dn > 0.0 {
                drow[x] = dz / dn;
            }
            // Large finite samples can overflow the f32 sums; such a pixel is averaged in f64.
            if !row[x].iter().all(|v| v.is_finite()) || drow[x].is_nan() || (dn > 0.0 && drow[x].is_infinite()) {
                (row[x], drow[x]) = mean_f64(t, k as usize, x, y);
            }
        }
    });
    Ok((img, depth))
}

/// Resolve a supersampled target and its auxiliary outputs down by `k` per side. The color is
/// [`try_resolve`]'s, bit for bit. Each output pixel's depth, normal and part ID are copied
/// together from one subsample of its `k × k` block: the one with the smallest finite depth, the
/// first in row-major order on ties. A block with no surface gives ∞, `[0, 0, 0]` and
/// [`NO_PART`]; depths are not averaged and IDs are not mixed.
///
/// Returns the errors of [`try_resolve`], and [`RenderError::BadDataLength`] when an auxiliary
/// plane does not hold width × height values.
pub fn try_resolve_aux(t: &AuxTarget, k: u32) -> Result<AuxImage, RenderError> {
    let tt = &t.target;
    if k == 0 || !tt.width.is_multiple_of(k) || !tt.height.is_multiple_of(k) {
        return Err(RenderError::BadFactor { k, width: tt.width, height: tt.height });
    }
    let expected = area(tt.width, tt.height).ok_or(RenderError::TooLarge(tt.width, tt.height))?;
    for found in [tt.color.len(), tt.depth.len(), t.aux.depth.len(), t.aux.normal.len(), t.aux.part_id.len()] {
        if found != expected {
            return Err(RenderError::BadDataLength { expected, found });
        }
    }
    let (image, _) = try_resolve(tt, k)?;
    let (w, out) = (image.width as usize, image.data.len());
    let mut aux = AuxPlanes { depth: vec![f32::INFINITY; out], normal: vec![[0.0; 3]; out], part_id: vec![NO_PART; out] };
    if out == 0 {
        return Ok(AuxImage { image, aux });
    }
    let (k, tw) = (k as usize, tt.width as usize);
    aux.depth.par_chunks_mut(w).zip(aux.normal.par_chunks_mut(w)).zip(aux.part_id.par_chunks_mut(w)).enumerate().for_each(
        |(y, ((drow, nrow), prow))| {
            for x in 0..w {
                let mut best: Option<usize> = None;
                for j in 0..k {
                    for i in 0..k {
                        let s = (y * k + j) * tw + x * k + i;
                        let z = t.aux.depth[s];
                        if z.is_finite() && best.is_none_or(|b| z < t.aux.depth[b]) {
                            best = Some(s);
                        }
                    }
                }
                if let Some(s) = best {
                    (drow[x], nrow[x], prow[x]) = (t.aux.depth[s], t.aux.normal[s], t.aux.part_id[s]);
                }
            }
        },
    );
    Ok(AuxImage { image, aux })
}

/// Output pixel (`x`, `y`) of a `k`-per-side resolve, summed in `f64`.
fn mean_f64(t: &Target, k: usize, x: usize, y: usize) -> ([f32; 4], f32) {
    let (mut acc, mut dz, mut dn) = ([0.0f64; 4], 0.0f64, 0u64);
    for j in 0..k {
        for i in 0..k {
            let s = (y * k + j) * t.width as usize + x * k + i;
            for (a, c) in acc.iter_mut().zip(t.color[s]) {
                *a += f64::from(c);
            }
            if t.depth[s].is_finite() {
                dz += f64::from(t.depth[s]);
                dn += 1;
            }
        }
    }
    let n = (k * k) as f64;
    let depth = if dn > 0 { (dz / dn as f64) as f32 } else { f32::INFINITY };
    (acc.map(|v| (v / n) as f32), depth)
}

/// Convert premultiplied linear pixels to premultiplied sRGB-encoded pixels in place: each
/// color channel is divided by alpha, encoded with [`linear_to_srgb`] and multiplied by alpha
/// again. Alpha stays linear, and pixels with alpha of 1e-6 or less are left as they are.
///
/// Every pixel is checked before any is changed. Returns [`RenderError::BadPixel`] with the first
/// pixel whose channels are not finite, whose alpha is negative or over 1 (beyond rounding), or
/// whose color divided by its alpha is not finite,
/// and [`RenderError::BadDataLength`] when the image does not hold width × height pixels.
pub fn try_encode(img: &mut Image) -> Result<(), RenderError> {
    let expected = area(img.width, img.height).ok_or(RenderError::TooLarge(img.width, img.height))?;
    if img.data.len() != expected {
        return Err(RenderError::BadDataLength { expected, found: img.data.len() });
    }
    let bad = |p: &[f32; 4]| {
        !p.iter().all(|c| c.is_finite()) || p[3] < 0.0 || p[3] > 1.0 + ALPHA_SLACK || (p[3] > 1e-6 && !p[..3].iter().all(|c| (c / p[3]).is_finite()))
    };
    // A pixel that fails the checks above is written as NaN, so the first invalid pixel is
    // reported.
    let encoded: Vec<[f32; 4]> = img
        .data
        .par_iter()
        .map(|&p| {
            if bad(&p) {
                return [f32::NAN; 4];
            }
            let mut p = p;
            let a = p[3];
            if a > 1e-6 {
                for c in &mut p[..3] {
                    *c = linear_to_srgb(*c / a) * a;
                }
            }
            p
        })
        .collect();
    if let Some(i) = encoded.iter().position(|p| !p.iter().all(|c| c.is_finite())) {
        return Err(RenderError::BadPixel(i));
    }
    img.data = encoded;
    Ok(())
}

/// Render a scene and resolve it by its supersampling factor ([`Scene::ssaa`], 0 read as 1):
/// the premultiplied linear image at output size and its depth. A factor that does not divide
/// the raster size is rejected before anything is rendered.
pub fn try_render_cpu(s: &Scene, limits: &Limits) -> Result<(Image, Vec<f32>), RenderError> {
    let k = s.ssaa.max(1);
    if !s.width.is_multiple_of(k) || !s.height.is_multiple_of(k) {
        return Err(RenderError::BadFactor { k, width: s.width, height: s.height });
    }
    try_resolve(&try_render(s, limits)?, k)
}

/// [`try_render_with_aux`] and [`try_resolve_aux`] by the scene's supersampling factor, as
/// [`try_render_cpu`]: the image is [`try_render_cpu`]'s, bit for bit. The resolve's output
/// planes count toward the byte check of [`try_render_with_aux`], which runs before anything is
/// rendered.
pub fn try_render_cpu_with_aux(s: &Scene, limits: &Limits) -> Result<AuxImage, RenderError> {
    let k = s.ssaa.max(1);
    if !s.width.is_multiple_of(k) || !s.height.is_multiple_of(k) {
        return Err(RenderError::BadFactor { k, width: s.width, height: s.height });
    }
    limits.check_aux(s.width, s.height, Some(k))?;
    try_resolve_aux(&try_render_with_aux(s, limits)?, k)
}
