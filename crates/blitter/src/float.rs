//! Float HDR/LDR blitting and compositing engine for RGBA32F (`[[f32; 4]]`).
//!
//! Features:
//! - Straight alpha Porter-Duff compositing with IEEE-754 bit exactness.
//! - Division-free premultiplied alpha compositing.
//! - High Dynamic Range (HDR) handling without premature clamping at 1.0.
//! - Optical text gamma compositing ($\gamma = 1.45$).
//! - Zero heap allocations.

use crate::BlendMode;
use crate::{AlphaMode, special};

/// Straight-alpha normal compositing with exact IEEE-754 bit preservation.
#[inline]
pub fn composite_normal_f32(backdrop: [f32; 4], source: [f32; 4], opacity: f32) -> [f32; 4] {
    let ab = backdrop[3];
    let as_ = source[3] * opacity;
    if as_ <= 0.0 {
        return backdrop;
    }
    let ao = as_ + ab * (1.0 - as_);
    if ao <= 0.0 {
        return [0.0; 4];
    }
    let kb = ab * (1.0 - as_);
    let inv = 1.0 / ao;
    [(kb * backdrop[0] + as_ * source[0]) * inv, (kb * backdrop[1] + as_ * source[1]) * inv, (kb * backdrop[2] + as_ * source[2]) * inv, ao]
}

/// Straight-alpha general blend compositing over backdrop.
#[inline]
pub fn composite_f32(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32, hdr: bool) -> [f32; 4] {
    let ab = backdrop[3];
    let as_ = source[3] * opacity;
    if as_ <= 0.0 {
        return backdrop;
    }
    let ao = as_ + ab * (1.0 - as_);
    if mode == BlendMode::Normal {
        if ao <= 0.0 {
            return [0.0; 4];
        }
        let kb = ab * (1.0 - as_);
        let inv = 1.0 / ao;
        return [
            (kb * backdrop[0] + as_ * source[0]) * inv,
            (kb * backdrop[1] + as_ * source[1]) * inv,
            (kb * backdrop[2] + as_ * source[2]) * inv,
            ao,
        ];
    }
    if ao <= 0.0 {
        return [0.0; 4];
    }
    let cb = [backdrop[0], backdrop[1], backdrop[2]];
    let cs = [source[0], source[1], source[2]];
    let b = if hdr { crate::blend::blend_rgb_hdr(mode, cb, cs) } else { crate::blend::blend_rgb(mode, cb, cs) };
    let mut out = [0.0f32; 4];
    for i in 0..3 {
        out[i] = ((1.0 - as_) * ab * cb[i] + (1.0 - ab) * as_ * cs[i] + as_ * ab * b[i]) / ao;
    }
    out[3] = ao;
    out
}

/// General compositing with optional text gamma compensation.
#[inline]
pub fn composite_gamma_f32(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32, gamma: f32, hdr: bool) -> [f32; 4] {
    if (gamma - 1.0).abs() < 1e-6 {
        return composite_f32(mode, backdrop, source, opacity, hdr);
    }
    let ab = backdrop[3];
    let as_ = source[3] * opacity;
    if as_ <= 0.0 {
        return backdrop;
    }
    let ao = as_ + ab * (1.0 - as_);
    if ao <= 0.0 {
        return [0.0; 4];
    }
    let cb = [backdrop[0], backdrop[1], backdrop[2]];
    let cs = [source[0], source[1], source[2]];
    let b = if hdr { crate::blend::blend_rgb_hdr(mode, cb, cs) } else { crate::blend::blend_rgb(mode, cb, cs) };
    let wb = (1.0 - as_) * ab / ao;
    let ws = (1.0 - ab) * as_ / ao;
    let wbs = as_ * ab / ao;
    let mut out = [0.0f32; 4];
    for i in 0..3 {
        out[i] = crate::blend::text_decode(
            wb * crate::blend::text_encode(cb[i], gamma)
                + ws * crate::blend::text_encode(cs[i], gamma)
                + wbs * crate::blend::text_encode(b[i], gamma),
            gamma,
        );
    }
    out[3] = ao;
    out
}

/// General compositing with separate opacity, fill, text gamma, and HDR modes.
pub fn composite_fill_f32(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32, fill: f32, gamma: f32, hdr: bool) -> [f32; 4] {
    if fill >= 1.0 || !special::fill_is_special(mode) {
        return composite_gamma_f32(mode, backdrop, source, opacity * fill.clamp(0.0, 1.0), gamma, hdr);
    }
    special::composite_special_eight_f32(mode, backdrop, source, opacity, fill, gamma, hdr)
}

/// Division-free premultiplied alpha normal compositing.
#[inline]
pub fn composite_premultiplied_f32(backdrop: [f32; 4], source: [f32; 4], opacity: f32) -> [f32; 4] {
    let as_ = source[3] * opacity;
    if as_ <= 0.0 {
        return backdrop;
    }
    let factor = 1.0 - as_;
    [
        source[0] * opacity + backdrop[0] * factor,
        source[1] * opacity + backdrop[1] * factor,
        source[2] * opacity + backdrop[2] * factor,
        as_ + backdrop[3] * factor,
    ]
}

/// Blits RGBA32F slice buffer in-place with zero heap allocations.
#[allow(clippy::too_many_arguments)]
pub fn blit_rgba32f_inner(
    dst: &mut [[f32; 4]],
    src: &[[f32; 4]],
    mode: BlendMode,
    opacity: f32,
    fill: f32,
    gamma: f32,
    hdr: bool,
    alpha_mode: AlphaMode,
    clipping: bool,
) {
    if opacity <= 0.0 || dst.is_empty() || src.is_empty() {
        return;
    }
    let n = dst.len().min(src.len());
    let dst = &mut dst[..n];
    let src = &src[..n];

    if alpha_mode == AlphaMode::Premultiplied && mode == BlendMode::Normal {
        for (d, s) in dst.iter_mut().zip(src) {
            if s[3] <= 0.0 {
                continue;
            }
            let orig_alpha = d[3];
            if clipping && orig_alpha <= 0.0 {
                continue;
            }
            let res = composite_premultiplied_f32(*d, *s, opacity);
            *d = res;
            if clipping {
                d[3] = orig_alpha;
            }
        }
        return;
    }

    if fill >= 1.0 && (gamma - 1.0).abs() < 1e-6 && !clipping {
        // Fast SIMD path for supported separable blend modes
        if crate::simd::composite_separable_rgb_span(dst, src, mode, opacity, hdr) {
            return;
        }
    }

    for (d, s) in dst.iter_mut().zip(src) {
        if s[3] <= 0.0 {
            continue;
        }
        let orig_alpha = d[3];
        if clipping {
            if orig_alpha <= 0.0 {
                continue;
            }
            let base_opaque = [d[0], d[1], d[2], 1.0];
            let res = composite_fill_f32(mode, base_opaque, *s, opacity, fill, gamma, hdr);
            *d = [res[0], res[1], res[2], orig_alpha];
        } else {
            *d = composite_fill_f32(mode, *d, *s, opacity, fill, gamma, hdr);
        }
    }
}
