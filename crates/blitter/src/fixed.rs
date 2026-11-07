//! Deterministic fixed-point integer compositing and blitting for `RGBA8` (`[u8]`) and `RGBA16` (`[u16]`).
//!
//! # Rounding Rules
//! - **Exact nearest-integer division by 255**: [`div255`] computes `((x + 128) * 257) >> 16`,
//!   which equals `round(x / 255) = (x + 127) / 255` for every integer `x` in `0..=65535` (255 is
//!   odd, so half-integers never occur).
//! - **Exact nearest-integer division by 65535**: [`div65535`] computes
//!   `(((x + 32768) * 65537) >> 32) as u32`, which equals `round(x / 65535) = (x + 32767) / 65535`
//!   for every integer `x` in `0..=4_294_836_225` (`65535 * 65535`).
//! - **General integer ratios** (`num / denom`): rounded half-up via `(num + denom / 2) / denom`.
//! - **Float fallback quantization** (`SoftLight`, `Reflect`, `Glow`, and `Hue`/`Saturation`/`Color`/`Luminosity`):
//!   clamped to `[0.0, 1.0]` and quantized half-up via `(v * MAX + 0.5) as u8` / `as u16`.
//! - **`opacity` quantization**: `NaN` or `<= 0.0` is treated as `0`; otherwise clamped to
//!   `[0.0, 1.0]` and quantized via `(opacity * MAX + 0.5) as u32` / `as u64`.

use crate::{AlphaMode, BlendMode, CompositeOp};

/// Nearest-integer division by `255` (`round(x / 255)`), exact for all `x` in `0..=65535`.
#[inline(always)]
pub fn div255(x: u32) -> u32 {
    ((x + 128) * 257) >> 16
}

/// Nearest-integer division by `65535` (`round(x / 65535)`), exact for all `x <= 65535 * 65535`.
#[inline(always)]
pub fn div65535(x: u64) -> u32 {
    (((x + 32768) * 65537) >> 32) as u32
}

/// Nearest-integer linear interpolation `round((a * t + b * (255 - t)) / 255)` for `t` in `0..=255`.
#[inline(always)]
pub fn lerp255(a: u8, b: u8, t: u8) -> u8 {
    div255(u32::from(a) * u32::from(t) + u32::from(b) * (255 - u32::from(t))) as u8
}

/// Nearest-integer linear interpolation `round((a * t + b * (65535 - t)) / 65535)` for `t` in `0..=65535`.
#[inline(always)]
pub fn lerp65535(a: u16, b: u16, t: u16) -> u16 {
    div65535(u64::from(a) * u64::from(t) + u64::from(b) * (65535 - u64::from(t))) as u16
}

// ---- RGBA8 Integer Blend Kernels ----

/// 8-bit `ColorBurn` (`W3C` §10.1.8: <https://www.w3.org/TR/compositing-1/#blendingcolorburn>).
#[inline]
pub fn color_burn_u8(cb: u8, cs: u8) -> u8 {
    if cb == 255 {
        255
    } else if cs == 0 {
        0
    } else {
        let num = (255 - u32::from(cb)) * 255;
        let d = u32::from(cs);
        let ratio = (num + d / 2) / d;
        255 - ratio.min(255) as u8
    }
}

/// 8-bit `ColorDodge` (`W3C` §10.1.7: <https://www.w3.org/TR/compositing-1/#blendingcolordodge>).
#[inline]
pub fn color_dodge_u8(cb: u8, cs: u8) -> u8 {
    if cb == 0 {
        0
    } else if cs == 255 {
        255
    } else {
        let d = 255 - u32::from(cs);
        let ratio = (u32::from(cb) * 255 + d / 2) / d;
        ratio.min(255) as u8
    }
}

/// 8-bit `HardLight` (`W3C` §10.1.9: <https://www.w3.org/TR/compositing-1/#blendinghardlight>).
///
/// Since `127 / 255 < 0.5 < 128 / 255`, `cs < 128` selects the multiply branch (`2 * cb * cs`) and
/// `cs >= 128` selects the screen branch (`1 - 2 * (1 - cb) * (1 - cs)`).
#[inline]
pub fn hard_light_u8(cb: u8, cs: u8) -> u8 {
    if cs < 128 { div255(2 * u32::from(cb) * u32::from(cs)) as u8 } else { (255 - div255(2 * (255 - u32::from(cb)) * (255 - u32::from(cs)))) as u8 }
}

