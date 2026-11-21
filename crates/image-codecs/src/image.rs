//! The crate's own flat raster image type.
//!
//! Samples are stored interleaved (`RGBARGBA…`), row-major, top-to-bottom,
//! in **native endianness**. Integer samples are unsigned and normalized to
//! `[0, max]`; float samples are nominally `[0, 1]` but may exceed it (HDR).

use std::fmt;

use crate::error::CodecError;
use crate::format::Format;
use crate::options::Limits;
use half::f16;

/// Channel layout of an [`Image`]. Alpha, when present, is always the last
/// channel and is *straight* (not premultiplied).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelLayout {
    Gray,
    GrayA,
    Rgb,
    Rgba,
    /// Ink amounts, `0 = no ink`, `max = full ink` (not Adobe-inverted).
    Cmyk,
    CmykA,
    /// CIE L*a*b* (D50), as PhotoCraft's Lab documents hold it: L* 0..100 as 0..1, a* and b*
    /// −128..127 as `(v + 128) / 255` (Photoshop's unsigned encoding).
    Lab,
    LabA,
}

impl ChannelLayout {
    pub const ALL: [ChannelLayout; 8] = [
        ChannelLayout::Gray,
        ChannelLayout::GrayA,
        ChannelLayout::Rgb,
        ChannelLayout::Rgba,
        ChannelLayout::Cmyk,
        ChannelLayout::CmykA,
        ChannelLayout::Lab,
        ChannelLayout::LabA,
    ];

    /// Total number of channels including alpha.
    pub const fn channels(self) -> usize {
        match self {
            ChannelLayout::Gray => 1,
            ChannelLayout::GrayA => 2,
            ChannelLayout::Rgb | ChannelLayout::Lab => 3,
            ChannelLayout::Rgba | ChannelLayout::Cmyk | ChannelLayout::LabA => 4,
            ChannelLayout::CmykA => 5,
        }
    }

    /// Number of colour (non-alpha) channels.
    pub const fn color_channels(self) -> usize {
        self.channels() - self.has_alpha() as usize
    }

    pub const fn has_alpha(self) -> bool {
        matches!(self, ChannelLayout::GrayA | ChannelLayout::Rgba | ChannelLayout::CmykA | ChannelLayout::LabA)
    }

    pub const fn is_gray(self) -> bool {
        matches!(self, ChannelLayout::Gray | ChannelLayout::GrayA)
    }

    pub const fn is_rgb(self) -> bool {
        matches!(self, ChannelLayout::Rgb | ChannelLayout::Rgba)
    }

    pub const fn is_cmyk(self) -> bool {
        matches!(self, ChannelLayout::Cmyk | ChannelLayout::CmykA)
    }

    pub const fn is_lab(self) -> bool {
        matches!(self, ChannelLayout::Lab | ChannelLayout::LabA)
    }

    /// The same colour model with alpha added.
    pub const fn with_alpha(self) -> Self {
        match self {
            ChannelLayout::Gray | ChannelLayout::GrayA => ChannelLayout::GrayA,
            ChannelLayout::Rgb | ChannelLayout::Rgba => ChannelLayout::Rgba,
            ChannelLayout::Cmyk | ChannelLayout::CmykA => ChannelLayout::CmykA,
            ChannelLayout::Lab | ChannelLayout::LabA => ChannelLayout::LabA,
        }
    }

    /// The same colour model without alpha.
    pub const fn without_alpha(self) -> Self {
        match self {
            ChannelLayout::Gray | ChannelLayout::GrayA => ChannelLayout::Gray,
            ChannelLayout::Rgb | ChannelLayout::Rgba => ChannelLayout::Rgb,
            ChannelLayout::Cmyk | ChannelLayout::CmykA => ChannelLayout::Cmyk,
            ChannelLayout::Lab | ChannelLayout::LabA => ChannelLayout::Lab,
        }
    }

    /// `true` if both layouts share a colour model (ignoring alpha).
    pub const fn same_model(self, other: Self) -> bool {
        (self.is_gray() && other.is_gray())
            || (self.is_rgb() && other.is_rgb())
            || (self.is_cmyk() && other.is_cmyk())
            || (self.is_lab() && other.is_lab())
    }
}

/// Storage type of one sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SampleType {
    U8,
    U16,
    /// IEEE 754 half float, stored as its 16 bit pattern.
    F16,
    F32,
}

impl SampleType {
    pub const ALL: [SampleType; 4] = [SampleType::U8, SampleType::U16, SampleType::F16, SampleType::F32];

    pub const fn bytes(self) -> usize {
        match self {
            SampleType::U8 => 1,
            SampleType::U16 | SampleType::F16 => 2,
            SampleType::F32 => 4,
        }
    }

    pub const fn is_float(self) -> bool {
        matches!(self, SampleType::F16 | SampleType::F32)
    }
}

/// Basic, format-independent metadata.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Metadata {
    /// Raw EXIF payload in TIFF structure (starting with `II*\0`/`MM\0*`),
    /// **without** the JPEG `Exif\0\0` prefix.
    pub exif: Option<Vec<u8>>,
    /// XMP packet as UTF-8.
    pub xmp: Option<String>,
    /// Horizontal and vertical resolution in dots per inch.
    pub dpi: Option<(f32, f32)>,
    /// Free-form key/value text (PNG tEXt/iTXt, TIFF ASCII tags).
    pub text: Vec<(String, String)>,
    /// HDR content light levels carried by formats such as PNG `cLLI`, in units of
    /// 0.0001 cd/m² per value. A zero value means that level is unknown.
    pub content_light_level: Option<crate::cicp::ContentLightLevel>,
    /// Photoshop image resources (`8BIM` resource blocks): TIFF tag 34377. Kept opaque here;
    /// `photocraft-io` reads and writes them through `photocraft-psd`.
    pub photoshop_resources: Option<Vec<u8>>,
    /// Photoshop layer data (TIFF tag 37724 `ImageSourceData`): a layered TIFF's layers. Kept
    /// opaque here; `photocraft-io` reads and writes it through `photocraft-psd`.
    pub photoshop_layers: Option<Vec<u8>>,
    /// Samples per pixel beyond the colour channels and the one alpha sample. TIFF extra
    /// samples (spot colours, extra alpha channels) the pixel layout does not hold. Each entry
    /// is one sample per pixel, native endian, in the image's [`SampleType`], row-major.
    pub extra_samples: Vec<ExtraSample>,
}

/// How many extra TIFF samples an encoded file may carry. Decoders keep at most this many.
/// Sixteen covers a UV-DTF stack (white, varnish, gloss, primer, and further spots) without
/// approaching the PSD 56-channel limit.
pub const MAX_EXTRA_SAMPLES: usize = 16;

/// One TIFF extra sample (a spot channel or an alpha channel that is not the image alpha).
#[derive(Debug, Clone, PartialEq)]
pub struct ExtraSample {
    /// TIFF ExtraSamples value: 0 unspecified, 1 associated alpha, 2 unassociated alpha.
    pub kind: u16,
    /// One sample per pixel, native endian, same sample type as the image.
    pub data: Vec<u8>,
}

impl Metadata {
    pub fn is_empty(&self) -> bool {
        self.exif.is_none()
            && self.xmp.is_none()
            && self.dpi.is_none()
            && self.text.is_empty()
            && self.content_light_level.is_none()
            && self.photoshop_resources.is_none()
            && self.photoshop_layers.is_none()
            && self.extra_samples.is_empty()
    }
}

