//! The export dialogs of the form-dialogs family: Export As, the format options asked after the file name is chosen,
//! and the output path row. The dialogs take plain format descriptions and settings from the app and give settings and
//! a path back; encoding stays in the app.
//!
//! [`export_file_name`] is the pure file-name rule the export dialogs share.

pub(crate) mod as_dialog;
pub(crate) mod options;
pub(crate) mod path_row;

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::PreviewRequest;
use crate::contract::SaveKey;
use crate::runtime::env::Label;
use craft_behavior::PixelSize;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub use as_dialog::{ExportAsDialog, ExportAsOutput, HostedExportAs};
pub use options::{
    DdsCompression, DdsOptions, ExportOptionsKind, FormatOptions, FormatOptionsDialog, FormatOptionsOutput, FormatOptionsWords, HostedFormatOptions,
    JpegFormat, JpegOptions, JpegPreset, TargaOptions, TiffCompression, TiffOptions, jpeg_quality_percent,
};
pub use path_row::{OutputPathOutput, OutputPathRow, OutputPathWords};

use super::{MAX_DOCUMENT_SIDE, Resample, SizeError, resize_from_scale, resize_from_side};

craft_ui::words! {
    /// The words of Export As, beside the common OK and Cancel.
    pub struct ExportWords {
        title: "export.title" = "Export As";
        export: "export.export" = "Export…";
        file_settings: "export.fileSettings" = "File Settings";
        format: "export.format" = "Format";
        quality: "export.quality" = "Quality";
        lossless: "export.lossless" = "Lossless";
        transparency: "export.transparency" = "Transparency";
        image_size: "export.imageSize" = "Image Size";
        scale: "export.scale" = "Scale";
        width: "export.width" = "Width";
        height: "export.height" = "Height";
        constrain: "export.constrain" = "Constrain Proportions";
        metadata: "export.metadata" = "Metadata";
        metadata_none: "export.metadataNone" = "None";
        metadata_exif: "export.metadataExif" = "EXIF";
        metadata_all: "export.metadataAll" = "All";
        color_space: "export.colorSpace" = "Color Space";
        document_profile: "export.documentProfile" = "Document Profile";
        convert_srgb: "export.convertSrgb" = "Convert to sRGB";
        embed_profile: "export.embedProfile" = "Embed Color Profile";
        bits: "export.bits" = "Bits/Pixel";
        auto: "export.auto" = "Auto";
        preview: "export.preview" = "Preview";
        ico_limit: "export.icoLimit" = "ICO is limited to 256 × 256 pixels";
        scale_range: "export.scaleRange" = "Scale must be within 1–1000 percent";
        pixel_range: "export.pixelRange" = "Export size must be within 1–300,000 pixels";
        no_estimate: "export.noEstimate" = "—";
        estimating: "export.estimating" = "…";
        untitled: "export.untitled" = "Untitled";
        scale_all: "export.scaleAll" = "Scale All";
        add_size: "export.addSize" = "Add a size";
        /// `{n}` is the row's number.
        size_n: template "export.sizeN" = "Size {n}";
        suffix: "export.suffix" = "Suffix";
        /// `{n}` is the row's number.
        suffix_n: template "export.suffixN" = "Suffix {n}";
        /// `{n}` is the row's number.
        remove_size: template "export.removeSize" = "Remove size {n}";
        folder_needed: "export.folderNeeded" = "Exporting several sizes at once needs a folder, which this build can't write to; export one size at a time";
        preset: "export.preset" = "Preset";
        new_preset_name: "export.newPresetName" = "Export Preset";
        /// `{size}`: the estimated file size, shown while the exact one is computed.
        estimate: template "export.estimate" = "≈ {size}";
        /// `{width}` and `{height}`: the output size in pixels.
        dimensions: template "export.dimensions" = "{width} × {height} px";
        /// `{format}`: the format's name, `{estimate}`: the size estimate or its placeholder.
        preview_line: template "export.previewLine" = "{format}  {estimate}";
    }
}

