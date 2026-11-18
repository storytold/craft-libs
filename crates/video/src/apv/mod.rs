//! Clean-room Advanced Professional Video (APV) codec, implemented strictly from IETF RFC 9924
//! (February 2026).
//!
//! - **Decoder** (feature `apv-decode`: [`decode_frame`], [`decode_frame_with`], [`decode_frame_into`], [`probe`]):
//!   every profile in RFC 9924 §9 (`422-10`, `422-12`, `444-10`, `444-12`, `4444-10`, `4444-12`,
//!   `400-10`) at 10–16 bits per sample, custom quantization matrices, multi-tile parallel decoding
//!   (behind the `threads` feature), 4:4:4:4 and auxiliary alpha PBU (`pbu_type == 27`) support,
//!   and HDR static metadata (`MDCV` and `CLL`, RFC 9924 §8).
//! - **Encoder** (feature `apv-encode`: [`Encoder`]): progressive intra frame encoding across all 7 profiles with fixed-QP
//!   or frame-budget rate control, plus `APVDecoderConfigurationRecord` (`apvC`) generation.
//!
//! See this module's README for specification edition, accuracy against external oracles, and design notes.

#![allow(clippy::needless_range_loop, clippy::type_complexity)]

pub mod dct;
#[cfg(feature = "apv-decode")]
pub mod decode;
#[cfg(feature = "apv-encode")]
pub mod encode;
pub mod header;
pub mod tables;

#[cfg(feature = "apv-decode")]
pub use decode::{DecodeOptions, decode_frame, decode_frame_into, decode_frame_with, probe};
#[cfg(feature = "apv-encode")]
pub use encode::{Encoder, EncoderConfig};
pub use header::{AU_SIGNATURE, FrameHeader, split_raw_bitstream, unwrap_au};

