//! Safe 8-pixel Structure-of-Arrays (SoA) chunked auto-vectorization for `RGBA32F` spans.
//!
//! # Vectorization Design (`#![forbid(unsafe_code)]`)
//! The workspace forbids `unsafe` (`[workspace.lints.rust] unsafe_code = "forbid"` in `Cargo.toml`
//! and `AGENTS.md`), and targets stable Rust (`edition = "2024"`). Rather than architecture-specific
//! `std::arch` intrinsics, this module transposes fixed 8-pixel chunks (`slice::as_chunks::<8>()`)
//! on the stack from Array-of-Structures (`[[f32; 4]; 8]`) into Structure-of-Arrays (`[f32; 8]` per
//! channel) with branchless inner loops so LLVM emits packed vector instructions (`SSE`/`AVX` on
//! `x86_64`, `NEON` on `aarch64`, `simd128` on `wasm32` when enabled).
//!
//! Every expression in [`normal_lanes`] and [`separable_blend_lanes`] matches the IEEE-754
//! operation tree of [`crate::float::composite_f32`], and tail pixels (`len % 8`) run through
//! [`crate::float::composite_f32`]. Rust does not fuse `+`/`*` into FMA on `f32`, so the chunked
//! path and [`crate::float::blit_rgba32f_scalar`] produce bit-identical IEEE-754 outputs across
//! CPUs.
//!
//! Provenance: [`normal_lanes`] is adapted from `storytold/photocraft`
//! (`crates/compose/src/psblend.rs`, MIT OR Apache-2.0; see root `NOTICE`) and generalized in
//! [`separable_blend_lanes`] to 9 additional separable modes.

use crate::BlendMode;

/// Number of `RGBA32F` pixels processed per SoA vector chunk (`8 * f32 = 256 bits`).
pub const LANES: usize = 8;

/// Minimum pixel count before native entry points split work across Rayon (`4096` pixels).
pub const PAR_THRESHOLD: usize = 4096;

/// Stack conversion from Array-of-Structures (`[[f32; 4]; 8]`) to Structure-of-Arrays (`[f32; 8]` x 4).
#[inline(always)]
pub fn transpose_from_aos(p: &[[f32; 4]; LANES]) -> ([f32; LANES], [f32; LANES], [f32; LANES], [f32; LANES]) {
    let r = std::array::from_fn(|i| p[i][0]);
    let g = std::array::from_fn(|i| p[i][1]);
    let b = std::array::from_fn(|i| p[i][2]);
    let a = std::array::from_fn(|i| p[i][3]);
    (r, g, b, a)
}

/// Stack conversion from Structure-of-Arrays (`[f32; 8]` x 4) to Array-of-Structures (`[[f32; 4]; 8]`).
#[inline(always)]
pub fn transpose_to_aos(r: [f32; LANES], g: [f32; LANES], b: [f32; LANES], a: [f32; LANES]) -> [[f32; 4]; LANES] {
    std::array::from_fn(|i| [r[i], g[i], b[i], a[i]])
}

/// Vectorized straight-alpha `Normal` `source-over` blend over an 8-pixel block (`W3C` §5 / §9.1.2).
#[inline]
#[allow(clippy::needless_range_loop)]
pub fn normal_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    let (b0, b1, b2, ab) = transpose_from_aos(b);
    let (s0, s1, s2, s3) = transpose_from_aos(s);
    let mut as_ = [0.0f32; LANES];
    let mut ao = [0.0f32; LANES];
    let mut kb = [0.0f32; LANES];
    let mut inv = [0.0f32; LANES];
    for i in 0..LANES {
        as_[i] = s3[i] * opacity;
        kb[i] = ab[i] * (1.0 - as_[i]);
        ao[i] = as_[i] + kb[i];
        inv[i] = if ao[i] > 0.0 { 1.0 / ao[i] } else { 0.0 };
    }
    for i in 0..LANES {
        let mix =
            [(kb[i] * b0[i] + as_[i] * s0[i]) * inv[i], (kb[i] * b1[i] + as_[i] * s1[i]) * inv[i], (kb[i] * b2[i] + as_[i] * s2[i]) * inv[i], ao[i]];
        let keep = (s3[i].is_nan() || s3[i] <= 0.0) | (as_[i].is_nan() || as_[i] <= 0.0);
        let gone = ao[i].is_nan() || ao[i] <= 0.0;
        b[i] = if keep {
            b[i]
        } else if gone {
            [0.0; 4]
        } else {
            mix
        };
    }
}

/// Vectorized straight-alpha `source-over` separable blend over an 8-pixel block (`W3C` §10):
/// `Co = ((1 - as) * ab * Cb + (1 - ab) * as * Cs + as * ab * B(Cb, Cs)) / ao`.
#[inline(always)]
#[allow(clippy::needless_range_loop)]
pub fn separable_blend_lanes<F>(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32, blend_fn: F)
where
    F: Fn([f32; LANES], [f32; LANES]) -> [f32; LANES],
{
    let (b0, b1, b2, ab) = transpose_from_aos(b);
    let (s0, s1, s2, s3) = transpose_from_aos(s);

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
        let keep = (s3[i].is_nan() || s3[i] <= 0.0) | (as_[i].is_nan() || as_[i] <= 0.0);
        let gone = ao[i].is_nan() || ao[i] <= 0.0;
        b[i] = if keep {
            b[i]
        } else if gone {
            [0.0; 4]
        } else {
            mix
        };
    }
}

/// Vectorized `Multiply` blend over 8 pixels (`W3C` §10.1.2).
#[inline]
pub fn multiply_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| cb[i] * cs[i]));
}

