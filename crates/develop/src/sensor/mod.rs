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

use crate::raster::resample::{Filter, resize};
use crate::raster::{Rgb32f, par_rows};
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;

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
    /// Develop camera RGB with the demosaic on the sensor's original colour scale.
    fn develop(&self, method: Method) -> Result<Rgb32f>;
    /// Demosaic after applying white balance `wb` to the mosaic, with gains relative to green.
    /// Directional interpolation then sees little colour difference across a neutral edge.
    /// The returned RGB is back on the sensor's scale for highlight reconstruction and colour conversion.
    /// Bilinear, monochrome and already demosaiced data use the same path as [`Self::develop`].
    /// Unusable gains or samples also fall back to that path.
    fn develop_balanced(&self, method: Method, wb: [f32; 3]) -> Result<Rgb32f>;
    /// [`Self::develop_balanced`] with a per-pixel sensor clip mask for [`highlight::reconstruct_masked`].
    /// The mask uses the normalised sensor threshold `clip` and covers the default crop.
    /// The mask is absent for an unclipped mosaic, data without a single-plane CFA, `OpcodeList3`,
    /// or a pixel-aspect correction that changes the output grid.
    fn develop_balanced_masked(&self, method: Method, wb: [f32; 3], clip: f32) -> Result<(Rgb32f, Option<Vec<u8>>)>;
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
        Ok(develop_full(self, self.normalized()?, method, None))
    }

    fn develop_balanced(&self, method: Method, wb: [f32; 3]) -> Result<Rgb32f> {
        Ok(develop_full(self, self.normalized()?, method, Some(wb)))
    }

    fn develop_balanced_masked(&self, method: Method, wb: [f32; 3], clip: f32) -> Result<(Rgb32f, Option<Vec<u8>>)> {
        let n = self.normalized()?;
        let mask = raw_clip_mask(self, &n, method, clip);
        Ok((develop_full(self, n, method, Some(wb)), mask))
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

fn develop_full(raw: &RawImage, mut n: Normalized, method: Method, wb: Option<[f32; 3]>) -> Rgb32f {
    let gains = wb.and_then(balance_gains).filter(|&gains| scale_mosaic(&mut n, method, gains));
    let window =
        if raw.opcodes.list3.is_empty() && method == Method::Ahd { demosaic::ahd_window(&n, raw.develop_crop(n.width, n.height)) } else { None };
    let windowed = window.is_some();
    let mut rgb = window.unwrap_or_else(|| demosaic(&n, method));
    drop(n);
    if let Some(gains) = gains {
        par_rows(&mut rgb.data, rgb.width, |_, row| {
            for p in row {
                p[0] /= gains[0];
                p[2] /= gains[2];
            }
        });
    }
    if windowed { stretched(raw, rgb) } else { raw.finish_demosaiced(rgb) }
}

fn balance_gains(wb: [f32; 3]) -> Option<[f32; 3]> {
    // A factor of 64 in either direction is already wider than camera white balances.
    const MAX_GAIN: f32 = 64.0;
    if !wb.iter().all(|v| v.is_finite() && *v > 0.0) {
        return None;
    }
    let gains = [wb[0] / wb[1], 1.0, wb[2] / wb[1]];
    (gains != [1.0; 3] && gains.iter().all(|g| (1.0 / MAX_GAIN..=MAX_GAIN).contains(g))).then_some(gains)
}

fn scale_mosaic(n: &mut Normalized, method: Method, gains: [f32; 3]) -> bool {
    let Some(cfa) = n.cfa.as_ref().filter(|_| n.cpp == 1) else { return false };
    if demosaic::mosaic_method(cfa, n.width, n.height, method) == Method::Bilinear {
        return false;
    }

    // Leave room for demosaic arithmetic and the division back to camera RGB, even for float DNGs.
    // Check the whole mosaic before scaling it so the fallback can reuse the normalised data.
    let largest_factor = gains.into_iter().map(|g| g.max(1.0 / g)).fold(1.0f32, f32::max);
    let limit = 1e30 / largest_factor;
    let fits = |v: &f32| v.abs() <= limit;
    #[cfg(not(target_arch = "wasm32"))]
    let safe = n.data.par_iter().all(fits);
    #[cfg(target_arch = "wasm32")]
    let safe = n.data.iter().all(fits);
    if !safe {
        return false;
    }
    par_rows(&mut n.data, n.width, |y, row| {
        let period: Vec<f32> = (0..cfa.width).map(|x| gains[cfa.color_at(x, y) as usize]).collect();
        for (sample, gain) in row.iter_mut().zip(period.iter().cycle()) {
            *sample *= gain;
        }
    });
    true
}

fn raw_clip_mask(raw: &RawImage, n: &Normalized, method: Method, clip: f32) -> Option<Vec<u8>> {
    let crop = raw.develop_crop(n.width, n.height);
    if !raw.opcodes.list3.is_empty() || stretched_height(crop.height, raw.pixel_aspect) != crop.height {
        return None;
    }
    clip_mask(n, crop, method, clip)
}

/// The [`highlight::reconstruct_masked`] mask for the output of [`demosaic()`] with `method` on the single-plane CFA
/// mosaic `n`, cropped to `crop` (one entry per pixel of `crop`, row-major). `n` holds sensor values normalised so
/// that 1.0 is white and before white balance; samples at or above `clip` count as clipped. Use it when the mosaic
/// is demosaicked by the caller (for example white-balanced first) rather than through [`RawDevelopExt`].
/// `None` when nothing is clipped, `n` is not a single-plane RGB CFA mosaic, `crop` does not fit inside it, or `clip`
/// is not a positive finite level.
pub fn clip_mask(n: &Normalized, crop: Rect, method: Method, clip: f32) -> Option<Vec<u8>> {
    use highlight::{CLIPPED_BITS, FROM_CLIPPED_BITS};

    // This radius covers samples read directly by the directional interpolators. Estimates
    // that build on interpolated green can depend on samples farther away.
    const REACH: usize = 2;

    let cfa = n.cfa.as_ref().filter(|c| n.cpp == 1 && cfa_valid(c) && c.pattern.iter().all(|&p| p <= 2))?;
    let (w, h) = (n.width, n.height);
    let fits = crop.width > 0
        && crop.height > 0
        && crop.x.checked_add(crop.width).is_some_and(|r| r <= w)
        && crop.y.checked_add(crop.height).is_some_and(|b| b <= h);
    if !clip.is_finite() || clip <= 0.0 || !fits || w.checked_mul(h) != Some(n.data.len()) {
        return None;
    }
    let is_clipped = |v: &f32| *v >= clip;
    #[cfg(not(target_arch = "wasm32"))]
    let has_clip = n.data.par_iter().any(is_clipped);
    #[cfg(target_arch = "wasm32")]
    let has_clip = n.data.iter().any(is_clipped);
    if !has_clip {
        return None;
    }
    if demosaic::mosaic_method(cfa, w, h, method) == Method::Bilinear {
        let mosaic = demosaic::Mosaic { w, h, data: &n.data, cfa };
        return Some(demosaic::bilinear::clip_mask(&mosaic, crop, clip));
    }

    // Collect clipped colours within reach horizontally, then combine those rows for each output pixel.
    let mut rows = vec![0u8; w * h];
    par_rows(&mut rows, w, |y, row| {
        let samples = &n.data[y * w..(y + 1) * w];
        let clipped: Vec<u8> =
            samples.iter().enumerate().map(|(x, v)| if is_clipped(v) { FROM_CLIPPED_BITS[cfa.color_at(x, y) as usize] } else { 0 }).collect();
        for (x, bits) in row.iter_mut().enumerate() {
            *bits = clipped[x.saturating_sub(REACH)..=(x + REACH).min(w - 1)].iter().fold(0, |a, b| a | b);
        }
    });
    let mut mask = vec![0u8; crop.width * crop.height];
    par_rows(&mut mask, crop.width, |oy, out| {
        let y = crop.y + oy;
        for yy in y.saturating_sub(REACH)..=(y + REACH).min(h - 1) {
            let start = yy * w + crop.x;
            out.iter_mut().zip(&rows[start..start + crop.width]).for_each(|(o, s)| *o |= s);
        }
        for (ox, bits) in out.iter_mut().enumerate() {
            let x = crop.x + ox;
            let colour = cfa.color_at(x, y);
            if is_clipped(&n.data[y * w + x]) {
                // Keep the interpolated values around a clipped measurement. They can carry thin dark detail.
                *bits = CLIPPED_BITS[colour as usize];
            } else if *bits != 0 {
                // A same-colour sample can affect the other channels through the green estimates.
                *bits |= (colour + 1) << highlight::MEASURED_SHIFT;
            }
        }
    });
    Some(mask)
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