/// APV codec errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("truncated APV bitstream")]
    Truncated,
    #[error("invalid APV bitstream: {0}")]
    Invalid(&'static str),
    #[error("corrupt APV tile {0}")]
    CorruptTile(usize),
    #[error("invalid APV encoder input: {0}")]
    Input(String),
    #[error("APV frame exceeds the decoder memory budget")]
    MemoryBudget,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Chroma format (`chroma_format_idc` in RFC 9924 §4.1 Table 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChromaFormat {
    /// `chroma_format_idc == 0`: 4:0:0 monochrome (1 component).
    Monochrome,
    /// `chroma_format_idc == 2`: 4:2:2 YCbCr (3 components, `SubWidthC = 2, SubHeightC = 1`).
    Yuv422,
    /// `chroma_format_idc == 3`: 4:4:4 YCbCr (3 components, `SubWidthC = 1, SubHeightC = 1`).
    Yuv444,
    /// `chroma_format_idc == 4`: 4:4:4:4 YCbCrA (4 components, `SubWidthC = 1, SubHeightC = 1`).
    Yuv4444,
}

impl ChromaFormat {
    pub fn from_idc(idc: u8) -> Option<Self> {
        match idc {
            0 => Some(Self::Monochrome),
            2 => Some(Self::Yuv422),
            3 => Some(Self::Yuv444),
            4 => Some(Self::Yuv4444),
            _ => None,
        }
    }

    pub fn idc(self) -> u8 {
        match self {
            Self::Monochrome => 0,
            Self::Yuv422 => 2,
            Self::Yuv444 => 3,
            Self::Yuv4444 => 4,
        }
    }

    pub fn num_comps(self) -> usize {
        match self {
            Self::Monochrome => 1,
            Self::Yuv422 | Self::Yuv444 => 3,
            Self::Yuv4444 => 4,
        }
    }

    pub fn sub_width_c(self) -> u32 {
        match self {
            Self::Yuv422 => 2,
            _ => 1,
        }
    }

    pub fn sub_height_c(self) -> u32 {
        1
    }

    pub fn chroma_width(self, width: u32) -> u32 {
        match self {
            Self::Monochrome => 0,
            Self::Yuv422 => width.div_ceil(2),
            Self::Yuv444 | Self::Yuv4444 => width,
        }
    }

    pub fn chroma_height(self, height: u32) -> u32 {
        match self {
            Self::Monochrome => 0,
            _ => height,
        }
    }

    pub fn has_alpha(self) -> bool {
        self == Self::Yuv4444
    }
}

/// APV profiles defined in RFC 9924 §9.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Profile {
    /// `profile_idc == 33`: 422-10 profile (4:2:2, 10-bit).
    #[default]
    P422_10,
    /// `profile_idc == 44`: 422-12 profile (4:2:2, 10–12-bit).
    P422_12,
    /// `profile_idc == 55`: 444-10 profile (4:2:2..4:4:4, 10-bit).
    P444_10,
    /// `profile_idc == 66`: 444-12 profile (4:2:2..4:4:4, 10–12-bit).
    P444_12,
    /// `profile_idc == 77`: 4444-10 profile (4:2:2..4:4:4:4, 10-bit).
    P4444_10,
    /// `profile_idc == 88`: 4444-12 profile (4:2:2..4:4:4:4, 10–12-bit).
    P4444_12,
    /// `profile_idc == 99`: 400-10 profile (4:0:0, 10-bit).
    P400_10,
}

impl Profile {
    pub const ALL: [Profile; 7] =
        [Profile::P422_10, Profile::P422_12, Profile::P444_10, Profile::P444_12, Profile::P4444_10, Profile::P4444_12, Profile::P400_10];

    pub fn idc(self) -> u8 {
        match self {
            Self::P422_10 => 33,
            Self::P422_12 => 44,
            Self::P444_10 => 55,
            Self::P444_12 => 66,
            Self::P4444_10 => 77,
            Self::P4444_12 => 88,
            Self::P400_10 => 99,
        }
    }

    pub fn from_idc(idc: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.idc() == idc)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::P422_10 => "APV 422-10",
            Self::P422_12 => "APV 422-12",
            Self::P444_10 => "APV 444-10",
            Self::P444_12 => "APV 444-12",
            Self::P4444_10 => "APV 4444-10",
            Self::P4444_12 => "APV 4444-12",
            Self::P400_10 => "APV 400-10",
        }
    }

    pub fn short_name(self) -> &'static str {
        match self {
            Self::P422_10 => "422-10",
            Self::P422_12 => "422-12",
            Self::P444_10 => "444-10",
            Self::P444_12 => "444-12",
            Self::P4444_10 => "4444-10",
            Self::P4444_12 => "4444-12",
            Self::P400_10 => "400-10",
        }
    }

    pub fn default_chroma(self) -> ChromaFormat {
        match self {
            Self::P422_10 | Self::P422_12 => ChromaFormat::Yuv422,
            Self::P444_10 | Self::P444_12 => ChromaFormat::Yuv444,
            Self::P4444_10 | Self::P4444_12 => ChromaFormat::Yuv4444,
            Self::P400_10 => ChromaFormat::Monochrome,
        }
    }

    pub fn default_bit_depth(self) -> u8 {
        match self {
            Self::P422_12 | Self::P444_12 | Self::P4444_12 => 12,
            _ => 10,
        }
    }

    /// Check whether `(chroma, bit_depth)` conforms to this profile per RFC 9924 §9.3.
    pub fn supports(self, chroma: ChromaFormat, bit_depth: u8) -> bool {
        let c = chroma.idc();
        let b = bit_depth.saturating_sub(8);
        match self {
            Self::P422_10 => c == 2 && b == 2,
            Self::P422_12 => c == 2 && (2..=4).contains(&b),
            Self::P444_10 => (2..=3).contains(&c) && b == 2,
            Self::P444_12 => (2..=3).contains(&c) && (2..=4).contains(&b),
            Self::P4444_10 => (2..=4).contains(&c) && b == 2,
            Self::P4444_12 => (2..=4).contains(&c) && (2..=4).contains(&b),
            Self::P400_10 => c == 0 && b == 2,
        }
    }

    /// Nominal target bytes per frame for `(width, height)`.
    pub fn nominal_frame_bytes(self, width: u32, height: u32) -> u32 {
        let bpp = match self {
            Self::P400_10 => 1.5,
            Self::P422_10 => 3.0,
            Self::P422_12 => 3.8,
            Self::P444_10 => 4.2,
            Self::P444_12 => 5.0,
            Self::P4444_10 => 5.2,
            Self::P4444_12 => 6.2,
        };
        let bytes = ((width as f64) * (height as f64) * bpp / 8.0).round() as u32;
        bytes.max(512)
    }

    /// Nominal data rate in Mbit/s at 1920x1080 29.97 fps.
    pub fn nominal_mbps_1080p30(self) -> f64 {
        (self.nominal_frame_bytes(1920, 1080) as f64) * 8.0 * 29.97 / 1e6
    }
}

