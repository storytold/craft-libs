//! Compositor and display video frames.
//!
//! [`VideoFrame`] is a picture the compositor, monitors and export share: planar Y'CbCr (8 or
//! 16-bit) from a decoder, sRGB-encoded RGBA8 from stills, or linear premultiplied RGBA f32 in the
//! compositor. Pixel data is `Arc`-shared so caches can hold frames for free. Colour is explicit
//! ([`craft_color::ColorInfo`]); time is a [`craft_time::Tick`].
//!
//! This is **not** the decoder-output picture type in `craft-video` (`craft_video::frame::VideoFrame`
//! in #1231 / #1768), which is a planar codec frame (per-plane bit depth, field order, colour
//! tags as H.273 codes). This type is the one `MediaSource::video_frame` returns.
//!
//! Came from FilmCraft's `filmcraft-frame` (the picture type and display conversions; the frame
//! pool and audio buffer stay out — audio is `craft_audio::AudioBuffer`).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::sync::Arc;

use craft_color::{
    ColorInfo, Matrix, Range, Transfer, linear_to_srgb_u8, normalize_c, normalize_y, srgb_u8_to_linear_table, to_linear, ycbcr_to_rgb,
};
use craft_time::Tick;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// Default per-buffer allocation ceiling (1 GiB), permitting an 8K RGBA-f32 frame.
/// Callers with a smaller budget should use the `*_with_limit` conversions.
pub const DEFAULT_MAX_FRAME_BYTES: usize = 1 << 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    InvalidLayout,
    InvalidBitDepth,
    InvalidSample,
    TooLarge,
    Allocation,
}
impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidLayout => "frame storage does not match its dimensions",
            Self::InvalidBitDepth => "unsupported frame sample bit depth",
            Self::InvalidSample => "frame sample exceeds its declared bit depth",
            Self::TooLarge => "frame exceeds the allocation limit",
            Self::Allocation => "unable to allocate frame buffer",
        })
    }
}
impl std::error::Error for FrameError {}
impl From<craft_color::InvalidBitDepth> for FrameError {
    fn from(_: craft_color::InvalidBitDepth) -> Self {
        Self::InvalidBitDepth
    }
}
pub type Result<T> = std::result::Result<T, FrameError>;

fn elements(width: u32, height: u32, channels: usize) -> Result<usize> {
    usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|n| n.checked_mul(channels))
        .ok_or(FrameError::TooLarge)
}

fn buffer<T: Default + Clone>(len: usize, max_bytes: usize) -> Result<Vec<T>> {
    if len.checked_mul(std::mem::size_of::<T>()).is_none_or(|bytes| bytes > max_bytes || bytes > isize::MAX as usize) {
        return Err(FrameError::TooLarge);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(len).map_err(|_| FrameError::Allocation)?;
    out.resize(len, T::default());
    Ok(out)
}

/// Chroma subsampling of planar Y'CbCr.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Chroma {
    C420,
    C422,
    C444,
}

impl Chroma {
    /// (horizontal shift, vertical shift)
    pub fn shifts(self) -> (u32, u32) {
        match self {
            Chroma::C420 => (1, 1),
            Chroma::C422 => (1, 0),
            Chroma::C444 => (0, 0),
        }
    }
    /// A short 4:2:0 / 4:2:2 / 4:4:4 label.
    pub fn label(self) -> &'static str {
        match self {
            Chroma::C420 => "4:2:0",
            Chroma::C422 => "4:2:2",
            Chroma::C444 => "4:4:4",
        }
    }
}

/// Pixel storage of a [`VideoFrame`].
#[derive(Clone, Debug)]
pub enum PixelData {
    /// Straight-alpha, sRGB/709-encoded RGBA, 8 bits per channel.
    Rgba8(Arc<Vec<u8>>),
    /// Premultiplied, linear-light RGBA f32 (compositor working format).
    RgbaF32(Arc<Vec<f32>>),
    /// 8-bit planar Y'CbCr (optionally with an alpha plane).
    Yuv8 { planes: [Arc<Vec<u8>>; 3], chroma: Chroma, alpha: Option<Arc<Vec<u8>>> },
    /// 9–16-bit planar Y'CbCr stored in u16 (`bits` significant bits).
    Yuv16 { planes: [Arc<Vec<u16>>; 3], chroma: Chroma, bits: u32, alpha: Option<Arc<Vec<u16>>> },
}

