//! Decode limits and encode options.

use crate::error::CodecError;
use crate::image::{ChannelLayout, SampleType};

/// Decompression-bomb guards. Dimension checks precede pixel allocation;
/// metadata decoding checks its budget before growing decoded output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_width: u32,
    pub max_height: u32,
    /// Maximum `width * height`.
    pub max_pixels: u64,
    /// Maximum bytes for the decoded pixel buffer (and passed on as the
    /// allocation budget to underlying decoders).
    /// PNG also applies this to the aggregate decoded text/XMP UTF-8 bytes,
    /// including keywords, separately from pixels. Animated WebP charges
    /// every retained frame's full pixel copy plus its own copy of the
    /// file's ICC profile against this budget. This is not a total RSS cap.
    pub max_alloc: u64,
}

impl Default for Limits {
    fn default() -> Self {
        // 2^30 pixels (e.g. 32768²) and, on 64-bit targets, 8 GiB: enough for the large
        // documents Photoshop users open (a 20000² RGBA 16-bit image is 3.2 GB). On 32-bit
        // targets (wasm32 has a 4 GiB address space shared with the document, history and
        // the decoder's own buffers) 1 GiB is the most one buffer can safely ask for.
        let max_alloc = if cfg!(target_pointer_width = "64") { 8 << 30 } else { 1 << 30 };
        Limits { max_width: 1 << 18, max_height: 1 << 18, max_pixels: 1 << 30, max_alloc }
    }
}

impl Limits {
    /// For a decoder that writes a band of rows at a time into the document's tiles instead of
    /// one buffer (TIFF, #8169): the largest document size, 300,000 px a side, on 64-bit
    /// targets, with `max_alloc` bounding each group of bands. 32-bit targets keep the
    /// [`Default`] limits: wasm32's 4 GiB address space can't hold a larger document anyway.
    pub fn banded() -> Self {
        let l = Limits::default();
        if cfg!(target_pointer_width = "64") { Limits { max_width: 300_000, max_height: 300_000, max_pixels: 300_000 * 300_000, ..l } } else { l }
    }

    /// Effectively unlimited (still bounded by `usize` overflow checks).
    pub fn none() -> Self {
        Limits { max_width: u32::MAX, max_height: u32::MAX, max_pixels: u64::MAX, max_alloc: u64::MAX }
    }

    /// Validate declared dimensions for an output buffer of the given layout
    /// and sample type.
    pub fn check(&self, width: u32, height: u32, layout: ChannelLayout, sample: SampleType) -> Result<(), CodecError> {
        self.check_bytes(width, height, (layout.channels() * sample.bytes()) as u64)
    }

    pub(crate) fn check_bytes(&self, width: u32, height: u32, bytes_per_pixel: u64) -> Result<(), CodecError> {
        self.check_size(width, height)?;
        let bytes = (width as u64 * height as u64).saturating_mul(bytes_per_pixel);
        if bytes > self.max_alloc {
            return Err(CodecError::LimitExceeded(format!("{bytes} bytes exceed max_alloc {}", self.max_alloc)));
        }
        if usize::try_from(bytes).is_err() {
            return Err(CodecError::LimitExceeded(format!("{bytes} bytes do not fit in memory")));
        }
        Ok(())
    }

    /// The dimension and pixel-count checks of [`Limits::check`], for a decoder that never holds
    /// the whole image in one buffer (`max_alloc` then bounds each piece it decodes).
    pub(crate) fn check_size(&self, width: u32, height: u32) -> Result<(), CodecError> {
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
        Ok(())
    }

    /// Decompression-bomb guard for formats that compress: refuse a file too small to hold
    /// `declared_bytes` of (stored, pre-expansion) image data, given that the compression
    /// never beats `max_ratio`:1. Called before the output buffer is allocated, so a
    /// header-only hostile file cannot make the decoder reserve gigabytes. A fixed 1 MiB of
    /// slack keeps tiny images and headers with large metadata unaffected.
    pub(crate) fn check_plausible(declared_bytes: u64, file_len: usize, max_ratio: u64) -> Result<(), CodecError> {
        let capacity = (file_len as u64).saturating_mul(max_ratio).saturating_add(1 << 20);
        if declared_bytes > capacity {
            return Err(CodecError::LimitExceeded(format!("a {file_len} byte file cannot hold {declared_bytes} bytes of image data")));
        }
        Ok(())
    }

    pub(crate) fn alloc_usize(&self) -> usize {
        usize::try_from(self.max_alloc).unwrap_or(usize::MAX)
    }
}

