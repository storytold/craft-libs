//! ICC profiles: the parsed model, the reader (ICC.1:2001-04 v2 and ICC.1:2010 v4) and the
//! connection of a profile to the PCS as evaluation stages.

use std::sync::Arc;

use crate::clut::Clut;
use crate::curve::Curve;
use crate::math::{self, Mat3};
use crate::pipeline::Stage;
use crate::{CmsError, Intent};

mod parse;

pub(crate) use parse::Reader;
use parse::parse_profile;

/// Profile/device class (header bytes 12..16).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProfileClass {
    #[allow(missing_docs)]
    Input,
    #[allow(missing_docs)]
    Display,
    #[allow(missing_docs)]
    Output,
    #[allow(missing_docs)]
    DeviceLink,
    #[allow(missing_docs)]
    ColorSpace,
    #[allow(missing_docs)]
    Abstract,
    #[allow(missing_docs)]
    NamedColor,
    /// A class signature this crate does not know, kept as read.
    Unknown(u32),
}

impl ProfileClass {
    /// Maps the four-character class signature (big-endian `u32`) to a class; unknown signatures
    /// give [`ProfileClass::Unknown`].
    pub fn from_sig(s: u32) -> Self {
        match &s.to_be_bytes() {
            b"scnr" => Self::Input,
            b"mntr" => Self::Display,
            b"prtr" => Self::Output,
            b"link" => Self::DeviceLink,
            b"spac" => Self::ColorSpace,
            b"abst" => Self::Abstract,
            b"nmcl" => Self::NamedColor,
            _ => Self::Unknown(s),
        }
    }
    /// The four-character class signature as a big-endian `u32`.
    pub fn sig(self) -> u32 {
        u32::from_be_bytes(*match self {
            Self::Input => b"scnr",
            Self::Display => b"mntr",
            Self::Output => b"prtr",
            Self::DeviceLink => b"link",
            Self::ColorSpace => b"spac",
            Self::Abstract => b"abst",
            Self::NamedColor => b"nmcl",
            Self::Unknown(s) => return s,
        })
    }
}

/// Data colour space (header bytes 16..20).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorSpace {
    #[allow(missing_docs)]
    Xyz,
    #[allow(missing_docs)]
    Lab,
    #[allow(missing_docs)]
    Luv,
    #[allow(missing_docs)]
    YCbCr,
    #[allow(missing_docs)]
    Yxy,
    #[allow(missing_docs)]
    Rgb,
    #[allow(missing_docs)]
    Gray,
    #[allow(missing_docs)]
    Hsv,
    #[allow(missing_docs)]
    Hls,
    #[allow(missing_docs)]
    Cmyk,
    #[allow(missing_docs)]
    Cmy,
    /// `nCLR` spaces with n = 2..=15 colorants.
    Color(u8),
    /// A colour space signature this crate does not know, kept as read.
    Unknown(u32),
}

impl ColorSpace {
    /// Maps the four-character colour space signature (big-endian `u32`) to a colour space; unknown
    /// signatures give [`ColorSpace::Unknown`].
    pub fn from_sig(s: u32) -> Self {
        let b = s.to_be_bytes();
        match &b {
            b"XYZ " => Self::Xyz,
            b"Lab " => Self::Lab,
            b"Luv " => Self::Luv,
            b"YCbr" => Self::YCbCr,
            b"Yxy " => Self::Yxy,
            b"RGB " => Self::Rgb,
            b"GRAY" => Self::Gray,
            b"HSV " => Self::Hsv,
            b"HLS " => Self::Hls,
            b"CMYK" => Self::Cmyk,
            b"CMY " => Self::Cmy,
            [d, b'C', b'L', b'R'] => match *d {
                b'2'..=b'9' => Self::Color(d - b'0'),
                b'A'..=b'F' => Self::Color(d - b'A' + 10),
                _ => Self::Unknown(s),
            },
            _ => Self::Unknown(s),
        }
    }
    /// The four-character colour space signature as a big-endian `u32`.
    pub fn sig(self) -> u32 {
        u32::from_be_bytes(match self {
            Self::Xyz => *b"XYZ ",
            Self::Lab => *b"Lab ",
            Self::Luv => *b"Luv ",
            Self::YCbCr => *b"YCbr",
            Self::Yxy => *b"Yxy ",
            Self::Rgb => *b"RGB ",
            Self::Gray => *b"GRAY",
            Self::Hsv => *b"HSV ",
            Self::Hls => *b"HLS ",
            Self::Cmyk => *b"CMYK",
            Self::Cmy => *b"CMY ",
            Self::Color(n) => [if n < 10 { b'0' + n } else { b'A' + n - 10 }, b'C', b'L', b'R'],
            Self::Unknown(s) => return s,
        })
    }
    /// Number of channels of the space.
    pub fn channels(self) -> usize {
        match self {
            Self::Gray => 1,
            Self::Cmyk => 4,
            Self::Color(n) => n as usize,
            Self::Unknown(_) => 0,
            _ => 3,
        }
    }
}