/// A compositor or display picture. Pixel buffers are `Arc`-shared.
#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub data: PixelData,
    pub color: ColorInfo,
    /// Pixel aspect ratio (num, den).
    pub par: (u32, u32),
    /// Presentation time in media time (informational).
    pub pts: Tick,
}

/// A rectangle of pixels: `x`, `y` is its top-left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Region {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Region {
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
    /// This rectangle cut to a `w`×`h` picture (empty when it lies outside).
    pub fn clip(self, w: usize, h: usize) -> Region {
        let x = self.x.min(w);
        let y = self.y.min(h);
        Region { x, y, w: self.w.min(w - x), h: self.h.min(h - y) }
    }
    /// Whether this rectangle is the whole `w`×`h` picture.
    pub fn is_full(&self, w: usize, h: usize) -> bool {
        (self.x, self.y, self.w, self.h) == (0, 0, w, h)
    }
}

impl VideoFrame {
    /// An sRGB-full, 1:1, zero-pts RGBA8 frame. `data` is `width × height × 4` bytes.
    pub fn rgba8(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        let frame = Self { width, height, data: PixelData::Rgba8(Arc::new(data)), color: ColorInfo::SRGB_FULL, par: (1, 1), pts: Tick::ZERO };
        frame.validate()?;
        Ok(frame)
    }

    /// A linear premultiplied RGBA f32 frame, sRGB-full tags, 1:1, zero pts.
    pub fn rgba_f32(width: u32, height: u32, data: Vec<f32>) -> Result<Self> {
        let frame = Self { width, height, data: PixelData::RgbaF32(Arc::new(data)), color: ColorInfo::SRGB_FULL, par: (1, 1), pts: Tick::ZERO };
        frame.validate()?;
        Ok(frame)
    }

    /// A transparent linear RGBA f32 frame of the given size.
    pub fn transparent_f32(width: u32, height: u32) -> Result<Self> {
        Self::rgba_f32(width, height, buffer(elements(width, height, 4)?, DEFAULT_MAX_FRAME_BYTES)?)
    }

    /// Validate all public fields before indexing pixel storage. Rechecked by conversions because
    /// callers can mutate these fields after construction. Padding beyond each required plane is allowed.
    pub fn validate(&self) -> Result<()> {
        let pixels = elements(self.width, self.height, 1)?;
        let rgba = pixels.checked_mul(4).ok_or(FrameError::TooLarge)?;
        let planar = |lengths: [usize; 3], chroma: Chroma, alpha: Option<usize>| {
            let (sx, sy) = chroma.shifts();
            let cw = (self.width as usize).div_ceil(1 << sx);
            let ch = (self.height as usize).div_ceil(1 << sy);
            let cp = cw.checked_mul(ch).ok_or(FrameError::TooLarge)?;
            if lengths[0] < pixels || lengths[1] < cp || lengths[2] < cp || alpha.is_some_and(|n| n < pixels) {
                return Err(FrameError::InvalidLayout);
            }
            Ok(())
        };
        match &self.data {
            PixelData::Rgba8(d) if d.len() == rgba => Ok(()),
            PixelData::RgbaF32(d) if d.len() == rgba => Ok(()),
            PixelData::Yuv8 { planes, chroma, alpha } => planar(planes.each_ref().map(|p| p.len()), *chroma, alpha.as_ref().map(|p| p.len())),
            PixelData::Yuv16 { planes, chroma, bits, alpha } => {
                if !(9..=16).contains(bits) {
                    return Err(FrameError::InvalidBitDepth);
                }
                planar(planes.each_ref().map(|p| p.len()), *chroma, alpha.as_ref().map(|p| p.len()))?;
                let max = (1u32 << bits) - 1;
                // Alpha is full-range at the declared depth. Out-of-range values would produce
                // opacity above one and corrupt premultiplied colors; unused padding is ignored.
                if alpha.as_ref().is_some_and(|a| a.iter().take(pixels).any(|&v| u32::from(v) > max)) {
                    return Err(FrameError::InvalidSample);
                }
                Ok(())
            }
            _ => Err(FrameError::InvalidLayout),
        }
    }