/// Something a decoder noticed about the file that the decoded pixels don't show: the file
/// held more than was decoded, or less than it should.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeWarning {
    /// Only the first frame of an animation was decoded; `total` counts every frame when known.
    MoreFrames { total: Option<u32> },
    /// Only the first page of a multi-page file was decoded; `total` counts every page when known.
    MorePages { total: Option<u32> },
    /// Only one part of a multi-part file was decoded; `total` counts every part when known.
    MoreParts { total: Option<u32> },
    /// The image opened on the full `display_window` stage; the data region sits at `origin`.
    DisplayWindowStage { stage: (u32, u32), origin: (i32, i32) },
    /// The image data ends early (truncated or damaged file); the decoder filled in the rest
    /// (a baseline JPEG's missing rows come out grey).
    Truncated { format: Format },
    /// Deep image data (per-pixel sample lists) was composited into the flat image; the samples
    /// themselves (Z and other deep channels) are not part of the flat result.
    DeepFlattened { format: Format, max_samples_per_pixel: u32 },
    /// Only the first image element of a DPX file was decoded; `total` counts them all.
    MoreElements { total: u32 },
    /// A DPX element's transfer characteristic and colorimetric specification (SMPTE 268M codes,
    /// 255 when undefined). The code values are imported as stored, without a colour transform.
    DpxEncoding { transfer: u8, colorimetric: u8 },
    /// The file's `count` extra samples (spot or extra alpha channels) did not fit the allocation
    /// limit next to the image, so only the image was decoded.
    ExtraSamplesDropped { format: Format, count: u32 },
    /// The file has `count` extra samples past the first [`MAX_EXTRA_SAMPLES`], which are all a
    /// decoder keeps; those past the cap were not imported.
    ExtraSamplesOverLimit { format: Format, count: u32 },
}

/// The SMPTE 268M name of a transfer characteristic or colorimetric code.
fn dpx_code_name(code: u8) -> &'static str {
    match code {
        0 => "user-defined",
        1 => "printing density",
        2 => "linear",
        3 => "logarithmic",
        4 => "unspecified video",
        5 => "SMPTE 274M",
        6 => "ITU-R BT.709",
        7 => "ITU-R BT.601 625-line",
        8 => "ITU-R BT.601 525-line",
        9 => "NTSC composite video",
        10 => "PAL composite video",
        11 => "Z linear",
        12 => "Z homogeneous",
        255 => "undefined",
        _ => "unknown",
    }
}

impl fmt::Display for DecodeWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeWarning::MoreFrames { total: Some(n) } => write!(f, "only the first of {n} frames was imported"),
            DecodeWarning::MoreFrames { total: None } => write!(f, "only the first frame of the animation was imported"),
            DecodeWarning::MorePages { total: Some(n) } => write!(f, "only the first of {n} pages was imported"),
            DecodeWarning::MorePages { total: None } => write!(f, "only the first page of the file was imported"),
            DecodeWarning::MoreParts { total: Some(n) } => write!(f, "only one of {n} EXR parts was imported"),
            DecodeWarning::MoreParts { total: None } => write!(f, "only one EXR part of the file was imported"),
            DecodeWarning::DisplayWindowStage { stage, origin } => {
                write!(f, "opened on the full display window stage ({}x{}); the data sits at ({}, {})", stage.0, stage.1, origin.0, origin.1)
            }
            DecodeWarning::Truncated { format } => {
                write!(f, "{} data ends early (the file is truncated or damaged); part of the image is missing", format.name())
            }
            DecodeWarning::DeepFlattened { format, max_samples_per_pixel } => write!(
                f,
                "{format} deep data was composited into a flat image (up to {max_samples_per_pixel} samples per pixel); per-pixel depth is not kept",
                format = format.name()
            ),
            DecodeWarning::MoreElements { total } => write!(f, "only the first of {total} image elements was imported"),
            DecodeWarning::DpxEncoding { transfer, colorimetric } => write!(
                f,
                "DPX transfer characteristic: {} ({transfer}), colorimetric: {} ({colorimetric}); the code values were imported as stored, without a colour transform",
                dpx_code_name(*transfer),
                dpx_code_name(*colorimetric)
            ),
            DecodeWarning::ExtraSamplesDropped { format, count } => write!(
                f,
                "{count} extra {} file channel{} (spot or alpha) did not fit in memory and {} not imported",
                format.name(),
                if *count == 1 { "" } else { "s" },
                if *count == 1 { "was" } else { "were" }
            ),
            DecodeWarning::ExtraSamplesOverLimit { format, count } => write!(
                f,
                "{count} extra {} file channel{} (spot or alpha) past the first {MAX_EXTRA_SAMPLES} {} not imported",
                format.name(),
                if *count == 1 { "" } else { "s" },
                if *count == 1 { "was" } else { "were" }
            ),
        }
    }
}

/// One channel of a [`DeepImage`]: the samples of every pixel, one flat list in pixel order.
#[derive(Debug, Clone, PartialEq)]
pub struct DeepChannel {
    /// The channel name as in the file (e.g. `R`, `G`, `B`, `A`, `Z`).
    pub name: String,
    /// The sample type as stored in the file; `samples` always holds the numeric value as
    /// f32 (half and 32-bit floats exactly; 32-bit unsigned integers exactly up to 2^24).
    pub sample: SampleType,
    /// Every sample of the channel, in pixel order; the samples of pixel `i` are
    /// `counts[i]..counts[i+1]` (see [`DeepImage::counts`]).
    pub samples: Vec<f32>,
}

/// Deep image data (OpenEXR `deepscanline`/`deeptile`): a variable-length list of samples
/// per pixel, each with its own colour, alpha and depth. All channels share the same sample
/// layout, so a sample is one index into every channel.
#[derive(Debug, Clone, PartialEq)]
pub struct DeepImage {
    pub width: u32,
    pub height: u32,
    /// The full stage the image opens on: the EXR `display_window` size, when the file declares
    /// one the data region fits into and the limits allow. `None` when absent, absurd or not
    /// containing the data region — then the image opens on the data window alone, as before.
    pub stage: Option<(u32, u32)>,
    /// Where this image's (0, 0) sits on the stage: `data_window.position - display_window.position`.
    /// `(0, 0)` when [`Self::stage`] is `None`.
    pub origin: (i32, i32),
    /// The sample channels in file order.
    pub channels: Vec<DeepChannel>,
    /// Cumulative sample counts, `len == width as usize * height as usize + 1`: pixel `i`
    /// holds the samples `counts[i]..counts[i+1]` of every channel. Monotonically
    /// non-decreasing, starting and ending at the total sample count.
    pub counts: Vec<u64>,
}

impl DeepImage {
    /// A channel by its exact name.
    pub fn channel(&self, name: &str) -> Option<&DeepChannel> {
        self.channels.iter().find(|c| c.name == name)
    }

    /// The samples of one pixel, as indices into every channel's `samples`.
    pub fn sample_range(&self, pixel: usize) -> std::ops::Range<u64> {
        let end = self.counts.get(pixel + 1).copied().unwrap_or(0);
        self.counts.get(pixel).copied().unwrap_or(end)..end
    }

    /// The total number of samples over all pixels.
    pub fn total_samples(&self) -> u64 {
        self.counts.last().copied().unwrap_or(0)
    }
}

/// One channel of an [`ExrPartInfo`]: the name as in the file (a Maya/Arnold AOV layer
/// keeps its prefix, e.g. `diffuse.R`) and the sample type that part stores it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExrChannelInfo {
    pub name: String,
    pub sample: SampleType,
}

