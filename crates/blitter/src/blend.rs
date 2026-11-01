//! Separable and non-separable RGB blend-mode kernels, derived solely from published specifications.
//!
//! # Specification References
//! - **W3C Compositing and Blending Level 1** (§10.1 Separable blend modes, §10.2 Non-separable
//!   blend modes): <https://www.w3.org/TR/compositing-1/#blending>
//! - **Adobe PDF Blend Modes: Addendum** (January 23, 2006; incorporated into ISO 32000-1:2008
//!   §11.3.5): <https://www.adobe.com/content/dam/acom/en/devnet/pdf/pdf_reference_archive/blend_modes.pdf>
//! - **Khronos `GL_NV_blend_equation_advanced`** (Table X.2, Issue 34):
//!   <https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt>
//! - **Khronos OpenVG `VG_KHR_advanced_blending`** (Tables 13 & 14):
//!   <https://registry.khronos.org/OpenVG/extensions/KHR/advanced_blending.txt>
//! - **Adobe Photoshop User Guide — Blending modes** (`Darker Color`, `Lighter Color`, `Pin Light`,
//!   `Divide`): <https://helpx.adobe.com/photoshop/using/blending-modes.html>
//! - **Jens Gruschel — Pegtop Blend Modes** (`Reflect`, `Glow`, `Negation`):
//!   <http://www.pegtop.net/delphi/articles/blendmodes/>
//! - **Paint.NET Documentation — Blend Modes** (`Reflect`, `Glow`, `Negation`, `Xor`):
//!   <https://www.getpaint.net/doc/latest/BlendModes.html>

use crate::BlendMode;

/// W3C `color-burn` separable kernel:
/// `if cb == 1 -> 1; else if cs == 0 -> 0; else 1 - min(1, (1 - cb) / cs)`.
///
/// Reference: <https://www.w3.org/TR/compositing-1/#blendingcolorburn>
#[inline]
pub fn color_burn(cb: f32, cs: f32) -> f32 {
    if cb >= 1.0 {
        1.0
    } else if cs <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - cb) / cs).min(1.0)
    }
}

/// W3C `color-dodge` separable kernel:
/// `if cb == 0 -> 0; else if cs == 1 -> 1; else min(1, cb / (1 - cs))`.
///
/// Reference: <https://www.w3.org/TR/compositing-1/#blendingcolordodge>
#[inline]
pub fn color_dodge(cb: f32, cs: f32) -> f32 {
    if cb <= 0.0 {
        0.0
    } else if cs >= 1.0 {
        1.0
    } else {
        (cb / (1.0 - cs)).min(1.0)
    }
}

/// Original PDF 1.4 / ISO 32000-1:2008 Table 136 `ColorBurn` kernel (prior to the 2006 Adobe
/// Addendum, where `cb = 1` with `cs = 0` yields `0` instead of `1`).
///
/// References:
/// - Khronos `VG_KHR_advanced_blending`, Table 13 (`VG_BLEND_COLORBURN_KHR`):
///   <https://registry.khronos.org/OpenVG/extensions/KHR/advanced_blending.txt>
/// - Khronos `GL_NV_blend_equation_advanced`, Issue 34:
///   <https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt>
/// - Paint.NET Blend Modes (`Color Burn`): <https://www.getpaint.net/doc/latest/BlendModes.html>
#[inline]
pub fn classic_color_burn(cb: f32, cs: f32) -> f32 {
    if cs <= 0.0 { 0.0 } else { (1.0 - (1.0 - cb) / cs).max(0.0) }
}

/// Original PDF 1.4 / ISO 32000-1:2008 Table 136 `ColorDodge` kernel (prior to the 2006 Adobe
/// Addendum, where `cb = 0` with `cs = 1` yields `1` instead of `0`).
///
/// References:
/// - Khronos `VG_KHR_advanced_blending`, Table 13 (`VG_BLEND_COLORDODGE_KHR`):
///   <https://registry.khronos.org/OpenVG/extensions/KHR/advanced_blending.txt>
/// - Khronos `GL_NV_blend_equation_advanced`, Issue 34:
///   <https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt>
/// - Paint.NET Blend Modes (`Color Dodge`): <https://www.getpaint.net/doc/latest/BlendModes.html>
#[inline]
pub fn classic_color_dodge(cb: f32, cs: f32) -> f32 {
    if cs >= 1.0 { 1.0 } else { (cb / (1.0 - cs)).min(1.0) }
}