/// Options for [`crate::decode_with`].
#[derive(Debug, Clone, Default)]
pub struct DecodeOptions {
    pub limits: Limits,
    /// Keep the pixels as stored instead of applying the EXIF / TIFF
    /// orientation (by default they are turned upright and the metadata's
    /// orientation is rewritten to 1).
    pub keep_orientation: bool,
}

/// PNG zlib effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PngCompression {
    None,
    Fast,
    #[default]
    Default,
    Best,
}

/// How a PNG's `sTER` chunk pairs its two views (PNG Extensions 1.4.0, §3.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngStereo {
    /// Side by side with the right-eye view first (cross-eyed viewing, mode 0).
    CrossFuse,
    /// Side by side with the left-eye view first (parallel viewing, mode 1).
    DivergingFuse,
}

impl PngStereo {
    /// The view for an `sTER` mode byte; reserved and unknown values are `None`.
    pub fn from_mode(mode: u8) -> Option<Self> {
        match mode {
            0 => Some(Self::CrossFuse),
            1 => Some(Self::DivergingFuse),
            _ => None,
        }
    }

    /// The `sTER` mode byte.
    pub fn mode(self) -> u8 {
        match self {
            Self::CrossFuse => 0,
            Self::DivergingFuse => 1,
        }
    }
}

/// TIFF strip compression. The default is none, as in Photoshop: uncompressed strips are the
/// only kind every TIFF reader opens (Deflate and the predictor are not baseline TIFF, #4484).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TiffCompression {
    #[default]
    None,
    Lzw,
    Deflate,
    PackBits,
}

/// JPEG scan layout: Photoshop's JPEG Options › Format Options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JpegFormat {
    /// Baseline ("Standard"): one sequential scan with the standard Huffman tables.
    #[default]
    Baseline,
    /// Baseline Optimized: one sequential scan with Huffman tables fitted to the image. Only at
    /// 4:4:4 for now: a 4:2:0 (subsampled RGB) baseline scan keeps the standard tables, because
    /// fitted tables make jpeg-encoder write one scan per component, which zune-jpeg 0.5.15 (our
    /// decoder) reads back wrongly when chroma is subsampled (see `web::encode_jpeg_rgb8`).
    Optimized,
    /// Progressive: the image refines over several scans as it loads.
    Progressive,
}

impl JpegFormat {
    /// `"baseline"`, `"optimized"` or `"progressive"`, as parameters and scripts spell it.
    pub fn name(self) -> &'static str {
        match self {
            JpegFormat::Baseline => "baseline",
            JpegFormat::Optimized => "optimized",
            JpegFormat::Progressive => "progressive",
        }
    }

    /// The inverse of [`Self::name`].
    pub fn parse(s: &str) -> Option<Self> {
        [JpegFormat::Baseline, JpegFormat::Optimized, JpegFormat::Progressive].into_iter().find(|f| f.name() == s)
    }
}

/// OpenEXR compression (lossless methods only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExrCompression {
    None,
    Rle,
    Zip1,
    #[default]
    Zip16,
    Piz,
}

/// Bits per sample of a DPX file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpxBits {
    Bits8,
    Bits10,
    Bits12,
    Bits16,
}

impl DpxBits {
    /// `8`, `10`, `12` or `16` as [`DpxBits`]; any other number is `None`.
    pub fn from_bits(bits: u64) -> Option<Self> {
        match bits {
            8 => Some(DpxBits::Bits8),
            10 => Some(DpxBits::Bits10),
            12 => Some(DpxBits::Bits12),
            16 => Some(DpxBits::Bits16),
            _ => None,
        }
    }

    /// The bits per sample.
    pub fn bits(self) -> u8 {
        match self {
            DpxBits::Bits8 => 8,
            DpxBits::Bits10 => 10,
            DpxBits::Bits12 => 12,
            DpxBits::Bits16 => 16,
        }
    }
}

/// How a DDS texture stores its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DdsCompression {
    /// Uncompressed, 8 bits per channel: BGRA (BGRX without alpha), or luminance with or
    /// without alpha for grayscale. Lossless.
    #[default]
    None,
    /// BC1 (DXT1): 4 bits per pixel. Alpha is on or off: texels under half opacity become
    /// transparent black.
    Bc1,
    /// BC2 (DXT3): 8 bits per pixel, with 4 bits of explicit alpha per texel.
    Bc2,
    /// BC3 (DXT5): 8 bits per pixel, with smooth interpolated alpha.
    Bc3,
}

impl DdsCompression {
    pub const ALL: [DdsCompression; 4] = [DdsCompression::None, DdsCompression::Bc1, DdsCompression::Bc2, DdsCompression::Bc3];