/// One part of a multi-part OpenEXR file (what a Maya/Arnold render writes per AOV):
/// header facts only, no pixel data. [`crate::exr_info`] lists them, and the index is
/// what [`crate::decode_exr_part`] decodes; [`crate::decode_as_with`] opens the part
/// the ranking in [`ExrPartInfo::color_rank`] prefers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExrPartInfo {
    /// Index into the part list, in file order.
    pub index: usize,
    /// The part `name` attribute, when the file records one (Arnold names parts after AOVs).
    pub name: Option<String>,
    /// The part `view` attribute for stereo files (`left`/`right`).
    pub view: Option<String>,
    pub width: u32,
    pub height: u32,
    /// Absolute data-window bounds: x, y, width and height.
    pub data_window: (i32, i32, u32, u32),
    /// Absolute display-window bounds: x, y, width and height.
    pub display_window: (i32, i32, u32, u32),
    /// True for `deepscanline`/`deeptile` parts; those are composited by the deep
    /// decoder ([`crate::decode_deep_exr`]), not decoded part by part.
    pub deep: bool,
    /// True when the part is stored as tiles instead of scanlines.
    pub tiled: bool,
    /// The part's channels in file order.
    pub channels: Vec<ExrChannelInfo>,
}

impl ExrPartInfo {
    /// How much this part looks like a colour image the plain [`Image`] decode can show:
    /// 4 RGBA, 3 RGB, 2 YA, 1 Y or any-single-channel, 0 anything else (e.g. `Z` beside `A`).
    /// The colour and alpha channels must share a complete layer prefix. The highest-ranked
    /// group wins; equal groups prefer unprefixed channels, then the first prefix alphabetically.
    /// The auto-pick takes the highest rank; ties go to the earlier part.
    pub fn color_rank(&self) -> u8 {
        exr_color_channels(self.channels.iter().map(|c| c.name.as_str())).map_or(0, |(layout, _)| layout.channels() as u8)
    }
}

/// One selection policy for header ranking/probing and flat decoding: never synthesize
/// RGB(A) or YA from separate layers. OpenEXR layers use the full prefix before the last
/// dot, including nested layers; a leading or trailing dot alone does not name a layer.
pub(crate) fn exr_color_channels<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<(ChannelLayout, Vec<usize>)> {
    #[derive(Default)]
    struct Group {
        r: Option<usize>,
        g: Option<usize>,
        b: Option<usize>,
        a: Option<usize>,
        y: Option<usize>,
    }

    // Sorted prefixes make rank ties deterministic independently of channel order, with
    // the empty (unprefixed) group first. Unsupported channels cannot complete a group.
    let mut groups = std::collections::BTreeMap::<&str, Group>::new();
    let mut count = 0usize;
    for (index, name) in names.into_iter().enumerate() {
        count = count.saturating_add(1);
        let (prefix, base) = name.rsplit_once('.').filter(|(prefix, base)| !prefix.is_empty() && !base.is_empty()).unwrap_or(("", name));
        if !matches!(base, "R" | "G" | "B" | "A" | "Y") {
            continue;
        }
        let group = groups.entry(prefix).or_default();
        let component = match base {
            "R" => &mut group.r,
            "G" => &mut group.g,
            "B" => &mut group.b,
            "A" => &mut group.a,
            "Y" => &mut group.y,
            _ => continue,
        };
        component.get_or_insert(index);
    }

    let mut best = None;
    let mut best_rank = 0;
    for group in groups.values() {
        let candidate = if let (Some(r), Some(g), Some(b)) = (group.r, group.g, group.b) {
            match group.a {
                Some(a) => (ChannelLayout::Rgba, vec![r, g, b, a]),
                None => (ChannelLayout::Rgb, vec![r, g, b]),
            }
        } else if let Some(y) = group.y {
            match group.a {
                Some(a) => (ChannelLayout::GrayA, vec![y, a]),
                None => (ChannelLayout::Gray, vec![y]),
            }
        } else {
            continue;
        };
        let rank = candidate.0.channels();
        if rank > best_rank {
            best = Some(candidate);
            best_rank = rank;
        }
    }
    best.or_else(|| (count == 1).then(|| (ChannelLayout::Gray, vec![0])))
}

/// Strip an optional `layer.` prefix from a channel name (`diffuse.R` → `R`).
pub(crate) fn base_channel_name(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// One Cryptomatte layer of an EXR file (Cryptomatte specification 1.2): per-pixel object or
/// material IDs with coverage, as written by Arnold, V-Ray, Redshift, Mantra/Karma or Cycles.
/// Header facts plus the parsed manifest; the samples come from
/// [`crate::decode_cryptomatte`].
#[derive(Debug, Clone, PartialEq)]
pub struct CryptomatteLayer {
    /// The part (see [`crate::exr_info`]) the channels and attributes live in.
    pub part: usize,
    /// The `cryptomatte/<key>/name` attribute, e.g. `CryptoAsset` or `crypto_asset`.
    pub name: String,
    /// The 7-hex-digit metadata key of the layer.
    pub key: String,
    /// The `conversion` attribute, when recorded (canonically `uint32_to_float32`).
    pub conversion: Option<String>,
    /// The manifest: object names with their ID values. A sidecar manifest file is only
    /// recorded in [`Self::sidecar_manifest`] and not read here (the codec is I/O free).
    pub manifest: Vec<(f32, String)>,
    /// The `manif_file` sidecar attribute, when the manifest is not embedded.
    pub sidecar_manifest: Option<String>,
    /// The stream channel bases in file order, e.g. `["crypto_asset00", "crypto_asset01"]`;
    /// each holds an ID (`.red`) and a coverage (`.green`) channel.
    pub channels: Vec<String>,
}

impl CryptomatteLayer {
    /// The manifest name for an ID value, when the manifest lists it.
    pub fn name_of(&self, id: f32) -> Option<&str> {
        self.manifest.iter().find(|(i, _)| *i == id).map(|(_, n)| n.as_str())
    }
}

/// Decoded Cryptomatte samples: per pixel the (ID, coverage) pairs with coverage above zero,
/// sorted by descending coverage. The IDs are the float values as stored (compared by value,
/// never by bit pattern, like the reference readers).
#[derive(Debug, Clone, PartialEq)]
pub struct CryptomatteBuffer {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<Vec<(f32, f32)>>,
}

impl CryptomatteBuffer {
    /// The pairs of one pixel; empty for a pixel covered by nothing.
    pub fn at(&self, x: u32, y: u32) -> &[(f32, f32)] {
        let i = y as usize * self.width as usize + x as usize;
        self.pixels.get(i).map_or(&[], |v| v)
    }
}

/// A single flat raster image.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    width: u32,
    height: u32,
    layout: ChannelLayout,
    sample: SampleType,
    data: Vec<u8>,
    /// Embedded ICC profile (must describe `layout`'s colour model).
    pub icc: Option<Vec<u8>>,
    /// The file's CICP colour description (a HEIF `nclx`), when it has one. Only an image whose
    /// `icc` is `None` relies on it.
    pub cicp: Option<crate::cicp::Cicp>,
    pub meta: Metadata,
    /// What the decoder noticed about the file: empty for a complete, single image and for
    /// images built in memory. Encoders ignore it.
    pub warnings: Vec<DecodeWarning>,
}

fn byte_len(width: u32, height: u32, layout: ChannelLayout, sample: SampleType) -> Option<usize> {
    (width as usize).checked_mul(height as usize)?.checked_mul(layout.channels())?.checked_mul(sample.bytes())
}

impl Image {
    /// A zero-filled image, or an error if the byte size overflows `usize`.
    pub fn new(width: u32, height: u32, layout: ChannelLayout, sample: SampleType) -> Result<Self, CodecError> {
        let len = byte_len(width, height, layout, sample).ok_or_else(|| CodecError::InvalidImage("image size overflows usize".into()))?;
        Ok(Image { width, height, layout, sample, data: vec![0; len], icc: None, cicp: None, meta: Metadata::default(), warnings: Vec::new() })
    }

