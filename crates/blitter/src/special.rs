//! Special Eight fill dynamics, neutral color pull, and clipping group isolation.
//!
//! Below 100% Fill, Photoshop treats eight modes differently:
//! Color Burn, Linear Burn, Color Dodge, Linear Dodge, Vivid Light, Linear Light,
//! Hard Mix, and Difference. Rather than scaling coverage, Fill pulls the source
//! color toward the mode's neutral color, while Opacity controls overall coverage.
//! Over transparent backdrops, Fill acts as standard coverage.

use crate::BlendMode;
use crate::blend::{self, text_decode, text_encode};
use crate::fixed::{div255, div65535};

/// Returns true if the blend mode belongs to Photoshop's "Special Eight".
#[inline]
pub const fn fill_is_special(mode: BlendMode) -> bool {
    matches!(
        mode,
        BlendMode::ColorBurn
            | BlendMode::LinearBurn
            | BlendMode::ColorDodge
            | BlendMode::LinearDodge
            | BlendMode::VividLight
            | BlendMode::LinearLight
            | BlendMode::HardMix
            | BlendMode::Difference
    )
}

/// Computes the blended RGB for a Special Eight mode with fill `f` in `[0.0, 1.0]`.
pub fn blend_rgb_fill(mode: BlendMode, cb: [f32; 3], cs: [f32; 3], fill: f32, hdr: bool) -> [f32; 3] {
    let f = fill.clamp(0.0, 1.0);
    if f >= 1.0 || !fill_is_special(mode) {
        return if hdr { blend::blend_rgb_hdr(mode, cb, cs) } else { blend::blend_rgb(mode, cb, cs) };
    }

    let toward = |neutral: f32| -> [f32; 3] { std::array::from_fn(|i| neutral + (cs[i] - neutral) * f) };

    let bl = |s: [f32; 3]| {
        if hdr { blend::blend_rgb_hdr(mode, cb, s) } else { blend::blend_rgb(mode, cb, s) }
    };

    match mode {
        // Hard Mix continuous formula when fill < 1.0:
        // (cb - f * (1.0 - cs)) / (1.0 - f), clamped to [0.0, 1.0].
        BlendMode::HardMix => {
            if (1.0 - f).abs() < 1e-6 {
                bl(cs)
            } else {
                std::array::from_fn(|i| ((cb[i] - f * (1.0 - cs[i])) / (1.0 - f)).clamp(0.0, 1.0))
            }
        }
        BlendMode::ColorBurn | BlendMode::LinearBurn => bl(toward(1.0)),
        BlendMode::VividLight | BlendMode::LinearLight => bl(toward(0.5)),
        _ => bl(toward(0.0)),
    }
}

/// Composites a Special Eight mode using the 3-component weight model ($w_b, w_{bs}, w_s$).
pub fn composite_special_eight_f32(
    mode: BlendMode,
    backdrop: [f32; 4],
    source: [f32; 4],
    opacity: f32,
    fill: f32,
    gamma: f32,
    hdr: bool,
) -> [f32; 4] {
    let opacity = if opacity.is_nan() { 0.0 } else { opacity.clamp(0.0, 1.0) };
    let fill = if fill.is_nan() { 1.0 } else { fill.clamp(0.0, 1.0) };
    let ab = backdrop[3];
    let over = source[3] * opacity;
    let alone = over * fill;
    if over <= 0.0 {
        return backdrop;
    }
    let ao = ab + (1.0 - ab) * alone;
    if ao <= 0.0 {
        return [0.0; 4];
    }

    let cb = [backdrop[0], backdrop[1], backdrop[2]];
    let cs = [source[0], source[1], source[2]];
    let b = blend_rgb_fill(mode, cb, cs, fill, hdr);

    let wb = ab * (1.0 - over) / ao;
    let wbs = ab * over / ao;
    let ws = (1.0 - ab) * alone / ao;

    let mut out = [0.0f32; 4];
    for i in 0..3 {
        out[i] = if (gamma - 1.0).abs() < 1e-6 {
            wb * cb[i] + wbs * b[i] + ws * cs[i]
        } else {
            text_decode(wb * text_encode(cb[i], gamma) + wbs * text_encode(b[i], gamma) + ws * text_encode(cs[i], gamma), gamma)
        };
    }
    out[3] = ao;
    out
}