/// 8-bit `VividLight` (`Khronos` `VIVIDLIGHT_NV`:
/// <https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt>).
#[inline]
pub fn vivid_light_u8(cb: u8, cs: u8) -> u8 {
    if cs == 0 {
        0
    } else if cs == 255 {
        255
    } else if cs < 128 {
        color_burn_u8(cb, cs * 2)
    } else {
        let d = 2 * (255 - u32::from(cs));
        let ratio = (u32::from(cb) * 255 + d / 2) / d;
        ratio.min(255) as u8
    }
}

/// 8-bit `HardMix` (`Khronos` `HARDMIX_NV`: `0` if `cb + cs < 255`, else `255`).
#[inline]
pub fn hard_mix_u8(cb: u8, cs: u8) -> u8 {
    if u16::from(cb) + u16::from(cs) < 255 { 0 } else { 255 }
}

/// 8-bit `Divide` (`Adobe Blending Modes`, with `0 / 0 -> 0` and `cb > 0 / 0 -> 255`).
#[inline]
pub fn divide_u8(cb: u8, cs: u8) -> u8 {
    if cs == 0 {
        if cb == 0 { 0 } else { 255 }
    } else {
        let d = u32::from(cs);
        ((u32::from(cb) * 255 + d / 2) / d).min(255) as u8
    }
}

