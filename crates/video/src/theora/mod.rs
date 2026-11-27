//! Clean-room pure-Rust Theora video codec (Xiph.Org specification).
//!
//! Provides a conformant Theora video decoder (`Decoder`) and keyframe/interframe
//! encoder (`Encoder`) operating over byte slices, planar buffers and Ogg streams.

#![allow(clippy::needless_range_loop, clippy::type_complexity, clippy::too_many_arguments, clippy::cognitive_complexity)]

pub mod decoder;
pub mod encoder;
pub mod header;
pub mod huffman;
pub mod idct;
pub mod loopfilter;
pub mod quant;
pub mod reconstruct;
pub mod tables;

pub use decoder::Decoder;
pub use encoder::{Encoder, EncoderConfig};
pub use header::{CommentHeader, IdentificationHeader, SetupHeader};

/// Theora pixel format (subsampling).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PixelFormat {
    /// 4:2:0 subsampling (default and VP3 format).
    #[default]
    Yuv420 = 0,
    /// 4:2:2 subsampling.
    Yuv422 = 2,
    /// 4:4:4 subsampling (no chroma subsampling).
    Yuv444 = 3,
}

impl PixelFormat {
    /// Number of blocks per macroblock for this pixel format (Table 6.6).
    pub fn blocks_per_macroblock(self) -> usize {
        match self {
            PixelFormat::Yuv420 => 6,
            PixelFormat::Yuv422 => 8,
            PixelFormat::Yuv444 => 12,
        }
    }
}

/// Theora color space specification (Table 6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColorSpace {
    /// Unspecified color space.
    #[default]
    Undefined = 0,
    /// Rec. 470M (NTSC).
    Rec470M = 1,
    /// Rec. 470BG (PAL).
    Rec470Bg = 2,
}

/// Frame coding type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameType {
    /// Intra keyframe (independent, updates golden and previous reference).
    Intra = 0,
    /// Inter frame (predicted from previous/golden reference).
    Inter = 1,
}

/// Theora stream information parsed from headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// Major bitstream version (must be 3).
    pub version_major: u8,
    /// Minor bitstream version (must be 2).
    pub version_minor: u8,
    /// Revision version.
    pub version_revision: u8,
    /// Coded frame width in pixels (multiple of 16).
    pub frame_width: u32,
    /// Coded frame height in pixels (multiple of 16).
    pub frame_height: u32,
    /// Displayable picture width in pixels.
    pub pic_width: u32,
    /// Displayable picture height in pixels.
    pub pic_height: u32,
    /// Picture region X offset from frame lower-left corner.
    pub pic_x: u32,
    /// Picture region Y offset from frame lower-left corner.
    pub pic_y: u32,
    /// Frame rate numerator.
    pub fps_num: u32,
    /// Frame rate denominator.
    pub fps_den: u32,
    /// Pixel aspect ratio numerator.
    pub par_num: u32,
    /// Pixel aspect ratio denominator.
    pub par_den: u32,
    /// Color space.
    pub color_space: ColorSpace,
    /// Chroma subsampling pixel format.
    pub pixel_format: PixelFormat,
    /// Nominal bitrate in bits per second.
    pub nominal_bitrate: u32,
    /// Quality hint (0..=63).
    pub quality: u8,
    /// Keyframe shift in granule position.
    pub keyframe_granule_shift: u8,
}

/// A decoded Theora video picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    /// Coded frame width.
    pub width: u32,
    /// Coded frame height.
    pub height: u32,
    /// Display picture width.
    pub pic_width: u32,
    /// Display picture height.
    pub pic_height: u32,
    /// Display picture horizontal offset.
    pub pic_x: u32,
    /// Display picture vertical offset.
    pub pic_y: u32,
    /// Pixel format.
    pub pixel_format: PixelFormat,
    /// Color space.
    pub color_space: ColorSpace,
    /// Whether this was an intra keyframe.
    pub keyframe: bool,
    /// Presentation timestamp (packet pts if passed).
    pub pts: Option<i64>,
    /// Planar Y (luma) samples.
    pub y: Vec<u8>,
    /// Planar Cb (U chroma) samples.
    pub u: Vec<u8>,
    /// Planar Cr (V chroma) samples.
    pub v: Vec<u8>,
    /// Row stride in samples for luma plane.
    pub y_stride: usize,
    /// Row stride in samples for chroma planes.
    pub uv_stride: usize,
}

/// Errors returned by Theora operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Bitstream or header was truncated before reading required fields.
    #[error("unexpected end of bitstream: {0}")]
    Truncated(&'static str),

    /// A header packet was malformed or failed validation.
    #[error("invalid Theora header: {0}")]
    InvalidHeader(&'static str),

    /// Bitstream version is not supported (only 3.2.x).
    #[error("unsupported Theora version {0}.{1}.{2}")]
    UnsupportedVersion(u8, u8, u8),

    /// Pixel format is invalid or unsupported.
    #[error("unsupported pixel format: {0}")]
    UnsupportedPixelFormat(u8),

    /// Packet data was corrupt or invalid according to the specification.
    #[error("corrupt bitstream: {0}")]
    Corrupt(&'static str),

    /// Operation requires an intra keyframe (e.g. first frame or missing reference).
    #[error("missing reference frame (expected keyframe)")]
    MissingReference,

    /// Dimensions did not match expectation.
    #[error("frame dimension mismatch: {0}")]
    DimensionMismatch(&'static str),

    /// Out of memory or allocation failed.
    #[error("allocation failed")]
    OutOfMemory,
}

/// Result type for Theora operations.
pub type Result<T> = std::result::Result<T, Error>;