/// A format Export As offers, described by the app (the dialogs never encode).
#[derive(Clone, Debug, PartialEq, crate::contract::Props)]
#[non_exhaustive]
pub struct ExportFormat {
    /// Stable id, also the settings value: `"png"`.
    pub id: &'static str,
    /// The list's label: "PNG".
    pub name: &'static str,
    /// The extension a picked file gets, without the dot.
    pub extension: &'static str,
    /// Shows Quality (JPG, lossy WebP).
    #[prop]
    pub quality: bool,
    /// Shows Lossless (WebP); Quality hides while it is checked.
    #[prop]
    pub lossless: bool,
    /// Shows Transparency.
    #[prop]
    pub transparency: bool,
    /// Shows Bits/Pixel with these choices besides Auto (TGA: 24, 32); empty for no such row.
    #[prop]
    pub bits: &'static [u8],
    /// The largest side the format can hold (ICO: 256); the dialog warns under the size when the output is larger.
    #[prop(quantity = "document pixels")]
    pub max_side: Option<u32>,
}

impl ExportFormat {
    /// A format with no options: `id` is also its extension.
    pub fn new(id: &'static str, name: &'static str) -> Self {
        ExportFormat { id, name, extension: id, quality: false, lossless: false, transparency: false, bits: &[], max_side: None }
    }

    /// PNG, JPG, WebP, TIFF, TGA and ICO.
    pub fn standard() -> Vec<ExportFormat> {
        const TGA_BITS: &[u8] = &[24, 32];
        const ICO_SIDE: u32 = 256;
        vec![
            ExportFormat::new("png", "PNG").transparency(true),
            ExportFormat::new("jpg", "JPG").quality(true),
            ExportFormat::new("webp", "WebP").quality(true).lossless(true).transparency(true),
            ExportFormat::new("tif", "TIFF").transparency(true),
            ExportFormat::new("tga", "TGA").transparency(true).bits(TGA_BITS),
            ExportFormat::new("ico", "ICO").transparency(true).max_side(ICO_SIDE),
        ]
    }
}

/// How the output size is given.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ExportSize {
    /// Percent of the document, 1 to 1000.
    Scale(f64),
    /// Pixel width and height (any size the aspect lock allows).
    #[non_exhaustive]
    Pixels {
        /// The width in pixels.
        width: u32,
        /// The height in pixels.
        height: u32,
    },
}

impl ExportSize {
    /// A size in pixels.
    pub fn pixels(width: u32, height: u32) -> Self {
        ExportSize::Pixels { width, height }
    }
}

/// What metadata goes into the file. `None` also drops the resolution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ExportMetadata {
    /// Writes no metadata.
    #[default]
    None,
    /// Writes the EXIF block only.
    Exif,
    /// Writes all metadata.
    All,
}

/// The colour profile of the file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ExportProfile {
    /// Uses the document's own profile.
    Document,
    /// Converts to sRGB.
    #[default]
    Srgb,
}

/// A Scale All size: a multiple of the export size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ScaleSize {
    #[allow(missing_docs)]
    #[serde(rename = "0.1x")]
    X0_1,
    #[allow(missing_docs)]
    #[serde(rename = "0.25x")]
    X0_25,
    #[allow(missing_docs)]
    #[serde(rename = "0.3x")]
    X0_3,
    #[allow(missing_docs)]
    #[serde(rename = "0.33x")]
    X0_33,
    #[allow(missing_docs)]
    #[serde(rename = "0.5x")]
    X0_5,
    #[allow(missing_docs)]
    #[serde(rename = "0.75x")]
    X0_75,
    /// The export size itself.
    #[default]
    #[serde(rename = "1x")]
    X1,
    #[allow(missing_docs)]
    #[serde(rename = "1.25x")]
    X1_25,
    #[allow(missing_docs)]
    #[serde(rename = "1.5x")]
    X1_5,
    #[allow(missing_docs)]
    #[serde(rename = "2x")]
    X2,
    #[allow(missing_docs)]
    #[serde(rename = "3x")]
    X3,
}

impl ScaleSize {
    /// Every size in the list's order.
    pub const ALL: [ScaleSize; 11] = [
        ScaleSize::X0_1,
        ScaleSize::X0_25,
        ScaleSize::X0_3,
        ScaleSize::X0_33,
        ScaleSize::X0_5,
        ScaleSize::X0_75,
        ScaleSize::X1,
        ScaleSize::X1_25,
        ScaleSize::X1_5,
        ScaleSize::X2,
        ScaleSize::X3,
    ];

    /// The sizes the add button gives, in turn: the first one no row has yet (0.3x is never added).
    pub const ADD_ORDER: [ScaleSize; 9] = [
        ScaleSize::X0_5,
        ScaleSize::X0_75,
        ScaleSize::X0_25,
        ScaleSize::X2,
        ScaleSize::X0_33,
        ScaleSize::X1_5,
        ScaleSize::X0_1,
        ScaleSize::X3,
        ScaleSize::X1_25,
    ];

