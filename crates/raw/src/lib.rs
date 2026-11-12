#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Camera raw files: the file-format layer. Bytes in, the sensor data out as a neutral [`RawImage`] with what
//! the file says about rendering it (CFA layout, black and white levels, active area, default crop, orientation,
//! colour matrices, as-shot white balance, DNG opcode lists and profile tables), and back for DNG.
//!
//! - [`probe`] recognises raw containers; [`decode`] / [`decode_with`] turn a file into a [`RawImage`];
//!   [`probe_info`] reads the same description from the headers alone; [`embedded_preview`] returns the largest
//!   embedded JPEG (or DNG 1.7 JPEG XL) preview; [`semantic_masks`] reads a DNG's semantic masks.
//! - [`write_dng`] writes a [`RawImage`] as a DNG (uncompressed, lossless JPEG or Deflate).
//!
//! # Where this crate stops
//!
//! The samples come out as the file stores them (after a DNG `LinearizationTable`): black is not subtracted,
//! nothing is scaled, cropped, white balanced or demosaiced, opcode lists and profile tables are parsed but not
//! applied, and no colour transform is computed from the matrices. Normalisation, demosaicing, highlight
//! handling, the DNG colour model's arithmetic, lens corrections, tone and everything after belong to the
//! develop pipeline, which reads the public fields of the types here.
//!
//! # Formats
//!
//! One module and one Cargo feature per maker or container family:
//!
//! | Feature | Formats |
//! |---|---|
//! | `dng` | Adobe DNG: uncompressed (any depth, packed), lossless JPEG, lossy JPEG, Deflate (integer and floating point), strips and tiles, CFA and LinearRaw, DNG 1.7 interleaved storage; reading and writing |
//! | `jxl` | JPEG XL compressed DNG (DNG 1.7, compression 52546) through `jxl-oxide`; implies `dng` |
//! | `crw` | Canon CRW (CIFF), decoded by `craft-crw` and returned through this crate's types |
//! | `cr2` | Canon CR2: the sliced lossless-JPEG mosaic, and sRAW / mRAW (subsampled YCbCr) as linear RGB |
//! | `mrw` | Minolta / Konica Minolta MRW: 12-bit packed and 16-bit word mosaics in the `\0MRM` block container |
//! | `erf` | Epson ERF (R-D1 family): 12-bit samples in 16-byte blocks |
//! | `mef` | Mamiya MEF (ZD): uncompressed 12-bit packed |
//! | `kodak` | Kodak DCS520C TIFF: two-component lossless JPEG with a response curve |
//! | `nef` | Nikon NEF and NRW: uncompressed, packed, Nikon's Huffman coding (lossless and lossy, 12 and 14 bits), small raw |
//! | `rw2` | Panasonic RW2, Leica RWL and the older Panasonic / Leica RAW: packed, formats 4 and 6, format 8 prefix-coded strips, 16-bit words |
//! | `raf` | Fujifilm RAF: uncompressed Bayer and X-Trans, Fujifilm's lossless and lossy compression, lens corrections |
//! | `orf` | Olympus / OM System ORF: uncompressed 16-bit, 12-bit packed and two-field layouts, 16-byte blocks; compressed refused |
//! | `arw` | Sony ARW (and SR2 / SRF containers): uncompressed, ARW2, lossless JPEG tiles, ARW6, the encrypted SR2SubIFD, lens corrections |
//! | `cr3` | Canon CR3: CRX lossless Bayer and C-RAW (lossy), through craft-metadata's CR3 container |
//! | `pef` | Pentax PEF: uncompressed (packed or 16-bit words) and Pentax's Huffman coding (compression 65535) |
//! | `srw` | Samsung SRW: the uncompressed variants (compression 32769 words, 32770 at 12 bits packed) |
//! | `hasselblad` | Hasselblad 3FR and FFF (and other CFA TIFFs in the DNG layout without a DNG version): uncompressed and Hasselblad's lossless JPEG; implies `dng` |
//!
//! [`probe`] names every container it can tell apart whatever the features; [`RawFormat::is_supported`] says
//! whether this build decodes one.
//!
//! # Hostile input
//!
//! The decoders never panic on input. Every offset from a file is bounds-checked, sizes are checked against
//! [`Limits`] and against the bytes actually present before anything is allocated, and malformed files return
//! [`RawError`].
//!
//! # Sources
//!
//! Written from public specifications only (TIFF 6.0, TIFF/EP, the Adobe DNG Specification 1.4 to 1.7, ITU-T
//! T.81 Annex H for lossless JPEG, Adobe's TIFF technical notes for Deflate and the floating-point predictor)
//! and from observation of files. No other raw decoder's source was consulted; see `README.md`.
#![forbid(unsafe_code)]

#[cfg(feature = "cr2")]
mod cr2;
#[cfg(feature = "cr3")]
mod cr3;
#[cfg(feature = "crw")]
mod crw;
#[cfg(feature = "cr3")]
mod crx;
#[cfg(feature = "cr3")]
mod crx_wavelet;
#[cfg(feature = "dng")]
mod dng;
#[cfg(feature = "dng")]
pub mod dngwrite;
#[cfg(feature = "erf")]
mod erf;
pub mod gaintable;
#[cfg(feature = "hasselblad")]
mod hasselblad;
// (the Huffman difference decoder Pentax, Nikon, Panasonic and Sony codings share)
#[cfg(feature = "arw")]
mod arw;
#[cfg(feature = "arw")]
mod arw6;
#[cfg(feature = "arw")]
mod arw6_curve;
#[cfg(all(test, feature = "arw"))]
mod arw6_testenc;
#[cfg(feature = "arw")]
mod arw_lens;
#[cfg(feature = "arw")]
mod arw_lens_rm6;
#[allow(dead_code)]
mod huffman;
#[cfg(feature = "jxl")]
mod jxl;
pub mod kelvin;
#[cfg(feature = "kodak")]
mod kodak;
pub mod lens_metadata;
pub mod ljpeg;
#[cfg(feature = "arw")]
mod llvc;
#[cfg(feature = "mef")]
mod mef;
#[cfg(feature = "mrw")]
pub mod mrw;
#[cfg(feature = "nef")]
mod nef;
#[cfg(feature = "nef")]
mod nef_lens;
#[cfg(feature = "nef")]
mod nefc;
pub mod opcodes;
#[cfg(feature = "orf")]
mod orf;
#[cfg(feature = "pef")]
mod pef;
mod preview;
pub mod profile;
#[cfg(feature = "raf")]
mod raf;
#[cfg(feature = "raf")]
mod raf_lens;
#[cfg(feature = "raf")]
mod rafc;
#[cfg(feature = "rw2")]
mod rw2;
#[cfg(feature = "dng")]
pub mod semantic;
pub mod sparse;
#[cfg(feature = "arw")]
mod sr2;
// (the TIFF pixel plumbing every TIFF-based maker shares; DNG is its only user so far)
#[allow(dead_code)] // each maker uses some of these helpers; which are live depends on the features
mod shared;
#[cfg(feature = "srw")]
mod srw;
#[cfg_attr(not(feature = "dng"), allow(dead_code))]
mod tiffraw;
#[cfg_attr(not(feature = "dng"), allow(dead_code))]
mod unpack;

