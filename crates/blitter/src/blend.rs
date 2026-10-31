//! Pure mathematical kernels for all 27 Photoshop blend modes + 6 Paint.NET modes.
//!
//! Provides bit-exact parity with Adobe Photoshop oracle test suites and W3C/PDF specifications.

use crate::BlendMode;

/// Backdrop values within `EDGE` of 0.0 or 1.0 count as exact in modes whose result jumps there
/// (Color Burn, Color Dodge, Hard Mix).
pub const EDGE: f32 = 1e-4;

/// Photoshop-exact Vivid Light: source extremes win (`cs <= 0.0 -> 0.0`, `cs >= 1.0 -> 1.0`).
#[inline]
pub fn vivid_light_ps(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        if cs <= 0.0 { 0.0 } else { 1.0 - ((1.0 - cb) / (2.0 * cs)).min(1.0) }
    } else if cs >= 1.0 {
        1.0
    } else {
        (cb / (2.0 * (1.0 - cs))).min(1.0)
    }
}

/// Generic Vivid Light used as the threshold input for Hard Mix (backdrop extremes win).
#[inline]
pub fn vivid_light_generic(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        if cb >= 1.0 - EDGE {
            1.0
        } else if cs <= 0.0 {
            0.0
        } else {
            1.0 - ((1.0 - cb) / (2.0 * cs)).min(1.0)
        }
    } else if cb <= EDGE {
        0.0
    } else if cs >= 1.0 {
        1.0
    } else {
        (cb / (2.0 * (1.0 - cs))).min(1.0)
    }
}

/// Photoshop-exact Hard Mix: thresholded generic vivid light at `0.5 - 1e-6`.
#[inline]
pub fn hard_mix_ps(cb: f32, cs: f32) -> f32 {
    if vivid_light_generic(cb, cs) >= 0.5 - 1e-6 { 1.0 } else { 0.0 }
}

/// Photoshop-exact Color Burn: near-white backdrop preserves white ($cb \ge 1.0 - 1e-4$).
#[inline]
pub fn color_burn(cb: f32, cs: f32) -> f32 {
    if cb >= 1.0 - EDGE {
        1.0
    } else if cs <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - cb) / cs).min(1.0)
    }
}

/// Photoshop-exact Color Dodge: near-black backdrop preserves black ($cb \le 1e-4$).
#[inline]
pub fn color_dodge(cb: f32, cs: f32) -> f32 {
    if cb <= EDGE {
        0.0
    } else if cs >= 1.0 {
        1.0
    } else {
        (cb / (1.0 - cs)).min(1.0)
    }
}

/// Photoshop-exact Divide: zero-denominator guards prevent division by zero or NaN.
#[inline]
pub fn divide(cb: f32, cs: f32, hi: f32) -> f32 {
    if cs <= 0.0 { if cb <= 0.0 { 0.0 } else { 1.0 } } else { (cb / cs).min(hi) }
}

/// Hard Light separable kernel.
#[inline]
pub fn hard_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        cb * 2.0 * cs
    } else {
        let s = 2.0 * cs - 1.0;
        cb + s - cb * s
    }
}

/// Photoshop's Soft Light: the W3C/PDF curve. In the upper half (`cs > 0.5`) the backdrop moves
/// towards `D(cb)`, which is `sqrt(cb)` above a quarter and the cubic `((16·cb − 12)·cb + 4)·cb`
/// below it; with `sqrt` throughout, dark backdrops came out up to 20 levels too light.
#[inline]
pub fn soft_light_ps(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        2.0 * cb * cs + cb * cb * (1.0 - 2.0 * cs)
    } else {
        let b = cb.max(0.0);
        let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
        2.0 * cb * (1.0 - cs) + d * (2.0 * cs - 1.0)
    }
}

/// Pin Light kernel.
#[inline]
pub fn pin_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 { cb.min(2.0 * cs) } else { cb.max(2.0 * cs - 1.0) }
}

