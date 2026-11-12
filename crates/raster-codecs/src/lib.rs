//! Small raster image formats, one module each, decoding untrusted bytes into one flat
//! [`Image`] and (where the format is written) encoding it back.
//!
//! | Module | Format | Read | Write |
//! |---|---|---|---|
//! | [`dds`] | DirectDraw Surface: uncompressed masks, BC1-BC3 (DXT1/3/5), the DX10 header | yes | yes |
//! | [`dicom`] | DICOM Part 10, uncompressed native pixel data | yes | no |
//! | [`fits`] | FITS primary and image-extension HDUs | yes | yes |
//! | [`iff`] | IFF ILBM (palette or 24-bit planes, masks) and PBM | yes | yes (24-plane ILBM) |
//! | [`pcx`] | ZSoft PCX, palettes and 24-bit RGB | yes | no |
//! | [`pnm`] | Netpbm PBM/PGM/PPM (plain and raw), PAM, PFM | yes | yes |
//! | [`xpm`] | X PixMap, XPM3 and XPM2 | yes | no |
//!
//! Every decoder takes [`Limits`] and checks the declared size against them before it
//! allocates. Nothing here panics on malformed input. Encoders take an image already in a
//! layout and sample type the format stores and refuse any other: choosing and converting is
//! the caller's business.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod dds;
mod dds_bc;
pub mod dicom;
pub mod fits;
pub mod iff;
pub mod pcx;
pub mod pnm;
pub mod xpm;

/// What went wrong. `format` is the format's short name (`"DDS"`, `"FITS"`, ...).
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("{format}: {reason}")]
    Unsupported { format: &'static str, reason: String },
    #[error("{format}: malformed data: {message}")]
    Malformed { format: &'static str, message: String },
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
    #[error("{format}: encode failed: {message}")]
    Encode { format: &'static str, message: String },
    #[error("invalid image: {0}")]
    InvalidImage(String),
}

impl CodecError {
    pub(crate) fn malformed(format: &'static str, e: impl std::fmt::Display) -> Self {
        CodecError::Malformed { format, message: e.to_string() }
    }
    pub(crate) fn encode(format: &'static str, e: impl std::fmt::Display) -> Self {
        CodecError::Encode { format, message: e.to_string() }
    }
    pub(crate) fn unsupported(format: &'static str, reason: impl Into<String>) -> Self {
        CodecError::Unsupported { format, reason: reason.into() }
    }
}

/// Channel layout of an [`Image`]. Alpha, when present, is the last channel and is straight
/// (not premultiplied).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelLayout {
    Gray,
    GrayA,
    Rgb,
    Rgba,
    /// Ink amounts, `0 = no ink`, `max = full ink`.
    Cmyk,
    CmykA,
}

impl ChannelLayout {
    /// Total number of channels including alpha.
    pub const fn channels(self) -> usize {
        match self {
            ChannelLayout::Gray => 1,
            ChannelLayout::GrayA => 2,
            ChannelLayout::Rgb => 3,
            ChannelLayout::Rgba | ChannelLayout::Cmyk => 4,
            ChannelLayout::CmykA => 5,
        }
    }
}

/// Storage type of one sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleType {
    U8,
    U16,
    F32,
}

impl SampleType {
    pub const fn bytes(self) -> usize {
        match self {
            SampleType::U8 => 1,
            SampleType::U16 => 2,
            SampleType::F32 => 4,
        }
    }
}

/// What a decoder noticed about a file it nevertheless decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeWarning {
    /// The file holds more images (planes, pages) than the one returned; `total` when known.
    MorePages { total: Option<u32> },
}

/// A flat raster image: samples interleaved (`RGBARGBA...`), row-major, top to bottom, in
/// native endianness. Integer samples are unsigned and span `0..=max`; float samples are
/// nominally `0..=1` and may exceed it.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    width: u32,
    height: u32,
    layout: ChannelLayout,
    sample: SampleType,
    data: Vec<u8>,
    /// Resolution in pixels per inch (x, y), when the file states one.
    pub dpi: Option<(f32, f32)>,
    /// Key/value text the file carries (FITS header cards).
    pub text: Vec<(String, String)>,
    /// Empty for a complete, single image and for images built in memory. Encoders ignore it.
    pub warnings: Vec<DecodeWarning>,
}

impl Image {
    /// Wraps raw interleaved, native-endian bytes; the length must match the size exactly.
    pub fn from_raw(width: u32, height: u32, layout: ChannelLayout, sample: SampleType, data: Vec<u8>) -> Result<Self, CodecError> {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(layout.channels()))
            .and_then(|n| n.checked_mul(sample.bytes()))
            .ok_or_else(|| CodecError::InvalidImage("image size overflows usize".into()))?;
        if data.len() != expected {
            return Err(CodecError::InvalidImage(format!(
                "buffer has {} bytes, expected {expected} for {width}x{height} {layout:?} {sample:?}",
                data.len()
            )));
        }
        Ok(Image { width, height, layout, sample, data, dpi: None, text: Vec::new(), warnings: Vec::new() })
    }

    pub fn from_u8(width: u32, height: u32, layout: ChannelLayout, data: Vec<u8>) -> Result<Self, CodecError> {
        Self::from_raw(width, height, layout, SampleType::U8, data)
    }

    pub fn from_u16(width: u32, height: u32, layout: ChannelLayout, data: &[u16]) -> Result<Self, CodecError> {
        Self::from_raw(width, height, layout, SampleType::U16, data.iter().flat_map(|v| v.to_ne_bytes()).collect())
    }

    pub fn from_f32(width: u32, height: u32, layout: ChannelLayout, data: &[f32]) -> Result<Self, CodecError> {
        Self::from_raw(width, height, layout, SampleType::F32, data.iter().flat_map(|v| v.to_ne_bytes()).collect())
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn layout(&self) -> ChannelLayout {
        self.layout
    }
    pub fn sample_type(&self) -> SampleType {
        self.sample
    }
    /// Raw interleaved native-endian bytes.
    pub fn data(&self) -> &[u8] {
        &self.data
    }
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// The samples as `u16` (only when the sample type is `U16`).
    pub fn to_u16_samples(&self) -> Option<Vec<u16>> {
        (self.sample == SampleType::U16).then(|| self.data.as_chunks::<2>().0.iter().map(|c| u16::from_ne_bytes(*c)).collect())
    }

    /// The samples as `f32` (only when the sample type is `F32`).
    pub fn to_f32_samples(&self) -> Option<Vec<f32>> {
        (self.sample == SampleType::F32).then(|| self.data.as_chunks::<4>().0.iter().map(|c| f32::from_ne_bytes(*c)).collect())
    }
}