    /// Sets the presentation timestamp and returns the frame.
    pub fn with_pts(mut self, pts: Tick) -> Self {
        self.pts = pts;
        self
    }

    /// A short format label for UI and logs.
    pub fn format_label(&self) -> String {
        match &self.data {
            PixelData::Rgba8(_) => "RGBA 8-bit".into(),
            PixelData::RgbaF32(_) => "RGBA 32-bit float".into(),
            PixelData::Yuv8 { chroma, .. } => format!("YUV {} 8-bit", chroma.label()),
            PixelData::Yuv16 { chroma, bits, .. } => format!("YUV {} {bits}-bit", chroma.label()),
        }
    }

    /// Approximate memory footprint in bytes (for caches).
    pub fn byte_size(&self) -> usize {
        match &self.data {
            PixelData::Rgba8(d) => d.len(),
            PixelData::RgbaF32(d) => d.len().saturating_mul(4),
            PixelData::Yuv8 { planes, alpha, .. } => {
                planes.iter().fold(0usize, |n, p| n.saturating_add(p.len())).saturating_add(alpha.as_ref().map_or(0, |a| a.len()))
            }
            PixelData::Yuv16 { planes, alpha, .. } => {
                planes.iter().fold(0usize, |n, p| n.saturating_add(p.len())).saturating_add(alpha.as_ref().map_or(0, |a| a.len())).saturating_mul(2)
            }
        }
    }

    /// Premultiplied linear-light RGBA f32, one pixel per source pixel, row-major, 4 samples each.
    pub fn to_linear_f32(&self) -> Result<Vec<f32>> {
        self.to_linear_f32_with_limit(DEFAULT_MAX_FRAME_BYTES)
    }