/// Paint.NET Reflect kernel.
#[inline]
pub fn reflect(cb: f32, cs: f32) -> f32 {
    if cs >= 1.0 { 1.0 } else { (cb * cb / (1.0 - cs)).min(1.0) }
}

/// Paint.NET Glow kernel (Reflect with swapped inputs).
#[inline]
pub fn glow(cb: f32, cs: f32) -> f32 {
    reflect(cs, cb)
}

/// Paint.NET Negation kernel.
#[inline]
pub fn negation(cb: f32, cs: f32) -> f32 {
    1.0 - (1.0 - cb - cs).abs()
}

/// Paint.NET Xor kernel (quantized bitwise XOR on 8-bit channels).
#[inline]
pub fn xor(cb: f32, cs: f32) -> f32 {
    let b = (cb.clamp(0.0, 1.0) * 255.0).round() as u8;
    let s = (cs.clamp(0.0, 1.0) * 255.0).round() as u8;
    f32::from(b ^ s) / 255.0
}

/// Paint.NET Color Burn (no backdrop white guard).
#[inline]
pub fn paint_net_color_burn(cb: f32, cs: f32) -> f32 {
    if cs <= 0.0 { 0.0 } else { (1.0 - (1.0 - cb) / cs).max(0.0) }
}

/// Paint.NET Color Dodge (no backdrop black guard).
#[inline]
pub fn paint_net_color_dodge(cb: f32, cs: f32) -> f32 {
    if cs >= 1.0 { 1.0 } else { (cb / (1.0 - cs)).min(1.0) }
}

/// Single-channel separable blend with `hi` clamping (1.0 for SDR, `f32::MAX` for HDR).
#[inline]
pub fn blend_channel(mode: BlendMode, cb: f32, cs: f32, hi: f32) -> f32 {
    match mode {
        BlendMode::Normal | BlendMode::Dissolve | BlendMode::PassThrough => cs,
        BlendMode::Darken => cb.min(cs),
        BlendMode::Multiply => cb * cs,
        BlendMode::ColorBurn => color_burn(cb, cs),
        BlendMode::LinearBurn => (cb + cs - 1.0).max(0.0),
        BlendMode::Lighten => cb.max(cs),
        BlendMode::Screen => cb + cs - cb * cs,
        BlendMode::ColorDodge => color_dodge(cb, cs),
        BlendMode::LinearDodge => (cb + cs).min(hi),
        BlendMode::Overlay => hard_light(cs, cb),
        BlendMode::SoftLight => soft_light_ps(cb, cs),
        BlendMode::HardLight => hard_light(cb, cs),
        BlendMode::VividLight => vivid_light_ps(cb, cs),
        BlendMode::LinearLight => (cb + 2.0 * cs - 1.0).clamp(0.0, 1.0),
        BlendMode::PinLight => pin_light(cb, cs),
        BlendMode::HardMix => hard_mix_ps(cb, cs),
        BlendMode::Difference => (cb - cs).abs(),
        BlendMode::Exclusion => cb + cs - 2.0 * cb * cs,
        BlendMode::Subtract => (cb - cs).max(0.0),
        BlendMode::Divide => divide(cb, cs, hi),
        BlendMode::Reflect => reflect(cb, cs),
        BlendMode::Glow => glow(cb, cs),
        BlendMode::Negation => negation(cb, cs),
        BlendMode::Xor => xor(cb, cs),
        BlendMode::PaintNetColorBurn => paint_net_color_burn(cb, cs),
        BlendMode::PaintNetColorDodge => paint_net_color_dodge(cb, cs),
        // Non-separable modes fall back to source channel if evaluated per channel
        BlendMode::DarkerColor | BlendMode::LighterColor | BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity => cs,
    }
}

/// Single-channel separable blend for 32-bit float documents, clipping at `f32::MAX`.
#[inline]
pub fn blend_channel_hdr(mode: BlendMode, cb: f32, cs: f32) -> f32 {
    blend_channel(mode, cb, cs, f32::MAX)
}

// ---- Non-separable color helpers (W3C Compositing and Blending Level 1) ----