/// Bounds on what a decoder will produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_width: u32,
    pub max_height: u32,
    /// Maximum `width * height`.
    pub max_pixels: u64,
    /// Maximum bytes for the decoded pixel buffer. Not a cap on total memory.
    pub max_alloc: u64,
}

impl Default for Limits {
    /// 2^18 pixels a side, 2^30 pixels, and 8 GiB of pixels on 64-bit targets (1 GiB on 32-bit).
    fn default() -> Self {
        let max_alloc = if cfg!(target_pointer_width = "64") { 8 << 30 } else { 1 << 30 };
        Limits { max_width: 1 << 18, max_height: 1 << 18, max_pixels: 1 << 30, max_alloc }
    }
}

impl Limits {
    /// Effectively unlimited (still bounded by `usize` overflow checks).
    pub fn none() -> Self {
        Limits { max_width: u32::MAX, max_height: u32::MAX, max_pixels: u64::MAX, max_alloc: u64::MAX }
    }

    /// Validates declared dimensions for an output buffer of the given layout and sample type.
    pub fn check(&self, width: u32, height: u32, layout: ChannelLayout, sample: SampleType) -> Result<(), CodecError> {
        if width == 0 || height == 0 {
            return Err(CodecError::InvalidImage(format!("zero-sized image {width}x{height}")));
        }
        if width > self.max_width || height > self.max_height {
            return Err(CodecError::LimitExceeded(format!("dimensions {width}x{height} exceed max {}x{}", self.max_width, self.max_height)));
        }
        let pixels = width as u64 * height as u64;
        if pixels > self.max_pixels {
            return Err(CodecError::LimitExceeded(format!("{pixels} pixels exceed max {}", self.max_pixels)));
        }
        let bytes = pixels.saturating_mul((layout.channels() * sample.bytes()) as u64);
        if bytes > self.max_alloc {
            return Err(CodecError::LimitExceeded(format!("{bytes} bytes exceed max_alloc {}", self.max_alloc)));
        }
        if usize::try_from(bytes).is_err() {
            return Err(CodecError::LimitExceeded(format!("{bytes} bytes do not fit in memory")));
        }
        Ok(())
    }

    /// Decompression-bomb guard for formats that compress: refuses a file too small to hold
    /// `declared_bytes` of stored image data when the compression never beats `max_ratio`:1.
    /// Called before the output buffer is allocated. 1 MiB of slack leaves tiny images alone.
    pub(crate) fn check_plausible(declared_bytes: u64, file_len: usize, max_ratio: u64) -> Result<(), CodecError> {
        let capacity = (file_len as u64).saturating_mul(max_ratio).saturating_add(1 << 20);
        if declared_bytes > capacity {
            return Err(CodecError::LimitExceeded(format!("a {file_len} byte file cannot hold {declared_bytes} bytes of image data")));
        }
        Ok(())
    }
}

/// A zero-filled buffer of `len` elements, allocated fallibly: an allocation failure is
/// [`CodecError::LimitExceeded`] instead of an abort.
pub(crate) fn alloc_zeroed<T: Copy + Default>(len: usize) -> Result<Vec<T>, CodecError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| CodecError::LimitExceeded(format!("cannot allocate {len} elements for the image")))?;
    v.resize(len, T::default());
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_refuse_each_bound() {
        let l = Limits { max_width: 10, max_height: 10, max_pixels: 50, max_alloc: 100 };
        let check = |w, h| l.check(w, h, ChannelLayout::Rgb, SampleType::U8);
        assert!(check(5, 5).is_ok());
        assert!(matches!(check(0, 5), Err(CodecError::InvalidImage(_))));
        assert!(matches!(check(11, 1), Err(CodecError::LimitExceeded(_))), "width");
        assert!(matches!(check(1, 11), Err(CodecError::LimitExceeded(_))), "height");
        assert!(matches!(check(10, 6), Err(CodecError::LimitExceeded(_))), "pixels");
        assert!(matches!(check(7, 5), Err(CodecError::LimitExceeded(_))), "bytes");
        assert!(matches!(Limits::check_plausible((1 << 20) + 129, 1, 128), Err(CodecError::LimitExceeded(_))));
        assert!(Limits::check_plausible((1 << 20) + 128, 1, 128).is_ok());
    }

    #[test]
    fn image_buffer_must_match_its_size() {
        assert!(Image::from_u8(2, 2, ChannelLayout::Rgb, vec![0; 12]).is_ok());
        assert!(matches!(Image::from_u8(2, 2, ChannelLayout::Rgb, vec![0; 11]), Err(CodecError::InvalidImage(_))));
        assert_eq!(Image::from_u16(1, 1, ChannelLayout::Gray, &[513]).ok().and_then(|i| i.to_u16_samples()), Some(vec![513]));
    }
}
