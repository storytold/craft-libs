//! Unsafe-free SIMD auto-vectorization across compile-time constant array chunks.
//!
//! Features:
//! - Safe chunking via standard library `slice::as_chunks_mut::<8>()` and `slice::as_chunks::<8>()`.
//! - Stack Structure-of-Arrays (SoA) transposition for vector registers.
//! - Parallel chunk processing via Rayon (`PAR_THRESHOLD = 4096`) on native platforms.
//! - Safe scalar remainder tail processing without pixel truncation.
//! - Non-panicking slice length clamping.
//! - Vectorized separable blend kernels for Multiply, Screen, Add, Darken, Lighten, Difference, LinearBurn, Exclusion, Subtract.

use crate::BlendMode;

pub const LANES: usize = 8;
pub const PAR_THRESHOLD: usize = 4096;

/// Stack-based conversion from Array-of-Structures (AoS) to Structure-of-Arrays (SoA).
#[inline(always)]
pub fn transpose_from_aos(p: &[[f32; 4]; LANES]) -> ([f32; LANES], [f32; LANES], [f32; LANES], [f32; LANES]) {
    let r = std::array::from_fn(|i| p[i][0]);
    let g = std::array::from_fn(|i| p[i][1]);
    let b = std::array::from_fn(|i| p[i][2]);
    let a = std::array::from_fn(|i| p[i][3]);
    (r, g, b, a)
}

/// Stack-based conversion from Structure-of-Arrays (SoA) to Array-of-Structures (AoS).
#[inline(always)]
pub fn transpose_to_aos(r: [f32; LANES], g: [f32; LANES], b: [f32; LANES], a: [f32; LANES]) -> [[f32; 4]; LANES] {
    std::array::from_fn(|i| [r[i], g[i], b[i], a[i]])
}

/// Vectorized Normal blend over a fixed block of 8 pixels without branching.
#[inline]
#[allow(clippy::needless_range_loop)]
pub fn normal_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    let lane = |p: &[[f32; 4]; LANES], c: usize| -> [f32; LANES] { std::array::from_fn(|i| p[i][c]) };
    let (b0, b1, b2, ab) = (lane(b, 0), lane(b, 1), lane(b, 2), lane(b, 3));
    let (s0, s1, s2, s3) = (lane(s, 0), lane(s, 1), lane(s, 2), lane(s, 3));
    let mut as_ = [0.0f32; LANES];
    let mut ao = [0.0f32; LANES];
    let mut kb = [0.0f32; LANES];
    let mut inv = [0.0f32; LANES];
    for i in 0..LANES {
        as_[i] = s3[i] * opacity;
        ao[i] = as_[i] + ab[i] * (1.0 - as_[i]);
        kb[i] = ab[i] * (1.0 - as_[i]);
        inv[i] = if ao[i] > 0.0 { 1.0 / ao[i] } else { 0.0 };
    }
    for i in 0..LANES {
        let mix =
            [(kb[i] * b0[i] + as_[i] * s0[i]) * inv[i], (kb[i] * b1[i] + as_[i] * s1[i]) * inv[i], (kb[i] * b2[i] + as_[i] * s2[i]) * inv[i], ao[i]];
        let keep = (s3[i] <= 0.0) | (as_[i] <= 0.0);
        let gone = ao[i] <= 0.0;
        b[i] = if keep {
            b[i]
        } else if gone {
            [0.0; 4]
        } else {
            mix
        };
    }
}