/// Fixed-point Special Eight composite for RGBA8.
pub fn composite_special_eight_u8(mode: BlendMode, backdrop: [u8; 4], source: [u8; 4], op8: u32, fill8: u32) -> [u8; 4] {
    let op8 = op8.min(255);
    let fill8 = fill8.min(255);
    let s_a = source[3] as u32;
    let over = div255(s_a * op8);
    if over == 0 {
        return backdrop;
    }
    let alone = div255(over * fill8);
    let b_a = backdrop[3] as u32;
    let ao = b_a + div255((255 - b_a) * alone);
    if ao == 0 {
        return [0, 0, 0, 0];
    }

    let cb = [backdrop[0], backdrop[1], backdrop[2]];
    let cs = [source[0], source[1], source[2]];

    // Calculate source pulled toward neutral or Hard Mix ramp
    let b: [u8; 3] = if fill8 >= 255 {
        crate::fixed::blend_rgb_u8(mode, cb, cs)
    } else {
        match mode {
            BlendMode::HardMix => {
                let denom = 255 - fill8;
                let mut h = [0u8; 3];
                for i in 0..3 {
                    let num = cb[i] as i32 * 255 - fill8 as i32 * (255 - cs[i] as i32);
                    h[i] = if num <= 0 { 0 } else { ((num + (denom as i32 / 2)) / denom as i32).min(255) as u8 };
                }
                h
            }
            BlendMode::ColorBurn | BlendMode::LinearBurn => {
                // Neutral = 255
                let s_pulled = std::array::from_fn(|i| (255 - div255((255 - cs[i] as u32) * fill8)).min(255) as u8);
                crate::fixed::blend_rgb_u8(mode, cb, s_pulled)
            }
            BlendMode::VividLight | BlendMode::LinearLight => {
                // Neutral = 128
                let s_pulled = std::array::from_fn(|i| {
                    let delta = cs[i] as i32 - 128;
                    let num = delta * fill8 as i32 + if delta >= 0 { 127 } else { -127 };
                    (128 + num / 255).clamp(0, 255) as u8
                });
                crate::fixed::blend_rgb_u8(mode, cb, s_pulled)
            }
            _ => {
                // Neutral = 0 (ColorDodge, LinearDodge, Difference)
                let s_pulled = std::array::from_fn(|i| div255(cs[i] as u32 * fill8) as u8);
                crate::fixed::blend_rgb_u8(mode, cb, s_pulled)
            }
        }
    };

    let wb = b_a * (255 - over);
    let wbs = b_a * over;
    let ws = (255 - b_a) * alone;
    let denom = ao * 255;

    let mut out = [0u8; 4];
    for c in 0..3 {
        let num = wb * cb[c] as u32 + wbs * b[c] as u32 + ws * cs[c] as u32;
        out[c] = ((num + denom / 2) / denom).min(255) as u8;
    }
    out[3] = ao.min(255) as u8;
    out
}

/// Fixed-point Special Eight composite for RGBA16.
pub fn composite_special_eight_u16(mode: BlendMode, backdrop: [u16; 4], source: [u16; 4], op16: u64, fill16: u64) -> [u16; 4] {
    let op16 = op16.min(65535);
    let fill16 = fill16.min(65535);
    let s_a = source[3] as u64;
    let over = div65535(s_a * op16) as u64;
    if over == 0 {
        return backdrop;
    }
    let alone = div65535(over * fill16) as u64;
    let b_a = backdrop[3] as u64;
    let ao = b_a + div65535((65535 - b_a) * alone) as u64;
    if ao == 0 {
        return [0, 0, 0, 0];
    }

    let cb = [backdrop[0], backdrop[1], backdrop[2]];
    let cs = [source[0], source[1], source[2]];

    let b: [u16; 3] = if fill16 >= 65535 {
        crate::fixed::blend_rgb_u16(mode, cb, cs)
    } else {
        match mode {
            BlendMode::HardMix => {
                let denom = 65535 - fill16;
                let mut h = [0u16; 3];
                for i in 0..3 {
                    let num = cb[i] as i64 * 65535 - fill16 as i64 * (65535 - cs[i] as i64);
                    h[i] = if num <= 0 { 0 } else { ((num + (denom as i64 / 2)) / denom as i64).min(65535) as u16 };
                }
                h
            }
            BlendMode::ColorBurn | BlendMode::LinearBurn => {
                let s_pulled = std::array::from_fn(|i| (65535 - div65535((65535 - cs[i] as u64) * fill16)).min(65535) as u16);
                crate::fixed::blend_rgb_u16(mode, cb, s_pulled)
            }
            BlendMode::VividLight | BlendMode::LinearLight => {
                let s_pulled = std::array::from_fn(|i| {
                    let delta = cs[i] as i64 - 32768;
                    let num = delta * fill16 as i64 + if delta >= 0 { 32767 } else { -32767 };
                    (32768 + num / 65535).clamp(0, 65535) as u16
                });
                crate::fixed::blend_rgb_u16(mode, cb, s_pulled)
            }
            _ => {
                let s_pulled = std::array::from_fn(|i| div65535(cs[i] as u64 * fill16) as u16);
                crate::fixed::blend_rgb_u16(mode, cb, s_pulled)
            }
        }
    };

    let wb = b_a * (65535 - over);
    let wbs = b_a * over;
    let ws = (65535 - b_a) * alone;
    let denom = ao * 65535;

    let mut out = [0u16; 4];
    for c in 0..3 {
        let num = wb * cb[c] as u64 + wbs * b[c] as u64 + ws * cs[c] as u64;
        out[c] = ((num + denom / 2) / denom).min(65535) as u16;
    }
    out[3] = ao.min(65535) as u16;
    out
}