#[cfg(feature = "dng")]
pub use dngwrite::{DngCompression, DngWriteOptions, write_dng};
pub use lens_metadata::LensMetadata;
pub use opcodes::{Opcode, OpcodeLists};
#[cfg(feature = "arw")]
pub use preview::embedded_preview_dynamic_range_optimized;
pub use preview::{
    PREVIEW_PREFIX, PreviewColorSpace, PreviewRange, bounded_preview, embedded_preview, embedded_preview_color_space, embedded_preview_covering,
    embedded_preview_from_range,
};
#[cfg(feature = "dng")]
pub use semantic::{SemanticMask, semantic_masks};

use craft_tiff::{Tiff, TiffError};
use serde::{Deserialize, Serialize};

/// A 3×3 matrix, row-major.
pub type Matrix3 = [[f64; 3]; 3];

/// The most samples any decoder allocates for, whatever [`Limits::max_samples`] says: sizes are computed in
/// `u64` and held to this before they become a `usize`, so they cannot wrap on a 32-bit target.
pub const MAX_SAMPLES: usize = 1 << 30;

/// Caps on what a file may make a decoder allocate, checked against the headers before anything is allocated.
/// The struct is `#[non_exhaustive]`: start from [`Limits::default`] and set the fields you change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Most samples (`width × height × samples per pixel`) an image may declare. Values above [`MAX_SAMPLES`]
    /// are held to it. Default: [`MAX_SAMPLES`].
    pub max_samples: usize,
    /// Most samples one lossy (baseline) JPEG tile may decode to; the JPEG's own header sizes that buffer, not
    /// the tile's tags. Default: 2²⁶.
    pub max_lossy_tile_samples: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_samples: MAX_SAMPLES, max_lossy_tile_samples: 1 << 26 }
    }
}

impl Limits {
    /// [`Self::max_samples`] held to [`MAX_SAMPLES`].
    pub(crate) fn samples(&self) -> usize {
        self.max_samples.min(MAX_SAMPLES)
    }
}

/// SMPTE ST 12-1 time code of a CinemaDNG frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Timecode {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    pub drop_frame: bool,
}

impl Timecode {
    /// The first time code of a CinemaDNG `TimeCodes` tag (SMPTE ST 331 / 12-1 packing: BCD frames, seconds,
    /// minutes, hours in bytes 0 to 3, the drop-frame flag in bit 6 of byte 0). `None` unless every digit is a
    /// decimal digit and the time is within a day.
    pub fn from_bytes(b: &[u8]) -> Option<Timecode> {
        let bcd = |v: u8, tens_mask: u8| {
            let (u, t) = (v & 0x0F, (v >> 4) & tens_mask);
            (u <= 9).then_some(t * 10 + u)
        };
        let [f, s, m, h, ..] = *b else { return None };
        let tc = Timecode { frames: bcd(f, 0x3)?, seconds: bcd(s, 0x7)?, minutes: bcd(m, 0x7)?, hours: bcd(h, 0x3)?, drop_frame: f & 0x40 != 0 };
        (tc.seconds < 60 && tc.minutes < 60 && tc.hours < 24).then_some(tc)
    }

    /// The eight bytes of one `TimeCodes` entry (the binary groups zero): what [`Self::from_bytes`] reads.
    pub fn to_bytes(self) -> [u8; 8] {
        let bcd = |v: u8| ((v / 10) << 4) | (v % 10);
        let drop = if self.drop_frame { 0x40 } else { 0 };
        [bcd(self.frames % 40) | drop, bcd(self.seconds % 60), bcd(self.minutes % 60), bcd(self.hours % 24), 0, 0, 0, 0]
    }
}

/// The CinemaDNG tags of a frame (CinemaDNG Image Data Format Specification 1.0): a CinemaDNG clip is a
/// sequence of DNG files, each carrying these.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CinemaTags {
    /// `FrameRate` as numerator over denominator, frames a second.
    pub frame_rate: Option<(i64, i64)>,
    /// The first entry of `TimeCodes`.
    pub timecode: Option<Timecode>,
    /// `ReelName`.
    pub reel_name: Option<String>,
}

/// Checked product of input-derived dimensions, in u64 so it cannot wrap
/// on either 32-bit (wasm32) or 64-bit targets. The arguments are u64 too, so callers can feed
/// unchecked products of header values (row bytes, sample counts) without truncating on 32-bit.
///
/// Returns `None` on overflow, zero, or when the product exceeds [`MAX_SAMPLES`].
pub(crate) fn checked_pixel_count(width: u64, height: u64) -> Option<u64> {
    width.checked_mul(height).filter(|&n| n > 0 && n <= MAX_SAMPLES as u64)
}

/// Exif `CFAPattern` (in the Exif IFD), which the Pentax family states its layout with.
pub(crate) const EXIF_CFA_PATTERN: u16 = 0xa302;

/// How the stored rows and columns map onto the upright picture: the eight Exif `Orientation` values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Orientation {
    #[default]
    Normal,
    FlipH,
    Rotate180,
    FlipV,
    Transpose,
    Rotate90,
    Transverse,
    Rotate270,
}

impl Orientation {
    /// From the Exif code (1 to 8); anything else is [`Orientation::Normal`].
    pub fn from_exif(v: u16) -> Orientation {
        match v {
            2 => Orientation::FlipH,
            3 => Orientation::Rotate180,
            4 => Orientation::FlipV,
            5 => Orientation::Transpose,
            6 => Orientation::Rotate90,
            7 => Orientation::Transverse,
            8 => Orientation::Rotate270,
            _ => Orientation::Normal,
        }
    }

    /// The Exif code (1 to 8).
    pub fn to_exif(self) -> u16 {
        match self {
            Orientation::Normal => 1,
            Orientation::FlipH => 2,
            Orientation::Rotate180 => 3,
            Orientation::FlipV => 4,
            Orientation::Transpose => 5,
            Orientation::Rotate90 => 6,
            Orientation::Transverse => 7,
            Orientation::Rotate270 => 8,
        }
    }
}