/// Which PCS a stage list ends in (float values: XYZ with Y = 1 for the white, or Lab L 0–100).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pcs {
    #[allow(missing_docs)]
    Xyz,
    #[allow(missing_docs)]
    Lab,
}

/// Encoding family of a LUT tag, which fixes how its PCS side is encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LutKind {
    /// `mft1` (8-bit), v4-style Lab encoding.
    Lut8,
    /// `mft2` (16-bit), legacy v2 Lab encoding.
    Lut16,
    /// `mAB ` / `mBA ` (v4), v4 Lab encoding.
    Ab,
}

/// A parsed `AToB`/`BToA` tag: device-side and PCS-side values are normalised to `[0, 1]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut {
    /// Encoding family of the tag.
    pub kind: LutKind,
    /// Number of device-side channels.
    pub inputs: usize,
    /// Number of PCS-side channels.
    pub outputs: usize,
    /// Stages from device side to PCS side for `AToB` tags, the reverse for `BToA` tags.
    pub stages: Vec<Stage>,
}

/// Largest colour difference (CIE ΔE*ab) for which [`Profile::same_colors`] treats two profiles
/// as one colour space: invisible, yet 20× what re-encoding a profile costs (sRGB as v2 tables
/// against v4 parametric curves differs by 0.025). Moving sRGB's red primary 2% costs 1.0.
pub const SAME_COLORS_MAX_DELTA_E: f64 = 0.5;

/// A parsed ICC profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// Major version and minor.bugfix byte (e.g. `(4, 0x30)` for 4.3, `(2, 0x10)` for 2.1).
    pub version: (u8, u8),
    #[allow(missing_docs)]
    pub class: ProfileClass,
    #[allow(missing_docs)]
    pub color_space: ColorSpace,
    #[allow(missing_docs)]
    pub pcs: Pcs,
    /// Intent stored in the profile header.
    pub rendering_intent: Intent,
    /// Text of the `desc` tag; empty when absent.
    pub description: String,
    /// Text of the `cprt` tag; empty when absent.
    pub copyright: String,
    /// `wtpt` as stored (XYZ; D50 when absent).
    pub white_point: [f64; 3],
    /// `bkpt` when present.
    pub black_point: Option<[f64; 3]>,
    /// `chad` chromatic adaptation matrix when present.
    pub chad: Option<Mat3>,
    /// Colorant matrix (columns rXYZ, gXYZ, bXYZ), D50-adapted.
    pub matrix: Option<Mat3>,
    /// Red, green and blue tone curves (`rTRC`, `gTRC`, `bTRC`) of a matrix/TRC profile.
    pub trc: Option<[Curve; 3]>,
    /// Gray tone curve (`kTRC`) of a gray profile.
    pub gray_trc: Option<Curve>,
    /// A2B0/A2B1/A2B2 (perceptual, colorimetric, saturation).
    pub a2b: [Option<Lut>; 3],
    /// B2A0/B2A1/B2A2 (perceptual, colorimetric, saturation).
    pub b2a: [Option<Lut>; 3],
    /// The bytes this profile was parsed from (or encoded to), kept for byte-exact embedding.
    pub(crate) bytes: Option<Arc<Vec<u8>>>,
    pub(crate) hash: std::sync::OnceLock<u64>,
}

impl Profile {
    /// Parses ICC profile bytes.
    pub fn parse(bytes: &[u8]) -> Result<Profile, CmsError> {
        parse::reset_budgets();
        let mut p = parse_profile(bytes)?;
        p.bytes = Some(Arc::new(bytes.to_vec()));
        Ok(p)
    }

    /// The ICC bytes of this profile: the original bytes when it was parsed, otherwise a fresh
    /// ICC v4 encoding.
    pub fn to_bytes(&self) -> Arc<Vec<u8>> {
        match &self.bytes {
            Some(b) => b.clone(),
            None => Arc::new(crate::write::encode(self)),
        }
    }

