//! Lens-correction data a raw file records, kept with where it came from and separately from what LightCraft
//! applies.
//!
//! A readable table is not a correction. Each component carries a [`SonyLensStatus`]: only
//! [`SonyLensStatus::Applied`] components were converted into the image's `OpcodeList3`, and that is decided by
//! independent validation per model, coding and component (see `docs/sony-lens-corrections.md`), never by the mere
//! presence of a table. Everything else here is metadata for diagnostics and later validation work.

use crate::Rect;
use serde::{Deserialize, Serialize};

/// Vendor lens-correction metadata of a raw file (see the module docs). Empty for formats that record none, or
/// whose tables are read straight into opcodes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LensMetadata {
    /// Sony ARW: the raw image IFD's shading, lateral chromatic aberration and distortion records, with their
    /// encrypted `SR2SubIFD` copies. `None` when neither directory holds any of them.
    #[serde(default)]
    pub sony: Option<SonyLensCorrections>,
}

/// Sony's in-file lens-correction record: three independent components, each a camera setting and a radial table.
///
/// Tag identities and storage types from the ExifTool EXIF / Sony tag-name documentation: raw-IFD `0x7031` / `0x7032`
/// (shading, "VignettingCorrection" / "VignettingCorrParams"), `0x7034` / `0x7035` (lateral chromatic aberration),
/// `0x7036` / `0x7037` (distortion), and the `SR2SubIFD` tables `0x797d`, `0x7980`, `0x7982`. The `SR2SubIFD` setting
/// tags `0x797c`, `0x797f`, `0x7981` are unnamed there; they sit next to the tables and equal the raw-IFD settings
/// in every file inspected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SonyLensCorrections {
    /// Shading (vignetting): 16 radial samples whose gain function, radius convention and processing stage are not
    /// established. Opaque signed data.
    pub shading: SonyComponent<[i16; 16]>,
    /// Lateral chromatic aberration: two 16-sample radial curves. A public analysis reads them as red, then blue,
    /// relative to green; that is not validated here.
    pub chromatic_aberration: SonyComponent<[[i16; 16]; 2]>,
    /// Distortion: 16 radial samples (see `docs/sony-lens-corrections.md` for the validated interpretation).
    pub distortion: SonyComponent<[i16; 16]>,
    /// Whether the encrypted `SR2SubIFD` was found and decrypted. `false`: no copies to fall back on or compare.
    pub sr2_read: bool,
    /// The framing and coding the tables have to be interpreted against.
    pub geometry: SonyLensGeometry,
}

/// One correction component: the camera's setting, its table, and whether LightCraft applies it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SonyComponent<T> {
    pub setting: SonyField<SonySetting>,
    /// The table's samples after the leading sample count, which must equal their number.
    pub table: SonyField<T>,
    pub status: SonyLensStatus,
}

/// One field read from the raw IFD and from the `SR2SubIFD`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SonyField<T> {
    /// The value in use: the raw IFD's when it holds a well-formed one, else the `SR2SubIFD`'s when the raw IFD
    /// lacks the tag. `None` when no copy is usable, and when the raw IFD's copy is malformed: a corrupt field is
    /// reported, not silently replaced by the other copy. The raw IFD's copy is the one the TIFF reader that decodes
    /// the image parsed (its first occurrence, within the reader's limits).
    pub value: Option<T>,
    pub source: Option<SonySource>,
    pub raw_ifd: SonyCopy,
    pub sr2: SonyCopy,
    /// Whether the two copies are equal, when both are well-formed. The raw IFD's wins when they differ.
    pub copies_agree: Option<bool>,
    /// Diagnostics: the tag occurs more than once in the raw IFD / the `SR2SubIFD`. Only the first occurrence is used,
    /// as the TIFF reader does (the raw IFD's: the first one whose values it could read).
    #[serde(default)]
    pub raw_ifd_repeated: bool,
    #[serde(default)]
    pub sr2_repeated: bool,
}

/// Where a value was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SonySource {
    /// The plain raw image IFD.
    RawIfd,
    /// The encrypted `SR2SubIFD`.
    Sr2SubIfd,
}

/// How one copy of a field was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SonyCopy {
    /// Not in that directory (or the `SR2SubIFD` could not be read).
    Absent,
    /// Present with exactly the supported type and layout.
    Valid,
    /// Present, but refused.
    Rejected(SonyFieldError),
}