/// Human-readable profile name from `profile_idc` (e.g. `"APV 422-10"`).
pub fn profile_name(profile_idc: u8) -> String {
    Profile::from_idc(profile_idc).map(|p| p.name().to_string()).unwrap_or_else(|| format!("APV (profile {profile_idc})"))
}

/// Colour description from `frame_header()` (ITU-T H.273 code points, RFC 9924 §5.3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorInfo {
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub full_range: bool,
}

impl Default for ColorInfo {
    /// Inferred values when `color_description_present_flag == 0` (RFC 9924 §5.3.5: 2, 2, 2, 0).
    fn default() -> Self {
        Self { primaries: 2, transfer: 2, matrix: 2, full_range: false }
    }
}

impl ColorInfo {
    pub const BT709: Self = Self { primaries: 1, transfer: 1, matrix: 1, full_range: false };
    pub const BT2020_PQ: Self = Self { primaries: 9, transfer: 16, matrix: 9, full_range: false };
    pub const BT2020_HLG: Self = Self { primaries: 9, transfer: 18, matrix: 9, full_range: false };
}

/// Mastering Display Color Volume metadata (`payloadType == 5`, RFC 9924 §8.2.3).
///
/// Chromaticities are in 0.16 fixed-point (1/65536), order `[R, G, B]`;
/// `max_luminance` is 24.8 fixed-point cd/m²; `min_luminance` is 18.14 fixed-point cd/m².
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MasteringDisplay {
    pub primaries: [(u16, u16); 3],
    pub white_point: (u16, u16),
    pub max_luminance: u32,
    pub min_luminance: u32,
}

/// Content Light-Level Information metadata (`payloadType == 6`, RFC 9924 §8.2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentLightLevel {
    pub max_cll: u16,
    pub max_fall: u16,
}

/// A decoded (or to-be-encoded) APV frame: planar `u16` samples, rows packed without padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub chroma: ChromaFormat,
    /// Significant bits per sample (`10..=16`, or `8..=16` when rescaled via the decoder's `DecodeOptions`).
    pub bit_depth: u8,
    pub y: Vec<u16>,
    pub cb: Vec<u16>,
    pub cr: Vec<u16>,
    pub alpha: Option<Vec<u16>>,
    pub color: ColorInfo,
    pub profile_idc: u8,
    pub level_idc: u8,
    pub band_idc: u8,
    pub mastering_display: Option<MasteringDisplay>,
    pub content_light: Option<ContentLightLevel>,
}

impl Frame {
    /// Allocate a black video-range frame.
    pub fn new(width: u32, height: u32, chroma: ChromaFormat, bit_depth: u8, with_alpha: bool) -> Self {
        let n = (width as usize) * (height as usize);
        let cw = chroma.chroma_width(width) as usize;
        let ch = chroma.chroma_height(height) as usize;
        let cn = cw * ch;
        // The fill values are those of the nearest depth the format has (8..=16 bits).
        let depth = bit_depth.clamp(8, 16) as u32;
        let shift = depth - 8;
        let has_alpha = with_alpha || chroma == ChromaFormat::Yuv4444;
        Self {
            width,
            height,
            chroma,
            bit_depth,
            y: vec![(16u32 << shift) as u16; n],
            cb: vec![(128u32 << shift) as u16; cn],
            cr: vec![(128u32 << shift) as u16; cn],
            alpha: has_alpha.then(|| vec![((1u32 << bit_depth.min(16)) - 1) as u16; n]),
            color: ColorInfo::BT709,
            profile_idc: Profile::P422_10.idc(),
            level_idc: 90,
            band_idc: 2,
            mastering_display: None,
            content_light: None,
        }
    }

    pub fn chroma_width(&self) -> u32 {
        self.chroma.chroma_width(self.width)
    }

    pub fn chroma_height(&self) -> u32 {
        self.chroma.chroma_height(self.height)
    }
}