/// What the raw container says about the camera and the shot, as far as a decoder needs it or as far as no
/// Exif reader could find it (a CRW has no Exif block). It is not the file's metadata: Exif, XMP and IPTC are
/// read from the same bytes by a metadata reader.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Camera {
    /// TIFF `Make` (or the container's own record), trimmed.
    pub make: Option<String>,
    /// TIFF `Model` (or the container's own record), trimmed.
    pub model: Option<String>,
    /// DNG `UniqueCameraModel`.
    pub unique_model: Option<String>,
    /// Capture time as Unix seconds on the camera's local clock, for containers that state it outside Exif
    /// (CRW's TimeStamp record). `None` for TIFF-based raws: their Exif block has it.
    pub capture_unix: Option<i64>,
}

impl Camera {
    /// From IFD0's `Make`, `Model` and `UniqueCameraModel`; empty strings are `None`.
    pub fn from_ifd0(ifd0: &craft_tiff::Ifd) -> Camera {
        let text = |tag| ifd0.string(tag).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        Camera {
            make: text(craft_tiff::tags::MAKE),
            model: text(craft_tiff::tags::MODEL),
            unique_model: text(craft_tiff::tags::UNIQUE_CAMERA_MODEL),
            capture_unix: None,
        }
    }
}

/// Errors from raw decoding.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RawError {
    #[error("not a recognised raw file")]
    NotRaw,
    #[error("unsupported raw variant: {0}")]
    Unsupported(String),
    #[error("corrupt raw data: {0}")]
    Corrupt(String),
    #[error("limit exceeded: {0}")]
    Limit(&'static str),
    #[error(transparent)]
    Tiff(#[from] TiffError),
}

pub type Result<T> = std::result::Result<T, RawError>;

/// Raw container formats recognised by [`probe`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RawFormat {
    Dng,
    Cr2,
    Cr3,
    Nef,
    Nrw,
    Arw,
    Raf,
    Orf,
    Rw2,
    Pef,
    Srw,
    /// Canon CRW (CIFF heap), the bodies before the CR2 era.
    Crw,
    /// Minolta MRW (`\0MRM` block container, `vendor::mrw`).
    Mrw,
    /// Sigma / Foveon X3F (`FOVb`).
    X3f,
    /// A TIFF-based raw that has no DNG version tag but describes itself the way DNG does (a
    /// full-resolution CFA image with black/white levels, a default crop and `AsShotNeutral`):
    /// Hasselblad 3FR and FFF. Read by `vendor::hasselblad` (the DNG reader plus what Hasselblad adds).
    CfaTiff,
    /// Kodak DCS520C TIFF: a two-component lossless JPEG CFA sub-IFD with a response curve (`vendor::kodak`).
    KodakDcs,
    /// Epson ERF: a CFA sub-IFD of 12-bit samples in 16-byte blocks (`vendor::erf`).
    Erf,
    /// Mamiya MEF: an uncompressed 12-bit packed CFA sub-IFD (`vendor::mef`).
    Mef,
    /// Another TIFF-based raw (3FR, IIQ, ERF, KDC, DCR, MOS, …).
    OtherTiff,
}

impl RawFormat {
    /// Whether [`decode`] reads this container in this build (possibly not every compression inside it): the
    /// format's decoder is in the crate and its Cargo feature is on.
    pub fn is_supported(self) -> bool {
        (self == RawFormat::Dng && cfg!(feature = "dng"))
            || (self == RawFormat::Crw && cfg!(feature = "crw"))
            || (self == RawFormat::CfaTiff && cfg!(feature = "hasselblad"))
            || (self == RawFormat::KodakDcs && cfg!(feature = "kodak"))
            || (self == RawFormat::Mrw && cfg!(feature = "mrw"))
            || (self == RawFormat::Erf && cfg!(feature = "erf"))
            || (self == RawFormat::Mef && cfg!(feature = "mef"))
            || (self == RawFormat::Cr2 && cfg!(feature = "cr2"))
            || (matches!(self, RawFormat::Nef | RawFormat::Nrw) && cfg!(feature = "nef"))
            || (self == RawFormat::Rw2 && cfg!(feature = "rw2"))
            || (self == RawFormat::Raf && cfg!(feature = "raf"))
            || (self == RawFormat::Orf && cfg!(feature = "orf"))
            || (self == RawFormat::Arw && cfg!(feature = "arw"))
            || (self == RawFormat::Cr3 && cfg!(feature = "cr3"))
            || (self == RawFormat::Pef && cfg!(feature = "pef"))
            || (self == RawFormat::Srw && cfg!(feature = "srw"))
    }
}