/// Vectorized `Screen` blend over 8 pixels (`W3C` §10.1.3).
#[inline]
pub fn screen_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| cb[i] + cs[i] - cb[i] * cs[i]));
}

/// Vectorized `LinearDodge` (Add) blend over 8 pixels (`Khronos` `LINEARDODGE_NV` / `PLUS_NV`).
#[inline]
pub fn add_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32, hdr: bool) {
    if hdr {
        separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| (cb[i] + cs[i]).min(f32::MAX)));
    } else {
        separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| (cb[i] + cs[i]).min(1.0)));
    }
}

/// Vectorized `Darken` blend over 8 pixels (`W3C` §10.1.5).
#[inline]
pub fn darken_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| cb[i].min(cs[i])));
}

/// Vectorized `Lighten` blend over 8 pixels (`W3C` §10.1.6).
#[inline]
pub fn lighten_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| cb[i].max(cs[i])));
}

/// Vectorized `Difference` blend over 8 pixels (`W3C` §10.1.11).
#[inline]
pub fn difference_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| (cb[i] - cs[i]).abs()));
}

/// Vectorized `LinearBurn` blend over 8 pixels (`Khronos` `LINEARBURN_NV`).
#[inline]
pub fn linear_burn_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| (cb[i] + cs[i] - 1.0).max(0.0)));
}

/// Vectorized `Exclusion` blend over 8 pixels (`W3C` §10.1.12).
#[inline]
pub fn exclusion_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| cb[i] + cs[i] - 2.0 * cb[i] * cs[i]));
}

/// Vectorized `Subtract` blend over 8 pixels (`Khronos` `MINUS_CLAMPED_NV`).
#[inline]
pub fn subtract_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], opacity: f32) {
    separable_blend_lanes(b, s, opacity, |cb, cs| std::array::from_fn(|i| (cb[i] - cs[i]).max(0.0)));
}

/// Returns `true` if `mode` has a branchless 8-lane SoA kernel in this module.
#[inline]
pub const fn supports_lanes(mode: BlendMode) -> bool {
    matches!(
        mode,
        BlendMode::Normal
            | BlendMode::Multiply
            | BlendMode::Screen
            | BlendMode::LinearDodge
            | BlendMode::Darken
            | BlendMode::Lighten
            | BlendMode::Difference
            | BlendMode::LinearBurn
            | BlendMode::Exclusion
            | BlendMode::Subtract
    )
}

/// Dispatches an 8-pixel block for supported separable modes, returning `false` otherwise.
#[inline]
pub fn blend_lanes(b: &mut [[f32; 4]; LANES], s: &[[f32; 4]; LANES], mode: BlendMode, opacity: f32, hdr: bool) -> bool {
    match mode {
        BlendMode::Normal => normal_lanes(b, s, opacity),
        BlendMode::Multiply => multiply_lanes(b, s, opacity),
        BlendMode::Screen => screen_lanes(b, s, opacity),
        BlendMode::LinearDodge => add_lanes(b, s, opacity, hdr),
        BlendMode::Darken => darken_lanes(b, s, opacity),
        BlendMode::Lighten => lighten_lanes(b, s, opacity),
        BlendMode::Difference => difference_lanes(b, s, opacity),
        BlendMode::LinearBurn => linear_burn_lanes(b, s, opacity),
        BlendMode::Exclusion => exclusion_lanes(b, s, opacity),
        BlendMode::Subtract => subtract_lanes(b, s, opacity),
        _ => return false,
    }
    true
}

/// Serial 8-lane vectorized span runner with scalar remainder tail.
///
/// Returns `false` without mutating `dst` if `mode` is not handled by [`blend_lanes`].
pub fn composite_separable_rgb_span_serial(dst: &mut [[f32; 4]], src: &[[f32; 4]], mode: BlendMode, opacity: f32, hdr: bool) -> bool {
    if !supports_lanes(mode) {
        return false;
    }
    if opacity.is_nan() || opacity <= 0.0 {
        return true;
    }
    let opacity = opacity.min(1.0);
    let n = dst.len().min(src.len());
    let (dc, dt) = dst[..n].as_chunks_mut::<LANES>();
    let (sc, st) = src[..n].as_chunks::<LANES>();

    for (b, s) in dc.iter_mut().zip(sc) {
        blend_lanes(b, s, mode, opacity, hdr);
    }
    for (b, s) in dt.iter_mut().zip(st) {
        if s[3].is_nan() || s[3] <= 0.0 {
            continue;
        }
        *b = crate::float::composite_f32(mode, *b, *s, opacity, hdr);
    }
    true
}

/// Vectorized span runner with Rayon parallelization above [`PAR_THRESHOLD`] on native targets.
pub fn composite_separable_rgb_span(dst: &mut [[f32; 4]], src: &[[f32; 4]], mode: BlendMode, opacity: f32, hdr: bool) -> bool {
    if !supports_lanes(mode) {
        return false;
    }
    if opacity.is_nan() || opacity <= 0.0 {
        return true;
    }
    let opacity = opacity.min(1.0);
    let n = dst.len().min(src.len());
    let dst = &mut dst[..n];
    let src = &src[..n];

    #[cfg(not(target_arch = "wasm32"))]
    if dst.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        dst.par_chunks_mut(PAR_THRESHOLD).zip(src.par_chunks(PAR_THRESHOLD)).for_each(|(d_chunk, s_chunk)| {
            composite_separable_rgb_span_serial(d_chunk, s_chunk, mode, opacity, hdr);
        });
        return true;
    }

    composite_separable_rgb_span_serial(dst, src, mode, opacity, hdr)
}