    /// Wrap raw interleaved, native-endian bytes.
    pub fn from_raw(width: u32, height: u32, layout: ChannelLayout, sample: SampleType, data: Vec<u8>) -> Result<Self, CodecError> {
        let expected = byte_len(width, height, layout, sample).ok_or_else(|| CodecError::InvalidImage("image size overflows usize".into()))?;
        if data.len() != expected {
            return Err(CodecError::InvalidImage(format!(
                "buffer has {} bytes, expected {expected} for {width}x{height} {layout:?} {sample:?}",
                data.len()
            )));
        }
        Ok(Image { width, height, layout, sample, data, icc: None, cicp: None, meta: Metadata::default(), warnings: Vec::new() })
    }

    pub fn from_u8(width: u32, height: u32, layout: ChannelLayout, data: Vec<u8>) -> Result<Self, CodecError> {
        Self::from_raw(width, height, layout, SampleType::U8, data)
    }

    pub fn from_u16(width: u32, height: u32, layout: ChannelLayout, data: &[u16]) -> Result<Self, CodecError> {
        let bytes = data.iter().flat_map(|v| v.to_ne_bytes()).collect();
        Self::from_raw(width, height, layout, SampleType::U16, bytes)
    }

    pub fn from_f16(width: u32, height: u32, layout: ChannelLayout, data: &[f16]) -> Result<Self, CodecError> {
        let bytes = data.iter().flat_map(|v| v.to_bits().to_ne_bytes()).collect();
        Self::from_raw(width, height, layout, SampleType::F16, bytes)
    }

    pub fn from_f32(width: u32, height: u32, layout: ChannelLayout, data: &[f32]) -> Result<Self, CodecError> {
        let bytes = data.iter().flat_map(|v| v.to_ne_bytes()).collect();
        Self::from_raw(width, height, layout, SampleType::F32, bytes)
    }

    /// Build an image from normalized f32 samples (`[0,1]` for integer
    /// targets; values are clamped and rounded when quantizing).
    pub fn from_normalized(width: u32, height: u32, layout: ChannelLayout, sample: SampleType, values: &[f32]) -> Result<Self, CodecError> {
        let data = quantize(values, sample);
        Self::from_raw(width, height, layout, sample, data)
    }

    pub fn with_icc(mut self, icc: Option<Vec<u8>>) -> Self {
        self.icc = icc;
        self
    }

    pub fn with_meta(mut self, meta: Metadata) -> Self {
        self.meta = meta;
        self
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn layout(&self) -> ChannelLayout {
        self.layout
    }
    pub fn sample_type(&self) -> SampleType {
        self.sample
    }
    pub fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }
    /// Number of samples (pixels × channels).
    pub fn sample_count(&self) -> usize {
        self.pixel_count() * self.layout.channels()
    }
    /// Raw interleaved native-endian bytes.
    pub fn data(&self) -> &[u8] {
        &self.data
    }
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// Borrow samples as `u8` (only when the sample type is `U8`).
    pub fn as_u8(&self) -> Option<&[u8]> {
        (self.sample == SampleType::U8).then_some(&self.data[..])
    }

    /// Copy samples out as `u16` (only when the sample type is `U16`).
    pub fn to_u16_samples(&self) -> Option<Vec<u16>> {
        (self.sample == SampleType::U16).then(|| self.data.as_chunks::<2>().0.iter().map(|c| u16::from_ne_bytes([c[0], c[1]])).collect())
    }

    /// Copy samples out as `f16` (only when the sample type is `F16`).
    pub fn to_f16_samples(&self) -> Option<Vec<f16>> {
        (self.sample == SampleType::F16)
            .then(|| self.data.as_chunks::<2>().0.iter().map(|c| f16::from_bits(u16::from_ne_bytes([c[0], c[1]]))).collect())
    }

    /// Copy samples out as `f32` (only when the sample type is `F32`).
    pub fn to_f32_samples(&self) -> Option<Vec<f32>> {
        (self.sample == SampleType::F32).then(|| self.data.as_chunks::<4>().0.iter().map(|c| f32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect())
    }

    /// Sample `index` (in samples, not bytes) as a normalized f32.
    pub fn sample_normalized(&self, index: usize) -> f32 {
        read_normalized(&self.data, self.sample, index)
    }

    /// Normalized f32 of channel `c` at pixel `(x, y)`.
    pub fn get(&self, x: u32, y: u32, c: usize) -> f32 {
        let idx = (y as usize * self.width as usize + x as usize) * self.layout.channels() + c;
        self.sample_normalized(idx)
    }

    /// All samples as normalized f32 (integers mapped to `[0,1]`).
    pub fn to_normalized(&self) -> Vec<f32> {
        to_normalized(&self.data, self.sample)
    }

    /// `true` when the image has alpha and at least one pixel is not fully
    /// opaque.
    pub fn has_translucency(&self) -> bool {
        if !self.layout.has_alpha() {
            return false;
        }
        let ch = self.layout.channels();
        (0..self.pixel_count()).any(|p| self.sample_normalized(p * ch + ch - 1) < 1.0)
    }

    /// `true` when this is a float image with any sample outside `[0, 1]`
    /// (or NaN).
    pub fn has_out_of_range(&self) -> bool {
        if !self.sample.is_float() {
            return false;
        }
        (0..self.sample_count()).any(|i| {
            let v = self.sample_normalized(i);
            !(0.0..=1.0).contains(&v)
        })
    }

    /// Convert to another layout and/or sample type. The ICC profile is kept
    /// only if the colour model is unchanged (an RGB profile on CMYK data
    /// would be wrong). Metadata is kept.
    pub fn convert(&self, layout: ChannelLayout, sample: SampleType) -> Image {
        if layout == self.layout && sample == self.sample {
            return self.clone();
        }
        let data = convert_data(&self.data, self.layout, self.sample, layout, sample);
        let mut meta = self.meta.clone();
        if sample != self.sample {
            // Extra samples are stored in the image's sample type, so they convert with it. A
            // plane of the wrong size is left as it is: importers drop it with a warning.
            let plane_len = (self.width as usize).saturating_mul(self.height as usize).saturating_mul(self.sample.bytes());
            for e in meta.extra_samples.iter_mut().filter(|e| e.data.len() == plane_len) {
                if let Ok(plane) = Image::from_raw(self.width, self.height, ChannelLayout::Gray, self.sample, std::mem::take(&mut e.data)) {
                    e.data = plane.convert(ChannelLayout::Gray, sample).data;
                }
            }
        }
        Image {
            width: self.width,
            height: self.height,
            layout,
            sample,
            data,
            icc: if layout.same_model(self.layout) { self.icc.clone() } else { None },
            cicp: if layout.same_model(self.layout) { self.cicp } else { None },
            meta,
            warnings: self.warnings.clone(),
        }
    }

    /// [`Image::convert`], borrowing `self` when it already has that layout and sample type
    /// (encoders use it so a full-size image isn't copied for nothing).
    pub fn converted(&self, layout: ChannelLayout, sample: SampleType) -> std::borrow::Cow<'_, Image> {
        if layout == self.layout && sample == self.sample {
            std::borrow::Cow::Borrowed(self)
        } else {
            std::borrow::Cow::Owned(self.convert(layout, sample))
        }
    }

    /// Interleaved RGBA, 8 bits per sample.
    pub fn to_rgba8(&self) -> Vec<u8> {
        convert_data(&self.data, self.layout, self.sample, ChannelLayout::Rgba, SampleType::U8)
    }