    /// The multiple of the export size.
    pub fn factor(self) -> f64 {
        match self {
            ScaleSize::X0_1 => 0.1,
            ScaleSize::X0_25 => 0.25,
            ScaleSize::X0_3 => 0.3,
            ScaleSize::X0_33 => 0.33,
            ScaleSize::X0_5 => 0.5,
            ScaleSize::X0_75 => 0.75,
            ScaleSize::X1 => 1.0,
            ScaleSize::X1_25 => 1.25,
            ScaleSize::X1_5 => 1.5,
            ScaleSize::X2 => 2.0,
            ScaleSize::X3 => 3.0,
        }
    }

    /// The size as the list shows it and a suffix names it: `0.5x`, `2x`.
    pub fn name(self) -> &'static str {
        match self {
            ScaleSize::X0_1 => "0.1x",
            ScaleSize::X0_25 => "0.25x",
            ScaleSize::X0_3 => "0.3x",
            ScaleSize::X0_33 => "0.33x",
            ScaleSize::X0_5 => "0.5x",
            ScaleSize::X0_75 => "0.75x",
            ScaleSize::X1 => "1x",
            ScaleSize::X1_25 => "1.25x",
            ScaleSize::X1_5 => "1.5x",
            ScaleSize::X2 => "2x",
            ScaleSize::X3 => "3x",
        }
    }
}

/// One Scale All row: a size and the suffix its file name takes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ScaleRow {
    /// The multiple of the export size.
    pub size: ScaleSize,
    /// What the file name of this size ends with, before the extension; empty for none.
    pub suffix: String,
}

impl ScaleRow {
    /// The most rows Scale All holds.
    pub const MAX_ROWS: usize = 10;
    /// The most characters of a suffix.
    pub const MAX_SUFFIX: usize = 64;

    /// A row of `size` with `suffix`.
    pub fn new(size: ScaleSize, suffix: impl Into<String>) -> Self {
        ScaleRow { size, suffix: suffix.into() }
    }

    /// The row the add button appends to `rows`: the next size of [`ScaleSize::ADD_ORDER`] no row has, with the suffix
    /// `@<size>`; `None` at [`MAX_ROWS`](Self::MAX_ROWS) or when every size is taken.
    pub fn next(rows: &[ScaleRow]) -> Option<ScaleRow> {
        if rows.len() >= Self::MAX_ROWS {
            return None;
        }
        let size = ScaleSize::ADD_ORDER.into_iter().find(|size| rows.iter().all(|row| row.size != *size))?;
        Some(ScaleRow::new(size, format!("@{}", size.name())))
    }
}

/// One file an export writes: its size and the suffix its name takes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExportOutput {
    /// The output size in pixels.
    pub size: PixelSize,
    /// What the file name ends with, before the extension; empty for none.
    pub suffix: String,
}

/// Export As's values: the dialog's result and what the app keeps between opens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExportOptions {
    /// An [`ExportFormat::id`].
    pub format: String,
    /// 1 to 100.
    pub quality: u8,
    /// Lossless encoding; applies to WebP.
    pub lossless: bool,
    /// Keeps the alpha channel for formats that support it.
    pub transparency: bool,
    /// The size to export at.
    pub size: ExportSize,
    /// Keeps the document's aspect ratio when one of the pixel dimensions is edited.
    pub lock_aspect: bool,
    /// What metadata goes into the file.
    pub metadata: ExportMetadata,
    /// The colour profile of the file.
    pub profile: ExportProfile,
    /// Embeds the colour profile in the file.
    pub embed_profile: bool,
    /// Bits per pixel; 0 is Auto.
    pub bits: u8,
    /// How the pixels are made when the output size differs from the document's.
    #[serde(default = "default_resample")]
    pub resample: Resample,
    /// Scale All: one file per row, each at its multiple of the export size. Empty reads as one 1x row with no
    /// suffix.
    #[serde(default)]
    pub scale_all: Vec<ScaleRow>,
}

impl Default for ExportOptions {
    /// PNG at 100 percent, aspect locked, quality 85, converted to sRGB with the profile embedded, no metadata.
    fn default() -> Self {
        Self {
            format: "png".to_owned(),
            quality: ExportFormatDefaults::default().jpeg_quality,
            lossless: false,
            transparency: true,
            size: ExportSize::Scale(FULL_SCALE),
            lock_aspect: true,
            metadata: ExportMetadata::None,
            profile: ExportProfile::Srgb,
            embed_profile: true,
            bits: 0,
            resample: default_resample(),
            scale_all: Vec::new(),
        }
    }
}