/// Recognise a raw file from its first bytes / TIFF structure.
pub fn probe(bytes: &[u8]) -> Option<RawFormat> {
    if bytes.len() >= 12 && &bytes[4..12] == b"ftypcrx " {
        return Some(RawFormat::Cr3);
    }
    if bytes.starts_with(b"FUJIFILMCCD-RAW") {
        return Some(RawFormat::Raf);
    }
    if bytes.starts_with(b"IIRO") || bytes.starts_with(b"IIRS") || bytes.starts_with(b"MMOR") {
        return Some(RawFormat::Orf);
    }
    if bytes.starts_with(b"IIU\0") {
        return Some(RawFormat::Rw2);
    }
    if bytes.starts_with(b"\0MRM") {
        return Some(RawFormat::Mrw);
    }
    if bytes.starts_with(b"FOVb") {
        return Some(RawFormat::X3f);
    }
    // (either byte order: the rule of craft-crw, the CRW decoder)
    if bytes.len() >= 14 && (bytes.starts_with(b"II") || bytes.starts_with(b"MM")) && &bytes[6..14] == b"HEAPCCDR" {
        return Some(RawFormat::Crw);
    }
    if bytes.len() >= 10 && bytes.starts_with(b"II*\0") && &bytes[8..10] == b"CR" {
        return Some(RawFormat::Cr2);
    }
    let (_, _) = Tiff::sniff(bytes)?;
    let opts = craft_tiff::ParseOptions { max_ifds: 256, ..Default::default() };
    let t = Tiff::parse_with(bytes, &opts).ok()?;
    let ifd0 = t.ifds.first()?;
    if ifd0.contains(craft_tiff::tags::DNG_VERSION) {
        return Some(RawFormat::Dng);
    }
    let make = t.find(craft_tiff::tags::MAKE).and_then(|e| e.value.as_str()).unwrap_or_default().to_ascii_uppercase();
    let has_cfa = has_raw_ifd(&t);
    // Pentax's reader takes the layout from the Exif `CFAPattern` when no IFD has a CFA photometric (uncompressed
    // and PackBits PEFs), so for the Pentax family that tag counts as raw evidence too
    let pentax_raw = has_cfa || t.exif().is_some_and(|e| e.contains(EXIF_CFA_PATTERN));
    // Samsung's raw IFD uses private compressions 32769..=32773 (`vendor::srw`), 32773 being PackBits elsewhere
    let samsung_raw = has_cfa || t.all_ifds().iter().any(|i| i.u16(craft_tiff::tags::COMPRESSION).is_some_and(|c| (32769..=32773).contains(&c)));
    // Sony raws without a CFA IFD (storytold/lightcraft#702): SRF (DSC-F828), whose IFD0 looks like RGB but holds 14-bit samples (12 or
    // 14 count: raw sensor depths; the raw is in Sony's SRF blocks), and the DSLR-A100, whose IFD0 is an old-style
    // JPEG thumbnail (compression 6, which today's TIFF writers don't produce) and whose `SubIFDs` tag addresses its
    // raw data. A camera-authored or exported TIFF has 8- or 16-bit samples, and its SubIFDs hold ordinary images
    let model = t.find(craft_tiff::tags::MODEL).and_then(|e| e.value.as_str()).unwrap_or_default().trim().to_ascii_uppercase();
    let sony_raw = has_cfa
        || ifd0.u64s(craft_tiff::tags::BITS_PER_SAMPLE).is_some_and(|b| b.iter().any(|v| matches!(v, 12 | 14)))
        || (model == "DSLR-A100" && ifd0.u16(craft_tiff::tags::COMPRESSION) == Some(6) && ifd0.contains(craft_tiff::tags::SUB_IFDS));
    if make.starts_with("CANON") && t.ifds.len() >= 4 && t.ifds[3].u16(craft_tiff::tags::COMPRESSION) == Some(6) {
        return Some(RawFormat::Cr2);
    }
    if make.starts_with("NIKON") && has_cfa {
        return Some(if t.all_ifds().len() > 1 { RawFormat::Nef } else { RawFormat::Nrw });
    }
    // Hasselblad's Sony-built bodies (the Lunar is a NEX-7 inside) write a Sony ARW under their own Make. Sony's
    // raw coding gives them away: compression 32767 or the raw IFD's tone-curve tag 0x7010, which no 3FR / FFF has
    let sony_coded = || t.all_ifds().iter().any(|i| i.u16(craft_tiff::tags::COMPRESSION) == Some(32767) || i.contains(0x7010));
    if (make.starts_with("SONY") && sony_raw) || (make.starts_with("HASSELBLAD") && sony_coded()) {
        return Some(RawFormat::Arw);
    }
    if (make.starts_with("PENTAX") || make.starts_with("RICOH")) && pentax_raw {
        return Some(RawFormat::Pef);
    }
    // a Samsung-branded body built on a Pentax design writes a Pentax-style maker note (the note's own magic)
    if make.starts_with("SAMSUNG") && pentax_raw && has_pentax_maker_note(&t, bytes) {
        return Some(RawFormat::Pef);
    }
    if make.starts_with("SAMSUNG") && samsung_raw {
        return Some(RawFormat::Srw);
    }
    #[cfg(feature = "erf")]
    if erf::raw_ifd(&t, bytes.len()).is_some() {
        return Some(RawFormat::Erf);
    }
    #[cfg(feature = "mef")]
    if mef::raw_ifd(&t, bytes.len()).is_some() {
        return Some(RawFormat::Mef);
    }
    #[cfg(feature = "kodak")]
    if kodak::raw_ifd(&t, bytes).is_some() {
        return Some(RawFormat::KodakDcs);
    }
    #[cfg(feature = "dng")]
    if dng::is_plain_cfa_tiff(&t, bytes) {
        return Some(RawFormat::CfaTiff);
    }
    if has_cfa || thumbnail_shell(&t, bytes.len()).is_some() || is_preview_container(ifd0) || is_leaf_mosaic(ifd0) {
        return Some(RawFormat::OtherTiff);
    }
    None
}

/// Leaf's private data block in IFD0, tag `0x8606` (`LeafData` in the ExifTool EXIF tag-name documentation, which
/// says the Leaf tags "are found in .MOS images from Leaf digital camera backs").
const LEAF_DATA: u16 = 0x8606;

/// An uncompressed Leaf MOS: IFD0 is the 16-bit mosaic itself (compression 1, BlackIsZero, one sample per pixel),
/// with no Make tag and no CFA photometric, so only Leaf's data block marks it as a raw (storytold/lightcraft#1275). A rendered
/// RGB image that kept the block stays an ordinary image.
fn is_leaf_mosaic(ifd0: &craft_tiff::Ifd) -> bool {
    ifd0.contains(LEAF_DATA) && ifd0.u16(craft_tiff::tags::SAMPLES_PER_PIXEL).unwrap_or(1) == 1
}

/// Whether the Exif maker note starts with the signature of the Pentax layouts (`AOC\0` or `PENTAX \0`).
fn has_pentax_maker_note(t: &Tiff, bytes: &[u8]) -> bool {
    let Some(e) = t.exif().and_then(|e| e.get(craft_tiff::tags::MAKER_NOTE)) else { return false };
    bytes.get(e.offset as usize..).is_some_and(|n| n.starts_with(b"AOC\0") || n.starts_with(b"PENTAX \0"))
}

/// Whether some IFD is marked as raw: CFA photometric, or a raw-only compression value (99 is not a
/// registered TIFF compression; Leaf MOS files use it for their tiled 16-bit lossless-JPEG raw),
/// or vendor-specific raw tags (Sony tone curve, CFA pattern, or Pentax compression 65535).
fn has_raw_ifd(t: &Tiff) -> bool {
    t.all_ifds().iter().any(|i| {
        i.u16(craft_tiff::tags::PHOTOMETRIC) == Some(craft_tiff::tags::photometric::CFA)
            || i.u16(craft_tiff::tags::COMPRESSION).is_some_and(|c| matches!(c, 34713 | 32767 | 32769 | 32770 | 99 | 65535))
            || i.contains(0x7010)
            || i.contains(craft_tiff::tags::CFA_PATTERN_EP)
            || i.contains(craft_tiff::tags::CFA_REPEAT_PATTERN_DIM)
    })
}

/// IFD0 that is not an image at all: no `ImageWidth` / `ImageLength`, but a JPEG preview pointer or
/// `SubIFDs` (the layout of Kodak KDC files, whose pictures sit in private tags). A plain TIFF reader
/// rejects such a file as malformed even though it carries a preview.
fn is_preview_container(ifd0: &craft_tiff::Ifd) -> bool {
    use craft_tiff::tags::{IMAGE_LENGTH, IMAGE_WIDTH, JPEG_INTERCHANGE_FORMAT, SUB_IFDS};
    !ifd0.contains(IMAGE_WIDTH) && !ifd0.contains(IMAGE_LENGTH) && (ifd0.contains(JPEG_INTERCHANGE_FORMAT) || ifd0.contains(SUB_IFDS))
}

