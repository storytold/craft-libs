//! Fixed-point integer fast path for RGBA8 and RGBA16 compositing and blitting.
//!
//! Avoids expensive float conversion round-trips:
//! - Division by 255 via `((x + 128) * 257) >> 16` (exact for all $x \in [0, 65535]$).
//! - Division by 65535 via `((x + 32768) * 65537) >> 32` (exact for all $x \in [0, 65535 \times 65535]$).
//! - Division-free premultiplied alpha paths.
//! - Zero heap allocations.

use crate::BlendMode;
use crate::{AlphaMode, special};

/// Exact integer division by 255 with rounding for $x \in [0, 65535]$.
#[inline(always)]
pub fn div255(x: u32) -> u32 {
    ((x + 128) * 257) >> 16
}

/// Exact integer division by 65535 with rounding for $x \le 65535 \times 65535$.
#[inline(always)]
pub fn div65535(x: u64) -> u32 {
    (((x + 32768) * 65537) >> 32) as u32
}

/// Linear interpolation between `a` and `b` by factor `t` in `[0, 255]`.
#[inline(always)]
pub fn lerp255(a: u8, b: u8, t: u8) -> u8 {
    div255(a as u32 * t as u32 + b as u32 * (255 - t as u32)) as u8
}

/// Linear interpolation between `a` and `b` by factor `t` in `[0, 65535]`.
#[inline(always)]
pub fn lerp65535(a: u16, b: u16, t: u16) -> u16 {
    div65535(a as u64 * t as u64 + b as u64 * (65535 - t as u64)) as u16
}

// ---- RGBA8 Integer Blend Kernels ----

#[inline]
pub fn color_burn_u8(cb: u8, cs: u8) -> u8 {
    if cb == 255 {
        255
    } else if cs == 0 {
        0
    } else {
        let num = (255 - cb as u32) * 255;
        let ratio = (num + cs as u32 / 2) / cs as u32;
        255 - ratio.min(255) as u8
    }
}

#[inline]
pub fn color_dodge_u8(cb: u8, cs: u8) -> u8 {
    if cb == 0 {
        0
    } else if cs == 255 {
        255
    } else {
        let d = 255 - cs as u32;
        let ratio = (cb as u32 * 255 + d / 2) / d;
        ratio.min(255) as u8
    }
}

#[inline]
pub fn hard_light_u8(cb: u8, cs: u8) -> u8 {
    if cs <= 128 {
        div255(2 * cb as u32 * cs as u32).min(255) as u8
    } else {
        255u32.saturating_sub(div255(2 * (255 - cb as u32) * (255 - cs as u32))) as u8
    }
}

#[inline]
pub fn vivid_light_u8(cb: u8, cs: u8) -> u8 {
    if cs == 0 {
        0
    } else if cs == 255 {
        255
    } else if cs <= 128 {
        color_burn_u8(cb, (cs as u32 * 2).min(255) as u8)
    } else {
        let d = 2 * (255 - cs as u32);
        let ratio = (cb as u32 * 255 + d / 2) / d;
        ratio.min(255) as u8
    }
}

#[inline]
pub fn hard_mix_u8(cb: u8, cs: u8) -> u8 {
    if cs == 0 && cb == 255 {
        255
    } else if cs == 255 && cb == 0 {
        0
    } else if cb as u32 + cs as u32 >= 255 {
        255
    } else {
        0
    }
}

#[inline]
pub fn divide_u8(cb: u8, cs: u8) -> u8 {
    if cs == 0 { if cb == 0 { 0 } else { 255 } } else { ((cb as u32 * 255 + cs as u32 / 2) / cs as u32).min(255) as u8 }
}