/// Why a present field was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SonyFieldError {
    /// Another TIFF field type than SSHORT (8) for a table, or SHORT (3) / SSHORT for a setting.
    FieldType(u16),
    /// Another number of values than the supported layout's: 17 (shading, distortion), 33 (CA) or 1 (setting).
    Count(u64),
    /// The leading sample count is not the layout's 16 (32 for CA). Older bodies store 11 (22) samples padded to the
    /// same length, a layout that is not interpreted; a count of 0 goes with an "unavailable" setting.
    CountPrefix(i16),
    /// The values lie outside the file (raw IFD) or the decrypted block (`SR2SubIFD`).
    OutsideData,
    /// In the raw IFD but past what the TIFF reader that decodes the image reads (8,192 entries per directory, a
    /// total value budget), so the image doesn't use it either.
    PastReaderLimits,
    /// A setting stored as a negative SSHORT.
    NegativeSetting(i16),
}

/// A camera correction setting: the stored number and its documented meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SonySetting {
    pub value: u16,
    pub meaning: SonySettingMeaning,
}

/// The documented meaning of a setting value (ExifTool EXIF tag-name documentation). Values are compared whole;
/// they are not read as bit masks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SonySettingMeaning {
    /// Shading 256, CA 0, distortion 0. The table is still recorded: Off does not mean absent.
    Off,
    /// Shading 257 or 272, CA 1, distortion 1.
    Auto,
    /// Distortion 17: Auto, fixed by the lens.
    ForcedAuto,
    /// "No correction params available": shading 511, CA 255, distortion 255.
    Unavailable,
    /// Any other value (e.g. a shading "Low" setting, whose number is not documented). Never treated as Off.
    Unknown(u16),
}

/// Whether a component is applied, and if not, why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SonyLensStatus {
    /// Converted into the image's `OpcodeList3` (ILCE-7RM4A, ILCE-7RM6 or eligible full-frame ILCE-7CR Bayer distortion).
    Applied,
    /// No usable table.
    NoTable,
    /// Decoded only: no independent validation covers this component's rendering semantics on any body (shading,
    /// lateral CA).
    NotValidatedComponent,
    /// Decoded only: the component is validated on other camera models, not on this one.
    NotValidatedModel,
    /// Decoded only: the model is validated, but not this coding or framing (linear YCbCr, a non-3:2 crop) or a
    /// table found only in the `SR2SubIFD`.
    NotValidatedMode,
    /// A7CR distortion is Off, unavailable, missing or unknown. Its uncorrected JPEG and render keep the same geometry.
    CameraCorrectionDisabled,
    /// Validated scope, but the table is all zeros: a valid no-op, nothing to apply.
    Neutral,
    /// Validated scope, but the conversion refused this table (values outside the validated range, a degenerate
    /// crop, or a fitted map that misses the table or folds).
    Rejected,
}

/// The framing and coding evidence for interpreting the tables, all in raw-frame pixels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SonyLensGeometry {
    /// The stored raw frame, including any padding (lossless L files are padded).
    pub storage_width: usize,
    pub storage_height: usize,
    /// Sony's crop tags (`0x74c7` / `0x74c8`) as recorded.
    pub sony_crop: Option<Rect>,
    /// The raw IFD's `DefaultCropOrigin` / `DefaultCropSize` as recorded.
    pub default_crop: Option<Rect>,
    /// The image's crop (the decoder's precedence: Sony's tags, then the default crop, then the recorded size).
    pub image_crop: Rect,
    /// The rectangle the validated distortion conversion is centred on and normalised to: the default crop when
    /// the file has one, else the image crop. Clipped to the frame.
    pub reference: Rect,
    /// TIFF compression of the raw image (1 uncompressed, 7 lossless, 32767 ARW2, 32766 ARW6).
    pub compression: u16,
    /// Downsized lossless M/S data (linear YCbCr, not sensor CFA).
    pub linear_ycbcr: bool,
    /// Both the image crop and the reference are 3:2 within 0.5 %, the only framing validated.
    pub native_3_2: bool,
}

impl<T> SonyField<T> {
    /// Whether either directory holds the tag (well-formed or not).
    pub fn is_present(&self) -> bool {
        self.raw_ifd != SonyCopy::Absent || self.sr2 != SonyCopy::Absent
    }
}
