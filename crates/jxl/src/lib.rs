#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! JPEG XL image decoding and encoding.
//!
//! Decoding incrementally feeds the input, checks image dimensions before loading remaining
//! frames, and applies a tracked-allocation budget. The allocation tracker covers allocations
//! made through `jxl-oxide`; it cannot account for every allocation inside a decoder dependency.
//! The first image frame is returned, with orientation already applied to its pixels.

#[cfg(feature = "decode")]
mod decode;
#[cfg(feature = "encode")]
mod encode;

/// Pixel channel layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// One grayscale channel.
    Gray,
    /// Grayscale and straight alpha.
    GrayAlpha,
    /// Red, green, blue.
    Rgb,
    /// Red, green, blue and straight alpha.
    Rgba,
}

/// Color encoding requested for rendered pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputColor {
    /// Use `jxl-oxide`'s default rendering target and report its rendered profile.
    #[default]
    DecoderDefault,
    /// For files with enum color encoding, render linear Rec. 2020. XYB files render to linear
    /// sRGB first and are converted to Rec. 2020 to avoid a `jxl-oxide` wide-gamut XYB issue.
    /// Files with an embedded ICC profile remain rendered in that profile's space; the source
    /// ICC is returned in [`Metadata::icc`] because this crate does not bundle an ICC CMS.
    LinearRec2020,
}

impl Layout {
    /// Number of interleaved channels.
    pub const fn channels(self) -> usize {
        match self {
            Layout::Gray => 1,
            Layout::GrayAlpha => 2,
            Layout::Rgb => 3,
            Layout::Rgba => 4,
        }
    }

    /// Whether this layout includes alpha.
    pub const fn has_alpha(self) -> bool {
        matches!(self, Layout::GrayAlpha | Layout::Rgba)
    }
}

/// Sample data accepted by the encoder.
#[derive(Clone, Copy, Debug)]
pub enum Samples<'a> {
    /// Unsigned 8-bit samples, already encoded using the selected color transfer curve.
    U8(&'a [u8]),
    /// Unsigned 16-bit samples, already encoded using the selected color transfer curve.
    U16(&'a [u16]),
    /// Floating point samples. These are accepted only for Rec. 2020 PQ encoding.
    F32(&'a [f32]),
}

/// A borrowed image to encode. Samples are interleaved and alpha is straight.
#[derive(Clone, Copy, Debug)]
pub struct ImageRef<'a> {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Channel layout.
    pub layout: Layout,
    /// Interleaved sample data.
    pub samples: Samples<'a>,
}

/// Metadata embedded in the JPEG XL container.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    /// ICC profile bytes, when present.
    pub icc: Option<Vec<u8>>,
    /// TIFF-structured Exif payload, without an `Exif\0\0` prefix.
    pub exif: Option<Vec<u8>>,
    /// XMP packet bytes.
    pub xmp: Option<Vec<u8>>,
}

/// JPEG XL operation error.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The image is malformed or cannot be decoded.
    #[error("JPEG XL: malformed image: {0}")]
    Malformed(String),
    /// The image exceeds a caller supplied size or allocation limit.
    #[error("JPEG XL: limit exceeded: {0}")]
    LimitExceeded(String),
    /// The encoded dimensions exceed the caller's pixel-count limit.
    #[error("JPEG XL: image dimensions {width}×{height} exceed the {max_pixels} pixel limit")]
    PixelLimitExceeded {
        /// Encoded image width.
        width: u32,
        /// Encoded image height.
        height: u32,
        /// Maximum accepted pixel count.
        max_pixels: u64,
    },
    /// The image layout or samples are unsupported.
    #[error("JPEG XL: invalid image: {0}")]
    InvalidImage(String),
    /// Encoding failed.
    #[error("JPEG XL: encode failed: {0}")]
    Encode(String),
    /// The requested operation is not available in this build.
    #[error("JPEG XL: unsupported: {0}")]
    Unsupported(String),
}

/// Result returned by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Decoded first frame and source metadata.
#[cfg(feature = "decode")]
#[derive(Clone, Debug)]
pub struct Decoded {
    /// Width after applying the codestream orientation.
    pub width: u32,
    /// Height after applying the codestream orientation.
    pub height: u32,
    /// Output channel layout.
    pub layout: Layout,
    /// Bits per sample signalled in the file.
    pub bit_depth: u8,
    /// Whether the source samples use a floating-point representation.
    pub float_samples: bool,
    /// Interleaved floating point samples in the decoder's rendered color encoding.
    pub pixels: Vec<f32>,
    /// Metadata from the source image.
    pub metadata: Metadata,
    /// Original orientation value. Pixels in `pixels` already have this orientation applied.
    pub source_orientation: u32,
    /// CICP value describing the rendered pixel encoding, when the decoder supplies one.
    pub rendered_cicp: Option<[u8; 4]>,
    /// ICC profile describing the rendered pixel encoding.
    pub rendered_icc: Vec<u8>,
    /// Whether the codestream uses XYB encoding.
    pub xyb_encoded: bool,
    /// Whether `pixels` are linear Rec. 2020. This is false when associated alpha requires keeping
    /// a modeled nonlinear source encoding so the caller can unpremultiply before its color transform.
    pub pixels_linear_rec2020: bool,
    /// Whether color samples in `pixels` remain associated with alpha. When true, divide them by
    /// alpha after any necessary transfer conversion; at zero alpha the color values are undefined.
    pub pixels_premultiplied: bool,
}

/// Options for bounded decoding.
#[cfg(feature = "decode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeLimits {
    /// Largest accepted pixel count. Defaults to 1 gigapixel.
    pub max_pixels: u64,
    /// Maximum allocation budget passed to `jxl-oxide`'s tracked allocator. `None` scales the
    /// budget from `max_pixels` and preserves fixed decoder overhead for small images.
    pub max_tracked_bytes: Option<u64>,
    /// Largest compressed input size. Defaults to `usize::MAX`.
    pub max_input_bytes: usize,
}

#[cfg(feature = "decode")]
impl Default for DecodeLimits {
    fn default() -> Self {
        Self { max_pixels: 1 << 30, max_tracked_bytes: None, max_input_bytes: usize::MAX }
    }
}

/// Decode the first image frame.
#[cfg(feature = "decode")]
pub use decode::{decode, decode_with_color};
/// Encode a JPEG XL image.
#[cfg(feature = "encode")]
pub use encode::{Effort, EncodeOptions, HDR_REFERENCE_WHITE_NITS, JxlColour, encode};