    pub fn to_linear_f32_with_limit(&self, max_bytes: usize) -> Result<Vec<f32>> {
        self.validate()?;
        let (w, h) = (self.width as usize, self.height as usize);
        let mut out = buffer::<f32>(elements(self.width, self.height, 4)?, max_bytes)?;
        if w == 0 || h == 0 {
            return Ok(out);
        }
        match &self.data {
            PixelData::RgbaF32(d) => {
                let n = out.len().min(d.len());
                out[..n].copy_from_slice(&d[..n]);
            }
            PixelData::Rgba8(d) => {
                let lut = srgb_u8_to_linear_table();
                for (o, s) in out.as_chunks_mut::<4>().0.iter_mut().zip(d.as_chunks::<4>().0) {
                    let a = s[3] as f32 / 255.0;
                    o[0] = lut[s[0] as usize] * a;
                    o[1] = lut[s[1] as usize] * a;
                    o[2] = lut[s[2] as usize] * a;
                    o[3] = a;
                }
            }
            PixelData::Yuv8 { planes, chroma, alpha } => {
                let (sx, sy) = chroma.shifts();
                let cw = w.div_ceil(1 << sx);
                let info = self.color;
                let lin: Vec<f32> = (0..4096).map(|i| to_linear(i as f32 / 4095.0, info.transfer)).collect();
                let q = |v: f32| lin[(v.clamp(0.0, 1.0) * 4095.0 + 0.5) as usize];
                let ytab: Vec<f32> = (0..256).map(|v| normalize_y(v, 8, info.range)).collect::<std::result::Result<_, _>>()?;
                let ctab: Vec<f32> = (0..256).map(|v| normalize_c(v, 8, info.range)).collect::<std::result::Result<_, _>>()?;
                let rgb_of = |y: usize, x: usize| {
                    let yy = ytab[planes[0][y * w + x] as usize];
                    let u = ctab[planes[1][(y >> sy) * cw + (x >> sx)] as usize];
                    let v = ctab[planes[2][(y >> sy) * cw + (x >> sx)] as usize];
                    let rgb = ycbcr_to_rgb(yy, u, v, info.matrix);
                    let a = alpha.as_ref().map_or(1.0, |al| al[y * w + x] as f32 / 255.0);
                    [q(rgb[0]) * a, q(rgb[1]) * a, q(rgb[2]) * a, a]
                };
                for y in 0..h {
                    for x in 0..w {
                        let o = (y * w + x) * 4;
                        out[o..o + 4].copy_from_slice(&rgb_of(y, x));
                    }
                }
            }
            PixelData::Yuv16 { planes, chroma, bits, alpha } => {
                let (sx, sy) = chroma.shifts();
                let cw = w.div_ceil(1 << sx);
                let info = self.color;
                let bits = *bits;
                let amax = ((1u32 << bits) - 1) as f32;
                let lin: Vec<f32> = (0..4096).map(|i| to_linear(i as f32 / 4095.0, info.transfer)).collect();
                let q = |v: f32| lin[(v.clamp(0.0, 1.0) * 4095.0 + 0.5) as usize];
                for y in 0..h {
                    for x in 0..w {
                        let yy = normalize_y(planes[0][y * w + x] as u32, bits, info.range)?;
                        let u = normalize_c(planes[1][(y >> sy) * cw + (x >> sx)] as u32, bits, info.range)?;
                        let v = normalize_c(planes[2][(y >> sy) * cw + (x >> sx)] as u32, bits, info.range)?;
                        let rgb = ycbcr_to_rgb(yy, u, v, info.matrix);
                        let a = alpha.as_ref().map_or(1.0, |al| al[y * w + x] as f32 / amax);
                        let o = (y * w + x) * 4;
                        out[o] = q(rgb[0]) * a;
                        out[o + 1] = q(rgb[1]) * a;
                        out[o + 2] = q(rgb[2]) * a;
                        out[o + 3] = a;
                    }
                }
            }
        }
        Ok(out)
    }

    /// Convert to straight-alpha sRGB RGBA8 for display (fast path for 8-bit Rec.709/sRGB Y'CbCr
    /// without alpha, and a copy for RGBA8).
    pub fn to_rgba8(&self) -> Result<Vec<u8>> {
        self.to_rgba8_with_limit(DEFAULT_MAX_FRAME_BYTES)
    }