/// W3C `hard-light` separable kernel:
/// `if cs <= 0.5 -> 2 * cb * cs; else Screen(cb, 2 * cs - 1)`.
///
/// Reference: <https://www.w3.org/TR/compositing-1/#blendinghardlight>
#[inline]
pub fn hard_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        cb * 2.0 * cs
    } else {
        let s = 2.0 * cs - 1.0;
        cb + s - cb * s
    }
}

/// W3C `soft-light` separable kernel:
/// - `cs <= 0.5`: `cb - (1 - 2 * cs) * cb * (1 - cb)`
/// - `cs > 0.5`: `cb + (2 * cs - 1) * (D(cb) - cb)`, where
///   `D(cb) = ((16 * cb - 12) * cb + 4) * cb` for `cb <= 0.25`, else `sqrt(cb)`.
///
/// References:
/// - <https://www.w3.org/TR/compositing-1/#blendingsoftlight>
/// - <https://www.adobe.com/content/dam/acom/en/devnet/pdf/pdf_reference_archive/blend_modes.pdf>
#[inline]
pub fn soft_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        2.0 * cb * cs + cb * cb * (1.0 - 2.0 * cs)
    } else {
        let b = cb.max(0.0);
        let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
        2.0 * cb * (1.0 - cs) + d * (2.0 * cs - 1.0)
    }
}

/// Khronos `VIVIDLIGHT_NV` separable kernel:
/// - `cs <= 0`: `0`
/// - `0 < cs < 0.5`: `1 - min(1, (1 - cb) / (2 * cs))`
/// - `0.5 <= cs < 1`: `min(1, cb / (2 * (1 - cs)))`
/// - `cs >= 1`: `1`
///
/// Reference: Khronos `GL_NV_blend_equation_advanced`, Table X.2 (`VIVIDLIGHT_NV`):
/// <https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt>
#[inline]
pub fn vivid_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.0 {
        0.0
    } else if cs < 0.5 {
        1.0 - ((1.0 - cb) / (2.0 * cs)).min(1.0)
    } else if cs < 1.0 {
        (cb / (2.0 * (1.0 - cs))).min(1.0)
    } else {
        1.0
    }
}

/// Khronos `VG_BLEND_PINLIGHT_KHR` separable kernel:
/// `if cs <= 0.5 -> min(cb, 2 * cs); else max(cb, 2 * cs - 1)`.
///
/// References:
/// - Khronos `VG_KHR_advanced_blending`, Table 14 (`VG_BLEND_PINLIGHT_KHR`):
///   <https://registry.khronos.org/OpenVG/extensions/KHR/advanced_blending.txt>
/// - Adobe Blending Modes (`Pin Light`):
///   <https://helpx.adobe.com/photoshop/using/blending-modes.html>
#[inline]
pub fn pin_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 { cb.min(2.0 * cs) } else { cb.max(2.0 * cs - 1.0) }
}

/// Khronos `HARDMIX_NV` / `VG_BLEND_HARDMIX_KHR` separable kernel:
/// `if cb + cs < 1.0 -> 0.0; else 1.0`.
///
/// References:
/// - Khronos `GL_NV_blend_equation_advanced`, Table X.2 (`HARDMIX_NV`):
///   <https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt>
/// - Khronos `VG_KHR_advanced_blending`, Table 14 (`VG_BLEND_HARDMIX_KHR`):
///   <https://registry.khronos.org/OpenVG/extensions/KHR/advanced_blending.txt>
#[inline]
pub fn hard_mix(cb: f32, cs: f32) -> f32 {
    if cb + cs < 1.0 { 0.0 } else { 1.0 }
}

/// Channel division (`cb / cs`, clamped at `hi`), with an explicit finite-output guard for
/// non-positive divisors (`cs <= 0` returns `0.0` when `cb <= 0`, else `1.0`) so `0.0 / 0.0`
/// never produces `NaN`.
///
/// Reference: Adobe Blending Modes (`Divide`):
/// <https://helpx.adobe.com/photoshop/using/blending-modes.html>
#[inline]
pub fn divide(cb: f32, cs: f32, hi: f32) -> f32 {
    if cs <= 0.0 { if cb <= 0.0 { 0.0 } else { 1.0 } } else { (cb / cs).min(hi) }
}

