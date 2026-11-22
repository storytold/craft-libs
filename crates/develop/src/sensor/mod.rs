//! Sensor-stage RAW development over [`craft_raw::RawImage`]: black/white normalization,
//! DNG `OpcodeList1/2/3`, Bayer (AHD, PPG, Bilinear) & X-Trans demosaicing, clip-aware highlight
//! reconstruction, DNG colour matrices, `ProfileGainTableMap` evaluation, and binned previews.

pub mod binned;
pub mod chart_table;
pub mod color;
pub mod demosaic;
pub mod gaintable;
pub mod highlight;
pub mod opcodes;

pub use craft_color_transform::Mat3;
pub use craft_raw::opcodes::{Opcode, OpcodeLists};
pub use craft_raw::{BlackLevel, Cfa, ColorData, MAX_SAMPLES, RawData, RawError, RawFormat, RawImage, RawInfo, Rect, Result, stretched_height};
pub use demosaic::{Method, demosaic};

use crate::raster::Rgb32f;
use crate::raster::resample::{Filter, resize};

/// Checked product of input-derived dimensions, in u64 so it cannot wrap on 32-bit or 64-bit
/// targets. Returns `None` on overflow, zero, or when the product exceeds [`MAX_SAMPLES`].
pub fn checked_pixel_count(width: u64, height: u64) -> Option<u64> {
    width.checked_mul(height).filter(|&n| n > 0 && n <= MAX_SAMPLES as u64)
}

/// A CFA pattern this crate can address: 1 to 16 sites a side, one colour index per site.
pub fn cfa_valid(c: &Cfa) -> bool {
    c.width > 0 && c.height > 0 && c.width <= 16 && c.height <= 16 && c.pattern.len() == c.width * c.height
}

/// Black-subtracted, white-normalised data covering the active area.
#[derive(Clone, Debug, PartialEq)]
pub struct Normalized {
    pub width: usize,
    pub height: usize,
    pub cpp: usize,
    /// Nominal range [0, 1] (values may exceed 1 above white or be negative from noise).
    pub data: Vec<f32>,
    /// CFA anchored at (0, 0) of this buffer.
    pub cfa: Option<Cfa>,
}

// DNG 1.7.1, chapters 5 and 7: the integer lookup follows OpcodeList1 and precedes
// black/white normalization. The decoder has normally applied it already.
fn deferred_linearization_table(raw: &RawImage) -> Option<&[u16]> {
    if raw.linearized || !matches!(raw.data, RawData::U16(_)) {
        return None;
    }
    raw.linearization_table.as_deref().filter(|table| !table.is_empty())
}

/// Sensor development operations on [`craft_raw::RawImage`].
pub trait RawDevelopExt {
    fn validate(&self) -> Result<()>;
    fn white_at(&self, s: usize) -> f32;
    fn normalized(&self) -> Result<Normalized>;
    fn develop(&self, method: Method) -> Result<Rgb32f>;
    fn finish_demosaiced(&self, rgb: Rgb32f) -> Rgb32f;
    fn develop_crop(&self, width: usize, height: usize) -> Rect;
    fn can_bin(&self, k: usize) -> bool;
    fn develop_binned(&self, k: usize, clip: f32) -> Result<Option<Rgb32f>>;
    fn develop_binned_masked(&self, k: usize, clip: f32) -> Result<Option<(Rgb32f, Vec<u8>)>>;
}

impl RawDevelopExt for RawImage {
    fn validate(&self) -> Result<()> {
        if self.width == 0 || self.height == 0 || !(1..=4).contains(&self.cpp) {
            return Err(RawError::Corrupt("bad dimensions".into()));
        }
        if self.cpp == 2 {
            return Err(RawError::Unsupported("2 samples per pixel".into()));
        }
        let expected = checked_pixel_count(self.width as u64, self.height as u64)
            .and_then(|n| n.checked_mul(self.cpp as u64))
            .ok_or_else(|| RawError::Corrupt("data length mismatch".into()))?;
        let actual = u64::try_from(self.data.len()).map_err(|_| RawError::Corrupt("data length mismatch".into()))?;
        if actual != expected {
            return Err(RawError::Corrupt("data length mismatch".into()));
        }
        if let Some(c) = &self.cfa
            && (!cfa_valid(c) || c.pattern.iter().any(|&p| p > 2))
        {
            return Err(RawError::Unsupported(format!("CFA pattern {}", c.name())));
        }
        let a = self.active_area;
        let fits = |x: usize, len: usize, limit: usize| x.checked_add(len).is_some_and(|end| end <= limit);
        if a.width == 0 || a.height == 0 || !fits(a.x, a.width, self.width) || !fits(a.y, a.height, self.height) {
            return Err(RawError::Corrupt("active area outside image".into()));
        }
        Ok(())
    }

    fn white_at(&self, s: usize) -> f32 {
        self.white.get(s).or_else(|| self.white.first()).copied().unwrap_or(65535.0)
    }

