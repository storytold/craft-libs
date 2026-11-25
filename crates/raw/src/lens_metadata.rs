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
    /// Separate Sony MakerNote camera-menu settings. These can exist without lens tables and are not
    /// declarations that the corresponding corrections have changed stored RAW pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sony_camera_settings: Option<SonyCameraSettings>,
    /// Candidate capture-gain data from a measured Sony MakerNote layout. Diagnostic only: its
    /// presence is not a declaration that every supported storage mode has applied these gains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sony_capture_gains: Option<SonyCaptureGains>,
    /// Older signature-selected Sony capture-gain layout, kept in its original Q12 units. Its adjacent
    /// parameters are not the newer record's radius/center fields. Diagnostic only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sony_capture_gains_q12: Option<SonyCaptureGainsQ12>,
    /// Three 32-knot curves and the native optical slot in the newer 0x9416 layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sony_capture_gains_q11: Option<Box<SonyCaptureGainsQ11>>,
}

impl LensMetadata {
    /// Whether the 16-word Sony shading table restates the recorded green
    /// capture curve as Q14 attenuation. Pixel measurements still establish
    /// when the capture gain has affected the samples.
    pub fn sony_shading_matches_capture_gain(&self) -> bool {
        let Some(shading) = self.sony.as_ref().map(|sony| &sony.shading) else { return false };
        let Some(words) = shading.table.value.filter(|_| shading.samples_per_channel == Some(16)) else { return false };
        let matches = |gains: [u32; 32], unity: u32| {
            words
                .into_iter()
                .zip(gains.into_iter().step_by(2))
                .all(|(word, gain)| gain >= unity && i64::from(word) == 16384 * i64::from(gain - unity) / i64::from(gain))
        };
        self.sony_capture_gains_q12.as_ref().is_some_and(|g| matches(g.gains_q12[1].map(u32::from), 4096))
            || self.sony_capture_gains.as_ref().is_some_and(|g| matches(g.gains_q16[1], 65536))
    }
}

/// The 4416-byte 0x9416 Q11 layouts with signatures `[8, 128, 90, 13]` and `[9, 128, 98, 13]`.
/// The recorded plane order is preserved. Unity is 2048.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SonyCaptureGainsQ11 {
    pub gains_q11: [[u16; 32]; 3],
    /// Shared sensor-coordinate center in the measured captures.
    pub center: [u16; 2],
    pub native_shading: SonyNativeShading,
}

/// Stored values from measured `0x9405` Q12 layouts. The source layout stays explicit because
/// the adjacent parameters need not have the same interpretation across record versions.
/// A record alone cannot establish the capture or optical coordinates. See [`crate::sony_shading`] for the measured scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SonyCaptureGainsQ12 {
    /// Older saved records omit this field and use the original 2300-byte layout.
    #[serde(default, skip_serializing_if = "SonyCaptureGainsQ12Layout::is_standard")]
    pub layout: SonyCaptureGainsQ12Layout,
    /// Original ARW sample precision. A lossless DNG may store 12-bit or 14-bit source samples in
    /// 16-bit words. Older DNG archives omit this evidence and cannot select a precision-specific map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_bits: Option<u32>,
    /// Four 32-knot arrays; 4096 denotes unity. Preserve the recorded plane order,
    /// or copy the shared sequence to each plane for the single-sequence layout.
    /// The measured RX1 Off descriptor leaves all phases at unity.
    pub gains_q12: [[u16; 32]; 4],
    /// Eight adjacent four-word groups, including unknown/sentinel values.
    /// The single-sequence layout has no such groups and leaves these zero.
    pub parameters: [[u16; 4]; 8],
    /// Sixteen signed native attenuation words, distinct from the capture gains.
    /// Some 2048-byte compact-camera records mark this slot absent and contain unrelated
    /// bytes there. Older archived arrays deserialize as `Some` without a format migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_shading: Option<[i16; 16]>,
    /// Three layout-specific descriptor bytes, preserved without assigning processing-history semantics.
    pub native_flags: [u8; 3],
}

/// Measured Q12 record layout; this does not select a rendering formula.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SonyCaptureGainsQ12Layout {
    /// 2300 bytes, signature 0xa3; gains/parameters/native words at 440/708/860.
    #[default]
    Standard2300,
    /// 1992 bytes, signature 0x88 and native-table discriminator at 840;
    /// gains/parameters/native words at 452/720/842.
    Legacy1992,
    /// 2048 bytes with a measured revision (0x89, 0x8a, 0x8b, 0x8e, 0x90 or 0x91).
    /// Revision-specific offsets and the native-slot discriminator remain explicit.
    Legacy2048 { revision: u8 },
    /// Early revisions 3 and 5, with one shared 32-word sequence at byte 1546.
    LegacySingle { bytes: u16 },
}

impl SonyCaptureGainsQ12Layout {
    fn is_standard(&self) -> bool {
        *self == Self::Standard2300
    }
}