/// Blends a single channel in 8-bit integer precision.
#[inline]
pub fn blend_channel_u8(mode: BlendMode, cb: u8, cs: u8) -> u8 {
    match mode {
        BlendMode::Normal | BlendMode::Dissolve | BlendMode::PassThrough => cs,
        BlendMode::Darken => cb.min(cs),
        BlendMode::Multiply => div255(cb as u32 * cs as u32) as u8,
        BlendMode::ColorBurn => color_burn_u8(cb, cs),
        BlendMode::LinearBurn => (cb as u32 + cs as u32).saturating_sub(255) as u8,
        BlendMode::Lighten => cb.max(cs),
        BlendMode::Screen => (255 - div255((255 - cb as u32) * (255 - cs as u32))) as u8,
        BlendMode::ColorDodge => color_dodge_u8(cb, cs),
        BlendMode::LinearDodge => cb.saturating_add(cs),
        BlendMode::Overlay => hard_light_u8(cs, cb),
        BlendMode::HardLight => hard_light_u8(cb, cs),
        BlendMode::VividLight => vivid_light_u8(cb, cs),
        BlendMode::LinearLight => (cb as i32 + 2 * cs as i32 - 255).clamp(0, 255) as u8,
        BlendMode::PinLight => {
            if cs <= 128 {
                cb.min(cs.saturating_mul(2))
            } else {
                cb.max((2 * cs as u32).saturating_sub(255) as u8)
            }
        }
        BlendMode::HardMix => hard_mix_u8(cb, cs),
        BlendMode::Difference => cb.abs_diff(cs),
        BlendMode::Exclusion => (cb as u32 + cs as u32).saturating_sub(div255(2 * cb as u32 * cs as u32)) as u8,
        BlendMode::Subtract => cb.saturating_sub(cs),
        BlendMode::Divide => divide_u8(cb, cs),
        BlendMode::Xor => cb ^ cs,
        BlendMode::Negation => (255 - (255 - cb as i32 - cs as i32).abs()) as u8,
        BlendMode::PaintNetColorBurn => color_burn_u8(cb, cs),
        BlendMode::PaintNetColorDodge => color_dodge_u8(cb, cs),
        // Fall back to float kernel for SoftLight, non-separable, and complex curves
        _ => {
            let res = crate::blend::blend_channel(mode, f32::from(cb) / 255.0, f32::from(cs) / 255.0, 1.0);
            (res.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
        }
    }
}

/// Blends an RGB triple in 8-bit integer precision.
#[inline]
pub fn blend_rgb_u8(mode: BlendMode, cb: [u8; 3], cs: [u8; 3]) -> [u8; 3] {
    if mode.is_non_separable() {
        let fb = [f32::from(cb[0]) / 255.0, f32::from(cb[1]) / 255.0, f32::from(cb[2]) / 255.0];
        let fs = [f32::from(cs[0]) / 255.0, f32::from(cs[1]) / 255.0, f32::from(cs[2]) / 255.0];
        let r = crate::blend::blend_rgb(mode, fb, fs);
        [(r[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8, (r[1].clamp(0.0, 1.0) * 255.0 + 0.5) as u8, (r[2].clamp(0.0, 1.0) * 255.0 + 0.5) as u8]
    } else {
        [blend_channel_u8(mode, cb[0], cs[0]), blend_channel_u8(mode, cb[1], cs[1]), blend_channel_u8(mode, cb[2], cs[2])]
    }
}

// ---- RGBA16 Integer Blend Kernels ----

#[inline]
pub fn color_burn_u16(cb: u16, cs: u16) -> u16 {
    if cb == 65535 {
        65535
    } else if cs == 0 {
        0
    } else {
        let num = (65535 - cb as u64) * 65535;
        let ratio = (num + cs as u64 / 2) / cs as u64;
        65535 - ratio.min(65535) as u16
    }
}

#[inline]
pub fn color_dodge_u16(cb: u16, cs: u16) -> u16 {
    if cb == 0 {
        0
    } else if cs == 65535 {
        65535
    } else {
        let d = 65535 - cs as u64;
        let ratio = (cb as u64 * 65535 + d / 2) / d;
        ratio.min(65535) as u16
    }
}

#[inline]
pub fn hard_light_u16(cb: u16, cs: u16) -> u16 {
    if cs <= 32768 {
        div65535(2 * cb as u64 * cs as u64).min(65535) as u16
    } else {
        65535u32.saturating_sub(div65535(2 * (65535 - cb as u64) * (65535 - cs as u64))) as u16
    }
}

#[inline]
pub fn vivid_light_u16(cb: u16, cs: u16) -> u16 {
    if cs == 0 {
        0
    } else if cs == 65535 {
        65535
    } else if cs <= 32768 {
        color_burn_u16(cb, (cs as u64 * 2).min(65535) as u16)
    } else {
        let d = 2 * (65535 - cs as u64);
        let ratio = (cb as u64 * 65535 + d / 2) / d;
        ratio.min(65535) as u16
    }
}

#[inline]
pub fn hard_mix_u16(cb: u16, cs: u16) -> u16 {
    if cs == 0 && cb == 65535 {
        65535
    } else if cs == 65535 && cb == 0 {
        0
    } else if cb as u64 + cs as u64 >= 65535 {
        65535
    } else {
        0
    }
}

#[inline]
pub fn divide_u16(cb: u16, cs: u16) -> u16 {
    if cs == 0 { if cb == 0 { 0 } else { 65535 } } else { ((cb as u64 * 65535 + cs as u64 / 2) / cs as u64).min(65535) as u16 }
}

/// Blends a single channel in 16-bit integer precision.
#[inline]
pub fn blend_channel_u16(mode: BlendMode, cb: u16, cs: u16) -> u16 {
    match mode {
        BlendMode::Normal | BlendMode::Dissolve | BlendMode::PassThrough => cs,
        BlendMode::Darken => cb.min(cs),
        BlendMode::Multiply => div65535(cb as u64 * cs as u64) as u16,
        BlendMode::ColorBurn => color_burn_u16(cb, cs),
        BlendMode::LinearBurn => (cb as u64 + cs as u64).saturating_sub(65535) as u16,
        BlendMode::Lighten => cb.max(cs),
        BlendMode::Screen => (65535 - div65535((65535 - cb as u64) * (65535 - cs as u64))) as u16,
        BlendMode::ColorDodge => color_dodge_u16(cb, cs),
        BlendMode::LinearDodge => cb.saturating_add(cs),
        BlendMode::Overlay => hard_light_u16(cs, cb),
        BlendMode::HardLight => hard_light_u16(cb, cs),
        BlendMode::VividLight => vivid_light_u16(cb, cs),
        BlendMode::LinearLight => (cb as i64 + 2 * cs as i64 - 65535).clamp(0, 65535) as u16,
        BlendMode::PinLight => {
            if cs <= 32768 {
                cb.min(cs.saturating_mul(2))
            } else {
                cb.max((2 * cs as u64).saturating_sub(65535) as u16)
            }
        }
        BlendMode::HardMix => hard_mix_u16(cb, cs),
        BlendMode::Difference => cb.abs_diff(cs),
        BlendMode::Exclusion => (cb as u64 + cs as u64).saturating_sub(div65535(2 * cb as u64 * cs as u64) as u64) as u16,
        BlendMode::Subtract => cb.saturating_sub(cs),
        BlendMode::Divide => divide_u16(cb, cs),
        BlendMode::Xor => cb ^ cs,
        BlendMode::Negation => (65535 - (65535 - cb as i64 - cs as i64).abs()) as u16,
        BlendMode::PaintNetColorBurn => color_burn_u16(cb, cs),
        BlendMode::PaintNetColorDodge => color_dodge_u16(cb, cs),
        _ => {
            let res = crate::blend::blend_channel(mode, f32::from(cb) / 65535.0, f32::from(cs) / 65535.0, 1.0);
            (res.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16
        }
    }
}

/// Blends an RGB triple in 16-bit integer precision.
#[inline]
pub fn blend_rgb_u16(mode: BlendMode, cb: [u16; 3], cs: [u16; 3]) -> [u16; 3] {
    if mode.is_non_separable() {
        let fb = [f32::from(cb[0]) / 65535.0, f32::from(cb[1]) / 65535.0, f32::from(cb[2]) / 65535.0];
        let fs = [f32::from(cs[0]) / 65535.0, f32::from(cs[1]) / 65535.0, f32::from(cs[2]) / 65535.0];
        let r = crate::blend::blend_rgb(mode, fb, fs);
        [(r[0].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16, (r[1].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16, (r[2].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16]
    } else {
        [blend_channel_u16(mode, cb[0], cs[0]), blend_channel_u16(mode, cb[1], cs[1]), blend_channel_u16(mode, cb[2], cs[2])]
    }
}

/// Blits an RGBA8 buffer in-place with zero heap allocations.
pub fn blit_rgba8_fixed(dst: &mut [u8], src: &[u8], mode: BlendMode, opacity: f32, fill: f32, alpha_mode: AlphaMode, clipping: bool) {
    if opacity <= 0.0 || dst.is_empty() || src.is_empty() {
        return;
    }
    let num_pixels = (dst.len() / 4).min(src.len() / 4);
    if num_pixels == 0 {
        return;
    }

    let op8 = (opacity.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    if op8 == 0 {
        return;
    }
    let fill8 = (fill.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    let is_special = special::fill_is_special(mode) && fill < 1.0;

    let dst_slice = &mut dst[..num_pixels * 4];
    let src_slice = &src[..num_pixels * 4];

    let (d_chunks, _) = dst_slice.as_chunks_mut::<4>();
    let (s_chunks, _) = src_slice.as_chunks::<4>();

    for (d, s) in d_chunks.iter_mut().zip(s_chunks) {
        let s_a = s[3] as u32;
        if s_a == 0 {
            continue;
        }

        let b_a = d[3] as u32;
        if clipping && b_a == 0 {
            continue;
        }

        let as_ = div255(s_a * op8);
        if as_ == 0 {
            continue;
        }

        if alpha_mode == AlphaMode::Premultiplied && mode == BlendMode::Normal && !is_special {
            let factor = 255 - as_;
            let s_r = div255(s[0] as u32 * op8);
            let s_g = div255(s[1] as u32 * op8);
            let s_b = div255(s[2] as u32 * op8);
            d[0] = (s_r + div255(d[0] as u32 * factor)).min(255) as u8;
            d[1] = (s_g + div255(d[1] as u32 * factor)).min(255) as u8;
            d[2] = (s_b + div255(d[2] as u32 * factor)).min(255) as u8;
            if !clipping {
                d[3] = (as_ + div255(b_a * factor)).min(255) as u8;
            }
            continue;
        }

        let eff_ba = if clipping { 255 } else { b_a };

        if is_special {
            let base_opaque = [d[0], d[1], d[2], eff_ba as u8];
            let res = special::composite_special_eight_u8(mode, base_opaque, *s, op8, fill8);
            d[0] = res[0];
            d[1] = res[1];
            d[2] = res[2];
            if !clipping {
                d[3] = res[3];
            }
        } else {
            let eff_as = if fill < 1.0 { div255(as_ * fill8) } else { as_ };
            if eff_as == 0 {
                continue;
            }

            let ao = eff_as + div255(eff_ba * (255 - eff_as));
            if ao == 0 {
                d[0] = 0;
                d[1] = 0;
                d[2] = 0;
                d[3] = 0;
                continue;
            }

            if mode == BlendMode::Normal {
                if eff_ba == 255 {
                    d[0] = lerp255(s[0], d[0], eff_as as u8);
                    d[1] = lerp255(s[1], d[1], eff_as as u8);
                    d[2] = lerp255(s[2], d[2], eff_as as u8);
                    if !clipping {
                        d[3] = 255;
                    }
                } else {
                    let kb = div255(eff_ba * (255 - eff_as));
                    d[0] = ((kb * d[0] as u32 + eff_as * s[0] as u32 + ao / 2) / ao).min(255) as u8;
                    d[1] = ((kb * d[1] as u32 + eff_as * s[1] as u32 + ao / 2) / ao).min(255) as u8;
                    d[2] = ((kb * d[2] as u32 + eff_as * s[2] as u32 + ao / 2) / ao).min(255) as u8;
                    if !clipping {
                        d[3] = ao as u8;
                    }
                }
            } else {
                let cb = [d[0], d[1], d[2]];
                let cs = [s[0], s[1], s[2]];
                let blended = blend_rgb_u8(mode, cb, cs);

                let wb = eff_ba * (255 - eff_as);
                let ws = (255 - eff_ba) * eff_as;
                let wbs = eff_as * eff_ba;
                let denom = ao * 255;

                for c in 0..3 {
                    let num = wb * cb[c] as u32 + ws * cs[c] as u32 + wbs * blended[c] as u32;
                    d[c] = ((num + denom / 2) / denom).min(255) as u8;
                }
                if !clipping {
                    d[3] = ao as u8;
                }
            }
        }
    }
}

/// Blits an RGBA16 buffer in-place with zero heap allocations.
pub fn blit_rgba16_fixed(dst: &mut [u16], src: &[u16], mode: BlendMode, opacity: f32, fill: f32, alpha_mode: AlphaMode, clipping: bool) {
    if opacity <= 0.0 || dst.is_empty() || src.is_empty() {
        return;
    }
    let num_pixels = (dst.len() / 4).min(src.len() / 4);
    if num_pixels == 0 {
        return;
    }

    let op16 = (opacity.clamp(0.0, 1.0) * 65535.0 + 0.5) as u64;
    if op16 == 0 {
        return;
    }
    let fill16 = (fill.clamp(0.0, 1.0) * 65535.0 + 0.5) as u64;
    let is_special = special::fill_is_special(mode) && fill < 1.0;

    let dst_slice = &mut dst[..num_pixels * 4];
    let src_slice = &src[..num_pixels * 4];

    let (d_chunks, _) = dst_slice.as_chunks_mut::<4>();
    let (s_chunks, _) = src_slice.as_chunks::<4>();

    for (d, s) in d_chunks.iter_mut().zip(s_chunks) {
        let s_a = s[3] as u64;
        if s_a == 0 {
            continue;
        }

        let b_a = d[3] as u64;
        if clipping && b_a == 0 {
            continue;
        }

        let as_ = div65535(s_a * op16) as u64;
        if as_ == 0 {
            continue;
        }

        if alpha_mode == AlphaMode::Premultiplied && mode == BlendMode::Normal && !is_special {
            let factor = 65535 - as_;
            let s_r = div65535(s[0] as u64 * op16) as u64;
            let s_g = div65535(s[1] as u64 * op16) as u64;
            let s_b = div65535(s[2] as u64 * op16) as u64;
            d[0] = (s_r + div65535(d[0] as u64 * factor) as u64).min(65535) as u16;
            d[1] = (s_g + div65535(d[1] as u64 * factor) as u64).min(65535) as u16;
            d[2] = (s_b + div65535(d[2] as u64 * factor) as u64).min(65535) as u16;
            if !clipping {
                d[3] = (as_ + div65535(b_a * factor) as u64).min(65535) as u16;
            }
            continue;
        }

        let eff_ba = if clipping { 65535 } else { b_a };

        if is_special {
            let base_opaque = [d[0], d[1], d[2], eff_ba as u16];
            let res = special::composite_special_eight_u16(mode, base_opaque, *s, op16, fill16);
            d[0] = res[0];
            d[1] = res[1];
            d[2] = res[2];
            if !clipping {
                d[3] = res[3];
            }
        } else {
            let eff_as = if fill < 1.0 { div65535(as_ * fill16) as u64 } else { as_ };
            if eff_as == 0 {
                continue;
            }

            let ao = eff_as + div65535(eff_ba * (65535 - eff_as)) as u64;
            if ao == 0 {
                d[0] = 0;
                d[1] = 0;
                d[2] = 0;
                d[3] = 0;
                continue;
            }

            if mode == BlendMode::Normal {
                if eff_ba == 65535 {
                    d[0] = lerp65535(s[0], d[0], eff_as as u16);
                    d[1] = lerp65535(s[1], d[1], eff_as as u16);
                    d[2] = lerp65535(s[2], d[2], eff_as as u16);
                    if !clipping {
                        d[3] = 65535;
                    }
                } else {
                    let kb = div65535(eff_ba * (65535 - eff_as)) as u64;
                    d[0] = ((kb * d[0] as u64 + eff_as * s[0] as u64 + ao / 2) / ao).min(65535) as u16;
                    d[1] = ((kb * d[1] as u64 + eff_as * s[1] as u64 + ao / 2) / ao).min(65535) as u16;
                    d[2] = ((kb * d[2] as u64 + eff_as * s[2] as u64 + ao / 2) / ao).min(65535) as u16;
                    if !clipping {
                        d[3] = ao as u16;
                    }
                }
            } else {
                let cb = [d[0], d[1], d[2]];
                let cs = [s[0], s[1], s[2]];
                let blended = blend_rgb_u16(mode, cb, cs);

                let wb = eff_ba * (65535 - eff_as);
                let ws = (65535 - eff_ba) * eff_as;
                let wbs = eff_as * eff_ba;
                let denom = ao * 65535;

                for c in 0..3 {
                    let num = wb * cb[c] as u64 + ws * cs[c] as u64 + wbs * blended[c] as u64;
                    d[c] = ((num + denom / 2) / denom).min(65535) as u16;
                }
                if !clipping {
                    d[3] = ao as u16;
                }
            }
        }
    }
}
