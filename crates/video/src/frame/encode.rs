//! Checked encoder input adapters. These change storage layout, never colour space, chroma
//! sampling, range or significant bit depth. Active samples outside their declared depth,
//! mixed plane depths, empty pictures and interlaced pictures are rejected.
//!
//! [`Planar8`] and [`Planar16`] borrow packed planes when their storage already matches. They
//! otherwise pack rows and widen/narrow storage without scaling values. Native views borrow
//! the adapter, so keep it alive through `encode`. Set the encoder's dimensions, depth, colour
//! tags, aspect ratio and timestamp separately from the source [`VideoFrame`]. Codec-specific
//! metadata absent from `VideoFrame` remains unset. Encoder configuration still validates
//! profile restrictions and size limits.
//!
//! ```
//! use craft_video::frame::{VideoFrame, FrameError};
//! use craft_video::frame::encode::Planar16;
//! fn prepare(frame: &VideoFrame) -> Result<(), FrameError> {
//!     let input = Planar16::try_from(frame)?;
//!     #[cfg(feature = "hevc-encode")]
//!     let native = input.hevc()?; // encoder.encode(&native), while input is alive
//!     Ok(())
//! }
//! ```
//!
//! [`Rgba8`] interleaves RGB or gray (replicated into R/G/B), with opaque alpha when absent.
//! Hap accepts these bytes; VP8 also accepts them but its encoder discards alpha.

#[cfg(any(
    test,
    feature = "h264-encode",
    feature = "hevc-encode",
    feature = "av1-encode",
    feature = "mpeg2-encode",
    feature = "mpeg4-encode",
    feature = "vp9-encode",
    feature = "prores",
    feature = "dnx",
    feature = "apv-encode"
))]
use super::Subsampling;
use super::{ColorFamily, FieldOrder, FrameError, Plane, Samples, VideoFrame};
use std::borrow::Cow;

mod owned;

/// Three borrowed colour planes in Y, Cb, Cr order.
pub type YuvPlanes<'a, T> = (&'a [T], &'a [T], &'a [T]);

/// Packed planar input with samples in `u8` or `u16` storage. Construct through [`Planar8`] or
/// [`Planar16`]; padding is excluded, and the source's metadata remains available via `frame()`.
#[derive(Debug)]
pub struct PlanarInput<'a, T: Clone> {
    frame: &'a VideoFrame,
    planes: Vec<Cow<'a, [T]>>,
    alpha: Option<Cow<'a, [T]>>,
}
/// Packed 8-bit samples (accepts either source storage at bit depth 8).
pub type Planar8<'a> = PlanarInput<'a, u8>;
/// Packed 8–16-bit samples in `u16` storage, without rescaling.
pub type Planar16<'a> = PlanarInput<'a, u16>;

impl<T: Clone> PlanarInput<'_, T> {
    /// Source frame, including dimensions and metadata needed by encoder configuration.
    pub fn frame(&self) -> &VideoFrame {
        self.frame
    }
    /// Packed colour planes in the source's order.
    pub fn planes(&self) -> &[Cow<'_, [T]>] {
        &self.planes
    }
    /// Packed alpha, when present.
    pub fn alpha(&self) -> Option<&[T]> {
        self.alpha.as_deref()
    }
}

fn reserve<T>(n: usize) -> Result<Vec<T>, FrameError> {
    let bytes = n.checked_mul(std::mem::size_of::<T>()).ok_or(FrameError::Plane("encoder input size overflows"))?;
    if bytes as u64 > crate::budget::DECODE_MEMORY_BUDGET {
        return Err(FrameError::Plane("encoder input exceeds the picture memory budget"));
    }
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| FrameError::Plane("cannot allocate encoder input"))?;
    Ok(v)
}

fn validate(f: &VideoFrame) -> Result<(), FrameError> {
    if f.width == 0 || f.height == 0 {
        return Err(FrameError::Shape("encoder input is empty"));
    }
    if f.field_order != FieldOrder::Progressive {
        return Err(FrameError::Shape("encoder input must be progressive"));
    }
    let depth = f.bit_depth();
    if !(8..=16).contains(&depth) {
        return Err(FrameError::Plane("encoder input depth must be 8 to 16"));
    }
    let mut total = 0u64;
    for p in f.planes.iter().chain(f.alpha.iter()) {
        if p.bit_depth != depth {
            return Err(FrameError::Plane("encoder input plane depths differ"));
        }
        let bytes = u64::from(p.width) * u64::from(p.height);
        total = bytes.checked_mul(2).and_then(|n| total.checked_add(n)).ok_or(FrameError::Plane("encoder input size overflows"))?;
        if total > crate::budget::DECODE_MEMORY_BUDGET {
            return Err(FrameError::Plane("encoder input exceeds the picture memory budget"));
        }
        for row in 0..p.height {
            if let Some(values) = p.row_u16(row)
                && values.iter().any(|&v| u32::from(v) >= 1u32 << depth)
            {
                return Err(FrameError::Plane("sample exceeds the declared bit depth"));
            }
        }
    }
    Ok(())
}