    /// Re-encodes the model as ICC v4 bytes and caches them (used for built-in profiles).
    pub fn with_encoded_bytes(mut self) -> Profile {
        self.bytes = None;
        self.hash = Default::default();
        let b = crate::write::encode(&self);
        self.bytes = Some(Arc::new(b));
        self
    }

    /// Number of device channels of the data colour space.
    pub fn channels(&self) -> usize {
        self.color_space.channels()
    }

    /// Hash of the profile's ICC bytes (stable within a process; used as a cache key). It is a
    /// fixed-key 64-bit hash, so a crafted profile can share another's: equal hashes only narrow
    /// the search, [`Profile::same_bytes`] decides.
    pub fn content_hash(&self) -> u64 {
        *self.hash.get_or_init(|| {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            self.to_bytes().hash(&mut h);
            h.finish()
        })
    }

    /// Whether `self` and `other` have the same ICC bytes.
    pub fn same_bytes(&self, other: &Profile) -> bool {
        if let (Some(a), Some(b)) = (&self.bytes, &other.bytes)
            && Arc::ptr_eq(a, b)
        {
            return true;
        }
        self.content_hash() == other.content_hash() && self.to_bytes() == other.to_bytes()
    }

    /// Whether `self` and `other` describe the same colours: same colour space, and every sample
    /// of a device-value grid has the same colour through both (relative colorimetric, within
    /// [`SAME_COLORS_MAX_DELTA_E`]). Different encodings of one space are the same colours: the
    /// sRGB IEC61966-2.1 Photoshop embeds (v2, 1024-entry tables) and the built-in sRGB (v4,
    /// parametric curves) differ in every byte. Colours are compared rather than round-tripped
    /// because CMYK → PCS → CMYK is not an identity even through a single profile.
    pub fn same_colors(&self, other: &Profile) -> bool {
        if self.same_bytes(other) {
            return true;
        }
        let n = self.channels();
        if self.color_space != other.color_space || n == 0 || n > 4 {
            return false;
        }
        let lab = crate::builtin::Builtin::LabD50.profile();
        let to_lab = |p: &Profile| crate::transform::Transform::new(p, lab, Intent::RelativeColorimetric, false);
        let (Ok(a), Ok(b)) = (to_lab(self), to_lab(other)) else {
            return false;
        };
        // 17 steps per channel catch a TRC that differs only in its toe; 9 keep CMYK at 6561 samples.
        let steps: usize = if n <= 3 { 17 } else { 9 };
        // Lab D50 is the PCS, normalised as in ICC v4.
        let decode = |o: [f32; 3]| [f64::from(o[0]) * 100.0, f64::from(o[1]) * 255.0 - 128.0, f64::from(o[2]) * 255.0 - 128.0];
        let mut input = [0f32; 4];
        (0..steps.pow(n as u32)).all(|i| {
            let mut k = i;
            for v in input.iter_mut().take(n) {
                *v = (k % steps) as f32 / (steps - 1) as f32;
                k /= steps;
            }
            let (mut la, mut lb) = ([0f32; 3], [0f32; 3]);
            a.eval(&input[..n], &mut la).is_ok()
                && b.eval(&input[..n], &mut lb).is_ok()
                && math::delta_e76(decode(la), decode(lb)) <= SAME_COLORS_MAX_DELTA_E
        })
    }

    /// Media white point used for ICC-absolute colorimetry: `wtpt`, except for v2 display
    /// profiles whose `wtpt` records the (unadapted) display white; those are treated as D50.
    pub fn media_white(&self) -> [f64; 3] {
        if self.version.0 < 4 && self.class == ProfileClass::Display {
            return math::D50;
        }
        self.white_point
    }

    /// An RGB view of a gray TRC profile: each of R, G, B carries the gray curve and maps to a
    /// third of the white, so `(v, v, v)` has exactly the colour of gray `v`. Used for
    /// composites of gray documents, which are stored as replicated RGB.
    pub fn gray_as_rgb(&self) -> Option<Profile> {
        let c = self.gray_trc.clone()?;
        if self.pcs != Pcs::Xyz {
            return None;
        }
        let w = math::D50;
        let m = [[w[0] / 3.0; 3], [w[1] / 3.0; 3], [w[2] / 3.0; 3]];
        let mut p = self.clone();
        p.color_space = ColorSpace::Rgb;
        p.description = format!("{} (as RGB)", self.description);
        p.matrix = Some(m);
        p.trc = Some([c.clone(), c.clone(), c]);
        p.gray_trc = None;
        // The gray LUTs take one input channel; this profile is the matrix/TRC model above.
        p.a2b = Default::default();
        p.b2a = Default::default();
        p.bytes = None;
        p.hash = Default::default();
        Some(p)
    }