/// Blends a single `RGBA8` color channel in integer precision.
#[inline]
pub fn blend_channel_u8(mode: BlendMode, cb: u8, cs: u8) -> u8 {
    match mode {
        BlendMode::Normal => cs,
        BlendMode::Darken => cb.min(cs),
        BlendMode::Multiply => div255(u32::from(cb) * u32::from(cs)) as u8,
        BlendMode::ColorBurn => color_burn_u8(cb, cs),
        BlendMode::LinearBurn => (u32::from(cb) + u32::from(cs)).saturating_sub(255) as u8,
        BlendMode::Lighten => cb.max(cs),
        BlendMode::Screen => (255 - div255((255 - u32::from(cb)) * (255 - u32::from(cs)))) as u8,
        BlendMode::ColorDodge => color_dodge_u8(cb, cs),
        BlendMode::LinearDodge => cb.saturating_add(cs),
        BlendMode::Overlay => hard_light_u8(cs, cb),
        BlendMode::HardLight => hard_light_u8(cb, cs),
        BlendMode::VividLight => vivid_light_u8(cb, cs),
        BlendMode::LinearLight => (i32::from(cb) + 2 * i32::from(cs) - 255).clamp(0, 255) as u8,
        BlendMode::PinLight => {
            if cs < 128 {
                cb.min(cs * 2)
            } else {
                cb.max((2 * u16::from(cs) - 255) as u8)
            }
        }
        BlendMode::HardMix => hard_mix_u8(cb, cs),
        BlendMode::Difference => cb.abs_diff(cs),
        BlendMode::Exclusion => (u32::from(cb) + u32::from(cs)).saturating_sub(div255(2 * u32::from(cb) * u32::from(cs))) as u8,
        BlendMode::Subtract => cb.saturating_sub(cs),
        BlendMode::Divide => divide_u8(cb, cs),
        BlendMode::Xor => cb ^ cs,
        BlendMode::Negation => (255 - (255 - i32::from(cb) - i32::from(cs)).abs()) as u8,
        BlendMode::ClassicColorBurn => {
            if cs == 0 {
                0
            } else {
                color_burn_u8(cb, cs)
            }
        }
        BlendMode::ClassicColorDodge => {
            if cs == 255 {
                255
            } else {
                color_dodge_u8(cb, cs)
            }
        }
        BlendMode::SoftLight | BlendMode::Reflect | BlendMode::Glow => {
            let res = crate::blend::blend_channel(mode, f32::from(cb) / 255.0, f32::from(cs) / 255.0, 1.0);
            (res.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
        }
        BlendMode::DarkerColor | BlendMode::LighterColor | BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity => cs,
    }
}

/// Blends an `RGBA8` RGB triple `[R, G, B]`.
#[inline]
pub fn blend_rgb_u8(mode: BlendMode, cb: [u8; 3], cs: [u8; 3]) -> [u8; 3] {
    match mode {
        BlendMode::DarkerColor => {
            let sum_s = u32::from(cs[0]) + u32::from(cs[1]) + u32::from(cs[2]);
            let sum_b = u32::from(cb[0]) + u32::from(cb[1]) + u32::from(cb[2]);
            if sum_s < sum_b { cs } else { cb }
        }
        BlendMode::LighterColor => {
            let sum_s = u32::from(cs[0]) + u32::from(cs[1]) + u32::from(cs[2]);
            let sum_b = u32::from(cb[0]) + u32::from(cb[1]) + u32::from(cb[2]);
            if sum_s > sum_b { cs } else { cb }
        }
        BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity => {
            let fb = [f32::from(cb[0]) / 255.0, f32::from(cb[1]) / 255.0, f32::from(cb[2]) / 255.0];
            let fs = [f32::from(cs[0]) / 255.0, f32::from(cs[1]) / 255.0, f32::from(cs[2]) / 255.0];
            let r = crate::blend::blend_rgb(mode, fb, fs);
            [(r[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8, (r[1].clamp(0.0, 1.0) * 255.0 + 0.5) as u8, (r[2].clamp(0.0, 1.0) * 255.0 + 0.5) as u8]
        }
        _ => [blend_channel_u8(mode, cb[0], cs[0]), blend_channel_u8(mode, cb[1], cs[1]), blend_channel_u8(mode, cb[2], cs[2])],
    }
}

// ---- RGBA16 Integer Blend Kernels ----

/// 16-bit `ColorBurn` (`W3C` §10.1.8: <https://www.w3.org/TR/compositing-1/#blendingcolorburn>).
#[inline]
pub fn color_burn_u16(cb: u16, cs: u16) -> u16 {
    if cb == 65535 {
        65535
    } else if cs == 0 {
        0
    } else {
        let num = (65535 - u64::from(cb)) * 65535;
        let d = u64::from(cs);
        let ratio = (num + d / 2) / d;
        65535 - ratio.min(65535) as u16
    }
}

/// 16-bit `ColorDodge` (`W3C` §10.1.7: <https://www.w3.org/TR/compositing-1/#blendingcolordodge>).
#[inline]
pub fn color_dodge_u16(cb: u16, cs: u16) -> u16 {
    if cb == 0 {
        0
    } else if cs == 65535 {
        65535
    } else {
        let d = 65535 - u64::from(cs);
        let ratio = (u64::from(cb) * 65535 + d / 2) / d;
        ratio.min(65535) as u16
    }
}

/// 16-bit `HardLight` (`W3C` §10.1.9: <https://www.w3.org/TR/compositing-1/#blendinghardlight>).
#[inline]
pub fn hard_light_u16(cb: u16, cs: u16) -> u16 {
    if cs < 32768 {
        div65535(2 * u64::from(cb) * u64::from(cs)) as u16
    } else {
        (65535 - div65535(2 * (65535 - u64::from(cb)) * (65535 - u64::from(cs)))) as u16
    }
}

/// 16-bit `VividLight` (`Khronos` `VIVIDLIGHT_NV`:
/// <https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt>).
#[inline]
pub fn vivid_light_u16(cb: u16, cs: u16) -> u16 {
    if cs == 0 {
        0
    } else if cs == 65535 {
        65535
    } else if cs < 32768 {
        color_burn_u16(cb, cs * 2)
    } else {
        let d = 2 * (65535 - u64::from(cs));
        let ratio = (u64::from(cb) * 65535 + d / 2) / d;
        ratio.min(65535) as u16
    }
}

/// 16-bit `HardMix` (`Khronos` `HARDMIX_NV`: `0` if `cb + cs < 65535`, else `65535`).
#[inline]
pub fn hard_mix_u16(cb: u16, cs: u16) -> u16 {
    if u32::from(cb) + u32::from(cs) < 65535 { 0 } else { 65535 }
}

/// 16-bit `Divide` (`Adobe Blending Modes`, with `0 / 0 -> 0` and `cb > 0 / 0 -> 65535`).
#[inline]
pub fn divide_u16(cb: u16, cs: u16) -> u16 {
    if cs == 0 {
        if cb == 0 { 0 } else { 65535 }
    } else {
        let d = u64::from(cs);
        ((u64::from(cb) * 65535 + d / 2) / d).min(65535) as u16
    }
}

/// Blends a single `RGBA16` color channel in integer precision.
#[inline]
pub fn blend_channel_u16(mode: BlendMode, cb: u16, cs: u16) -> u16 {
    match mode {
        BlendMode::Normal => cs,
        BlendMode::Darken => cb.min(cs),
        BlendMode::Multiply => div65535(u64::from(cb) * u64::from(cs)) as u16,
        BlendMode::ColorBurn => color_burn_u16(cb, cs),
        BlendMode::LinearBurn => (u64::from(cb) + u64::from(cs)).saturating_sub(65535) as u16,
        BlendMode::Lighten => cb.max(cs),
        BlendMode::Screen => (65535 - div65535((65535 - u64::from(cb)) * (65535 - u64::from(cs)))) as u16,
        BlendMode::ColorDodge => color_dodge_u16(cb, cs),
        BlendMode::LinearDodge => cb.saturating_add(cs),
        BlendMode::Overlay => hard_light_u16(cs, cb),
        BlendMode::HardLight => hard_light_u16(cb, cs),
        BlendMode::VividLight => vivid_light_u16(cb, cs),
        BlendMode::LinearLight => (i64::from(cb) + 2 * i64::from(cs) - 65535).clamp(0, 65535) as u16,
        BlendMode::PinLight => {
            if cs < 32768 {
                cb.min(cs * 2)
            } else {
                cb.max((2 * u32::from(cs) - 65535) as u16)
            }
        }
        BlendMode::HardMix => hard_mix_u16(cb, cs),
        BlendMode::Difference => cb.abs_diff(cs),
        BlendMode::Exclusion => (u64::from(cb) + u64::from(cs)).saturating_sub(u64::from(div65535(2 * u64::from(cb) * u64::from(cs)))) as u16,
        BlendMode::Subtract => cb.saturating_sub(cs),
        BlendMode::Divide => divide_u16(cb, cs),
        BlendMode::Xor => cb ^ cs,
        BlendMode::Negation => (65535 - (65535 - i64::from(cb) - i64::from(cs)).abs()) as u16,
        BlendMode::ClassicColorBurn => {
            if cs == 0 {
                0
            } else {
                color_burn_u16(cb, cs)
            }
        }
        BlendMode::ClassicColorDodge => {
            if cs == 65535 {
                65535
            } else {
                color_dodge_u16(cb, cs)
            }
        }
        BlendMode::SoftLight | BlendMode::Reflect | BlendMode::Glow => {
            let res = crate::blend::blend_channel(mode, f32::from(cb) / 65535.0, f32::from(cs) / 65535.0, 1.0);
            (res.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16
        }
        BlendMode::DarkerColor | BlendMode::LighterColor | BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity => cs,
    }
}

/// Blends an `RGBA16` RGB triple `[R, G, B]`.
#[inline]
pub fn blend_rgb_u16(mode: BlendMode, cb: [u16; 3], cs: [u16; 3]) -> [u16; 3] {
    match mode {
        BlendMode::DarkerColor => {
            let sum_s = u64::from(cs[0]) + u64::from(cs[1]) + u64::from(cs[2]);
            let sum_b = u64::from(cb[0]) + u64::from(cb[1]) + u64::from(cb[2]);
            if sum_s < sum_b { cs } else { cb }
        }
        BlendMode::LighterColor => {
            let sum_s = u64::from(cs[0]) + u64::from(cs[1]) + u64::from(cs[2]);
            let sum_b = u64::from(cb[0]) + u64::from(cb[1]) + u64::from(cb[2]);
            if sum_s > sum_b { cs } else { cb }
        }
        BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity => {
            let fb = [f32::from(cb[0]) / 65535.0, f32::from(cb[1]) / 65535.0, f32::from(cb[2]) / 65535.0];
            let fs = [f32::from(cs[0]) / 65535.0, f32::from(cs[1]) / 65535.0, f32::from(cs[2]) / 65535.0];
            let r = crate::blend::blend_rgb(mode, fb, fs);
            [
                (r[0].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16,
                (r[1].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16,
                (r[2].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16,
            ]
        }
        _ => [blend_channel_u16(mode, cb[0], cs[0]), blend_channel_u16(mode, cb[1], cs[1]), blend_channel_u16(mode, cb[2], cs[2])],
    }
}

// ---- Serial Fixed-Point Slice Blitters ----

/// Serial fixed-point `RGBA8` blitter with zero heap allocations.
pub fn blit_rgba8_fixed(dst: &mut [u8], src: &[u8], mode: BlendMode, opacity: f32, alpha_mode: AlphaMode, op: CompositeOp) {
    if opacity.is_nan() || opacity <= 0.0 || dst.is_empty() || src.is_empty() {
        return;
    }
    let num_pixels = (dst.len() / 4).min(src.len() / 4);
    if num_pixels == 0 {
        return;
    }
    let op8 = (opacity.min(1.0) * 255.0 + 0.5) as u32;
    if op8 == 0 {
        return;
    }

    let (d_chunks, _) = dst[..num_pixels * 4].as_chunks_mut::<4>();
    let (s_chunks, _) = src[..num_pixels * 4].as_chunks::<4>();

    match alpha_mode {
        AlphaMode::Straight => {
            for (d, s) in d_chunks.iter_mut().zip(s_chunks) {
                blit_pixel_u8_straight(d, *s, mode, op8, op);
            }
        }
        AlphaMode::Premultiplied => {
            for (d, s) in d_chunks.iter_mut().zip(s_chunks) {
                blit_pixel_u8_premul(d, *s, mode, op8, op);
            }
        }
    }
}

#[inline]
fn blit_pixel_u8_straight(d: &mut [u8; 4], s: [u8; 4], mode: BlendMode, op8: u32, op: CompositeOp) {
    let s_a = u32::from(s[3]);
    if s_a == 0 {
        return;
    }
    let b_a = u32::from(d[3]);
    if op == CompositeOp::SourceAtop && b_a == 0 {
        return;
    }
    let as_ = div255(s_a * op8);
    if as_ == 0 {
        return;
    }

    if op == CompositeOp::SourceAtop {
        let cb = [d[0], d[1], d[2]];
        let cs = [s[0], s[1], s[2]];
        let blended = blend_rgb_u8(mode, cb, cs);
        d[0] = lerp255(blended[0], cb[0], as_ as u8);
        d[1] = lerp255(blended[1], cb[1], as_ as u8);
        d[2] = lerp255(blended[2], cb[2], as_ as u8);
        return;
    }

    let ao = as_ + div255(b_a * (255 - as_));
    if ao == 0 {
        *d = [0; 4];
        return;
    }

    if mode == BlendMode::Normal {
        if b_a == 255 {
            d[0] = lerp255(s[0], d[0], as_ as u8);
            d[1] = lerp255(s[1], d[1], as_ as u8);
            d[2] = lerp255(s[2], d[2], as_ as u8);
            d[3] = 255;
        } else {
            let kb = div255(b_a * (255 - as_));
            d[0] = ((kb * u32::from(d[0]) + as_ * u32::from(s[0]) + ao / 2) / ao).min(255) as u8;
            d[1] = ((kb * u32::from(d[1]) + as_ * u32::from(s[1]) + ao / 2) / ao).min(255) as u8;
            d[2] = ((kb * u32::from(d[2]) + as_ * u32::from(s[2]) + ao / 2) / ao).min(255) as u8;
            d[3] = ao as u8;
        }
        return;
    }

    let cb = [d[0], d[1], d[2]];
    let cs = [s[0], s[1], s[2]];
    let blended = blend_rgb_u8(mode, cb, cs);
    let wb = b_a * (255 - as_);
    let ws = (255 - b_a) * as_;
    let wbs = as_ * b_a;
    let denom = ao * 255;
    for c in 0..3 {
        let num = wb * u32::from(cb[c]) + ws * u32::from(cs[c]) + wbs * u32::from(blended[c]);
        d[c] = ((num + denom / 2) / denom).min(255) as u8;
    }
    d[3] = ao as u8;
}

#[inline]
fn blit_pixel_u8_premul(d: &mut [u8; 4], s: [u8; 4], mode: BlendMode, op8: u32, op: CompositeOp) {
    let s_a = u32::from(s[3]);
    if s_a == 0 {
        return;
    }
    let b_a = u32::from(d[3]);
    if op == CompositeOp::SourceAtop && b_a == 0 {
        return;
    }
    let as_ = div255(s_a * op8);
    if as_ == 0 {
        return;
    }

    if mode == BlendMode::Normal {
        let factor = 255 - as_;
        let s_r = div255(u32::from(s[0]) * op8);
        let s_g = div255(u32::from(s[1]) * op8);
        let s_b = div255(u32::from(s[2]) * op8);
        match op {
            CompositeOp::SourceOver => {
                d[0] = (s_r + div255(u32::from(d[0]) * factor)).min(255) as u8;
                d[1] = (s_g + div255(u32::from(d[1]) * factor)).min(255) as u8;
                d[2] = (s_b + div255(u32::from(d[2]) * factor)).min(255) as u8;
                d[3] = (as_ + div255(b_a * factor)).min(255) as u8;
            }
            CompositeOp::SourceAtop => {
                d[0] = (div255(s_r * b_a) + div255(u32::from(d[0]) * factor)).min(255) as u8;
                d[1] = (div255(s_g * b_a) + div255(u32::from(d[1]) * factor)).min(255) as u8;
                d[2] = (div255(s_b * b_a) + div255(u32::from(d[2]) * factor)).min(255) as u8;
            }
        }
        return;
    }

    let s_straight = [
        ((u32::from(s[0]) * 255 + s_a / 2) / s_a).min(255) as u8,
        ((u32::from(s[1]) * 255 + s_a / 2) / s_a).min(255) as u8,
        ((u32::from(s[2]) * 255 + s_a / 2) / s_a).min(255) as u8,
        s[3],
    ];
    let mut d_straight = if let Some(nz_ba) = std::num::NonZeroU32::new(b_a) {
        let div = nz_ba.get();
        [
            ((u32::from(d[0]) * 255 + div / 2) / div).min(255) as u8,
            ((u32::from(d[1]) * 255 + div / 2) / div).min(255) as u8,
            ((u32::from(d[2]) * 255 + div / 2) / div).min(255) as u8,
            d[3],
        ]
    } else {
        [0; 4]
    };

    blit_pixel_u8_straight(&mut d_straight, s_straight, mode, op8, op);
    let ao = u32::from(d_straight[3]);
    d[0] = div255(u32::from(d_straight[0]) * ao) as u8;
    d[1] = div255(u32::from(d_straight[1]) * ao) as u8;
    d[2] = div255(u32::from(d_straight[2]) * ao) as u8;
    d[3] = d_straight[3];
}

/// Serial fixed-point `RGBA16` blitter with zero heap allocations.
pub fn blit_rgba16_fixed(dst: &mut [u16], src: &[u16], mode: BlendMode, opacity: f32, alpha_mode: AlphaMode, op: CompositeOp) {
    if opacity.is_nan() || opacity <= 0.0 || dst.is_empty() || src.is_empty() {
        return;
    }
    let num_pixels = (dst.len() / 4).min(src.len() / 4);
    if num_pixels == 0 {
        return;
    }
    let op16 = (opacity.min(1.0) * 65535.0 + 0.5) as u64;
    if op16 == 0 {
        return;
    }

    let (d_chunks, _) = dst[..num_pixels * 4].as_chunks_mut::<4>();
    let (s_chunks, _) = src[..num_pixels * 4].as_chunks::<4>();

    match alpha_mode {
        AlphaMode::Straight => {
            for (d, s) in d_chunks.iter_mut().zip(s_chunks) {
                blit_pixel_u16_straight(d, *s, mode, op16, op);
            }
        }
        AlphaMode::Premultiplied => {
            for (d, s) in d_chunks.iter_mut().zip(s_chunks) {
                blit_pixel_u16_premul(d, *s, mode, op16, op);
            }
        }
    }
}

#[inline]
fn blit_pixel_u16_straight(d: &mut [u16; 4], s: [u16; 4], mode: BlendMode, op16: u64, op: CompositeOp) {
    let s_a = u64::from(s[3]);
    if s_a == 0 {
        return;
    }
    let b_a = u64::from(d[3]);
    if op == CompositeOp::SourceAtop && b_a == 0 {
        return;
    }
    let as_ = u64::from(div65535(s_a * op16));
    if as_ == 0 {
        return;
    }

    if op == CompositeOp::SourceAtop {
        let cb = [d[0], d[1], d[2]];
        let cs = [s[0], s[1], s[2]];
        let blended = blend_rgb_u16(mode, cb, cs);
        d[0] = lerp65535(blended[0], cb[0], as_ as u16);
        d[1] = lerp65535(blended[1], cb[1], as_ as u16);
        d[2] = lerp65535(blended[2], cb[2], as_ as u16);
        return;
    }

    let ao = as_ + u64::from(div65535(b_a * (65535 - as_)));
    if ao == 0 {
        *d = [0; 4];
        return;
    }

    if mode == BlendMode::Normal {
        if b_a == 65535 {
            d[0] = lerp65535(s[0], d[0], as_ as u16);
            d[1] = lerp65535(s[1], d[1], as_ as u16);
            d[2] = lerp65535(s[2], d[2], as_ as u16);
            d[3] = 65535;
        } else {
            let kb = u64::from(div65535(b_a * (65535 - as_)));
            d[0] = ((kb * u64::from(d[0]) + as_ * u64::from(s[0]) + ao / 2) / ao).min(65535) as u16;
            d[1] = ((kb * u64::from(d[1]) + as_ * u64::from(s[1]) + ao / 2) / ao).min(65535) as u16;
            d[2] = ((kb * u64::from(d[2]) + as_ * u64::from(s[2]) + ao / 2) / ao).min(65535) as u16;
            d[3] = ao as u16;
        }
        return;
    }

    let cb = [d[0], d[1], d[2]];
    let cs = [s[0], s[1], s[2]];
    let blended = blend_rgb_u16(mode, cb, cs);
    let wb = b_a * (65535 - as_);
    let ws = (65535 - b_a) * as_;
    let wbs = as_ * b_a;
    let denom = ao * 65535;
    for c in 0..3 {
        let num = wb * u64::from(cb[c]) + ws * u64::from(cs[c]) + wbs * u64::from(blended[c]);
        d[c] = ((num + denom / 2) / denom).min(65535) as u16;
    }
    d[3] = ao as u16;
}

#[inline]
fn blit_pixel_u16_premul(d: &mut [u16; 4], s: [u16; 4], mode: BlendMode, op16: u64, op: CompositeOp) {
    let s_a = u64::from(s[3]);
    if s_a == 0 {
        return;
    }
    let b_a = u64::from(d[3]);
    if op == CompositeOp::SourceAtop && b_a == 0 {
        return;
    }
    let as_ = u64::from(div65535(s_a * op16));
    if as_ == 0 {
        return;
    }

    if mode == BlendMode::Normal {
        let factor = 65535 - as_;
        let s_r = u64::from(div65535(u64::from(s[0]) * op16));
        let s_g = u64::from(div65535(u64::from(s[1]) * op16));
        let s_b = u64::from(div65535(u64::from(s[2]) * op16));
        match op {
            CompositeOp::SourceOver => {
                d[0] = (s_r + u64::from(div65535(u64::from(d[0]) * factor))).min(65535) as u16;
                d[1] = (s_g + u64::from(div65535(u64::from(d[1]) * factor))).min(65535) as u16;
                d[2] = (s_b + u64::from(div65535(u64::from(d[2]) * factor))).min(65535) as u16;
                d[3] = (as_ + u64::from(div65535(b_a * factor))).min(65535) as u16;
            }
            CompositeOp::SourceAtop => {
                d[0] = (u64::from(div65535(s_r * b_a)) + u64::from(div65535(u64::from(d[0]) * factor))).min(65535) as u16;
                d[1] = (u64::from(div65535(s_g * b_a)) + u64::from(div65535(u64::from(d[1]) * factor))).min(65535) as u16;
                d[2] = (u64::from(div65535(s_b * b_a)) + u64::from(div65535(u64::from(d[2]) * factor))).min(65535) as u16;
            }
        }
        return;
    }

    let s_straight = [
        ((u64::from(s[0]) * 65535 + s_a / 2) / s_a).min(65535) as u16,
        ((u64::from(s[1]) * 65535 + s_a / 2) / s_a).min(65535) as u16,
        ((u64::from(s[2]) * 65535 + s_a / 2) / s_a).min(65535) as u16,
        s[3],
    ];
    let mut d_straight = if let Some(nz_ba) = std::num::NonZeroU64::new(b_a) {
        let div = nz_ba.get();
        [
            ((u64::from(d[0]) * 65535 + div / 2) / div).min(65535) as u16,
            ((u64::from(d[1]) * 65535 + div / 2) / div).min(65535) as u16,
            ((u64::from(d[2]) * 65535 + div / 2) / div).min(65535) as u16,
            d[3],
        ]
    } else {
        [0; 4]
    };

    blit_pixel_u16_straight(&mut d_straight, s_straight, mode, op16, op);
    let ao = u64::from(d_straight[3]);
    d[0] = div65535(u64::from(d_straight[0]) * ao) as u16;
    d[1] = div65535(u64::from(d_straight[1]) * ao) as u16;
    d[2] = div65535(u64::from(d_straight[2]) * ao) as u16;
    d[3] = d_straight[3];
}