fn len(p: &Plane) -> Result<usize, FrameError> {
    usize::try_from(u64::from(p.width) * u64::from(p.height)).map_err(|_| FrameError::Plane("encoder input size overflows"))
}

fn plane8(p: &Plane) -> Result<Cow<'_, [u8]>, FrameError> {
    let n = len(p)?;
    if let Samples::U8(v) = &p.samples
        && p.stride == p.width as usize
    {
        return v.get(..n).map(Cow::Borrowed).ok_or(FrameError::Plane("short plane"));
    }
    let mut out = reserve(n)?;
    for row in 0..p.height {
        match &p.samples {
            Samples::U8(_) => out.extend_from_slice(p.row_u8(row).ok_or(FrameError::Plane("short row"))?),
            Samples::U16(_) => {
                for &v in p.row_u16(row).ok_or(FrameError::Plane("short row"))? {
                    out.push(u8::try_from(v).map_err(|_| FrameError::Plane("sample does not fit u8"))?);
                }
            }
        }
    }
    Ok(Cow::Owned(out))
}

fn plane16(p: &Plane) -> Result<Cow<'_, [u16]>, FrameError> {
    let n = len(p)?;
    if let Samples::U16(v) = &p.samples
        && p.stride == p.width as usize
    {
        return v.get(..n).map(Cow::Borrowed).ok_or(FrameError::Plane("short plane"));
    }
    let mut out = reserve(n)?;
    for row in 0..p.height {
        match &p.samples {
            Samples::U16(_) => out.extend_from_slice(p.row_u16(row).ok_or(FrameError::Plane("short row"))?),
            Samples::U8(_) => out.extend(p.row_u8(row).ok_or(FrameError::Plane("short row"))?.iter().copied().map(u16::from)),
        }
    }
    Ok(Cow::Owned(out))
}

impl<'a> TryFrom<&'a VideoFrame> for Planar8<'a> {
    type Error = FrameError;
    fn try_from(f: &'a VideoFrame) -> Result<Self, FrameError> {
        validate(f)?;
        if f.bit_depth() != 8 {
            return Err(FrameError::Plane("8-bit input required; rescale explicitly"));
        }
        Ok(Self { frame: f, planes: f.planes.iter().map(plane8).collect::<Result<_, _>>()?, alpha: f.alpha.as_ref().map(plane8).transpose()? })
    }
}
impl<'a> TryFrom<&'a VideoFrame> for Planar16<'a> {
    type Error = FrameError;
    fn try_from(f: &'a VideoFrame) -> Result<Self, FrameError> {
        validate(f)?;
        Ok(Self { frame: f, planes: f.planes.iter().map(plane16).collect::<Result<_, _>>()?, alpha: f.alpha.as_ref().map(plane16).transpose()? })
    }
}

// These input structs have a single shared chroma stride; packing also handles source planes
// with different strides, and VP9's slice-only API requires packed rows.
#[cfg(any(
    feature = "h264-encode",
    feature = "hevc-encode",
    feature = "av1-encode",
    feature = "mpeg2-encode",
    feature = "mpeg4-encode",
    feature = "vp9-encode"
))]
impl<T: Clone> PlanarInput<'_, T> {
    fn yuv(&self, subs: &[Subsampling], depths: &[u8]) -> Result<YuvPlanes<'_, T>, FrameError> {
        if self.frame.family != ColorFamily::YCbCr || !subs.contains(&self.frame.subsampling) || self.alpha.is_some() {
            return Err(FrameError::Shape("encoder requires YCbCr with supported subsampling and no alpha"));
        }
        if !depths.contains(&self.frame.bit_depth()) {
            return Err(FrameError::Plane("unsupported encoder input depth"));
        }
        let [y, u, v] = self.planes.as_slice() else {
            return Err(FrameError::Shape("three colour planes required"));
        };
        Ok((y, u, v))
    }
}