    pub fn to_rgba8_with_limit(&self, max_bytes: usize) -> Result<Vec<u8>> {
        self.validate()?;
        let (w, h) = (self.width as usize, self.height as usize);
        // Empty pictures have no rows; Rayon rejects a zero-sized row chunk.
        if w == 0 || h == 0 {
            return Ok(Vec::new());
        }
        match &self.data {
            PixelData::Rgba8(d) => {
                let mut out = buffer::<u8>(d.len(), max_bytes)?;
                out.copy_from_slice(d);
                Ok(out)
            }
            PixelData::Yuv8 { planes, chroma, alpha: None } if matches!(self.color.transfer, Transfer::Bt709 | Transfer::Srgb) => {
                let (sx, sy) = chroma.shifts();
                let cw = w.div_ceil(1 << sx);
                let (kr, kb) = self.color.matrix.kr_kb();
                let kg = 1.0 - kr - kb;
                let (ys, yo, cs) = match self.color.range {
                    Range::Limited => (255.0 / 219.0, 16.0, 255.0 / 224.0),
                    Range::Full => (1.0, 0.0, 1.0),
                };
                let (crr, cbb) = (2.0 * (1.0 - kr) * cs, 2.0 * (1.0 - kb) * cs);
                let (cgr, cgb) = (crr * kr / kg, cbb * kb / kg);
                let mut out = buffer::<u8>(elements(self.width, self.height, 4)?, max_bytes)?;
                out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
                    let cy = y >> sy;
                    let yrow = &planes[0][y * w..y * w + w];
                    let urow = &planes[1][cy * cw..cy * cw + cw];
                    let vrow = &planes[2][cy * cw..cy * cw + cw];
                    for x in 0..w {
                        let yy = (yrow[x] as f32 - yo) * ys;
                        let u = urow[x >> sx] as f32 - 128.0;
                        let v = vrow[x >> sx] as f32 - 128.0;
                        let o = &mut row[x * 4..x * 4 + 4];
                        o[0] = (yy + crr * v).round().clamp(0.0, 255.0) as u8;
                        o[1] = (yy - cgr * v - cgb * u).round().clamp(0.0, 255.0) as u8;
                        o[2] = (yy + cbb * u).round().clamp(0.0, 255.0) as u8;
                        o[3] = 255;
                    }
                });
                Ok(out)
            }
            _ => {
                let lin = self.to_linear_f32_with_limit(max_bytes)?;
                let mut out = buffer::<u8>(elements(self.width, self.height, 4)?, max_bytes)?;
                out.par_chunks_mut(w * 4).zip(lin.par_chunks(w * 4)).for_each(|(o, s)| linear_premul_to_srgb8(s, o));
                Ok(out)
            }
        }
    }

    /// Luma plane (8-bit, for scopes and thumbnails). Y'CbCr 8-bit returns the Y plane; others go
    /// through [`Self::to_rgba8`] and Rec.709 luma of the encoded RGB.
    pub fn luma8(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let pixels = elements(self.width, self.height, 1)?;
        let mut out = buffer::<u8>(pixels, DEFAULT_MAX_FRAME_BYTES)?;
        match &self.data {
            PixelData::Yuv8 { planes, .. } => out.copy_from_slice(&planes[0][..pixels]),
            _ => {
                let rgba = self.to_rgba8()?;
                for (o, p) in out.iter_mut().zip(rgba.as_chunks::<4>().0) {
                    *o = (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) as u8;
                }
            }
        }
        Ok(out)
    }
}