/// Pegtop / Paint.NET `Reflect` separable kernel:
/// `if cs >= 1.0 -> 1.0; else min(1.0, cb * cb / (1.0 - cs))`.
///
/// References:
/// - <http://www.pegtop.net/delphi/articles/blendmodes/>
/// - <https://www.getpaint.net/doc/latest/BlendModes.html>
#[inline]
pub fn reflect(cb: f32, cs: f32) -> f32 {
    if cs >= 1.0 { 1.0 } else { (cb * cb / (1.0 - cs)).min(1.0) }
}

/// Pegtop / Paint.NET `Glow` separable kernel (`Reflect` with swapped arguments).
///
/// References:
/// - <http://www.pegtop.net/delphi/articles/blendmodes/>
/// - <https://www.getpaint.net/doc/latest/BlendModes.html>
#[inline]
pub fn glow(cb: f32, cs: f32) -> f32 {
    reflect(cs, cb)
}

/// Pegtop / Paint.NET `Negation` separable kernel: `1.0 - |1.0 - cb - cs|`.
///
/// References:
/// - <http://www.pegtop.net/delphi/articles/blendmodes/>
/// - <https://www.getpaint.net/doc/latest/BlendModes.html>
#[inline]
pub fn negation(cb: f32, cs: f32) -> f32 {
    1.0 - (1.0 - cb - cs).abs()
}

/// Paint.NET `Xor` separable kernel: bitwise XOR on 8-bit quantized channels (`round(v * 255)`).
///
/// Reference: <https://www.getpaint.net/doc/latest/BlendModes.html>
#[inline]
pub fn xor(cb: f32, cs: f32) -> f32 {
    let b = (cb.clamp(0.0, 1.0) * 255.0).round() as u8;
    let s = (cs.clamp(0.0, 1.0) * 255.0).round() as u8;
    f32::from(b ^ s) / 255.0
}

/// Single-channel separable blend `B(cb, cs)` with `hi` upper bound (`1.0` for SDR, `f32::MAX`
/// for HDR).
///
/// Non-separable modes require all three RGB channels (use [`blend_rgb`] or [`blend_rgb_hdr`]);
/// if passed here they return `cs`.
#[inline]
pub fn blend_channel(mode: BlendMode, cb: f32, cs: f32, hi: f32) -> f32 {
    match mode {
        BlendMode::Normal => cs,
        BlendMode::Darken => cb.min(cs),
        BlendMode::Multiply => cb * cs,
        BlendMode::ColorBurn => color_burn(cb, cs),
        BlendMode::LinearBurn => (cb + cs - 1.0).max(0.0),
        BlendMode::Lighten => cb.max(cs),
        BlendMode::Screen => cb + cs - cb * cs,
        BlendMode::ColorDodge => color_dodge(cb, cs),
        BlendMode::LinearDodge => (cb + cs).min(hi),
        BlendMode::Overlay => hard_light(cs, cb),
        BlendMode::SoftLight => soft_light(cb, cs),
        BlendMode::HardLight => hard_light(cb, cs),
        BlendMode::VividLight => vivid_light(cb, cs),
        BlendMode::LinearLight => (cb + 2.0 * cs - 1.0).clamp(0.0, 1.0),
        BlendMode::PinLight => pin_light(cb, cs),
        BlendMode::HardMix => hard_mix(cb, cs),
        BlendMode::Difference => (cb - cs).abs(),
        BlendMode::Exclusion => cb + cs - 2.0 * cb * cs,
        BlendMode::Subtract => (cb - cs).max(0.0),
        BlendMode::Divide => divide(cb, cs, hi),
        BlendMode::Reflect => reflect(cb, cs),
        BlendMode::Glow => glow(cb, cs),
        BlendMode::Negation => negation(cb, cs),
        BlendMode::Xor => xor(cb, cs),
        BlendMode::ClassicColorBurn => classic_color_burn(cb, cs),
        BlendMode::ClassicColorDodge => classic_color_dodge(cb, cs),
        BlendMode::DarkerColor | BlendMode::LighterColor | BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity => cs,
    }
}