/// Generic vectorized separable blend over a fixed block of 8 pixels.
///
/// Implements W3C/PDF Porter-Duff compositing with arbitrary separable per-channel blend functions:
/// $$c_o = \frac{(1 - a_s) a_b c_b + (1 - a_b) a_s c_s + a_s a_b B(c_b, c_s)}{a_o}$$
#[inline(always)]
#[allow(clippy::needless_range_loop)]
pub fn separable_blend_lanes<F>(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32, blend_fn: F)
where
    F: Fn([f32; LANES], [f32; LANES]) -> [f32; LANES],
{
    let lane = |p: &[[f32; 4]; LANES], c: usize| -> [f32; LANES] { std::array::from_fn(|i| p[i][c]) };
    let (b0, b1, b2, ab) = (lane(b, 0), lane(b, 1), lane(b, 2), lane(b, 3));
    let (s0, s1, s2, s3) = (lane(s, 0), lane(s, 1), lane(s, 2), lane(s, 3));

    let mut as_ = [0.0f32; LANES];
    let mut ao = [0.0f32; LANES];
    let mut wb = [0.0f32; LANES];
    let mut ws = [0.0f32; LANES];
    let mut wbs = [0.0f32; LANES];
    let mut inv = [0.0f32; LANES];

    for i in 0..LANES {
        as_[i] = s3[i] * opacity;
        wb[i] = (1.0 - as_[i]) * ab[i];
        ws[i] = (1.0 - ab[i]) * as_[i];
        wbs[i] = as_[i] * ab[i];
        ao[i] = as_[i] + wb[i];
        inv[i] = if ao[i] > 0.0 { 1.0 / ao[i] } else { 0.0 };
    }

    let br = blend_fn(b0, s0);
    let bg = blend_fn(b1, s1);
    let bb = blend_fn(b2, s2);

    for i in 0..LANES {
        let mix = [
            (wb[i] * b0[i] + ws[i] * s0[i] + wbs[i] * br[i]) * inv[i],
            (wb[i] * b1[i] + ws[i] * s1[i] + wbs[i] * bg[i]) * inv[i],
            (wb[i] * b2[i] + ws[i] * s2[i] + wbs[i] * bb[i]) * inv[i],
            ao[i],
        ];
        let keep = (s3[i] <= 0.0) | (as_[i] <= 0.0);
        let gone = ao[i] <= 0.0;
        b[i] = if keep {
            b[i]
        } else if gone {
            [0.0; 4]
        } else {
            mix
        };
    }
}

/// Vectorized Multiply blend over 8 pixels.
#[inline]
pub fn multiply_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| cb[i] * cs[i]));
}

/// Vectorized Screen blend over 8 pixels.
#[inline]
pub fn screen_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| cb[i] + cs[i] - cb[i] * cs[i]));
}

/// Vectorized Add (Linear Dodge) blend over 8 pixels.
#[inline]
pub fn add_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32, hdr: bool) {
    if hdr {
        separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| cb[i] + cs[i]));
    } else {
        separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| (cb[i] + cs[i]).min(1.0)));
    }
}

/// Vectorized Darken blend over 8 pixels.
#[inline]
pub fn darken_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| cb[i].min(cs[i])));
}

/// Vectorized Lighten blend over 8 pixels.
#[inline]
pub fn lighten_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| cb[i].max(cs[i])));
}

/// Vectorized Difference blend over 8 pixels.
#[inline]
pub fn difference_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| (cb[i] - cs[i]).abs()));
}

/// Vectorized Linear Burn blend over 8 pixels.
#[inline]
pub fn linear_burn_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| (cb[i] + cs[i] - 1.0).max(0.0)));
}

/// Vectorized Exclusion blend over 8 pixels.
#[inline]
pub fn exclusion_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| cb[i] + cs[i] - 2.0 * cb[i] * cs[i]));
}

/// Vectorized Subtract blend over 8 pixels.
#[inline]
pub fn subtract_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| (cb[i] - cs[i]).max(0.0)));
}