impl Planar8<'_> {
    /// H.264 8-bit 4:2:0 view. Configure the encoder for this frame's dimensions.
    #[cfg(feature = "h264-encode")]
    pub fn h264(&self) -> Result<crate::h264::YuvFrame<'_>, FrameError> {
        let (y, u, v) = self.yuv(&[Subsampling::S420], &[8])?;
        Ok(crate::h264::YuvFrame { y, u, v, y_stride: self.frame.width as usize, uv_stride: self.frame.width.div_ceil(2) as usize })
    }
    /// MPEG-2 8-bit 4:2:0 or 4:2:2 view. Match the encoder's chroma configuration.
    #[cfg(feature = "mpeg2-encode")]
    pub fn mpeg2(&self) -> Result<crate::mpeg2::YuvFrame<'_>, FrameError> {
        let (y, cb, cr) = self.yuv(&[Subsampling::S420, Subsampling::S422], &[8])?;
        Ok(crate::mpeg2::YuvFrame { y, cb, cr, y_stride: self.frame.width as usize, c_stride: self.frame.width.div_ceil(2) as usize })
    }
    /// MPEG-4 Part 2 8-bit 4:2:0 view.
    #[cfg(feature = "mpeg4-encode")]
    pub fn mpeg4(&self) -> Result<crate::mpeg4::YuvFrame<'_>, FrameError> {
        let (y, u, v) = self.yuv(&[Subsampling::S420], &[8])?;
        Ok(crate::mpeg4::YuvFrame { y, u, v, y_stride: self.frame.width as usize, uv_stride: self.frame.width.div_ceil(2) as usize })
    }
    /// Packed 8-bit 4:2:0 slices for VP9's `encode_frame(y, u, v, force_key)`.
    #[cfg(feature = "vp9-encode")]
    pub fn vp9(&self) -> Result<YuvPlanes<'_, u8>, FrameError> {
        self.yuv(&[Subsampling::S420], &[8])
    }
}
impl Planar16<'_> {
    /// H.264 high-depth view. Configure the encoder with the same 9- or 10-bit depth.
    #[cfg(feature = "h264-encode")]
    pub fn h264(&self) -> Result<crate::h264::YuvFrame16<'_>, FrameError> {
        let (y, u, v) = self.yuv(&[Subsampling::S420], &[9, 10])?;
        Ok(crate::h264::YuvFrame16 { y, u, v, y_stride: self.frame.width as usize, uv_stride: self.frame.width.div_ceil(2) as usize })
    }
    /// HEVC 8- or 10-bit 4:2:0 view. Match the encoder profile to the source depth.
    #[cfg(feature = "hevc-encode")]
    pub fn hevc(&self) -> Result<crate::hevc::Frame<'_>, FrameError> {
        let (y, u, v) = self.yuv(&[Subsampling::S420], &[8, 10])?;
        Ok(crate::hevc::Frame { y, u, v, y_stride: self.frame.width as usize, uv_stride: self.frame.width.div_ceil(2) as usize })
    }
    /// AV1 8- or 10-bit 4:2:0 view. Match the configured depth to the source depth.
    #[cfg(feature = "av1-encode")]
    pub fn av1(&self) -> Result<crate::av1::Frame<'_>, FrameError> {
        let (y, u, v) = self.yuv(&[Subsampling::S420], &[8, 10])?;
        Ok(crate::av1::Frame { y, u, v, y_stride: self.frame.width as usize, uv_stride: self.frame.width.div_ceil(2) as usize })
    }
}

/// Interleaved 8-bit RGBA pixels for Hap and VP8's `encode_rgba` APIs. No YCbCr-to-RGB
/// conversion is performed. The caller chooses a Hap variant that preserves alpha, or explicitly
/// accepts alpha loss in VP8 / opaque Hap variants.
#[derive(Debug)]
pub struct Rgba8 {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}
impl TryFrom<&VideoFrame> for Rgba8 {
    type Error = FrameError;
    fn try_from(f: &VideoFrame) -> Result<Self, FrameError> {
        let p = Planar8::try_from(f)?;
        if !matches!(f.family, ColorFamily::Rgb | ColorFamily::Gray) {
            return Err(FrameError::Shape("RGBA input requires RGB or gray"));
        }
        let n = len(f.planes.first().ok_or(FrameError::Shape("missing plane"))?)?;
        let mut pixels = reserve(n.checked_mul(4).ok_or(FrameError::Shape("RGBA size overflows"))?)?;
        for i in 0..n {
            let first = p.planes.first().and_then(|p| p.get(i)).copied().ok_or(FrameError::Plane("short plane"))?;
            for channel in 0..3 {
                let v = if f.family == ColorFamily::Gray {
                    first
                } else {
                    p.planes.get(channel).and_then(|p| p.get(i)).copied().ok_or(FrameError::Plane("short plane"))?
                };
                pixels.push(v);
            }
            pixels.push(p.alpha.as_ref().and_then(|a| a.get(i)).copied().unwrap_or(255));
        }
        Ok(Self { width: f.width, height: f.height, pixels })
    }
}

#[cfg(test)]
mod tests;