/// A TIFF whose first image is only a reduced copy of a picture stored somewhere else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ThumbnailShell {
    /// Size of the first image (IFD0).
    pub thumb: (u32, u32),
    /// Size of the picture it stands in for.
    pub full: (u32, u32),
}

/// Some raw files are a TIFF "shell" around a private block: IFD0 holds a small RGB preview and nothing
/// in the TIFF structure marks the real image as raw (no CFA photometric, no raw compression). The
/// structure still gives them away, with a generic rule that names no maker or model: the file
/// declares a picture at least 4x larger than IFD0 in both directions, and either
/// - a `SubIFDs` child of IFD0 is that bigger image (an 8-bit grey mosaic in one such file), or
/// - the Exif `PixelXDimension` / `PixelYDimension` say so *and* the file holds enough bytes outside
///   every IFD's strips and tiles to carry that many pixels at one bit each (a downsized copy that kept
///   its old Exif size has no such block).
///
/// A multi-page TIFF is not caught (its pages are the IFD chain, not `SubIFDs`), nor a pyramid whose
/// IFD0 is the full image.
fn thumbnail_shell(t: &Tiff, len: usize) -> Option<ThumbnailShell> {
    use craft_tiff::tags::{IMAGE_LENGTH, IMAGE_WIDTH, PIXEL_X_DIMENSION, PIXEL_Y_DIMENSION};
    let ifd0 = t.ifds.first()?;
    let (w0, h0) = (ifd0.u32(IMAGE_WIDTH)?, ifd0.u32(IMAGE_LENGTH)?);
    if w0 == 0 || h0 == 0 {
        return None;
    }
    let much_bigger = |(w, h): (u32, u32)| u64::from(w) >= 4 * u64::from(w0) && u64::from(h) >= 4 * u64::from(h0);
    let shell = |full| Some(ThumbnailShell { thumb: (w0, h0), full });
    let mut best: Option<(u32, u32)> = None;
    for s in &ifd0.sub_ifds {
        if s.image().is_ok()
            && let (Some(w), Some(h)) = (s.u32(IMAGE_WIDTH), s.u32(IMAGE_LENGTH))
            && much_bigger((w, h))
            && best.is_none_or(|(bw, bh)| u64::from(w) * u64::from(h) > u64::from(bw) * u64::from(bh))
        {
            best = Some((w, h));
        }
    }
    if let Some(full) = best {
        return shell(full);
    }
    let exif = t.exif()?;
    let full = (exif.u32(PIXEL_X_DIMENSION)?, exif.u32(PIXEL_Y_DIMENSION)?);
    if !much_bigger(full) {
        return None;
    }
    // bytes covered by image data (strips and tiles shared between IFDs, e.g. IFD0 and its thumbnail, count once)
    let mut seen = std::collections::BTreeMap::new();
    for ifd in t.all_ifds() {
        if let Ok(info) = ifd.image() {
            for c in info.chunks(len as u64) {
                seen.insert(c.offset, c.len);
            }
        }
    }
    let covered = seen.values().fold(0u64, |a, &l| a.saturating_add(l));
    let spare = (len as u64).saturating_sub(covered);
    (spare.saturating_mul(8) >= u64::from(full.0) * u64::from(full.1)).then_some(())?;
    shell(full)
}

/// Why [`decode`] gives up on a file [`probe`] called [`RawFormat::OtherTiff`].
fn other_tiff_reason(bytes: &[u8]) -> String {
    let t = Tiff::parse_with(bytes, &craft_tiff::ParseOptions { max_ifds: 256, ..Default::default() }).ok();
    // a DNG-style CFA IFD whose lossless JPEG the lossless decoder rejects: say why
    #[cfg(feature = "dng")]
    if let Some(t) = &t
        && let Some(info) = dng::raw_ifd(t).and_then(|i| i.image().ok())
        && info.compression == craft_tiff::tags::compression::JPEG
        && let Some(src) = info.chunks(bytes.len() as u64).first().and_then(|c| craft_tiff::image::chunk_bytes(bytes, c))
        && let Err(e) = ljpeg::frame_info(src)
    {
        return format!("raw image coded as lossless JPEG that is not decoded yet ({e})");
    }
    match t.as_ref().filter(|t| !has_raw_ifd(t)).and_then(|t| thumbnail_shell(t, bytes.len())) {
        Some(s) => format!(
            "{}x{} raw image in a private block, not decoded yet (the file's first image is a {}x{} reduced copy)",
            s.full.0, s.full.1, s.thumb.0, s.thumb.1
        ),
        None => "OtherTiff files are not decoded yet".to_string(),
    }
}

/// Decode a raw file with the default [`Limits`].
pub fn decode(bytes: &[u8]) -> Result<RawImage> {
    decode_with(bytes, &Limits::default())
}

/// Decode a raw file within `limits`.
pub fn decode_with(bytes: &[u8], limits: &Limits) -> Result<RawImage> {
    decode_in(bytes, Mode::Full, limits)
}

/// [`decode_with`] at a smaller size, for a picture whose default crop needs a long edge of only `min_long` px: a
/// DNG that keeps reduced-resolution copies of its LinearRaw image (HDR and panorama merges keep 256 to 2048 px
/// ones) decodes the smallest that covers it. The image has the full one's levels and colour; its active area and
/// crop are scaled to its size. `Ok(None)` when the file keeps no such copy: decode it whole.
#[cfg(feature = "dng")]
pub fn decode_reduced(bytes: &[u8], min_long: usize, limits: &Limits) -> Result<Option<RawImage>> {
    if probe(bytes) != Some(RawFormat::Dng) {
        return Ok(None);
    }
    dng::decode_reduced(bytes, min_long, limits)
}

/// Everything [`decode`] learns about a raw file except its samples: geometry, orientation, CFA, colour data,
/// as-shot white balance, opcode lists (embedded lens corrections) and the camera, read from the headers without
/// decompressing the pixel data, for imports. Equal to [`RawImage::info`] of the decoded image. Black and white
/// levels are not included (some formats measure them from the samples).
pub fn probe_info(bytes: &[u8]) -> Result<RawInfo> {
    probe_info_with(bytes, &Limits::default())
}

/// [`probe_info`] within `limits`: a file that declares more than they allow fails as [`decode_with`] would.
pub fn probe_info_with(bytes: &[u8], limits: &Limits) -> Result<RawInfo> {
    decode_in(bytes, Mode::Header, limits).map(RawImage::into_info)
}

/// How much of a file a decoder reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Headers and samples.
    Full,
    /// Headers only: the returned image has no samples, and black/white levels measured from
    /// the samples are placeholders. Only [`RawImage::into_info`] may look at it.
    Header,
}