    /// The name scripts and the CLI use: `none`, `bc1`, `bc2` or `bc3`.
    pub fn name(self) -> &'static str {
        match self {
            DdsCompression::None => "none",
            DdsCompression::Bc1 => "bc1",
            DdsCompression::Bc2 => "bc2",
            DdsCompression::Bc3 => "bc3",
        }
    }

    /// [`Self::name`] back to a compression, also taking the DXT names (`dxt1`, `dxt3`,
    /// `dxt5`), in any case. Anything else is `None`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "none" => Some(DdsCompression::None),
            "bc1" | "dxt1" => Some(DdsCompression::Bc1),
            "bc2" | "dxt3" => Some(DdsCompression::Bc2),
            "bc3" | "dxt5" => Some(DdsCompression::Bc3),
            _ => None,
        }
    }

    /// A block compression (lossy), not uncompressed pixels.
    pub fn is_block(self) -> bool {
        self != DdsCompression::None
    }
}

/// What a DDS file is written with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DdsOptions {
    pub compression: DdsCompression,
    /// Write the full mip chain down to 1×1, each level a box filter of the one above.
    pub mipmaps: bool,
}

/// The longer side, in pixels, at which a TIFF image pyramid stops: the last level written is
/// the first one no longer than this ([`EncodeOptions::tiff_pyramid`]).
pub const TIFF_PYRAMID_MIN: u32 = 64;

/// Options for [`crate::encode`].
#[derive(Debug, Clone)]
pub struct EncodeOptions {
    /// JPEG quality 1..=100 (also AVIF quality when enabled).
    pub jpeg_quality: u8,
    /// JPEG 4:2:0 chroma subsampling (`false` = 4:4:4).
    pub jpeg_chroma_subsampling: bool,
    /// JPEG scan layout (baseline, baseline optimized or progressive).
    pub jpeg_format: JpegFormat,
    pub png_compression: PngCompression,
    /// Write Adam7-interlaced PNG.
    pub png_interlaced: bool,
    /// Write an `sTER` chunk marking the image as a stereo pair (PNG Extensions 1.4.0, §3.6),
    /// so a stereo PNG keeps its mark when saved again (#6063).
    pub png_stereo: Option<PngStereo>,
    /// WebP: lossless (VP8L, via `image-webp`) or lossy (our VP8 encoder, see
    /// `codecs::vp8`) with [`Self::webp_quality`].
    pub webp_lossless: bool,
    /// Lossy WebP quality 0..=100 (the source application's scale; 75–85 is typical for photos).
    pub webp_quality: u8,
    pub tiff_compression: TiffCompression,
    /// Always write TIFF as BigTIFF (8-byte offsets). Without it a TIFF is written as BigTIFF
    /// only when it could pass the 4 GiB a classic TIFF can address.
    pub tiff_bigtiff: bool,
    /// TIFF: after the full image, write Photoshop's "Save Image Pyramid" levels: copies at half,
    /// a quarter, … of the size, each marked reduced-resolution (`NewSubfileType` 1), down to
    /// [`crate::TIFF_PYRAMID_MIN`] px on the longer side. Readers that don't know pyramids open
    /// the full image, which comes first.
    pub tiff_pyramid: bool,
    pub exr_compression: ExrCompression,
    /// DPX bits per sample. `None` follows the image: 8 for 8-bit, 16 otherwise.
    pub dpx_bits: Option<DpxBits>,
    /// DDS compression and mip chain.
    pub dds: DdsOptions,
    /// Embed the ICC profile when the format supports it.
    pub embed_icc: bool,
    /// Embed EXIF/XMP/DPI/text when the format supports it.
    pub embed_metadata: bool,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        EncodeOptions {
            jpeg_quality: 90,
            jpeg_chroma_subsampling: true,
            jpeg_format: JpegFormat::Baseline,
            png_compression: PngCompression::Default,
            png_interlaced: false,
            png_stereo: None,
            webp_lossless: true,
            webp_quality: 80,
            tiff_compression: TiffCompression::None,
            tiff_bigtiff: false,
            tiff_pyramid: false,
            exr_compression: ExrCompression::Zip16,
            dpx_bits: None,
            dds: DdsOptions::default(),
            embed_icc: true,
            embed_metadata: true,
        }
    }
}

/// A zero-filled buffer of `len` elements, allocated fallibly: an allocation failure becomes
/// [`CodecError::LimitExceeded`] instead of aborting the process (which `catch_unwind` cannot
/// stop).
pub(crate) fn alloc_zeroed<T: Copy + Default>(len: usize) -> Result<Vec<T>, CodecError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| CodecError::LimitExceeded(format!("cannot allocate {len} elements for the image")))?;
    v.resize(len, T::default());
    Ok(v)
}