    /// Interleaved RGBA, 16 bits per sample.
    pub fn to_rgba16(&self) -> Vec<u16> {
        u16_samples(&convert_data(&self.data, self.layout, self.sample, ChannelLayout::Rgba, SampleType::U16))
    }

    /// [`Image::to_rgba8`], with the output checked against `limits` (dimensions, pixel count and
    /// `max_alloc`) before any pixel is converted. A size over the limits, or a reservation the
    /// allocator refuses, returns [`CodecError::LimitExceeded`]. An image with no pixels gives an
    /// empty vector.
    pub fn try_to_rgba8(&self, limits: &Limits) -> Result<Vec<u8>, CodecError> {
        self.rgba_within(SampleType::U8, limits)
    }

    /// [`Image::to_rgba16`], with the output checked against `limits` as in
    /// [`Image::try_to_rgba8`]. The samples are converted into bytes first and then copied into the
    /// `u16` vector, so the peak is twice the size that `limits` checks.
    pub fn try_to_rgba16(&self, limits: &Limits) -> Result<Vec<u16>, CodecError> {
        let bytes = self.rgba_within(SampleType::U16, limits)?;
        let mut out = Vec::new();
        out.try_reserve_exact(bytes.len() / 2)
            .map_err(|e| CodecError::LimitExceeded(format!("cannot reserve {} u16 samples for RGBA: {e}", bytes.len() / 2)))?;
        out.extend(bytes.as_chunks::<2>().0.iter().map(|c| u16::from_ne_bytes([c[0], c[1]])));
        Ok(out)
    }

    /// [`Image::to_rgba8`], consuming the image. An RGBA8 image gives back its own pixel buffer
    /// without a copy. Any other image is converted as [`Image::try_to_rgba8`] converts it, with
    /// the same `limits` check, and its buffer is dropped once the conversion returns.
    pub fn into_rgba8(self, limits: &Limits) -> Result<Vec<u8>, CodecError> {
        if self.layout == ChannelLayout::Rgba && self.sample == SampleType::U8 {
            return Ok(self.data);
        }
        self.try_to_rgba8(limits)
    }

    /// RGBA samples in `sample` (U8 or U16), the bytes [`convert_data`] gives, with the output
    /// checked against `limits` and reserved with `try_reserve_exact`. Integer sources are
    /// converted by [`convert_int_direct_into`]. Other sources are converted one pixel at a time,
    /// so the f32 copies of the whole image that [`convert_data`] makes for them are not allocated.
    fn rgba_within(&self, sample: SampleType, limits: &Limits) -> Result<Vec<u8>, CodecError> {
        if self.width == 0 || self.height == 0 {
            return Ok(Vec::new());
        }
        limits.check(self.width, self.height, ChannelLayout::Rgba, sample)?;
        let len = byte_len(self.width, self.height, ChannelLayout::Rgba, sample)
            .ok_or_else(|| CodecError::LimitExceeded("RGBA size overflows usize".into()))?;
        let mut out = Vec::new();
        out.try_reserve_exact(len).map_err(|e| CodecError::LimitExceeded(format!("cannot reserve {len} bytes for RGBA: {e}")))?;
        if int_direct_applies(self.layout, self.sample, ChannelLayout::Rgba, sample) {
            convert_int_direct_into(&self.data, self.layout, self.sample, ChannelLayout::Rgba, sample, &mut out);
            return Ok(out);
        }
        let fc = self.layout.channels();
        let mut px = [0.0f32; 5];
        let mut rgba = Vec::with_capacity(ChannelLayout::Rgba.channels());
        for p in 0..self.pixel_count() {
            for (c, v) in px[..fc].iter_mut().enumerate() {
                *v = read_normalized(&self.data, self.sample, p * fc + c);
            }
            rgba.clear();
            convert_pixel(&px[..fc], self.layout, ChannelLayout::Rgba, &mut rgba);
            quantize_into(&rgba, sample, &mut out);
        }
        Ok(out)
    }

    /// Interleaved RGBA as f32 (integer inputs normalized to `[0,1]`), with the same values as
    /// [`Image::convert`] to RGBA F32. The output buffer is checked against `limits` (dimensions,
    /// pixel count and `max_alloc`) and reserved before any pixel is converted, and the pixels are
    /// written straight into it. A size over the limits, or a reservation the allocator refuses,
    /// returns [`CodecError::LimitExceeded`]. An image with no pixels gives an empty vector.
    pub fn to_rgba_f32(&self, limits: &Limits) -> Result<Vec<f32>, CodecError> {
        if self.width == 0 || self.height == 0 {
            return Ok(Vec::new());
        }
        limits.check(self.width, self.height, ChannelLayout::Rgba, SampleType::F32)?;
        let len = self
            .pixel_count()
            .checked_mul(ChannelLayout::Rgba.channels())
            .ok_or_else(|| CodecError::LimitExceeded("RGBA f32 size overflows usize".into()))?;
        let mut out = Vec::new();
        out.try_reserve_exact(len).map_err(|e| CodecError::LimitExceeded(format!("cannot reserve {len} f32 samples for RGBA: {e}")))?;
        let fc = self.layout.channels();
        let mut px = [0.0f32; 5];
        for p in 0..self.pixel_count() {
            for (c, v) in px[..fc].iter_mut().enumerate() {
                *v = read_normalized(&self.data, self.sample, p * fc + c);
            }
            convert_pixel(&px[..fc], self.layout, ChannelLayout::Rgba, &mut out);
        }
        Ok(out)
    }
}

/// The samples of `data` (`from` layout and sample type) in the `layout` and `sample` type: the
/// arithmetic of [`Image::convert`], on borrowed bytes.
fn convert_data(data: &[u8], from: ChannelLayout, from_sample: SampleType, layout: ChannelLayout, sample: SampleType) -> Vec<u8> {
    if layout == from && sample == from_sample {
        data.to_vec()
    } else if sample == from_sample && !from.has_alpha() && layout == from.with_alpha() {
        // Fast path (opening any opaque file): copy the samples, append an opaque alpha.
        let bps = sample.bytes();
        let src = from.channels() * bps;
        let one: Vec<u8> = match sample {
            SampleType::U8 => vec![u8::MAX],
            SampleType::U16 => u16::MAX.to_ne_bytes().to_vec(),
            SampleType::F16 => f16::ONE.to_bits().to_ne_bytes().to_vec(),
            SampleType::F32 => 1.0f32.to_ne_bytes().to_vec(),
        };
        let mut out = Vec::with_capacity(data.len() / src * (src + bps));
        for px in data.chunks_exact(src) {
            out.extend_from_slice(px);
            out.extend_from_slice(&one);
        }
        out
    } else if let Some(fast) = convert_int_direct(data, from, from_sample, layout, sample) {
        fast
    } else if layout == from {
        quantize(&to_normalized(data, from_sample), sample)
    } else {
        let src = to_normalized(data, from_sample);
        let values = convert_layout(&src, from, layout);
        quantize(&values, sample)
    }
}

/// [`Image::convert`] on borrowed pixels: `rows` rows of `width` pixels in `from` (layout, sample
/// type) become the same pixels in `to`, bit for bit what the image conversion gives. Lets a
/// caller convert a band of a decoded image without copying it into an [`Image`] first.
pub fn convert_samples(
    src: &[u8],
    width: u32,
    rows: u32,
    from: (ChannelLayout, SampleType),
    to: (ChannelLayout, SampleType),
) -> Result<Vec<u8>, CodecError> {
    let expected = byte_len(width, rows, from.0, from.1).ok_or_else(|| CodecError::InvalidImage("image size overflows usize".into()))?;
    if src.len() != expected {
        return Err(CodecError::InvalidImage(format!(
            "buffer has {} bytes, expected {expected} for {width}x{rows} {:?} {:?}",
            src.len(),
            from.0,
            from.1
        )));
    }
    Ok(convert_data(src, from.0, from.1, to.0, to.1))
}

