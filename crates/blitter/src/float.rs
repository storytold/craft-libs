//! Floating-point (`RGBA32F`, `[[f32; 4]]`) Porter-Duff compositing and slice blitting.
//!
//! # Specification References
//! - **W3C Compositing and Blending Level 1**:
//!   - §5 Simple alpha compositing (straight & premultiplied):
//!     <https://www.w3.org/TR/compositing-1/#simplealphacompositing>
//!   - §9.1.2 `source-over` and §9.1.6 `source-atop`:
//!     <https://www.w3.org/TR/compositing-1/#porterduffcompositingoperators>
//!   - §10 General Formula for Compositing and Blending:
//!     <https://www.w3.org/TR/compositing-1/#generalformula>
//! - **W3C SVG Compositing Specification** (`src-over` `(X,Y,Z) = (1,1,1)`, `src-atop`
//!   `(X,Y,Z) = (1,0,1)`): <https://www.w3.org/TR/SVGCompositing/>
//! - **Khronos `GL_NV_blend_equation_advanced`** (Table X.1):
//!   <https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt>

use crate::{AlphaMode, BlendMode, CompositeOp};

#[inline(always)]
fn non_positive(x: f32) -> bool {
    x.is_nan() || x <= 0.0
}

/// Straight-alpha `Normal` `source-over` compositing (`W3C` §5 / §9.1.2).
///
/// Non-positive or `NaN` effective source alpha leaves `backdrop` unchanged.
#[inline]
pub fn composite_normal_f32(backdrop: [f32; 4], source: [f32; 4], opacity: f32) -> [f32; 4] {
    if non_positive(source[3]) {
        return backdrop;
    }
    let as_ = source[3] * opacity;
    if non_positive(as_) {
        return backdrop;
    }
    let ab = backdrop[3];
    let kb = ab * (1.0 - as_);
    let ao = as_ + kb;
    if non_positive(ao) {
        return [0.0; 4];
    }
    let inv = 1.0 / ao;
    [(kb * backdrop[0] + as_ * source[0]) * inv, (kb * backdrop[1] + as_ * source[1]) * inv, (kb * backdrop[2] + as_ * source[2]) * inv, ao]
}

/// Straight-alpha `source-over` compositing with `mode` (`W3C` §10):
/// `ao = as + ab * (1 - as)`,
/// `Co = ((1 - as) * ab * Cb + (1 - ab) * as * Cs + as * ab * B(Cb, Cs)) / ao`.
///
/// Uses the exact same operation order (`wb`, `ws`, `wbs`, `ao = as + wb`, `* (1.0 / ao)`) as
/// [`crate::simd::separable_blend_lanes`] so the vectorized and scalar paths agree bit-for-bit.
#[inline]
pub fn composite_f32(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32, hdr: bool) -> [f32; 4] {
    if mode == BlendMode::Normal {
        return composite_normal_f32(backdrop, source, opacity);
    }
    if non_positive(source[3]) {
        return backdrop;
    }
    let as_ = source[3] * opacity;
    if non_positive(as_) {
        return backdrop;
    }
    let ab = backdrop[3];
    let wb = (1.0 - as_) * ab;
    let ws = (1.0 - ab) * as_;
    let wbs = as_ * ab;
    let ao = as_ + wb;
    if non_positive(ao) {
        return [0.0; 4];
    }
    let inv = 1.0 / ao;
    let cb = [backdrop[0], backdrop[1], backdrop[2]];
    let cs = [source[0], source[1], source[2]];
    let b = if hdr { crate::blend::blend_rgb_hdr(mode, cb, cs) } else { crate::blend::blend_rgb(mode, cb, cs) };
    [(wb * cb[0] + ws * cs[0] + wbs * b[0]) * inv, (wb * cb[1] + ws * cs[1] + wbs * b[1]) * inv, (wb * cb[2] + ws * cs[2] + wbs * b[2]) * inv, ao]
}

/// Straight-alpha `source-atop` compositing with `mode` (`W3C` §9.1.6 + §10; `(X, Y, Z) = (1, 0, 1)`):
/// for `ab > 0`, `ao = ab` and `Co = (1 - as) * Cb + as * B(Cb, Cs)`.
#[inline]
pub fn composite_atop_f32(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32, hdr: bool) -> [f32; 4] {
    if non_positive(backdrop[3]) || non_positive(source[3]) {
        return backdrop;
    }
    let as_ = source[3] * opacity;
    if non_positive(as_) {
        return backdrop;
    }
    let cb = [backdrop[0], backdrop[1], backdrop[2]];
    let cs = [source[0], source[1], source[2]];
    let b = if hdr { crate::blend::blend_rgb_hdr(mode, cb, cs) } else { crate::blend::blend_rgb(mode, cb, cs) };
    let rem = 1.0 - as_;
    [rem * cb[0] + as_ * b[0], rem * cb[1] + as_ * b[1], rem * cb[2] + as_ * b[2], backdrop[3]]
}