/// Convert a row of premultiplied linear f32 RGBA into straight sRGB RGBA8.
pub fn linear_premul_to_srgb8(src: &[f32], dst: &mut [u8]) {
    for (s, o) in src.as_chunks::<4>().0.iter().zip(dst.as_chunks_mut::<4>().0) {
        let a = s[3].clamp(0.0, 1.0);
        if a <= 0.0 {
            o.copy_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        let inv = 1.0 / a;
        o[0] = linear_to_srgb_u8(s[0] * inv);
        o[1] = linear_to_srgb_u8(s[1] * inv);
        o[2] = linear_to_srgb_u8(s[2] * inv);
        o[3] = (a * 255.0 + 0.5) as u8;
    }
}

/// Matrix used when a decoder does not signal one: BT.601 for SD, BT.709 otherwise (common practice).
pub fn default_matrix(width: u32, height: u32) -> Matrix {
    if width <= 1024 && height <= 576 { Matrix::Bt601 } else { Matrix::Bt709 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_storage_and_bit_depth_are_errors() {
        let make = |data| VideoFrame { width: 2, height: 2, data, color: ColorInfo::REC709, par: (1, 1), pts: Tick::ZERO };
        let short = make(PixelData::Yuv8 { planes: std::array::from_fn(|_| Arc::new(vec![0])), chroma: Chroma::C420, alpha: None });
        assert_eq!(short.to_rgba8(), Err(FrameError::InvalidLayout));
        assert_eq!(short.to_linear_f32(), Err(FrameError::InvalidLayout));
        assert_eq!(short.luma8(), Err(FrameError::InvalidLayout));
        for bits in [0, 8, 17, 32, u32::MAX] {
            let frame = make(PixelData::Yuv16 { planes: std::array::from_fn(|_| Arc::new(vec![0; 4])), chroma: Chroma::C444, bits, alpha: None });
            assert_eq!(frame.to_linear_f32(), Err(FrameError::InvalidBitDepth));
        }
        assert!(VideoFrame::rgba8(2, 2, vec![0; 15]).is_err());
        assert!(VideoFrame::rgba_f32(2, 2, vec![0.0; 15]).is_err());
        let mut valid = VideoFrame::rgba8(1, 1, vec![0; 4]).unwrap();
        valid.width = 2; // public mutations must be checked again, not just by constructors.
        assert_eq!(valid.to_linear_f32(), Err(FrameError::InvalidLayout));
    }

    #[test]
    fn odd_subsampled_frames_validate_chroma_and_alpha_independently() {
        let make = |bits| VideoFrame {
            width: 3,
            height: 3,
            data: PixelData::Yuv16 {
                planes: [Arc::new(vec![128; 9]), Arc::new(vec![128; 4]), Arc::new(vec![128; 4])],
                chroma: Chroma::C420,
                bits,
                alpha: Some(Arc::new(vec![255; 9])),
            },
            color: ColorInfo::REC709,
            par: (1, 1),
            pts: Tick::ZERO,
        };
        for bits in [9, 16] {
            let mut valid = make(bits);
            assert_eq!(valid.to_linear_f32().unwrap().len(), 36);
            assert_eq!(valid.to_rgba8().unwrap().len(), 36);
            if let PixelData::Yuv16 { planes, .. } = &mut valid.data {
                planes[1] = Arc::new(vec![128; 3]);
            }
            assert_eq!(valid.to_linear_f32(), Err(FrameError::InvalidLayout));
            let mut valid = make(bits);
            if let PixelData::Yuv16 { alpha, .. } = &mut valid.data {
                *alpha = Some(Arc::new(vec![255; 8]));
            }
            assert_eq!(valid.to_rgba8(), Err(FrameError::InvalidLayout));
        }
        let mut fast = VideoFrame {
            width: 3,
            height: 3,
            data: PixelData::Yuv8 {
                planes: [Arc::new(vec![128; 9]), Arc::new(vec![128; 4]), Arc::new(vec![128; 4])],
                chroma: Chroma::C420,
                alpha: None,
            },
            color: ColorInfo::REC709,
            par: (1, 1),
            pts: Tick::ZERO,
        };
        assert_eq!(fast.to_rgba8().unwrap().len(), 36);
        fast.height = 4;
        assert_eq!(fast.to_rgba8(), Err(FrameError::InvalidLayout));
    }

    #[test]
    fn yuv16_alpha_respects_declared_bit_depth() {
        let make = |alpha, bits| VideoFrame {
            width: 1,
            height: 1,
            data: PixelData::Yuv16 {
                planes: [Arc::new(vec![128]), Arc::new(vec![128]), Arc::new(vec![128])],
                chroma: Chroma::C444,
                bits,
                alpha: Some(Arc::new(vec![alpha])),
            },
            color: ColorInfo::REC709,
            par: (1, 1),
            pts: Tick::ZERO,
        };
        for bits in [9, 10, 12, 16] {
            let max = ((1u32 << bits) - 1) as u16;
            let valid = make(max, bits);
            assert_eq!(valid.to_linear_f32().unwrap()[3], 1.0);
            assert_eq!(valid.to_rgba8().unwrap()[3], 255);
            if bits < 16 {
                let invalid = make(max + 1, bits);
                assert!(invalid.to_linear_f32().is_err());
                assert!(invalid.to_rgba8().is_err());
                assert!(make(u16::MAX, bits).validate().is_err());
            }
        }
    }
    #[test]
    fn frame_allocations_are_checked_before_reserving() {
        assert!(matches!(VideoFrame::transparent_f32(u32::MAX, u32::MAX), Err(FrameError::TooLarge)));
        let frame = VideoFrame::rgba8(1, 1, vec![0; 4]).unwrap();
        assert_eq!(frame.to_rgba8_with_limit(3), Err(FrameError::TooLarge));
        assert_eq!(frame.to_linear_f32_with_limit(15), Err(FrameError::TooLarge));
        assert_eq!(frame.to_linear_f32_with_limit(16).unwrap(), vec![0.0; 4]);
    }

    #[test]
    fn empty_frames_convert_without_zero_sized_chunks() {
        for (w, h) in [(0, 0), (0, 1), (1, 0)] {
            let frame = VideoFrame::transparent_f32(w, h).unwrap();
            assert!(frame.to_rgba8().unwrap().is_empty());
            assert!(frame.to_linear_f32().unwrap().is_empty());
            assert!(frame.luma8().unwrap().is_empty());
        }
    }

    #[test]
    fn rgba8_roundtrips_through_to_rgba8() {
        let px = vec![10u8, 20, 30, 255, 0, 0, 0, 0];
        let f = VideoFrame::rgba8(2, 1, px.clone()).unwrap().with_pts(Tick(42));
        assert_eq!(f.pts, Tick(42));
        assert_eq!(f.to_rgba8().unwrap(), px);
        assert_eq!(f.format_label(), "RGBA 8-bit");
        assert_eq!(f.byte_size(), 8);
        assert_eq!(f.color, ColorInfo::SRGB_FULL);
    }

    #[test]
    fn rgba8_to_linear_premultiplies() {
        // 50% red, 50% alpha, sRGB encoded. Linear of 128/255, times 0.5 alpha.
        let f = VideoFrame::rgba8(1, 1, vec![128, 0, 0, 128]).unwrap();
        let lin = f.to_linear_f32().unwrap();
        let a = 128.0 / 255.0;
        let r = srgb_u8_to_linear_table()[128] * a;
        assert!((lin[0] - r).abs() < 1e-6);
        assert_eq!(lin[1], 0.0);
        assert!((lin[3] - a).abs() < 1e-6);
        let back = f.to_rgba8().unwrap();
        assert_eq!(back[0], 128);
        assert_eq!(back[3], 128);
    }

    #[test]
    fn limited_709_gray_yuv8_is_near_neutral() {
        // Limited-range mid gray: Y=126 (~0.5), Cb=Cr=128.
        let y = Arc::new(vec![126u8; 4]);
        let c = Arc::new(vec![128u8; 4]);
        let f = VideoFrame {
            width: 2,
            height: 2,
            data: PixelData::Yuv8 { planes: [y, c.clone(), c], chroma: Chroma::C444, alpha: None },
            color: ColorInfo::REC709,
            par: (1, 1),
            pts: Tick::ZERO,
        };
        let rgba = f.to_rgba8().unwrap();
        for p in rgba.as_chunks::<4>().0 {
            assert!((p[0] as i16 - p[1] as i16).abs() <= 1, "{p:?}");
            assert!((p[1] as i16 - p[2] as i16).abs() <= 1, "{p:?}");
            assert!(p[0] > 90 && p[0] < 160, "{p:?}");
            assert_eq!(p[3], 255);
        }
        assert_eq!(f.luma8().unwrap(), vec![126, 126, 126, 126]);
        assert_eq!(f.format_label(), "YUV 4:4:4 8-bit");
    }

    #[test]
    fn default_matrix_is_601_for_sd_and_709_otherwise() {
        assert_eq!(default_matrix(720, 576), Matrix::Bt601);
        assert_eq!(default_matrix(1920, 1080), Matrix::Bt709);
    }

    #[test]
    fn region_clip() {
        let r = Region { x: 8, y: 8, w: 16, h: 16 }.clip(10, 10);
        assert_eq!(r, Region { x: 8, y: 8, w: 2, h: 2 });
        assert!(Region { x: 20, y: 0, w: 4, h: 4 }.clip(10, 10).is_empty());
        assert!(Region { x: 0, y: 0, w: 10, h: 10 }.is_full(10, 10));
    }

    #[test]
    fn transparent_float_encodes_as_zero_rgba8() {
        let f = VideoFrame::transparent_f32(2, 1).unwrap();
        assert_eq!(f.to_rgba8().unwrap(), vec![0; 8]);
        assert_eq!(f.byte_size(), 32); // 2 pixels × 4 f32 channels × 4 bytes
    }
}