#[cfg_attr(not(any(feature = "dng", feature = "crw")), allow(unused_variables))]
fn decode_in(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    let mut raw = decode_format(bytes, mode, limits)?;
    if mode == Mode::Header {
        // the samples weren't read, so no source a decoder reported for a measured level holds
        raw.level_sources = LevelSources::default();
    }
    Ok(raw)
}

fn decode_format(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    // (unused when every format feature is off)
    let _ = (mode, limits);
    match probe(bytes).ok_or(RawError::NotRaw)? {
        #[cfg(feature = "dng")]
        RawFormat::Dng => dng::decode(bytes, mode, limits),
        #[cfg(feature = "crw")]
        RawFormat::Crw => crw::decode(bytes, mode, limits),
        #[cfg(feature = "hasselblad")]
        RawFormat::CfaTiff => hasselblad::decode(bytes, mode, limits),
        #[cfg(feature = "kodak")]
        RawFormat::KodakDcs => kodak::decode(bytes, mode, limits),
        #[cfg(feature = "mrw")]
        RawFormat::Mrw => mrw::decode(bytes, mode, limits),
        #[cfg(feature = "erf")]
        RawFormat::Erf => erf::decode(bytes, mode, limits),
        #[cfg(feature = "mef")]
        RawFormat::Mef => mef::decode(bytes, mode, limits),
        #[cfg(feature = "cr2")]
        RawFormat::Cr2 => cr2::decode(bytes, mode, limits),
        #[cfg(feature = "nef")]
        RawFormat::Nef | RawFormat::Nrw => nef::decode(bytes, mode, limits),
        #[cfg(feature = "rw2")]
        RawFormat::Rw2 => rw2::decode(bytes, mode, limits),
        #[cfg(feature = "raf")]
        RawFormat::Raf => raf::decode(bytes, mode, limits),
        #[cfg(feature = "orf")]
        RawFormat::Orf => orf::decode(bytes, mode, limits),
        #[cfg(feature = "arw")]
        RawFormat::Arw => arw::decode(bytes, mode, limits),
        #[cfg(feature = "cr3")]
        RawFormat::Cr3 => cr3::decode(bytes, mode, limits),
        #[cfg(feature = "pef")]
        RawFormat::Pef => pef::decode(bytes, mode, limits),
        #[cfg(feature = "srw")]
        RawFormat::Srw => srw::decode(bytes, mode, limits),
        RawFormat::OtherTiff => Err(RawError::Unsupported(other_tiff_reason(bytes))),
        other => Err(RawError::Unsupported(format!("{other:?} files are not decoded by this build"))),
    }
}

/// A raw file's description without its samples (see [`probe_info`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawInfo {
    pub format: RawFormat,
    /// Full sensor data dimensions (including masked borders).
    pub width: usize,
    pub height: usize,
    pub cpp: usize,
    pub cfa: Option<Cfa>,
    pub bits: u32,
    pub active_area: Rect,
    /// Default crop relative to the active area.
    pub crop: Rect,
    pub orientation: Orientation,
    pub color: ColorData,
    pub wb_multipliers: Option<[f32; 3]>,
    pub opcodes: OpcodeLists,
    pub camera: Camera,
    /// CinemaDNG frame tags (empty for a still).
    #[serde(default)]
    pub cinema: CinemaTags,
    /// Lens-correction tables the file records, decoded but not necessarily applied (see [`lens_metadata`]); only
    /// validated components are in `opcodes`.
    #[serde(default)]
    pub lens_metadata: LensMetadata,
    /// See [`RawImage::pixel_aspect`].
    #[serde(default = "square_pixels")]
    pub pixel_aspect: f32,
}

fn square_pixels() -> f32 {
    1.0
}

/// Rows of a picture `height` rows tall once its pixels (taller than wide by `aspect`) are made square.
pub fn stretched_height(height: usize, aspect: f32) -> usize {
    if (aspect - 1.0).abs() < 0.01 { height } else { ((height as f32 * aspect).round() as usize).max(1) }
}

impl RawInfo {
    /// Size of the developed image before orientation: the default crop, else the active area, with the rows
    /// stretched for non-square pixels.
    pub fn developed_size(&self) -> (usize, usize) {
        let c = self.crop.clipped(self.active_area.width, self.active_area.height);
        let (w, h) = if c.width > 1 && c.height > 1 { (c.width, c.height) } else { (self.active_area.width, self.active_area.height) };
        (w, stretched_height(h, self.pixel_aspect))
    }
}

/// Sensor samples.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RawData {
    U16(Vec<u16>),
    /// Floating-point DNGs (already linear, typically white = 1.0).
    F32(Vec<f32>),
}

impl RawData {
    pub fn len(&self) -> usize {
        match self {
            RawData::U16(v) => v.len(),
            RawData::F32(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[inline]
    pub fn get(&self, i: usize) -> f32 {
        match self {
            RawData::U16(v) => v[i] as f32,
            RawData::F32(v) => v[i],
        }
    }
}

/// Colour filter array pattern. Colour indices: 0 = red, 1 = green, 2 = blue.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cfa {
    pub width: usize,
    pub height: usize,
    /// `width × height` colour indices, row-major, anchored at image pixel (0, 0).
    pub pattern: Vec<u8>,
}

impl Cfa {
    /// A 2×2 Bayer pattern from a string such as `"RGGB"`.
    pub fn bayer(s: &str) -> Option<Cfa> {
        let p: Vec<u8> = s
            .chars()
            .map(|c| {
                Some(match c {
                    'R' => 0,
                    'G' => 1,
                    'B' => 2,
                    _ => return None,
                })
            })
            .collect::<Option<_>>()?;
        (p.len() == 4).then_some(Cfa { width: 2, height: 2, pattern: p })
    }
    /// A 2×2 Bayer layout named in code (`"RGGB"`, `"BGGR"`, `"GRBG"`, `"GBRG"`). Only for literal
    /// names (tested below); anything else falls back to RGGB instead of failing.
    #[cfg_attr(not(feature = "crw"), allow(dead_code))]
    pub(crate) fn bayer_static(s: &'static str) -> Cfa {
        Cfa::bayer(s).unwrap_or(Cfa { width: 2, height: 2, pattern: vec![0, 1, 1, 2] })
    }
    /// The Fujifilm X-Trans 6×6 layout (as commonly documented), anchored at (0, 0).
    pub fn xtrans() -> Cfa {
        let rows = ["GGRGGB", "GGBGGR", "BRGRBG", "GGBGGR", "GGRGGB", "RBGBRG"];
        let pattern = rows
            .iter()
            .flat_map(|r| {
                r.chars().map(|c| match c {
                    'R' => 0,
                    'G' => 1,
                    _ => 2,
                })
            })
            .collect();
        Cfa { width: 6, height: 6, pattern }
    }
    #[inline]
    pub fn color_at(&self, x: usize, y: usize) -> u8 {
        self.pattern[(y % self.height) * self.width + (x % self.width)]
    }
    /// The same pattern anchored at `(dx, dy)` of the current anchor (e.g. after cropping to an active area).
    pub fn shifted(&self, dx: usize, dy: usize) -> Cfa {
        let pattern = (0..self.height).flat_map(|y| (0..self.width).map(move |x| (x, y))).map(|(x, y)| self.color_at(x + dx, y + dy)).collect();
        Cfa { width: self.width, height: self.height, pattern }
    }
    pub fn is_bayer(&self) -> bool {
        if self.width != 2 || self.height != 2 {
            return false;
        }
        let mut c = [0; 3];
        for &p in &self.pattern {
            if p > 2 {
                return false;
            }
            c[p as usize] += 1;
        }
        c == [1, 2, 1]
    }
    /// Pattern name like `"RGGB"` for 2×2 patterns.
    pub fn name(&self) -> String {
        self.pattern.iter().map(|&c| ['R', 'G', 'B', '?'][c.min(3) as usize]).collect()
    }
    fn valid(&self) -> bool {
        self.width > 0 && self.height > 0 && self.width <= 16 && self.height <= 16 && self.pattern.len() == self.width * self.height
    }
}

/// Integer rectangle (pixels).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    pub fn new(x: usize, y: usize, width: usize, height: usize) -> Rect {
        Rect { x, y, width, height }
    }
    /// Clip to a `w × h` area.
    pub fn clipped(self, w: usize, h: usize) -> Rect {
        let x = self.x.min(w);
        let y = self.y.min(h);
        Rect { x, y, width: self.width.min(w - x), height: self.height.min(h - y) }
    }
}