/// Premultiplied-alpha `Normal` `source-over` compositing (`W3C` §5):
/// `co = cs * opacity + cb * (1 - as * opacity)`, `ao = as * opacity + ab * (1 - as * opacity)`.
#[inline]
pub fn composite_premultiplied_f32(backdrop: [f32; 4], source: [f32; 4], opacity: f32) -> [f32; 4] {
    if non_positive(source[3]) {
        return backdrop;
    }
    let as_ = source[3] * opacity;
    if non_positive(as_) {
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

/// Pure scalar `RGBA32F` slice blitter (reference fallback for [`crate::simd`]).
///
/// Exposed so callers and tests can run the scalar fallback and the 8-lane chunked SoA path on
/// identical inputs and verify bit-identical results.
pub fn blit_rgba32f_scalar(dst: &mut [[f32; 4]], src: &[[f32; 4]], mode: BlendMode, opacity: f32, hdr: bool, alpha_mode: AlphaMode, op: CompositeOp) {
    if non_positive(opacity) || dst.is_empty() || src.is_empty() {
        return;
    }
    let opacity = opacity.min(1.0);
    let n = dst.len().min(src.len());
    let dst = &mut dst[..n];
    let src = &src[..n];

    match alpha_mode {
        AlphaMode::Straight => blit_rgba32f_straight_scalar(dst, src, mode, opacity, hdr, op),
        AlphaMode::Premultiplied => blit_rgba32f_premul_scalar(dst, src, mode, opacity, hdr, op),
    }
}

/// Serial `RGBA32F` slice blitter: dispatches supported straight-alpha `SourceOver` modes through
/// [`crate::simd::composite_separable_rgb_span_serial`] and all other combinations through
/// [`blit_rgba32f_scalar`].
pub fn blit_rgba32f_serial(dst: &mut [[f32; 4]], src: &[[f32; 4]], mode: BlendMode, opacity: f32, hdr: bool, alpha_mode: AlphaMode, op: CompositeOp) {
    if non_positive(opacity) || dst.is_empty() || src.is_empty() {
        return;
    }
    let opacity = opacity.min(1.0);
    let n = dst.len().min(src.len());
    let dst = &mut dst[..n];
    let src = &src[..n];

    if alpha_mode == AlphaMode::Straight
        && op == CompositeOp::SourceOver
        && crate::simd::composite_separable_rgb_span_serial(dst, src, mode, opacity, hdr)
    {
        return;
    }

    match alpha_mode {
        AlphaMode::Straight => blit_rgba32f_straight_scalar(dst, src, mode, opacity, hdr, op),
        AlphaMode::Premultiplied => blit_rgba32f_premul_scalar(dst, src, mode, opacity, hdr, op),
    }
}

fn blit_rgba32f_straight_scalar(dst: &mut [[f32; 4]], src: &[[f32; 4]], mode: BlendMode, opacity: f32, hdr: bool, op: CompositeOp) {
    match op {
        CompositeOp::SourceOver => {
            for (d, s) in dst.iter_mut().zip(src) {
                if non_positive(s[3]) {
                    continue;
                }
                *d = composite_f32(mode, *d, *s, opacity, hdr);
            }
        }
        CompositeOp::SourceAtop => {
            for (d, s) in dst.iter_mut().zip(src) {
                if non_positive(d[3]) || non_positive(s[3]) {
                    continue;
                }
                *d = composite_atop_f32(mode, *d, *s, opacity, hdr);
            }
        }
    }
}

fn blit_rgba32f_premul_scalar(dst: &mut [[f32; 4]], src: &[[f32; 4]], mode: BlendMode, opacity: f32, hdr: bool, op: CompositeOp) {
    if mode == BlendMode::Normal {
        blit_rgba32f_premul_normal(dst, src, opacity, op);
        return;
    }

    for (d, s) in dst.iter_mut().zip(src) {
        if non_positive(s[3]) {
            continue;
        }
        let ab = d[3];
        if op == CompositeOp::SourceAtop && non_positive(ab) {
            continue;
        }
        let inv_sa = 1.0 / s[3];
        let s_straight = [s[0] * inv_sa, s[1] * inv_sa, s[2] * inv_sa, s[3]];
        let d_straight = if ab > 0.0 {
            let inv_ab = 1.0 / ab;
            [d[0] * inv_ab, d[1] * inv_ab, d[2] * inv_ab, ab]
        } else {
            [0.0; 4]
        };

        let res = match op {
            CompositeOp::SourceOver => composite_f32(mode, d_straight, s_straight, opacity, hdr),
            CompositeOp::SourceAtop => composite_atop_f32(mode, d_straight, s_straight, opacity, hdr),
        };
        let ao = res[3];
        *d = [res[0] * ao, res[1] * ao, res[2] * ao, ao];
    }
}

fn blit_rgba32f_premul_normal(dst: &mut [[f32; 4]], src: &[[f32; 4]], opacity: f32, op: CompositeOp) {
    match op {
        CompositeOp::SourceOver => {
            for (d, s) in dst.iter_mut().zip(src) {
                if non_positive(s[3]) {
                    continue;
                }
                *d = composite_premultiplied_f32(*d, *s, opacity);
            }
        }
        CompositeOp::SourceAtop => {
            // W3C SVG Compositing (`src-atop`): Dca' = Sca * Da + Dca * (1 - Sa), Da' = Da.
            for (d, s) in dst.iter_mut().zip(src) {
                let da = d[3];
                if non_positive(da) || non_positive(s[3]) {
                    continue;
                }
                let sa = s[3] * opacity;
                if non_positive(sa) {
                    continue;
                }
                let factor = 1.0 - sa;
                let s_scale = opacity * da;
                *d = [s[0] * s_scale + d[0] * factor, s[1] * s_scale + d[1] * factor, s[2] * s_scale + d[2] * factor, da];
            }
        }
    }
}
