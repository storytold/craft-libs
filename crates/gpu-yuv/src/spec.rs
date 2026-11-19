//! What a conversion produces: the plane format, the matrix and the range, and the code values
//! they give.

use std::fmt;

/// The 4:2:0 layout a conversion writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// 8 bits: the luma plane, then one plane of interleaved Cb and Cr samples.
    Nv12,
    /// 10 bits in the high bits of little-endian 16-bit words: the luma plane, then one plane of
    /// interleaved Cb and Cr samples.
    P010,
    /// 8 bits: the luma plane, then the Cb plane, then the Cr plane.
    I420,
}

impl Format {
    /// Bits per sample value.
    pub fn bits(self) -> u32 {
        match self {
            Format::P010 => 10,
            Format::Nv12 | Format::I420 => 8,
        }
    }

    /// Bytes each sample takes in memory.
    pub fn sample_bytes(self) -> u32 {
        match self {
            Format::P010 => 2,
            Format::Nv12 | Format::I420 => 1,
        }
    }

    /// The left shift from a sample value to the stored word: 6 for P010, else 0.
    pub fn shift(self) -> u32 {
        match self {
            Format::P010 => 6,
            Format::Nv12 | Format::I420 => 0,
        }
    }

    /// The format's usual name: `NV12`, `P010` or `I420`.
    pub fn name(self) -> &'static str {
        match self {
            Format::Nv12 => "NV12",
            Format::P010 => "P010",
            Format::I420 => "I420",
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The RGB to Y'CbCr matrix (ITU-R BT.601, BT.709 and BT.2020 non-constant luminance).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Matrix {
    Bt601,
    Bt709,
    Bt2020,
}

impl Matrix {
    /// The red and blue luma weights, Kr and Kb.
    pub fn kr_kb(self) -> (f32, f32) {
        match self {
            Matrix::Bt601 => (0.299, 0.114),
            Matrix::Bt709 => (0.2126, 0.0722),
            Matrix::Bt2020 => (0.2627, 0.0593),
        }
    }
}

/// The code values the signal spans.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Range {
    /// Luma from 16 to 235 and chroma from 16 to 240, scaled by 4 at 10 bits.
    Limited,
    /// Every code value, from 0 to 255 (1023 at 10 bits); chroma is centered on 128 (512).
    Full,
}

/// One conversion: the plane format, the matrix and the range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Conversion {
    pub format: Format,
    pub matrix: Matrix,
    pub range: Range,
}

impl Conversion {
    pub fn new(format: Format, matrix: Matrix, range: Range) -> Conversion {
        Conversion { format, matrix, range }
    }
}

/// The weights and the scale of one conversion. The code value of luma is
/// `dot(y, rgb) * y_scale + y_offset`, of Cb `dot(cb, rgb) * c_scale + c_offset`, and of Cr
/// `dot(cr, rgb) * c_scale + c_offset`, rounded with halves away from zero and clamped to
/// `0..=max`, for `rgb` from 0 to 1. That rounding is `INT` in BT.709 and BT.2020 and `Round` in
/// BT.2100.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Coefficients {
    pub y: [f32; 3],
    pub cb: [f32; 3],
    pub cr: [f32; 3],
    pub y_scale: f32,
    pub y_offset: f32,
    pub c_scale: f32,
    pub c_offset: f32,
    pub max: f32,
}

impl Coefficients {
    pub fn of(c: Conversion) -> Coefficients {
        let (kr, kb) = c.matrix.kr_kb();
        let kg = 1.0 - kr - kb;
        let (db, dr) = (2.0 * (1.0 - kb), 2.0 * (1.0 - kr));
        let bits = c.format.bits();
        let max = ((1u32 << bits) - 1) as f32;
        let step = (1u32 << (bits - 8)) as f32;
        let (y_scale, y_offset, c_scale, c_offset) = match c.range {
            Range::Limited => (219.0 * step, 16.0 * step, 224.0 * step, 128.0 * step),
            Range::Full => (max, 0.0, max, (1u32 << (bits - 1)) as f32),
        };
        Coefficients { y: [kr, kg, kb], cb: [-kr / db, -kg / db, 0.5], cr: [0.5, -kg / dr, -kb / dr], y_scale, y_offset, c_scale, c_offset, max }
    }

    /// The code values of luma, Cb and Cr for `rgb`.
    pub fn codes(&self, rgb: [f32; 3]) -> [u16; 3] {
        let dot = |w: [f32; 3]| w[0] * rgb[0] + w[1] * rgb[1] + w[2] * rgb[2];
        let code = |v: f32| v.round().clamp(0.0, self.max) as u16;
        [
            code(dot(self.y) * self.y_scale + self.y_offset),
            code(dot(self.cb) * self.c_scale + self.c_offset),
            code(dot(self.cr) * self.c_scale + self.c_offset),
        ]
    }
}

/// What a conversion cannot do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// An output side is zero or odd; 4:2:0 needs both sides even.
    OutputSize { width: u32, height: u32 },
    /// A side is larger than the device's largest 2D texture.
    TooLarge { width: u32, height: u32, limit: u32 },
    /// The frame's byte sizes do not fit in the integer types that hold them.
    Overflow { width: u32, height: u32 },
    /// A read-back buffer for the frame would be larger than the device's largest buffer.
    BufferLimit { needed: u64, limit: u64 },
    /// A source side is zero, or the pixels are not `width × height × 4` bytes.
    Source { width: u32, height: u32, bytes: usize },
    /// A buffer or a frame does not have the size the layout needs.
    BufferSize { needed: u64, actual: u64 },
    /// A buffer offset is not a multiple of `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`.
    Offset(u64),
    /// The planes were made for another format than the converter's.
    Mismatch { converter: Format, planes: Format },
    /// A frame layout's fields are not what [`crate::FrameLayout::new`] gives for its format and
    /// size.
    Layout,
    /// A buffer of `bytes` bytes for the frame could not be allocated.
    Memory { bytes: usize },
    /// The device reported an error (validation, out of memory, a failed map or poll) while
    /// [`crate::YuvProvider`] ran a job on it.
    Device(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::OutputSize { width, height } => write!(f, "a 4:2:0 picture of {width}×{height} needs both sides even and not zero"),
            Error::TooLarge { width, height, limit } => write!(f, "{width}×{height} is larger than the largest texture side, {limit}"),
            Error::Overflow { width, height } => write!(f, "a frame of {width}×{height} is too large to hold"),
            Error::BufferLimit { needed, limit } => write!(f, "a read-back buffer of {needed} bytes is larger than the largest buffer, {limit}"),
            Error::Source { width, height, bytes } => write!(f, "{bytes} bytes are not an RGBA picture of {width}×{height}"),
            Error::BufferSize { needed, actual } => write!(f, "{actual} bytes where {needed} are needed"),
            Error::Offset(offset) => write!(f, "buffer offset {offset} is not a multiple of {}", wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
            Error::Mismatch { converter, planes } => write!(f, "{planes} planes given to a {converter} converter"),
            Error::Layout => f.write_str("the frame layout does not match its format and size"),
            Error::Memory { bytes } => write!(f, "a buffer of {bytes} bytes could not be allocated"),
            Error::Device(message) => write!(f, "the device reported an error: {message}"),
        }
    }
}

impl std::error::Error for Error {}