/// Black level model (DNG `BlackLevelRepeatDim`, `BlackLevel`, `BlackLevelDeltaH/V`). The repeat pattern and
/// the deltas are anchored at the active area's top-left corner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlackLevel {
    pub repeat_rows: usize,
    pub repeat_cols: usize,
    /// `repeat_rows × repeat_cols × cpp` values.
    pub values: Vec<f32>,
    /// Per active-area column.
    pub delta_h: Vec<f32>,
    /// Per active-area row.
    pub delta_v: Vec<f32>,
}

impl Default for BlackLevel {
    fn default() -> Self {
        BlackLevel { repeat_rows: 1, repeat_cols: 1, values: vec![0.0], delta_h: vec![], delta_v: vec![] }
    }
}

impl BlackLevel {
    pub fn uniform(v: f32) -> BlackLevel {
        BlackLevel { values: vec![v], ..Default::default() }
    }
    /// Black level at active-area coordinates (x, y) for sample `s` of `cpp`.
    #[inline]
    pub fn at(&self, x: usize, y: usize, s: usize, cpp: usize) -> f32 {
        let (r, c) = (self.repeat_rows.max(1), self.repeat_cols.max(1));
        let idx = ((y % r) * c + (x % c)) * cpp + s;
        let base = self.values.get(idx).or_else(|| self.values.first()).copied().unwrap_or(0.0);
        base + self.delta_h.get(x).copied().unwrap_or(0.0) + self.delta_v.get(y).copied().unwrap_or(0.0)
    }
    /// Mean of the repeat-pattern values (for display / heuristics).
    pub fn mean(&self) -> f32 {
        if self.values.is_empty() { 0.0 } else { self.values.iter().sum::<f32>() / self.values.len() as f32 }
    }
}

/// Where a black or white level came from. Rendering ignores it; code that compares or stacks frames can refuse
/// a [`LevelSource::Fallback`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum LevelSource {
    /// The decoder doesn't say (every decoder that hasn't been annotated yet).
    #[default]
    Unknown,
    /// Stated by the file: a tag, a maker note or a table.
    Metadata,
    /// Measured from the file's masked (optical black) samples.
    MaskedPixels,
    /// Inferred from the picture samples, e.g. a saturation plateau.
    Inferred,
    /// Neither read from the file nor measured from it: a default (0 or a format constant for black, the full bit
    /// range or a format constant for white). Not necessarily wrong: a frame with no clipped highlights has no
    /// saturation plateau to find.
    Fallback,
}

/// The [`LevelSource`] of a [`RawImage`]'s black and white levels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LevelSources {
    /// Where [`RawImage::black`] came from.
    pub black: LevelSource,
    /// Where [`RawImage::white`] came from.
    pub white: LevelSource,
}

/// DNG colour tags (DNG 1.7 chapter 6). Matrices are 3×3 (three-colour cameras).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ColorData {
    /// `CalibrationIlluminant1/2` (Exif LightSource codes; 0 = unknown).
    pub illuminant: [u16; 2],
    /// `ColorMatrix1/2`: XYZ → reference camera.
    pub color_matrix: [Option<Matrix3>; 2],
    /// `ForwardMatrix1/2`: white-balanced camera → XYZ D50.
    pub forward_matrix: [Option<Matrix3>; 2],
    /// `CameraCalibration1/2`.
    pub camera_calibration: [Option<Matrix3>; 2],
    pub analog_balance: Option<[f64; 3]>,
    pub as_shot_neutral: Option<[f64; 3]>,
    pub as_shot_white_xy: Option<[f64; 2]>,
    /// EV to add for a "normal" rendering (`BaselineExposure` + `BaselineExposureOffset`).
    /// A full CR3 decode also includes compensation for sample-derived white headroom;
    /// a header-only probe cannot determine that adjustment.
    pub baseline_exposure: f64,
    /// `BaselineSharpness`: sharpening relative to a reference camera (`None`: the DNG default,
    /// 1). Read and written back by DNG export.
    #[serde(default)]
    pub baseline_sharpness: Option<f64>,
    /// The file's own camera-profile look (`ProfileHueSatMap*`, `ProfileLookTable*`, `ProfileToneCurve`) and its
    /// `ProfileGainTableMap*`, as data: the develop pipeline applies them, and a DNG export writes them back.
    #[serde(default)]
    pub profile: profile::ProfileLook,
    /// The camera's white-balance presets at stated temperatures (Canon CR2), which give a raw without colour
    /// matrices a Kelvin scale ([`kelvin`]). Not a DNG tag: the DNG writer does not write it.
    #[serde(default)]
    pub kelvin_presets: Option<kelvin::KelvinPresets>,
}