/// Integer fast path of [`Image::convert`] for U8/U16 sources and targets whose result is
/// a pure channel copy: same colour model (alpha added as opaque, or dropped) or gray to
/// RGB. Bit-identical to the generic f32 route (see the equivalence tests) but without
/// the two intermediate f32 buffers. Returns `None` for everything else.
fn convert_int_direct(data: &[u8], from: ChannelLayout, from_sample: SampleType, layout: ChannelLayout, sample: SampleType) -> Option<Vec<u8>> {
    if !int_direct_applies(from, from_sample, layout, sample) {
        return None;
    }
    let n = data.len() / (from.channels() * from_sample.bytes());
    let mut out = Vec::with_capacity(n.checked_mul(layout.channels())?.checked_mul(sample.bytes())?);
    convert_int_direct_into(data, from, from_sample, layout, sample, &mut out);
    Some(out)
}

/// `true` when [`convert_int_direct`] handles the conversion from `from` to `layout`.
fn int_direct_applies(from: ChannelLayout, from_sample: SampleType, layout: ChannelLayout, sample: SampleType) -> bool {
    let int = |s: SampleType| matches!(s, SampleType::U8 | SampleType::U16);
    int(from_sample) && int(sample) && ((from.is_gray() && layout.is_rgb()) || layout.same_model(from))
}

/// [`convert_int_direct`], appending to `out`. The caller checks [`int_direct_applies`] first.
fn convert_int_direct_into(data: &[u8], from: ChannelLayout, from_sample: SampleType, layout: ChannelLayout, sample: SampleType, out: &mut Vec<u8>) {
    let one: &[u8] = match sample {
        SampleType::U8 => &[u8::MAX],
        _ => &u16::MAX.to_ne_bytes(),
    };
    let fc = from.channels();
    let sb = from_sample.bytes();
    let (src_color, dst_color) = (from.color_channels(), layout.color_channels());
    macro_rules! run {
        ($conv:expr) => {{
            let conv = $conv;
            for px in data.chunks_exact(fc * sb) {
                for j in 0..dst_color {
                    let c = if src_color == 1 { 0 } else { j };
                    conv(&px[c * sb..(c + 1) * sb], &mut *out);
                }
                if layout.has_alpha() {
                    if from.has_alpha() {
                        conv(&px[src_color * sb..(src_color + 1) * sb], &mut *out);
                    } else {
                        out.extend_from_slice(one);
                    }
                }
            }
        }};
    }
    match (from_sample, sample) {
        (SampleType::U8, SampleType::U8) => run!(|s: &[u8], o: &mut Vec<u8>| o.push(s[0])),
        (SampleType::U16, SampleType::U16) => run!(|s: &[u8], o: &mut Vec<u8>| o.extend_from_slice(s)),
        (SampleType::U8, SampleType::U16) => run!(|s: &[u8], o: &mut Vec<u8>| o.extend_from_slice(&(u16::from(s[0]) * 257).to_ne_bytes())),
        _ => run!(|s: &[u8], o: &mut Vec<u8>| o.push(((u32::from(u16::from_ne_bytes([s[0], s[1]])) + 128) / 257) as u8)),
    }
}

// ---------------------------------------------------------------------------
// Sample conversion helpers
// ---------------------------------------------------------------------------

#[inline]
pub(crate) fn read_normalized(data: &[u8], sample: SampleType, i: usize) -> f32 {
    match sample {
        SampleType::U8 => data[i] as f32 / 255.0,
        SampleType::U16 => u16::from_ne_bytes([data[2 * i], data[2 * i + 1]]) as f32 / 65535.0,
        SampleType::F16 => f16::from_bits(u16::from_ne_bytes([data[2 * i], data[2 * i + 1]])).to_f32(),
        SampleType::F32 => f32::from_ne_bytes([data[4 * i], data[4 * i + 1], data[4 * i + 2], data[4 * i + 3]]),
    }
}

pub(crate) fn to_normalized(data: &[u8], sample: SampleType) -> Vec<f32> {
    match sample {
        SampleType::U8 => data.iter().map(|&v| v as f32 / 255.0).collect(),
        SampleType::U16 => data.as_chunks::<2>().0.iter().map(|c| u16::from_ne_bytes([c[0], c[1]]) as f32 / 65535.0).collect(),
        SampleType::F16 => data.as_chunks::<2>().0.iter().map(|c| f16::from_bits(u16::from_ne_bytes([c[0], c[1]])).to_f32()).collect(),
        SampleType::F32 => data.as_chunks::<4>().0.iter().map(|c| f32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect(),
    }
}

#[inline]
fn clamp01(v: f32) -> f32 {
    if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }
}

pub(crate) fn quantize(values: &[f32], sample: SampleType) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * sample.bytes());
    quantize_into(values, sample, &mut out);
    out
}

/// [`quantize`], appending to `out`.
fn quantize_into(values: &[f32], sample: SampleType, out: &mut Vec<u8>) {
    match sample {
        SampleType::U8 => out.extend(values.iter().map(|&v| (clamp01(v) * 255.0).round() as u8)),
        SampleType::U16 => out.extend(values.iter().flat_map(|&v| ((clamp01(v) * 65535.0).round() as u16).to_ne_bytes())),
        SampleType::F16 => out.extend(values.iter().flat_map(|&v| f16::from_f32(v).to_bits().to_ne_bytes())),
        SampleType::F32 => out.extend(values.iter().flat_map(|&v| v.to_ne_bytes())),
    }
}

/// Native-endian byte pairs as `u16` samples.
fn u16_samples(data: &[u8]) -> Vec<u16> {
    data.as_chunks::<2>().0.iter().map(|c| u16::from_ne_bytes([c[0], c[1]])).collect()
}

/// Rec. 709 luma weights applied to encoded values.
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

fn cmyk_to_rgb(c: f32, m: f32, y: f32, k: f32) -> [f32; 3] {
    [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)]
}

fn rgb_to_cmyk(r: f32, g: f32, b: f32) -> [f32; 4] {
    let k = 1.0 - r.max(g).max(b);
    if k >= 1.0 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let d = 1.0 - k;
    [(1.0 - r - k) / d, (1.0 - g - k) / d, (1.0 - b - k) / d, k]
}

/// Convert normalized interleaved samples between layouts. Naive
/// (non-colour-managed) formulas; colour-managed conversion belongs in
/// `photocraft-color`.
pub(crate) fn convert_layout(src: &[f32], from: ChannelLayout, to: ChannelLayout) -> Vec<f32> {
    if from == to {
        return src.to_vec();
    }
    let fc = from.channels();
    let tc = to.channels();
    let n = src.len() / fc;
    let mut out = Vec::with_capacity(n * tc);
    for px in src.chunks_exact(fc) {
        convert_pixel(px, from, to, &mut out);
    }
    out
}