    fn normalized(&self) -> Result<Normalized> {
        self.validate()?;
        let (w, cpp) = (self.width, self.cpp);
        let a = self.active_area;
        let stage1: Option<Vec<f32>> = if self.opcodes.list1.is_empty() {
            None
        } else {
            let mut v: Vec<f32> = (0..self.data.len()).map(|i| self.data.get(i)).collect();
            opcodes::apply_list(&self.opcodes.list1, &mut v, self.width, self.height, cpp, self.cfa.as_ref(), 65535.0);
            Some(v)
        };
        let table = deferred_linearization_table(self);
        let src = |i: usize| {
            let code = stage1.as_ref().map_or_else(|| self.data.get(i), |v| v[i]);
            if let Some(table) = table
                && code.is_finite()
            {
                // The LUT is indexed by unsigned integer codes. List1's scratch buffer
                // uses floats, so round to the nearest code at the lookup boundary.
                let index = usize::from(code.round() as u16).min(table.len() - 1);
                f32::from(table[index])
            } else {
                code
            }
        };
        let scale: Vec<f32> = (0..cpp)
            .map(|s| {
                let range = self.white_at(s) - self.black.mean();
                if range > 0.0 { 1.0 / range } else { 1.0 }
            })
            .collect();
        let mut out = vec![0f32; a.width * a.height * cpp];
        let row_len = a.width * cpp;
        crate::raster::par_rows(&mut out, row_len, |y, row| {
            let base = ((a.y + y) * w + a.x) * cpp;
            for x in 0..a.width {
                for s in 0..cpp {
                    let v = src(base + x * cpp + s);
                    let b = self.black.at(x, y, s, cpp);
                    let n = (v - b) * scale[s];
                    row[x * cpp + s] = if n.is_finite() { n } else { 0.0 };
                }
            }
        });
        let cfa = self.cfa.as_ref().map(|c| c.shifted(a.x, a.y));
        opcodes::apply_list(&self.opcodes.list2, &mut out, a.width, a.height, cpp, cfa.as_ref(), 1.0);
        Ok(Normalized { width: a.width, height: a.height, cpp, data: out, cfa })
    }

    fn develop(&self, method: Method) -> Result<Rgb32f> {
        let n = self.normalized()?;
        if self.opcodes.list3.is_empty() && method == Method::Ahd {
            let c = self.develop_crop(n.width, n.height);
            if let Some(rgb) = demosaic::ahd_window(&n, c) {
                drop(n);
                return Ok(stretched(self, rgb));
            }
        }
        let rgb = demosaic(&n, method);
        drop(n);
        Ok(self.finish_demosaiced(rgb))
    }

    fn finish_demosaiced(&self, mut rgb: Rgb32f) -> Rgb32f {
        opcodes::apply_list3(&self.opcodes.list3, &mut rgb);
        let c = self.develop_crop(rgb.width, rgb.height);
        let rgb =
            if c.x == 0 && c.y == 0 && c.width == rgb.width && c.height == rgb.height { rgb } else { rgb.into_crop(c.x, c.y, c.width, c.height) };
        stretched(self, rgb)
    }

    fn develop_crop(&self, width: usize, height: usize) -> Rect {
        let c = self.crop.clipped(width, height);
        if c.width == 0 || c.height == 0 { Rect::new(0, 0, width, height) } else { c }
    }

    fn can_bin(&self, k: usize) -> bool {
        binned::can_bin(self, k)
    }

    fn develop_binned(&self, k: usize, clip: f32) -> Result<Option<Rgb32f>> {
        binned::develop_binned(self, k, clip)
    }

    fn develop_binned_masked(&self, k: usize, clip: f32) -> Result<Option<(Rgb32f, Vec<u8>)>> {
        binned::develop_binned_masked(self, k, clip)
    }
}

fn stretched(raw: &RawImage, rgb: Rgb32f) -> Rgb32f {
    let h = stretched_height(rgb.height, raw.pixel_aspect);
    if h == rgb.height {
        return rgb;
    }
    resize(&rgb, rgb.width, h, Filter::Mitchell)
}

#[cfg(test)]
mod checked_arithmetic_tests {
    use super::*;

    #[test]
    fn checked_pixel_count_wrap_inputs_rejected() {
        assert_eq!(checked_pixel_count(65536, 65536), None);
        assert_eq!(checked_pixel_count(65536, 65537), None);
        assert_eq!(checked_pixel_count(u64::from(u32::MAX), 2), None);
        assert_eq!(checked_pixel_count(1 << 40, 1 << 40), None);
    }

    #[test]
    fn checked_pixel_count_over_cap_rejected() {
        assert_eq!(checked_pixel_count((1 << 30) + 1, 1), None);
        assert_eq!(checked_pixel_count(1, (1 << 30) + 1), None);
    }

    #[test]
    fn checked_pixel_count_at_cap_accepted() {
        assert_eq!(checked_pixel_count(1 << 30, 1), Some(1 << 30));
        assert_eq!(checked_pixel_count(1 << 15, 1 << 15), Some(1 << 30));
    }

    #[test]
    fn checked_pixel_count_small_accepted() {
        assert_eq!(checked_pixel_count(100, 100), Some(10_000));
        assert_eq!(checked_pixel_count(1, 1), Some(1));
        assert_eq!(checked_pixel_count(0, 100), None);
        assert_eq!(checked_pixel_count(100, 0), None);
    }
}