/// Four-plane record from four observed signatures of the decoded 4416-byte `0x9416` block.
/// Layout selection follows the signature, not the camera name or lens age.
/// These are stored values, not a rendering prescription.
/// The A1 II wall series supports Q16 gains; radial spacing, channel calibration and other modes
/// still need validation. Do not apply or remove them solely because this record is present.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SonyCaptureGains {
    /// Four 32-knot arrays; 65536 is unity in the measured A1 II captures. Plane order is preserved.
    pub gains_q16: [[u32; 32]; 4],
    /// Four adjacent values, preserved without assigning an unverified radius formula.
    pub radius_parameters: [u16; 4],
    /// Per-plane [x, y] values matching native sensor crop centers in the measured files.
    /// They need not be coordinates of a resized/lossless-M/S decoded image.
    pub centers: [[u16; 2]; 4],
    /// Native attenuation copy and format marker from the same measured MakerNote block.
    /// Absent in older serialized records; its presence alone does not authorize correction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_shading: Option<SonyNativeShading>,
}

/// Stored native lens-table slot from measured 4416-byte `0x9416` layouts.
/// Older layouts store only 16 words before their marker; newer layouts store 32 plus an endpoint.
/// In measured A1 II files, [1,1] accompanies 16 values and stored zero padding; [2,2] accompanies
/// an interleaved 33-value curve. Other markers need not identify a usable copy (RX1R III's
/// [1,0] slot is zero despite a nonzero raw table). Retain them without guessing their meaning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SonyNativeShading {
    /// Number of words physically stored in `values`, excluding the endpoint: 16 or 32.
    /// Missing in older LightCraft archives, which supported only the 32-word layout.
    #[serde(default = "native_shading_samples", skip_serializing_if = "native_shading_full")]
    pub stored_samples: u8,
    /// Unstored tail words are normalized to zero; consult `stored_samples` before interpreting.
    pub values: [i16; 32],
    /// Zero when absent from the 16-word layout.
    pub endpoint: i16,
    pub marker: [u8; 2],
}

fn native_shading_samples() -> u8 {
    32
}

fn native_shading_full(samples: &u8) -> bool {
    *samples == 32
}

/// Capture settings from Sony MakerNote LONG[1] tags 0x2011, 0x2012 and 0x2013.
/// Kept separately from raw/SR2 table settings: the A1 II wall pair records raw shading 272 in both
/// Off and Auto captures. Do not infer these settings from the presence or contents of a table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SonyCameraSettings {
    pub shading: SonyCameraSetting,
    pub chromatic_aberration: SonyCameraSetting,
    pub distortion: SonyCameraSetting,
}

/// A recorded camera setting, not a processing-history flag. Auto does not universally mean baked
/// into RAW, and Off does not mean that the lens table is absent. Documented values are 0, 2 and u32::MAX;
/// the A1 II's shading Low value 3 is verified by a controlled capture series.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SonyCameraSetting {
    /// No entry in the parsed MakerNote (also when the note cannot be parsed).
    #[default]
    Absent,
    Off,
    Auto,
    /// Shading value 3 on ILCE-1M2 only; verified against the photographer's Off/Auto/Low sequence.
    Low,
    Unavailable,
    /// Preserve undocumented values, including 1; do not guess that they mean Off or Low.
    Unknown(u32),
    /// A parsed entry with an unsupported type or count.
    Rejected(SonyFieldError),
}

/// Sony's in-file lens-correction record: three independent components, each a raw/SR2 setting and a radial table.
///
/// Tag identities and storage types from the ExifTool EXIF / Sony tag-name documentation: raw-IFD `0x7031` / `0x7032`
/// (shading, "VignettingCorrection" / "VignettingCorrParams"), `0x7034` / `0x7035` (lateral chromatic aberration),
/// `0x7036` / `0x7037` (distortion), and the `SR2SubIFD` tables `0x797d`, `0x7980`, `0x7982`. The `SR2SubIFD` setting
/// tags `0x797c`, `0x797f`, `0x7981` are unnamed there; they sit next to the tables and equal the raw-IFD settings
/// in every file inspected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SonyLensCorrections {
    /// Shading (vignetting): up to 16 radial samples. `crate::sony_shading` separately selects the
    /// measured remaining-gain configurations; presence alone does not authorize a correction.
    /// Only `samples_per_channel` entries are samples; unused entries are zero.
    pub shading: SonyComponent<[i16; 16]>,
    /// Lateral chromatic aberration: red and blue curves relative to green, with 11 or 16 radial samples,
    /// zero-padded to 16 each. `crate::sony_ca` separately selects the validated sensor-stage map
    /// used by the app (see `docs/sony-reference-validation.md`).
    pub chromatic_aberration: SonyComponent<[[i16; 16]; 2]>,
    /// Distortion: 11 or 16 radial samples, zero-padded to 16 (see `samples_per_channel`).
    pub distortion: SonyComponent<[i16; 16]>,
    /// Alternate 19-knot distortion record found in newer body/lens combinations. Its presence
    /// does not authorize a warp; the legacy component's status describes only the legacy table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extended_distortion: Option<SonyExtendedDistortion>,
    /// Whether the encrypted `SR2SubIFD` was found and decrypted. `false`: no copies to fall back on or compare.
    pub sr2_read: bool,
    /// The framing and coding the tables have to be interpreted against.
    pub geometry: SonyLensGeometry,
}