/// A decoded raw image.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawImage {
    pub format: RawFormat,
    /// Full sensor data dimensions (including masked borders).
    pub width: usize,
    pub height: usize,
    /// Samples per pixel: 1 = CFA or monochrome, 3 = linear RGB (LinearRaw / demosaiced DNG).
    pub cpp: usize,
    pub data: RawData,
    /// CFA pattern anchored at pixel (0, 0) of the full data; `None` for linear / monochrome data.
    pub cfa: Option<Cfa>,
    /// Significant bits of the stored samples (informational).
    pub bits: u32,
    pub black: BlackLevel,
    /// White (clip) level per sample (after linearization).
    pub white: Vec<f32>,
    /// Area of valid image data within the full data.
    pub active_area: Rect,
    /// Default crop relative to the active area.
    pub crop: Rect,
    pub orientation: Orientation,
    pub color: ColorData,
    /// As-shot white-balance multipliers from vendor maker notes (green = 1), when known (non-DNG).
    pub wb_multipliers: Option<[f32; 3]>,
    /// Whether a `LinearizationTable` was applied to `data`.
    pub linearized: bool,
    /// Height of a sensor pixel over its width (1.0 for square pixels). The D1X and D1H write 2.0: their frame is
    /// 4028 x 1324 and the picture is 3:2 only once the rows are doubled. A developer stretches the rows by
    /// this factor.
    pub pixel_aspect: f32,
    pub opcodes: OpcodeLists,
    pub camera: Camera,
    /// CinemaDNG frame tags (empty for a still).
    #[serde(default)]
    pub cinema: CinemaTags,
    /// Lens-correction tables the file records, decoded but not necessarily applied (see [`lens_metadata`]); only
    /// validated components are in `opcodes`.
    #[serde(default)]
    pub lens_metadata: LensMetadata,
    /// Where `black` and `white` came from. Both [`LevelSource::Unknown`] after a headers-only decode, whose
    /// measured levels are placeholders.
    #[serde(default)]
    pub level_sources: LevelSources,
}

impl RawImage {
    /// The description without the samples (what [`probe_info`] returns for the same file).
    pub fn info(&self) -> RawInfo {
        RawInfo {
            format: self.format,
            width: self.width,
            height: self.height,
            cpp: self.cpp,
            cfa: self.cfa.clone(),
            bits: self.bits,
            active_area: self.active_area,
            crop: self.crop,
            orientation: self.orientation,
            color: self.color.clone(),
            wb_multipliers: self.wb_multipliers,
            opcodes: self.opcodes.clone(),
            camera: self.camera.clone(),
            cinema: self.cinema.clone(),
            lens_metadata: self.lens_metadata.clone(),
            pixel_aspect: self.pixel_aspect,
        }
    }

    /// [`Self::info`], dropping the samples.
    pub fn into_info(self) -> RawInfo {
        RawInfo {
            format: self.format,
            width: self.width,
            height: self.height,
            cpp: self.cpp,
            cfa: self.cfa,
            bits: self.bits,
            active_area: self.active_area,
            crop: self.crop,
            orientation: self.orientation,
            color: self.color,
            wb_multipliers: self.wb_multipliers,
            opcodes: self.opcodes,
            camera: self.camera,
            cinema: self.cinema,
            lens_metadata: self.lens_metadata,
            pixel_aspect: self.pixel_aspect,
        }
    }

    /// Validate what a decoder read in `mode` ([`Mode::Header`]: everything but the samples).
    #[cfg_attr(not(any(feature = "dng", feature = "crw")), allow(dead_code))]
    pub(crate) fn validate_for(&self, mode: Mode) -> Result<()> {
        self.check(mode == Mode::Full)
    }

    /// Validate internal consistency (dimensions vs data length, CFA shape, rectangles).
    pub fn validate(&self) -> Result<()> {
        self.check(true)
    }

    fn check(&self, samples: bool) -> Result<()> {
        if self.width == 0 || self.height == 0 || !(1..=4).contains(&self.cpp) {
            return Err(RawError::Corrupt("bad dimensions".into()));
        }
        // two samples a pixel are neither one colour nor RGB: nothing downstream can develop them (storytold/lightcraft#1511)
        if self.cpp == 2 {
            return Err(RawError::Unsupported("2 samples per pixel".into()));
        }
        if samples {
            let expected = crate::checked_pixel_count(self.width as u64, self.height as u64)
                .and_then(|n| n.checked_mul(self.cpp as u64))
                .ok_or_else(|| RawError::Corrupt("data length mismatch".into()))?;
            let actual = u64::try_from(self.data.len()).map_err(|_| RawError::Corrupt("data length mismatch".into()))?;
            if actual != expected {
                return Err(RawError::Corrupt("data length mismatch".into()));
            }
        }
        if let Some(c) = &self.cfa
            && (!c.valid() || c.pattern.iter().any(|&p| p > 2))
        {
            return Err(RawError::Unsupported(format!("CFA pattern {}", c.name())));
        }
        let a = self.active_area;
        // checked: a crop origin from a maker note can be `u64::MAX`, and `x + width` would wrap
        // back inside the image here and again in `normalized`'s index arithmetic.
        let fits = |x: usize, len: usize, limit: usize| x.checked_add(len).is_some_and(|end| end <= limit);
        if a.width == 0 || a.height == 0 || !fits(a.x, a.width, self.width) || !fits(a.y, a.height, self.height) {
            return Err(RawError::Corrupt("active area outside image".into()));
        }
        Ok(())
    }

    /// White level for sample `s`.
    pub fn white_at(&self, s: usize) -> f32 {
        self.white.get(s).or_else(|| self.white.first()).copied().unwrap_or(65535.0)
    }
}

#[cfg(test)]
#[cfg(feature = "dng")]
mod tests;

#[cfg(test)]
mod checked_arithmetic_tests {
    use super::*;

    #[test]
    fn checked_pixel_count_wrap_inputs_rejected() {
        // 65536 × 65536 = 2^32 wraps to 0 on wasm32 usize; u64 math keeps it exact.
        assert_eq!(checked_pixel_count(65536, 65536), None);
        assert_eq!(checked_pixel_count(65536, 65537), None);
        // u32::MAX × 2 wraps on 32-bit usize.
        assert_eq!(checked_pixel_count(u64::from(u32::MAX), 2), None);
        // inputs wider than a 32-bit usize reach the gate untruncated (2^40 × 2^40 overflows u64).
        assert_eq!(checked_pixel_count(1 << 40, 1 << 40), None);
    }

    #[test]
    fn checked_pixel_count_cap_and_zero() {
        assert_eq!(checked_pixel_count((1 << 30) + 1, 1), None);
        assert_eq!(checked_pixel_count(1 << 15, 1 << 15), Some(1 << 30));
        assert_eq!(checked_pixel_count(100, 100), Some(10_000));
        assert_eq!(checked_pixel_count(0, 100), None);
    }

    #[test]
    fn orientation_codes_round_trip() {
        for code in 1..=8u16 {
            assert_eq!(Orientation::from_exif(code).to_exif(), code);
        }
        assert_eq!(Orientation::from_exif(0), Orientation::Normal);
        assert_eq!(Orientation::from_exif(9), Orientation::Normal);
    }
}