/// Dispatches a chunk of 8 pixels for supported blend modes.
/// Returns `true` if handled by vectorization, `false` if the mode requires scalar math.
#[inline]
pub fn blend_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], mode: BlendMode, opacity: f32, hdr: bool) -> bool {
    match mode {
        BlendMode::Normal => {
            normal_lanes(b, s, opacity);
            true
        }
        BlendMode::Multiply => {
            multiply_lanes(b, s, opacity);
            true
        }
        BlendMode::Screen => {
            screen_lanes(b, s, opacity);
            true
        }
        BlendMode::LinearDodge => {
            add_lanes(b, s, opacity, hdr);
            true
        }
        BlendMode::Darken => {
            darken_lanes(b, s, opacity);
            true
        }
        BlendMode::Lighten => {
            lighten_lanes(b, s, opacity);
            true
        }
        BlendMode::Difference => {
            difference_lanes(b, s, opacity);
            true
        }
        BlendMode::LinearBurn => {
            linear_burn_lanes(b, s, opacity);
            true
        }
        BlendMode::Exclusion => {
            exclusion_lanes(b, s, opacity);
            true
        }
        BlendMode::Subtract => {
            subtract_lanes(b, s, opacity);
            true
        }
        _ => false,
    }
}

/// Sequential vectorized span runner for supported separable modes.
pub fn composite_separable_rgb_span_serial(dst: &mut [[f32; 4]], src: &[[f32; 4]], mode: BlendMode, opacity: f32, hdr: bool) -> bool {
    match mode {
        BlendMode::Normal
        | BlendMode::Multiply
        | BlendMode::Screen
        | BlendMode::LinearDodge
        | BlendMode::Darken
        | BlendMode::Lighten
        | BlendMode::Difference
        | BlendMode::LinearBurn
        | BlendMode::Exclusion
        | BlendMode::Subtract => {}
        _ => return false,
    }

    let n = dst.len().min(src.len());
    let (dc, dt) = dst[..n].as_chunks_mut::<LANES>();
    let (sc, st) = src[..n].as_chunks::<LANES>();

    for (b, s) in dc.iter_mut().zip(sc) {
        blend_lanes(b, s, mode, opacity, hdr);
    }
    for (b, s) in dt.iter_mut().zip(st) {
        if s[3] <= 0.0 {
            continue;
        }
        *b = crate::float::composite_f32(mode, *b, *s, opacity, hdr);
    }
    true
}

/// Span runner with Rayon parallelization on native targets and scalar fallback on WASM.
pub fn composite_separable_rgb_span(dst: &mut [[f32; 4]], src: &[[f32; 4]], mode: BlendMode, opacity: f32, hdr: bool) -> bool {
    let n = dst.len().min(src.len());
    let dst = &mut dst[..n];
    let src = &src[..n];

    #[cfg(not(target_arch = "wasm32"))]
    if dst.len() >= PAR_THRESHOLD {
        match mode {
            BlendMode::Normal
            | BlendMode::Multiply
            | BlendMode::Screen
            | BlendMode::LinearDodge
            | BlendMode::Darken
            | BlendMode::Lighten
            | BlendMode::Difference
            | BlendMode::LinearBurn
            | BlendMode::Exclusion
            | BlendMode::Subtract => {
                use rayon::prelude::*;
                dst.par_chunks_mut(PAR_THRESHOLD).zip(src.par_chunks(PAR_THRESHOLD)).for_each(|(d_chunk, s_chunk)| {
                    composite_separable_rgb_span_serial(d_chunk, s_chunk, mode, opacity, hdr);
                });
                return true;
            }
            _ => return false,
        }
    }

    composite_separable_rgb_span_serial(dst, src, mode, opacity, hdr)
}

/// Backwards-compatible alias for Normal mode serial span.
#[inline]
pub fn composite_normal_rgb_span_serial(dst: &mut [[f32; 4]], src: &[[f32; 4]], opacity: f32) {
    composite_separable_rgb_span_serial(dst, src, BlendMode::Normal, opacity, false);
}

/// Backwards-compatible alias for Normal mode parallel span.
#[inline]
pub fn composite_normal_rgb_span(dst: &mut [[f32; 4]], src: &[[f32; 4]], opacity: f32) {
    composite_separable_rgb_span(dst, src, BlendMode::Normal, opacity, false);
}