/// The method a settings value saved without one reads as.
fn default_resample() -> Resample {
    Resample::Automatic
}

/// The scale of the document's own size, in percent.
const FULL_SCALE: f64 = 100.0;
/// The least and greatest quality.
const QUALITY: std::ops::RangeInclusive<u8> = 1..=100;
/// The least side of an output, in pixels.
const MIN_SIDE: f64 = 1.0;

impl ExportOptions {
    /// The options as the controls can hold them for a document of size `source`: a format the list has (the first
    /// when not), a resample method of `methods` (Automatic when listed, else the first; any method when `methods` is empty), a quality
    /// of 1 to 100, and a size the document can take. A linked pixel width keeps its width and
    /// takes the height from this document (the last export's proportions would stretch another shape); unlinked
    /// pixels stay as asked when in range; anything out of range becomes 100 percent.
    #[must_use]
    pub fn fitted(mut self, formats: &[ExportFormat], methods: &[Resample], source: PixelSize) -> Self {
        if let Some(first) = formats.first()
            && !formats.iter().any(|format| format.id == self.format)
        {
            self.format = first.id.to_owned();
        }
        if !methods.is_empty() && !methods.contains(&self.resample) {
            self.resample = if methods.contains(&default_resample()) { default_resample() } else { methods[0] };
        }
        self.quality = self.quality.clamp(*QUALITY.start(), *QUALITY.end());
        let in_range = |side: u32| (MIN_SIDE..=MAX_DOCUMENT_SIDE).contains(&f64::from(side));
        self.size = match self.size {
            ExportSize::Pixels { width, height } if !self.lock_aspect && in_range(width) && in_range(height) => self.size,
            ExportSize::Pixels { width, .. } => match resize_from_side(source, f64::from(width), 0.0, true, true) {
                Ok((size, scale)) if (1.0..=1000.0).contains(&scale) => ExportSize::pixels(size.width, size.height),
                _ => ExportSize::Scale(FULL_SCALE),
            },
            ExportSize::Scale(scale) if resize_from_scale(source, scale).is_ok() => ExportSize::Scale(scale),
            ExportSize::Scale(_) => ExportSize::Scale(FULL_SCALE),
        };
        self
    }

    /// The output size in pixels for a document of size `source`.
    pub fn output_size(&self, source: PixelSize) -> PixelSize {
        match self.size {
            ExportSize::Scale(scale) => resize_from_scale(source, scale).unwrap_or(source),
            ExportSize::Pixels { width, height } => PixelSize::new(width, height),
        }
    }

    /// Scale All's rows: the set ones, or one 1x row with no suffix.
    pub fn scale_rows(&self) -> Vec<ScaleRow> {
        if self.scale_all.is_empty() { vec![ScaleRow::default()] } else { self.scale_all.clone() }
    }

    /// The files an export of a document of size `source` writes: one per Scale All row, each side the output size's
    /// times the row's multiple, rounded, in 1 to the largest document side.
    pub fn outputs(&self, source: PixelSize) -> Vec<ExportOutput> {
        let size = self.output_size(source);
        let side = |n: u32, factor: f64| (f64::from(n) * factor).round().clamp(1.0, MAX_DOCUMENT_SIDE) as u32;
        self.scale_rows()
            .into_iter()
            .map(|row| ExportOutput {
                size: PixelSize::new(side(size.width, row.size.factor()), side(size.height, row.size.factor())),
                suffix: row.suffix,
            })
            .collect()
    }
}

/// The file size line under the preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SizeEstimate {
    /// Scaled from an encode of the downscaled proxy: shown with "≈".
    Proxy(u64),
    /// From an encode of the full image.
    Exact(u64),
}

/// The quality and WebP mode a format takes when the user switches to it, from the app's Export preferences. Qualities
/// are 1 to 100.
#[derive(Clone, Copy, Debug, PartialEq, Eq, crate::contract::Props)]
#[non_exhaustive]
pub struct ExportFormatDefaults {
    /// Quality when JPG is chosen.
    #[prop]
    pub jpeg_quality: u8,
    /// Quality when WebP is chosen.
    #[prop]
    pub webp_quality: u8,
    /// Lossless when WebP is chosen.
    #[prop]
    pub webp_lossless: bool,
}