/// Single-channel separable blend for 32-bit float HDR surfaces (`hi = f32::MAX`).
#[inline]
pub fn blend_channel_hdr(mode: BlendMode, cb: f32, cs: f32) -> f32 {
    blend_channel(mode, cb, cs, f32::MAX)
}

// ---- Non-separable color helpers (W3C Compositing and Blending Level 1, §10.2) ----

/// W3C non-separable `Lum(C) = 0.3 * C_red + 0.59 * C_green + 0.11 * C_blue`.
///
/// Reference: <https://www.w3.org/TR/compositing-1/#blendingnonseparable>
#[inline]
pub fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

/// W3C non-separable `Sat(C) = max(C) - min(C)`.
///
/// Reference: <https://www.w3.org/TR/compositing-1/#blendingnonseparable>
#[inline]
pub fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

/// W3C non-separable `ClipColor(C)` generalized to `[0.0, top]` (`top = 1.0` in SDR, `f32::MAX`
/// in unclamped HDR).
///
/// When `l <= 0.0` or `l >= top` (possible with out-of-range HDR inputs) or `d == 0.0` (a pure
/// grey), returns `l` to avoid `0.0 / 0.0` (`NaN`) and match the continuous limit of the W3C
/// scale expression as `l` approaches the boundary.
///
/// Reference: <https://www.w3.org/TR/compositing-1/#blendingnonseparable>
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

/// W3C non-separable `SetLum(C, l)` clipped to `[0.0, top]`.
///
/// Reference: <https://www.w3.org/TR/compositing-1/#blendingnonseparable>
pub fn set_lum(c: [f32; 3], l: f32, top: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d], top)
}

/// W3C non-separable `SetSat(C, s)`.
///
/// Reference: <https://www.w3.org/TR/compositing-1/#blendingnonseparable>
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

/// Blends an RGB triple `B(Cb, Cs)` for SDR `[0.0, 1.0]` surfaces.
#[inline]
pub fn blend_rgb(mode: BlendMode, cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    blend_rgb_within(mode, cb, cs, 1.0, 1.0)
}

/// Blends an RGB triple `B(Cb, Cs)` for 32-bit float HDR surfaces (`top = f32::MAX`, `hi = f32::MAX`).
#[inline]
pub fn blend_rgb_hdr(mode: BlendMode, cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    blend_rgb_within(mode, cb, cs, f32::MAX, f32::MAX)
}

/// Blends an RGB triple `B(Cb, Cs)` with upper gamut bound `top` and separable Dodge/Divide limit `hi`.
///
/// - `Hue`, `Saturation`, `Color`, `Luminosity`: W3C Compositing and Blending Level 1, §10.2:
///   <https://www.w3.org/TR/compositing-1/#blendingnonseparable>
/// - `DarkerColor`, `LighterColor`: compares the sum of all RGB channel values (`R + G + B`) and
///   selects the whole pixel with the lower or higher total, per Adobe's Blending Modes reference:
///   <https://helpx.adobe.com/photoshop/using/blending-modes.html>
pub fn blend_rgb_within(mode: BlendMode, cb: [f32; 3], cs: [f32; 3], top: f32, hi: f32) -> [f32; 3] {
    match mode {
        BlendMode::Hue => set_lum(set_sat(cs, sat(cb)), lum(cb), top),
        BlendMode::Saturation => set_lum(set_sat(cb, sat(cs)), lum(cb), top),
        BlendMode::Color => set_lum(cs, lum(cb), top),
        BlendMode::Luminosity => set_lum(cb, lum(cs), top),
        BlendMode::DarkerColor => {
            if cs[0] + cs[1] + cs[2] < cb[0] + cb[1] + cb[2] {
                cs
            } else {
                cb
            }
        }
        BlendMode::LighterColor => {
            if cs[0] + cs[1] + cs[2] > cb[0] + cb[1] + cb[2] {
                cs
            } else {
                cb
            }
        }
        m => [blend_channel(m, cb[0], cs[0], hi), blend_channel(m, cb[1], cs[1], hi), blend_channel(m, cb[2], cs[2], hi)],
    }
}