/// Relative luminance calculation for RGB colors ($0.30 R + 0.59 G + 0.11 B$).
#[inline]
pub fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

/// Color saturation (max channel minus min channel).
#[inline]
pub fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

/// ClipColor: moves `c` towards its grey ([`lum`]) until every channel is within `0.0..=top`.
pub fn clip_color(c: [f32; 3], top: f32) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0.0 {
        let d = l - n;
        for v in &mut out {
            *v = if l > 0.0 && d > 0.0 { l + (*v - l) * l / d } else { l };
        }
    }
    if x > top {
        let d = x - l;
        for v in &mut out {
            *v = if l < top && d > 0.0 { l + (*v - l) * (top - l) / d } else { l };
        }
    }
    out
}

/// SetLum: adjusts luminosity of `c` to `l`, clipping into `0.0..=top`.
pub fn set_lum(c: [f32; 3], l: f32, top: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d], top)
}

/// SetSat: sets saturation `s` on RGB triple `c`.
pub fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let mut idx = [0usize, 1, 2];
    if c[idx[0]] > c[idx[1]] {
        idx.swap(0, 1);
    }
    if c[idx[1]] > c[idx[2]] {
        idx.swap(1, 2);
    }
    if c[idx[0]] > c[idx[1]] {
        idx.swap(0, 1);
    }
    let (imin, imid, imax) = (idx[0], idx[1], idx[2]);
    let mut out = [0.0f32; 3];
    if c[imax] > c[imin] {
        out[imid] = (c[imid] - c[imin]) * s / (c[imax] - c[imin]);
        out[imax] = s;
    }
    out[imin] = 0.0;
    out
}

/// Blend a full RGB triple `B(Cb, Cs)` for SDR surfaces (`top = 1.0`, `hi = 1.0`).
#[inline]
pub fn blend_rgb(mode: BlendMode, cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    blend_rgb_within(mode, cb, cs, 1.0, 1.0)
}

/// Blend a full RGB triple `B(Cb, Cs)` for 32-bit float HDR surfaces (`top = f32::MAX`, `hi = f32::MAX`).
#[inline]
pub fn blend_rgb_hdr(mode: BlendMode, cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    blend_rgb_within(mode, cb, cs, f32::MAX, f32::MAX)
}

/// General RGB triple blending kernel with custom upper gamut boundary and divide/dodge limits.
pub fn blend_rgb_within(mode: BlendMode, cb: [f32; 3], cs: [f32; 3], top: f32, hi: f32) -> [f32; 3] {
    match mode {
        BlendMode::Hue => set_lum(set_sat(cs, sat(cb)), lum(cb), top),
        BlendMode::Saturation => set_lum(set_sat(cb, sat(cs)), lum(cb), top),
        BlendMode::Color => set_lum(cs, lum(cb), top),
        BlendMode::Luminosity => set_lum(cb, lum(cs), top),
        BlendMode::DarkerColor => {
            if lum(cs) < lum(cb) {
                cs
            } else {
                cb
            }
        }
        BlendMode::LighterColor => {
            if lum(cs) > lum(cb) {
                cs
            } else {
                cb
            }
        }
        m => [blend_channel(m, cb[0], cs[0], hi), blend_channel(m, cb[1], cs[1], hi), blend_channel(m, cb[2], cs[2], hi)],
    }
}

/// Standard Photoshop text gamma setting (1.45 by default).
pub const TEXT_GAMMA: f32 = 1.45;

/// sRGB transfer function: encoded -> linear.
#[inline]
pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// sRGB transfer function: linear -> encoded.
#[inline]
pub fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Into text blending space: linear light raised to `1 / gamma`.
#[inline]
pub fn text_encode(v: f32, gamma: f32) -> f32 {
    srgb_to_linear(v.max(0.0)).powf(1.0 / gamma)
}

/// Inverse of [`text_encode`]: out of text blending space.
#[inline]
pub fn text_decode(v: f32, gamma: f32) -> f32 {
    linear_to_srgb(v.max(0.0).powf(gamma))
}