impl Default for ExportFormatDefaults {
    /// Quality 85 for both, lossless WebP.
    fn default() -> Self {
        Self { jpeg_quality: 85, webp_quality: 85, webp_lossless: true }
    }
}

/// The preview and estimate the app computes for one revision of the settings. The dialog reports each revision it
/// wants in `ExportAsOutput::wanted_preview`; the app gives the answer back with `ExportAsDialog::preview` for as long
/// as it is the newest. An answer for a revision older than the one last asked for is dropped.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ExportPreview {
    /// The revision of `wanted_preview` this answers.
    pub revision: u64,
    /// What the preview pane shows: the encoded proxy, or that it is still being made or failed.
    pub state: super::PreviewState,
    /// The file size estimate for these settings; `None` until the app has one.
    pub estimate: Option<SizeEstimate>,
}

impl ExportPreview {
    /// The answer for `revision`.
    pub fn new(revision: u64, state: super::PreviewState) -> Self {
        ExportPreview { revision, state, estimate: None }
    }

    /// The same answer with the size estimate of the settings.
    #[must_use]
    pub fn estimate(mut self, estimate: SizeEstimate) -> Self {
        self.estimate = Some(estimate);
        self
    }
}

/// What Export As answers with at Export: the options, the chosen path and the output size.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ExportAsResult {
    /// The settings the user confirmed.
    pub options: ExportOptions,
    /// The file the user chose to write; the folder when the export writes more than one file.
    pub path: PathBuf,
    /// The output size in pixels.
    pub size: PixelSize,
    /// The files to write ([`ExportOptions::outputs`]). With more than one, `path` is the folder the user chose and
    /// the app names each file `<name><suffix>.<extension>`.
    pub outputs: Vec<ExportOutput>,
    remember: Option<(SaveKey, ExportOptions)>,
}

impl ExportAsResult {
    /// Stores the settings of a dialog built with [`ExportAsDialog::remember`] for the next open. The app calls it
    /// after its export succeeds, so a failed export leaves the previous settings; without a key it does nothing.
    pub fn remember(&self, ctx: &egui::Context) {
        if let Some((key, options)) = &self.remember {
            crate::runtime::memory::set_saved_value::<Option<ExportOptions>>(ctx, key, Some(options.clone()));
        }
    }
}

/// The numbering of the settings an app computes something for (a preview, a size estimate): a new revision for each
/// change of the settings, reported once, and the rule that decides whether an answer is still wanted.
#[derive(Clone)]
struct Revisions<T> {
    /// The newest revision of the settings.
    revision: u64,
    /// The settings `revision` numbers.
    requested: Option<T>,
    /// The newest revision reported by [`request`](Revisions::request).
    taken: u64,
}

impl<T> Default for Revisions<T> {
    fn default() -> Self {
        Revisions { revision: 0, requested: None, taken: 0 }
    }
}

impl<T: Clone + PartialEq> Revisions<T> {
    /// Numbers the settings anew when they differ from the ones the newest revision stands for, and returns the request
    /// when the revision was not reported yet.
    fn request(&mut self, settings: &T) -> Option<PreviewRequest<T>> {
        if self.requested.as_ref() != Some(settings) {
            self.revision += 1;
            self.requested = Some(settings.clone());
        }
        (self.revision != self.taken).then(|| {
            self.taken = self.revision;
            PreviewRequest::new(self.revision, settings.clone())
        })
    }

    /// Whether an answer for `revision` is not older than the newest one reported.
    fn adopts(&self, revision: u64) -> bool {
        revision >= self.taken
    }
}

/// The file name a save dialog suggests for `name`: each control character and each character a file name can't hold
/// (`< > : " / \ | ? *`) turned into `_`, at most 80 characters; spaces and dots at the ends stay as the name holds
/// them. `fallback` for a name of nothing but whitespace and dots.
pub(crate) fn export_file_name(name: &str, fallback: &str) -> String {
    let name: String = name.chars().take(MAX_NAME_CHARS).map(|c| if c.is_control() || RESERVED.contains(c) { '_' } else { c }).collect();
    if name.chars().all(|c| c.is_whitespace() || c == '.') { fallback.to_owned() } else { name }
}

const MAX_NAME_CHARS: usize = 80;
const RESERVED: &str = "<>:\"/\\|?*";

/// The words for a size error.
pub(crate) fn size_error_label(words: &ExportWords, error: SizeError) -> Label {
    match error {
        SizeError::Scale => words.scale_range.clone(),
        _ => words.pixel_range.clone(),
    }
}