    /// `true` for matrix/TRC RGB or gray TRC profiles without LUTs.
    pub fn is_matrix_shaper(&self) -> bool {
        self.a2b.iter().all(Option::is_none) && (self.matrix.is_some() && self.trc.is_some() || self.gray_trc.is_some())
    }

    /// Whether colours can be converted into this profile for every intent. Input profiles with
    /// only AToB tables (Apple's Wide Color Sharing Profile on iPhone photos) can't: they
    /// describe a source but have no PCS → device path.
    pub fn can_be_destination(&self) -> bool {
        [Intent::Perceptual, Intent::RelativeColorimetric, Intent::Saturation].into_iter().all(|i| self.pcs_to_device(i).is_ok())
    }

    fn lut_for(luts: &[Option<Lut>; 3], intent: Intent) -> Option<&Lut> {
        let i = match intent {
            Intent::Perceptual => 0,
            Intent::RelativeColorimetric | Intent::AbsoluteColorimetric => 1,
            Intent::Saturation => 2,
        };
        luts[i].as_ref().or(luts[0].as_ref())
    }

    /// Whether the profile has a dedicated table for `intent` (matrix profiles support every
    /// intent colorimetrically).
    pub fn supports_intent(&self, intent: Intent, input: bool) -> bool {
        let luts = if input { &self.a2b } else { &self.b2a };
        let i = match intent {
            Intent::Perceptual => 0,
            Intent::RelativeColorimetric | Intent::AbsoluteColorimetric => 1,
            Intent::Saturation => 2,
        };
        luts[i].is_some() || luts.iter().all(Option::is_none)
    }

    /// Stages mapping normalised device values to the float PCS.
    pub fn device_to_pcs(&self, intent: Intent) -> Result<(Vec<Stage>, Pcs), CmsError> {
        if let Some(l) = Self::lut_for(&self.a2b, intent) {
            let mut st = l.stages.clone();
            st.push(decode_pcs(self.pcs, l.kind));
            return Ok((st, self.pcs));
        }
        match self.color_space {
            ColorSpace::Rgb => {
                let (Some(m), Some(trc)) = (&self.matrix, &self.trc) else {
                    return Err(CmsError::Unsupported("RGB profile without matrix/TRC or AToB tags".into()));
                };
                Ok((vec![Stage::Curves(trc.to_vec()), Stage::matrix3(m)], Pcs::Xyz))
            }
            ColorSpace::Gray => {
                let c = self.gray_trc.clone().ok_or_else(|| CmsError::Unsupported("gray profile without kTRC".into()))?;
                if self.pcs == Pcs::Lab {
                    // The curve yields L*/100.
                    let m = Stage::Matrix { rows: 3, cols: 1, m: vec![100.0, 0.0, 0.0], offset: vec![0.0; 3] };
                    Ok((vec![Stage::Curves(vec![c]), m], Pcs::Lab))
                } else {
                    let w = math::D50;
                    let m = Stage::Matrix { rows: 3, cols: 1, m: w.to_vec(), offset: vec![0.0; 3] };
                    Ok((vec![Stage::Curves(vec![c]), m], Pcs::Xyz))
                }
            }
            ColorSpace::Lab => Ok((vec![Stage::DecodeLabV4], Pcs::Lab)),
            ColorSpace::Xyz => Ok((vec![Stage::DecodeXyz], Pcs::Xyz)),
            cs => Err(CmsError::Unsupported(format!("{cs:?} profile without AToB tags"))),
        }
    }

