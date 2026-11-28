// SPDX-License-Identifier: MIT OR Apache-2.0
//! Dependency-free writers for high dynamic range images: OpenEXR (uncompressed scanlines, half or float RGBA) and
//! Radiance HDR (RGBE). EXR writes values as they are, so results above 1.0 or below 0.0 survive (to the sample
//! precision); RGBE stores no sign, so negative components become zero.
use crate::math;
use crate::tex::Tex;

/// f32 -> IEEE half, rounded to nearest even; overflow becomes infinity, tiny values become zero or subnormals.
pub fn f32_to_half(f: f32) -> u16 {
    let b = f.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let man = b & 0x007f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = man | 0x0080_0000;
        let shift = (14 - e) as u32;
        let mut h = (m >> shift) as u16;
        let rem = m & ((1 << shift) - 1);
        let half = 1u32 << (shift - 1);
        if rem > half || (rem == half && (h & 1) == 1) {
            h += 1;
        }
        return sign | h;
    }
    let mut h = sign | ((e as u16) << 10) | (man >> 13) as u16;
    let rem = man & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && (h & 1) == 1) {
        h += 1; // may carry into the exponent, which is the correct rounding
    }
    h
}

fn attr(out: &mut Vec<u8>, name: &str, ty: &str, value: &[u8]) {
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    out.extend_from_slice(ty.as_bytes());
    out.push(0);
    out.extend_from_slice(&(value.len() as i32).to_le_bytes());
    out.extend_from_slice(value);
}

/// OpenEXR, single part, scanline, no compression. `float` selects 32-bit float channels, otherwise half.
pub fn encode_exr(t: &Tex, float: bool) -> Vec<u8> {
    let (w, h) = (t.w, t.h);
    let ptype: i32 = if float { 2 } else { 1 };
    let bytes = if float { 4 } else { 2 };
    let mut out = vec![0x76, 0x2f, 0x31, 0x01, 2, 0, 0, 0];
    let mut ch = Vec::new();
    for name in ["A", "B", "G", "R"] {
        ch.extend_from_slice(name.as_bytes());
        ch.push(0);
        ch.extend_from_slice(&ptype.to_le_bytes());
        ch.extend_from_slice(&[0, 0, 0, 0]); // pLinear + 3 reserved
        ch.extend_from_slice(&1i32.to_le_bytes());
        ch.extend_from_slice(&1i32.to_le_bytes());
    }
    ch.push(0);
    attr(&mut out, "channels", "chlist", &ch);
    attr(&mut out, "compression", "compression", &[0]);
    let mut bx = Vec::new();
    for v in [0i32, 0, w as i32 - 1, h as i32 - 1] {
        bx.extend_from_slice(&v.to_le_bytes());
    }
    attr(&mut out, "dataWindow", "box2i", &bx);
    attr(&mut out, "displayWindow", "box2i", &bx);
    attr(&mut out, "lineOrder", "lineOrder", &[0]);
    attr(&mut out, "pixelAspectRatio", "float", &1.0f32.to_le_bytes());
    let mut sc = Vec::new();
    sc.extend_from_slice(&0.0f32.to_le_bytes());
    sc.extend_from_slice(&0.0f32.to_le_bytes());
    attr(&mut out, "screenWindowCenter", "v2f", &sc);
    attr(&mut out, "screenWindowWidth", "float", &1.0f32.to_le_bytes());
    out.push(0); // end of header
    let line_bytes = 4 * w * bytes;
    let table = out.len();
    let first = table + 8 * h;
    for y in 0..h {
        out.extend_from_slice(&((first + y * (8 + line_bytes)) as u64).to_le_bytes());
    }
    for y in 0..h {
        out.extend_from_slice(&(y as i32).to_le_bytes());
        out.extend_from_slice(&(line_bytes as u32).to_le_bytes());
        for c in [3usize, 2, 1, 0] {
            for x in 0..w {
                // A short pixel buffer (never the case for a valid Tex) reads as zeros instead of panicking.
                let v = t.px.get(y * w + x).and_then(|p| p.get(c)).copied().unwrap_or(0.0);
                if float {
                    out.extend_from_slice(&v.to_le_bytes());
                } else {
                    out.extend_from_slice(&f32_to_half(v).to_le_bytes());
                }
            }
        }
    }
    out
}

/// Radiance RGBE (`.hdr`), uncompressed pixels, alpha is dropped.
pub fn encode_hdr(t: &Tex) -> Vec<u8> {
    let mut out = format!("#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y {} +X {}\n", t.h, t.w).into_bytes();
    for p in &t.px {
        let m = p[0].max(p[1]).max(p[2]);
        // (NaN too: it isn't finite)
        if m <= 1e-32 || !m.is_finite() {
            out.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        // m = frac * 2^e with frac in [0.5, 1)
        let e = math::log2(m).floor() as i32 + 1;
        let scale = 256.0 / math::powi(2f32, e);
        out.extend_from_slice(&[(p[0].max(0.0) * scale) as u8, (p[1].max(0.0) * scale) as u8, (p[2].max(0.0) * scale) as u8, (e + 128) as u8]);
    }
    out
}

/// IEEE half -> f32 (exact).
pub fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0f32 } else { 1.0 };
    let e = ((h >> 10) & 0x1f) as i32;
    let m = (h & 0x3ff) as f32;
    match e {
        0 => sign * m * math::powi(2.0f32, -24),
        0x1f => {
            if m == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * (1.0 + m / 1024.0) * math::powi(2.0f32, e - 15),
    }
}