/// One pixel of [`convert_layout`]: the normalized samples `px` (`from` layout) appended to `out`
/// in the `to` layout.
fn convert_pixel(px: &[f32], from: ChannelLayout, to: ChannelLayout, out: &mut Vec<f32>) {
    let alpha = if from.has_alpha() { px[from.channels() - 1] } else { 1.0 };
    let color = &px[..from.color_channels()];
    if from.same_model(to) {
        out.extend_from_slice(color);
    } else {
        let rgb = match from {
            ChannelLayout::Gray | ChannelLayout::GrayA => [color[0]; 3],
            ChannelLayout::Rgb | ChannelLayout::Rgba => [color[0], color[1], color[2]],
            ChannelLayout::Cmyk | ChannelLayout::CmykA => cmyk_to_rgb(color[0], color[1], color[2], color[3]),
            ChannelLayout::Lab | ChannelLayout::LabA => crate::lab::lab_to_srgb([color[0], color[1], color[2]]),
        };
        match to {
            ChannelLayout::Gray | ChannelLayout::GrayA => {
                out.push(rgb[0] * LUMA[0] + rgb[1] * LUMA[1] + rgb[2] * LUMA[2]);
            }
            ChannelLayout::Rgb | ChannelLayout::Rgba => out.extend_from_slice(&rgb),
            ChannelLayout::Cmyk | ChannelLayout::CmykA => out.extend_from_slice(&rgb_to_cmyk(rgb[0], rgb[1], rgb[2])),
            ChannelLayout::Lab | ChannelLayout::LabA => out.extend_from_slice(&crate::lab::srgb_to_lab(rgb)),
        }
    }
    if to.has_alpha() {
        out.push(alpha);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn adding_alpha_fast_path_matches_the_general_conversion() {
        for sample in SampleType::ALL {
            for layout in [ChannelLayout::Gray, ChannelLayout::Rgb, ChannelLayout::Cmyk] {
                let n = 7 * 3 * layout.channels();
                let vals: Vec<f32> = (0..n).map(|i| (i as f32 * 0.137) % 1.0).collect();
                let img = super::Image::from_normalized(7, 3, layout, sample, &vals).unwrap();
                let fast = img.convert(layout.with_alpha(), sample);
                let slow = super::quantize(&super::convert_layout(&img.to_normalized(), layout, layout.with_alpha()), sample);
                assert_eq!(fast.data(), &slow[..], "{layout:?} {sample:?}");
            }
        }
    }

    use super::*;

    #[test]
    fn convert_samples_matches_image_convert_and_rejects_a_wrong_length() {
        for sample in SampleType::ALL {
            let vals: Vec<f32> = (0..5 * 2 * 3).map(|i| (i as f32 * 0.31) % 1.0).collect();
            let img = Image::from_normalized(5, 2, ChannelLayout::Rgb, sample, &vals).unwrap();
            for (layout, to) in [(ChannelLayout::Rgba, sample), (ChannelLayout::Gray, SampleType::F32), (ChannelLayout::Rgb, SampleType::U8)] {
                let want = img.convert(layout, to);
                let got = convert_samples(img.data(), 5, 2, (ChannelLayout::Rgb, sample), (layout, to)).unwrap();
                assert_eq!(got, want.data(), "{sample:?} to {layout:?} {to:?}");
            }
            assert!(
                convert_samples(img.data().get(1..).unwrap_or_default(), 5, 2, (ChannelLayout::Rgb, sample), (ChannelLayout::Rgba, sample)).is_err()
            );
        }
    }

    /// The generic f32 route `convert` used before the integer fast paths (the reference).
    fn convert_via_f32(img: &Image, layout: ChannelLayout, sample: SampleType) -> Vec<u8> {
        quantize(&convert_layout(&img.to_normalized(), img.layout, layout), sample)
    }

    #[test]
    fn int_fast_path_matches_f32_route_for_every_value() {
        // Every U8 and every U16 value through every depth pair (alpha goes through the same per-sample conversion; covered by the layout test).
        let all8: Vec<u8> = (0..=255).collect();
        let all16: Vec<u16> = (0..=65535).collect();
        for (src, to) in [
            (SampleType::U8, SampleType::U8),
            (SampleType::U8, SampleType::U16),
            (SampleType::U16, SampleType::U8),
            (SampleType::U16, SampleType::U16),
        ] {
            let img = if src == SampleType::U8 {
                Image::from_u8(256, 1, ChannelLayout::Gray, all8.clone()).unwrap()
            } else {
                Image::from_u16(65536, 1, ChannelLayout::Gray, &all16).unwrap()
            };
            for layout in [ChannelLayout::Gray, ChannelLayout::Rgb] {
                assert_eq!(img.convert(layout, to).data(), &convert_via_f32(&img, layout, to)[..], "{src:?}->{to:?} {layout:?}");
            }
        }
    }

    #[test]
    fn int_fast_path_matches_f32_route_for_every_layout() {
        let mut seed = 0x9e3779b97f4a7c15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for from_s in [SampleType::U8, SampleType::U16] {
            for from_l in ChannelLayout::ALL {
                let n = 64 * from_l.channels();
                let mut vals = Vec::new();
                for i in 0..n {
                    // Edge values (0, 1, max-1, max, mid) mixed with random ones.
                    let r = next();
                    let v = match i % 7 {
                        0 => 0.0,
                        1 => 1.0,
                        2 => 1.0 / 255.0,
                        3 => 254.0 / 255.0,
                        _ => (r % 65536) as f32 / 65535.0,
                    };
                    vals.push(v);
                }
                let img = Image::from_normalized(64, 1, from_l, from_s, &vals).unwrap();
                for to_l in ChannelLayout::ALL {
                    for to_s in SampleType::ALL {
                        let got = img.convert(to_l, to_s);
                        assert_eq!(got.data(), &convert_via_f32(&img, to_l, to_s)[..], "{from_l:?}/{from_s:?} -> {to_l:?}/{to_s:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn u8_u16_roundtrip_exact() {
        let v: Vec<u8> = (0..=255).collect();
        let img = Image::from_u8(256, 1, ChannelLayout::Gray, v.clone()).unwrap();
        let back = img.convert(ChannelLayout::Gray, SampleType::U16).convert(ChannelLayout::Gray, SampleType::U8);
        assert_eq!(back.data(), &v[..]);
    }

    #[test]
    fn u16_f32_roundtrip_exact() {
        let v: Vec<u16> = (0..=65535u32).step_by(7).map(|x| x as u16).collect();
        let img = Image::from_u16(v.len() as u32, 1, ChannelLayout::Gray, &v).unwrap();
        let back = img.convert(ChannelLayout::Gray, SampleType::F32).convert(ChannelLayout::Gray, SampleType::U16);
        assert_eq!(back.to_u16_samples().unwrap(), v);
    }

    #[test]
    fn icc_dropped_on_model_change() {
        let img = Image::new(2, 2, ChannelLayout::Cmyk, SampleType::U8).unwrap().with_icc(Some(vec![1, 2, 3]));
        assert!(img.convert(ChannelLayout::Rgb, SampleType::U8).icc.is_none());
        assert!(img.convert(ChannelLayout::CmykA, SampleType::U16).icc.is_some());
    }

    #[test]
    fn from_raw_rejects_bad_len() {
        assert!(Image::from_raw(2, 2, ChannelLayout::Rgb, SampleType::U8, vec![0; 11]).is_err());
    }

    #[test]
    fn rgba_f32_limit_rejects_before_allocation() {
        // 3x2 pixels: the RGBA f32 output is 3 * 2 * 4 * 4 = 96 bytes. Most sources here are
        // smaller than that, so the source fitting the budget does not let the output through.
        for sample in SampleType::ALL {
            for layout in ChannelLayout::ALL {
                let img = Image::new(3, 2, layout, sample).unwrap();
                let tight = Limits::new(3, 2, 6, 95);
                assert!(matches!(img.to_rgba_f32(&tight), Err(CodecError::LimitExceeded(_))), "{layout:?} {sample:?}: max_alloc");
                let exact = Limits::new(3, 2, 6, 96);
                assert_eq!(img.to_rgba_f32(&exact).map(|v| v.len()).ok(), Some(24), "{layout:?} {sample:?}");
                for limits in [Limits::new(2, 2, 6, 96), Limits::new(3, 1, 6, 96), Limits::new(3, 2, 5, 96)] {
                    assert!(matches!(img.to_rgba_f32(&limits), Err(CodecError::LimitExceeded(_))), "{layout:?} {sample:?}: {limits:?}");
                }
            }
        }
    }

    #[test]
    fn rgba_f32_matches_existing_conversions() {
        // Edge, mid and (for float samples) out-of-range values, so the CMYK, Lab and alpha
        // formulas and the f16 and f32 reads are all compared bit for bit.
        let base = [0.0, 1.0, 0.5, 0.25, 0.75, 1.0 / 255.0, 0.3, 0.9, 1.5, -0.25];
        for sample in SampleType::ALL {
            for layout in ChannelLayout::ALL {
                let n = 7 * 3 * layout.channels();
                let vals: Vec<f32> = (0..n).map(|i| base[(i * 7 + i / 3) % base.len()]).collect();
                let img = Image::from_normalized(7, 3, layout, sample, &vals).unwrap();
                let got = img.to_rgba_f32(&Limits::default()).unwrap();
                let want = convert_layout(&img.to_normalized(), layout, ChannelLayout::Rgba);
                let via_convert = img.convert(ChannelLayout::Rgba, SampleType::F32).to_f32_samples().unwrap();
                let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
                assert_eq!(bits(&got), bits(&want), "{layout:?} {sample:?}");
                assert_eq!(bits(&got), bits(&via_convert), "{layout:?} {sample:?}: convert");
            }
        }
    }

    #[test]
    fn rgba_f32_of_an_empty_image_is_empty() {
        for (w, h) in [(0, 0), (0, 3), (4, 0)] {
            let img = Image::new(w, h, ChannelLayout::Rgb, SampleType::U8).unwrap();
            assert_eq!(img.to_rgba_f32(&Limits::new(1, 1, 1, 0)).ok(), Some(Vec::new()), "{w}x{h}");
        }
    }

    #[test]
    fn rgba8_of_gray() {
        let img = Image::from_u8(1, 1, ChannelLayout::GrayA, vec![10, 20]).unwrap();
        assert_eq!(img.to_rgba8(), vec![10, 10, 10, 20]);
    }

    #[test]
    fn rgba8_and_rgba16_limits_reject_before_allocation() {
        // 3x2 pixels: the RGBA8 output is 3 * 2 * 4 = 24 bytes and the RGBA16 output 48. A gray
        // U8 source is 6 bytes, so the source fitting the budget does not let the output through.
        for sample in SampleType::ALL {
            for layout in ChannelLayout::ALL {
                let img = Image::new(3, 2, layout, sample).unwrap();
                let over =
                    |bytes| [Limits::new(3, 2, 6, bytes - 1), Limits::new(2, 2, 6, bytes), Limits::new(3, 1, 6, bytes), Limits::new(3, 2, 5, bytes)];
                for limits in over(24) {
                    assert!(matches!(img.try_to_rgba8(&limits), Err(CodecError::LimitExceeded(_))), "{layout:?} {sample:?}: {limits:?}");
                }
                for limits in over(48) {
                    assert!(matches!(img.try_to_rgba16(&limits), Err(CodecError::LimitExceeded(_))), "{layout:?} {sample:?}: {limits:?}");
                }
                if layout != ChannelLayout::Rgba || sample != SampleType::U8 {
                    for limits in over(24) {
                        assert!(matches!(img.clone().into_rgba8(&limits), Err(CodecError::LimitExceeded(_))), "{layout:?} {sample:?}: {limits:?}");
                    }
                }
                assert_eq!(img.try_to_rgba8(&Limits::new(3, 2, 6, 24)).map(|v| v.len()).ok(), Some(24), "{layout:?} {sample:?}");
                assert_eq!(img.try_to_rgba16(&Limits::new(3, 2, 6, 48)).map(|v| v.len()).ok(), Some(24), "{layout:?} {sample:?}");
                assert_eq!(img.clone().into_rgba8(&Limits::new(3, 2, 6, 24)).map(|v| v.len()).ok(), Some(24), "{layout:?} {sample:?}");
            }
        }
    }

    #[test]
    fn checked_rgba8_and_rgba16_match_the_unchecked_conversions() {
        // The same edge, mid and out-of-range values as the f32 comparison, so the integer fast
        // path and the per-pixel path are both compared with Image::convert bit for bit.
        let base = [0.0, 1.0, 0.5, 0.25, 0.75, 1.0 / 255.0, 0.3, 0.9, 1.5, -0.25];
        for sample in SampleType::ALL {
            for layout in ChannelLayout::ALL {
                let n = 7 * 3 * layout.channels();
                let vals: Vec<f32> = (0..n).map(|i| base[(i * 7 + i / 3) % base.len()]).collect();
                let img = Image::from_normalized(7, 3, layout, sample, &vals).unwrap();
                let want8 = img.convert(ChannelLayout::Rgba, SampleType::U8);
                let want16 = img.convert(ChannelLayout::Rgba, SampleType::U16).to_u16_samples().unwrap();
                assert_eq!(img.to_rgba8(), want8.data(), "{layout:?} {sample:?}");
                assert_eq!(img.try_to_rgba8(&Limits::default()).unwrap(), want8.data(), "{layout:?} {sample:?}: checked");
                assert_eq!(img.clone().into_rgba8(&Limits::default()).unwrap(), want8.data(), "{layout:?} {sample:?}: into");
                assert_eq!(img.to_rgba16(), want16, "{layout:?} {sample:?}");
                assert_eq!(img.try_to_rgba16(&Limits::default()).unwrap(), want16, "{layout:?} {sample:?}: checked");
            }
        }
    }

    #[test]
    fn into_rgba8_of_an_rgba8_image_keeps_its_buffer() {
        let img = Image::from_u8(2, 1, ChannelLayout::Rgba, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let ptr = img.data().as_ptr();
        // The image already holds its output, so a budget of 0 bytes does not apply to it.
        let out = img.into_rgba8(&Limits::new(1, 1, 1, 0)).unwrap();
        assert_eq!(out.as_ptr(), ptr);
        assert_eq!(out, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn checked_rgba8_and_rgba16_of_an_empty_image_are_empty() {
        for (w, h) in [(0, 0), (0, 3), (4, 0)] {
            let img = Image::new(w, h, ChannelLayout::Gray, SampleType::U16).unwrap();
            let none = Limits::new(1, 1, 1, 0);
            assert_eq!(img.try_to_rgba8(&none).ok(), Some(Vec::new()), "{w}x{h}");
            assert_eq!(img.try_to_rgba16(&none).ok(), Some(Vec::new()), "{w}x{h}");
            assert_eq!(img.into_rgba8(&none).ok(), Some(Vec::new()), "{w}x{h}");
        }
    }

    #[test]
    fn cmyk_rgb_basic() {
        let img = Image::from_u8(1, 1, ChannelLayout::Cmyk, vec![0, 0, 0, 255]).unwrap();
        assert_eq!(img.to_rgba8(), vec![0, 0, 0, 255]);
        let img = Image::from_u8(1, 1, ChannelLayout::Rgb, vec![255, 0, 0]).unwrap();
        let c = img.convert(ChannelLayout::Cmyk, SampleType::U8);
        assert_eq!(c.data(), &[0, 255, 255, 0]);
    }
}
