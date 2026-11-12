//! 3D lookup tables for applying a display transform on the GPU.

use crate::lutfile::MAX_SIZE;
use crate::{CmsError, Transform, invalid_arg};

/// An `size³` RGBA lookup table: entry `r + g·size + b·size²` holds the output for input
/// `(r, g, b) / (size − 1)`, alpha 1. This is the memory order of a 3D texture whose x axis is
/// red, so it uploads directly (sample at `(c·(size−1) + 0.5) / size` with linear filtering).
#[derive(Clone, Debug, PartialEq)]
pub struct Lut3d {
    size: usize,
    data: Vec<[f32; 4]>,
}

impl Lut3d {
    /// Samples a 3-input transform (1-output transforms are replicated to gray) on a `size`³
    /// grid, `size` from 2 to [`MAX_SIZE`].
    pub fn from_transform(t: &Transform, size: usize) -> Result<Lut3d, CmsError> {
        if t.inputs() != 3 || !(t.outputs() == 3 || t.outputs() == 1) {
            return Err(invalid_arg(format!("display LUTs need a 3→3 or 3→1 transform, not {}→{}", t.inputs(), t.outputs())));
        }
        if !(2..=MAX_SIZE).contains(&size) {
            return Err(invalid_arg(format!("3D LUT size {size} (2 to {MAX_SIZE})")));
        }
        let mut data = vec![[0.0f32, 0.0, 0.0, 1.0]; size * size * size];
        let s = (size - 1) as f32;
        let mut out = [0.0f32; 16];
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    t.eval(&[r as f32 / s, g as f32 / s, b as f32 / s], &mut out)?;
                    let v = if t.outputs() == 1 { [out[0]; 3] } else { [out[0], out[1], out[2]] };
                    data[r + g * size + b * size * size] = [v[0], v[1], v[2], 1.0];
                }
            }
        }
        Ok(Lut3d { size, data })
    }

    /// Samples `f` (RGB in, RGB out) on a `size`³ grid, `size` from 2 to [`MAX_SIZE`]: a display
    /// chain that is more than one [`Transform`].
    pub fn from_fn(size: usize, mut f: impl FnMut([f32; 3]) -> [f32; 3]) -> Result<Lut3d, CmsError> {
        if !(2..=MAX_SIZE).contains(&size) {
            return Err(invalid_arg(format!("3D LUT size {size} (2 to {MAX_SIZE})")));
        }
        let s = (size - 1) as f32;
        let mut data = Vec::with_capacity(size * size * size);
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    let v = f([r as f32 / s, g as f32 / s, b as f32 / s]);
                    data.push([v[0], v[1], v[2], 1.0]);
                }
            }
        }
        Ok(Lut3d { size, data })
    }

    /// Edge length.
    pub fn size(&self) -> usize {
        self.size
    }

    /// `size`³ RGBA entries, red fastest.
    pub fn data(&self) -> &[[f32; 4]] {
        &self.data
    }

    /// Trilinear lookup (what GPU filtering computes).
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        let n = self.size;
        let loc = |v: f32| {
            let p = v.clamp(0.0, 1.0) * (n - 1) as f32;
            let i = (p as usize).min(n - 2);
            (i, p - i as f32)
        };
        let (r0, fr) = loc(rgb[0]);
        let (g0, fg) = loc(rgb[1]);
        let (b0, fb) = loc(rgb[2]);
        let at = |r: usize, g: usize, b: usize| self.data[r + g * n + b * n * n];
        let mut o = [0.0f32; 3];
        for (k, ov) in o.iter_mut().enumerate() {
            let c = |r, g, b| at(r, g, b)[k];
            let x00 = c(r0, g0, b0) + (c(r0 + 1, g0, b0) - c(r0, g0, b0)) * fr;
            let x10 = c(r0, g0 + 1, b0) + (c(r0 + 1, g0 + 1, b0) - c(r0, g0 + 1, b0)) * fr;
            let x01 = c(r0, g0, b0 + 1) + (c(r0 + 1, g0, b0 + 1) - c(r0, g0, b0 + 1)) * fr;
            let x11 = c(r0, g0 + 1, b0 + 1) + (c(r0 + 1, g0 + 1, b0 + 1) - c(r0, g0 + 1, b0 + 1)) * fr;
            let y0 = x00 + (x10 - x00) * fg;
            let y1 = x01 + (x11 - x01) * fg;
            *ov = y0 + (y1 - y0) * fb;
        }
        o
    }

    /// Little-endian RGBA16F texels (8 bytes each), e.g. for `wgpu::TextureFormat::Rgba16Float`.
    pub fn to_rgba16f_bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(self.data.len() * 8);
        for px in &self.data {
            for v in px {
                o.extend_from_slice(&f32_to_f16(*v).to_le_bytes());
            }
        }
        o
    }

    /// RGBA8 texels (for backends without float 3D textures).
    pub fn to_rgba8(&self) -> Vec<u8> {
        self.data.iter().flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)).collect()
    }
}

/// IEEE 754 binary16 bits of `v` (round to nearest even; overflow → ±inf, NaN kept).
pub fn f32_to_f16(v: f32) -> u16 {
    let x = v.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xFF) as i32;
    let man = x & 0x7F_FFFF;
    if exp == 0xFF {
        return sign | 0x7C00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1F {
        return sign | 0x7C00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = 1u32 << (shift - 1);
        let rem = m & ((1 << shift) - 1);
        let mut r = m >> shift;
        if rem > half || (rem == half && r & 1 == 1) {
            r += 1;
        }
        return sign | r as u16;
    }
    let mut r = ((e as u32) << 10) | (man >> 13);
    let rem = man & 0x1FFF;
    if rem > 0x1000 || (rem == 0x1000 && r & 1 == 1) {
        r += 1;
    }
    sign | r as u16
}

/// binary16 bits → f32.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = ((h >> 10) & 0x1F) as i32;
    let m = (h & 0x3FF) as f32;
    match e {
        0 => sign * m * 2f32.powi(-24),
        0x1F => {
            if m == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * (1.0 + m / 1024.0) * 2f32.powi(e - 15),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_float_roundtrip() {
        for v in [0.0f32, 1.0, 0.5, 0.333, 1e-5, 65504.0, -2.25, 0.1] {
            let back = f16_to_f32(f32_to_f16(v));
            assert!((back - v).abs() <= v.abs() * 1e-3 + 1e-7, "{v} -> {back}");
        }
        assert_eq!(f32_to_f16(1.0), 0x3C00);
        assert_eq!(f32_to_f16(1e6), 0x7C00);
    }

    #[test]
    fn from_fn_samples_red_fastest_and_bounds_the_size() {
        let lut = Lut3d::from_fn(3, |[r, g, b]| [r, g * 0.5, 1.0 - b]).unwrap();
        assert_eq!(lut.size(), 3);
        assert_eq!(lut.data().len(), 27);
        assert_eq!(lut.data()[1], [0.5, 0.0, 1.0, 1.0]);
        assert_eq!(lut.data()[3], [0.0, 0.25, 1.0, 1.0]);
        assert_eq!(lut.data()[26], [1.0, 0.5, 0.0, 1.0]);
        for size in [0, 1, MAX_SIZE + 1, usize::MAX] {
            assert!(matches!(Lut3d::from_fn(size, |v| v), Err(CmsError::InvalidArgument(_))), "{size}");
        }
    }
}