    /// Stages mapping the float PCS (of the returned kind) to normalised device values.
    pub fn pcs_to_device(&self, intent: Intent) -> Result<(Vec<Stage>, Pcs), CmsError> {
        if let Some(l) = Self::lut_for(&self.b2a, intent) {
            let mut st = vec![encode_pcs(self.pcs, l.kind)];
            st.extend(l.stages.iter().cloned());
            return Ok((st, self.pcs));
        }
        match self.color_space {
            ColorSpace::Rgb => {
                let (Some(m), Some(trc)) = (&self.matrix, &self.trc) else {
                    return Err(CmsError::Unsupported("RGB profile without matrix/TRC or BToA tags".into()));
                };
                let inv = math::invert(m).ok_or_else(|| CmsError::Invalid("singular colorant matrix".into()))?;
                Ok((vec![Stage::matrix3(&inv), Stage::InvCurves(trc.to_vec())], Pcs::Xyz))
            }
            ColorSpace::Gray => {
                let c = self.gray_trc.clone().ok_or_else(|| CmsError::Unsupported("gray profile without kTRC".into()))?;
                if self.pcs == Pcs::Lab {
                    let m = Stage::Matrix { rows: 1, cols: 3, m: vec![0.01, 0.0, 0.0], offset: vec![0.0] };
                    Ok((vec![m, Stage::InvCurves(vec![c])], Pcs::Lab))
                } else {
                    let m = Stage::Matrix { rows: 1, cols: 3, m: vec![0.0, 1.0, 0.0], offset: vec![0.0] };
                    Ok((vec![m, Stage::InvCurves(vec![c])], Pcs::Xyz))
                }
            }
            ColorSpace::Lab => Ok((vec![Stage::EncodeLabV4], Pcs::Lab)),
            ColorSpace::Xyz => Ok((vec![Stage::EncodeXyz], Pcs::Xyz)),
            cs => Err(CmsError::Unsupported(format!("{cs:?} profile without BToA tags"))),
        }
    }
}

fn decode_pcs(pcs: Pcs, kind: LutKind) -> Stage {
    match (pcs, kind) {
        (Pcs::Xyz, _) => Stage::DecodeXyz,
        (Pcs::Lab, LutKind::Lut16) => Stage::DecodeLabV2,
        (Pcs::Lab, _) => Stage::DecodeLabV4,
    }
}

fn encode_pcs(pcs: Pcs, kind: LutKind) -> Stage {
    match (pcs, kind) {
        (Pcs::Xyz, _) => Stage::EncodeXyz,
        (Pcs::Lab, LutKind::Lut16) => Stage::EncodeLabV2,
        (Pcs::Lab, _) => Stage::EncodeLabV4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::Builtin;

    /// `p` with the content hash of `other`: the hash is a fixed-key 64-bit SipHash, so a
    /// crafted profile can have another's (#8613). Setting it stands in for finding one.
    fn with_hash_of(p: &Profile, other: &Profile) -> Profile {
        let p = Profile { hash: std::sync::OnceLock::from(other.content_hash()), ..p.clone() };
        assert_eq!(p.content_hash(), other.content_hash());
        p
    }

    /// #8613: a profile that shares sRGB's hash is not taken for sRGB.
    #[test]
    fn a_hash_collision_is_not_the_same_colours() {
        let srgb = Builtin::Srgb.profile();
        let p3 = with_hash_of(Builtin::DisplayP3.profile(), srgb);
        assert!(!p3.same_bytes(srgb));
        assert!(!p3.same_colors(srgb));
        // Controls: equal bytes are the same, whatever object holds them.
        assert!(srgb.same_bytes(srgb));
        assert!(srgb.same_bytes(&Profile::parse(&srgb.to_bytes()).unwrap()));
        assert!(Builtin::DisplayP3.profile().same_colors(Builtin::DisplayP3.profile()));
    }

    /// #8613: the transform cache does not hand a colliding profile the other's transform.
    #[test]
    fn a_hash_collision_does_not_get_the_cached_transform() {
        use crate::transform::{Transform, TransformOptions, cached};
        let (srgb, lab) = (Builtin::Srgb.profile(), Builtin::LabD50.profile());
        let p3 = with_hash_of(Builtin::DisplayP3.profile(), srgb);
        let opts = TransformOptions::default();
        let green = [0.0, 1.0, 0.0];
        let eval = |t: &Transform| {
            let mut out = [0f32; 3];
            t.eval(&green, &mut out).unwrap();
            out
        };
        let want = eval(&Transform::with_options(Builtin::DisplayP3.profile(), lab, opts).unwrap());
        assert_ne!(want, eval(&Transform::with_options(srgb, lab, opts).unwrap()), "P3 green is not sRGB green");
        let sr = eval(&cached(srgb, lab, opts).unwrap());
        assert_eq!(eval(&cached(&p3, lab, opts).unwrap()), want);
        // Control: sRGB still gets its own transform after the colliding lookup.
        assert_eq!(eval(&cached(srgb, lab, opts).unwrap()), sr);
    }
}
