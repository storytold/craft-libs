#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Colour management for the Craft suite: a pure-Rust ICC v2/v4 colour management module.
//!
//! * [`Profile`] parses ICC profiles (matrix/TRC RGB, gray TRC, `mft1`/`mft2`/`mAB `/`mBA `
//!   LUT profiles incl. CMYK↔Lab, `curv`/`para` curves) and writes ICC v4 profiles.
//! * [`Transform`] connects two profiles through the PCS (XYZ/Lab D50) with a rendering
//!   [`Intent`] and optional black point compensation, and converts 8/16-bit and float pixel
//!   buffers through precomputed tables (tetrahedral 3D/4D interpolation, rayon on native).
//! * [`builtin`] ships sRGB, Display P3, Adobe RGB (1998)-compatible, ProPhoto-compatible,
//!   linear sRGB, Rec. 2020, Gray 2.2, sGray, Lab D50, a synthetic coated CMYK profile and a
//!   Coated FOGRA39 profile fitted to Fogra's characterisation data.
//! * [`NamedColors`] reads a named-colour profile's spot colours and converts them into a
//!   destination profile.
//! * [`Lut3d`] samples a display transform into a 3D texture for the GPU canvas.
//! * [`GpuMatrixShaper`] preserves near-black precision for separable RGB display transforms.
//!
//! Implemented from the public ICC.1:2010 (v4.3) and ICC.1:2001-04 (v2) specifications,
//! CIE 15, and Adobe's published black point compensation paper.
#![forbid(unsafe_code)]

pub mod builtin;
pub mod clut;
pub mod convert;
pub mod curve;
pub mod gamut;
mod gpu_matrix_shaper;
pub mod lut3d;
pub mod lutfile;
pub mod math;
pub mod named;
pub mod pipeline;
pub mod profile;
pub mod synth;
pub mod transform;
mod write;

pub use builtin::Builtin;
pub use clut::Clut;
pub use curve::Curve;
pub use gamut::GamutCheck;
pub use gpu_matrix_shaper::GpuMatrixShaper;
pub use lut3d::Lut3d;
pub use named::{NamedColor, NamedColors};
pub use profile::{ColorSpace, Pcs, Profile, ProfileClass};
pub use transform::{SampleKind, Transform, TransformOptions, black_point, cached};
pub use write::profile_id;

/// ICC rendering intent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Intent {
    #[allow(missing_docs)]
    Perceptual = 0,
    #[default]
    #[allow(missing_docs)]
    RelativeColorimetric = 1,
    #[allow(missing_docs)]
    Saturation = 2,
    #[allow(missing_docs)]
    AbsoluteColorimetric = 3,
}

impl Intent {
    /// All four intents, in ICC numeric order.
    pub const ALL: [Intent; 4] = [Intent::Perceptual, Intent::RelativeColorimetric, Intent::Saturation, Intent::AbsoluteColorimetric];

    /// Maps the ICC intent number (0 to 3) to an intent; any other value gives `None`.
    pub fn from_u32(v: u32) -> Option<Intent> {
        Some(match v {
            0 => Intent::Perceptual,
            1 => Intent::RelativeColorimetric,
            2 => Intent::Saturation,
            3 => Intent::AbsoluteColorimetric,
            _ => return None,
        })
    }

    /// Parses `perceptual`, `relative`/`relativeColorimetric`, `saturation`,
    /// `absolute`/`absoluteColorimetric` (case-insensitive).
    pub fn parse(s: &str) -> Option<Intent> {
        let n: String = s.to_ascii_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        Some(match n.as_str() {
            "perceptual" | "0" => Intent::Perceptual,
            "relative" | "relativecolorimetric" | "colorimetric" | "1" => Intent::RelativeColorimetric,
            "saturation" | "2" => Intent::Saturation,
            "absolute" | "absolutecolorimetric" | "3" => Intent::AbsoluteColorimetric,
            _ => return None,
        })
    }

    /// Stable lowercase name (`perceptual`, `relative`, `saturation`, `absolute`) that
    /// [`Intent::parse`] accepts back.
    pub fn id(self) -> &'static str {
        match self {
            Intent::Perceptual => "perceptual",
            Intent::RelativeColorimetric => "relative",
            Intent::Saturation => "saturation",
            Intent::AbsoluteColorimetric => "absolute",
        }
    }
}

/// Errors from parsing profiles, building transforms, or arguments that do not fit an operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CmsError {
    /// The data ends before a structure it announces.
    Truncated,
    /// Missing `acsp` signature: not an ICC profile.
    BadSignature,
    /// The profile is malformed; the message says where.
    Invalid(String),
    /// The profile is valid but uses a feature this crate does not implement.
    Unsupported(String),
    /// A caller argument does not fit the operation: a pixel stride below the channel count, a
    /// buffer that is not whole pixels or is too short, a slice shorter than the channel count,
    /// or table dimensions out of range.
    InvalidArgument(String),
}

impl std::fmt::Display for CmsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CmsError::Truncated => write!(f, "ICC profile is truncated"),
            CmsError::BadSignature => write!(f, "not an ICC profile (missing 'acsp' signature)"),
            CmsError::Invalid(m) => write!(f, "invalid ICC profile: {m}"),
            CmsError::Unsupported(m) => write!(f, "unsupported ICC feature: {m}"),
            CmsError::InvalidArgument(m) => write!(f, "invalid argument: {m}"),
        }
    }
}

impl std::error::Error for CmsError {}

pub(crate) fn invalid_arg(m: impl Into<String>) -> CmsError {
    CmsError::InvalidArgument(m.into())
}

/// Checks that `slice` holds at least `n` values (single-colour evaluation).
pub(crate) fn check_len(what: &str, len: usize, n: usize) -> Result<(), CmsError> {
    if len < n { Err(invalid_arg(format!("{what} has {len} values, {n} needed"))) } else { Ok(()) }
}

#[cfg(test)]
extern crate self as craft_cms;

#[cfg(test)]
#[path = "../tests/main.rs"]
mod integration;