/// Independently observed encrypted SR2 fields `0x7983` and `0x7984`. These are a separate
/// distortion record, not a second copy of `0x7037`/`0x7982`. Synthetic A1 II exports respond
/// to the 19-knot table while ignoring changes to the legacy table, so a usable record takes that
/// table's place; the two records must never compose two corrections.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SonyExtendedDistortion {
    /// Older diagnostic archives did not select or convert this record.
    #[serde(default = "extended_unvalidated")]
    pub status: SonyLensStatus,
    /// Exact SHORT[2] values. Observed pairs include [0/1/17, 11/16] and [65535, 65535].
    /// Keep undocumented values intact rather than inferring processing history or geometry.
    pub setting: SonyField<[u16; 2]>,
    /// Exact SSHORT[20] layout headed by 19, followed by 19 signed knots. A zero prefix
    /// is retained as a rejected/unavailable table, never confused with 19 neutral knots.
    pub table: SonyField<[i16; 19]>,
}

fn extended_unvalidated() -> SonyLensStatus {
    SonyLensStatus::NotValidatedComponent
}

/// One correction component: its raw/SR2 setting, its table, and whether LightCraft applies it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SonyComponent<T> {
    /// Raw/SR2 table setting, which can differ from the camera-menu setting in `SonyCameraSettings`.
    pub setting: SonyField<SonySetting>,
    /// The table's active samples after the leading count, zero-padded to the fixed storage capacity.
    pub table: SonyField<T>,
    /// Active radial knots per channel (11 or 16), excluding storage padding. `None` when the table was refused
    /// or absent. Older serialized records omit this field and, when a table exists, contain 16 active knots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples_per_channel: Option<u8>,
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
    /// Another TIFF field type than SSHORT (8) for a table, SHORT (3) / SSHORT for a raw/SR2 setting,
    /// or LONG (4) for a MakerNote camera setting.
    FieldType(u16),
    /// Another number of values than the supported layout's: 17 (legacy shading/distortion),
    /// 33 (CA), 20 (alternate distortion), or 1/2 (setting).
    Count(u64),
    /// The leading sample count is not 11/16 (22/32 for CA), or 19 for alternate distortion. A count of 0 goes with an
    /// "unavailable" setting and does not describe a usable table.
    CountPrefix(i16),
    /// The values lie outside the file (raw IFD) or the decrypted block (`SR2SubIFD`).
    OutsideData,
    /// In the raw IFD but past what the TIFF reader that decodes the image reads (8,192 entries per directory, a
    /// total value budget), so the image doesn't use it either.
    PastReaderLimits,
    /// A setting stored as a negative SSHORT.
    NegativeSetting(i16),
}

/// A raw/SR2 correction setting: the stored number and its documented meaning. This is not necessarily
/// the camera-menu setting: shading 272 was observed with MakerNote shading both Off and Auto.
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
    /// Converted into the image's `OpcodeList3` for a validated camera and shooting mode.
    Applied,
    /// The alternate record supplied the lens warp; this legacy table is not used.
    Superseded,
    /// No usable table.
    NoTable,
    /// Decoded only in the ordinary rendering path: complete component semantics/integration are not validated.
    /// Lateral CA and selected A1 II shading configurations use separately selected sensor/camera-RGB stages.
    NotValidatedComponent,
    /// Not an opcode: the component is validated on other camera models, not on this one. Distortion still gets
    /// the general conversion's warp beside the opcodes ([`crate::OpcodeLists::fallback_warp`]).
    NotValidatedModel,
    /// Not an opcode: the model is validated, but not this coding or framing (linear YCbCr, a non-3:2 crop) or a
    /// table found only in the `SR2SubIFD`. The general conversion applies as for [`Self::NotValidatedModel`].
    NotValidatedMode,
    /// The camera did not correct distortion (Off, unavailable, missing or unknown), on a validated body or in the
    /// alternate record. The lens warp, if any, is beside the opcodes.
    CameraCorrectionDisabled,
    /// Validated scope with a measured no-op or an all-zero table.
    Neutral,
    /// Validated scope, but the conversion refused this table (values outside the validated range, a degenerate
    /// crop, or a fitted map that misses the table or folds).
    Rejected,
}

/// The framing and coding evidence for interpreting the tables. The image crop and reference
/// use raw-frame pixels. A high-resolution Pixel Shift ARQ can record single-shot crop tags.
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
