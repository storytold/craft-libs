//! The export dialogs of the form-dialogs family: Export As, the format options asked after the file name is chosen,
//! and the output path row. The dialogs take plain format descriptions and settings from the app and give settings and
//! a path back; encoding stays in the app.
//!
//! [`export_file_name`] is the pure file-name rule the export dialogs share.

mod as_dialog;
mod options;
mod path_row;

#[cfg(feature = "testing")]
mod stories;

use crate::runtime::env::Label;
use craft_behavior::PixelSize;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub use as_dialog::{ExportAsDialog, ExportAsOutput};
pub use options::{
    DdsCompression, DdsOptions, ExportOptionsKind, FormatOptions, FormatOptionsDialog, FormatOptionsOutput, FormatOptionsWords, JpegFormat,
    JpegOptions, JpegPreset, TargaOptions, TiffOptions, jpeg_quality_percent,
};
pub use path_row::{OutputPathOutput, OutputPathRow, OutputPathWords};

use super::{MAX_DOCUMENT_SIDE, Resample, SizeError, resize_from_scale, resize_from_side};

craft_ui::words! {
    /// The words of Export As, beside the common OK and Cancel.
    pub struct ExportWords {
        title: "export.title" = "Export As";
        export: "export.export" = "Export…";
        format: "export.format" = "Format";
        formats: "export.formats" = "Formats";
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
        untitled: "export.untitled" = "Untitled";
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
    #[default]
    Document,
    /// Converts to sRGB.
    Srgb,
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
}

impl Default for ExportOptions {
    /// PNG at 100 percent, aspect locked, quality 85, document profile embedded, no metadata.
    fn default() -> Self {
        Self {
            format: "png".to_owned(),
            quality: ExportFormatDefaults::default().jpeg_quality,
            lossless: false,
            transparency: true,
            size: ExportSize::Scale(FULL_SCALE),
            lock_aspect: true,
            metadata: ExportMetadata::None,
            profile: ExportProfile::Document,
            embed_profile: true,
            bits: 0,
            resample: default_resample(),
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
    /// when not), a quality of 1 to 100, and a size the document can take. A linked pixel width keeps its width and
    /// takes the height from this document (the last export's proportions would stretch another shape); unlinked
    /// pixels stay as asked when in range; anything out of range becomes 100 percent.
    #[must_use]
    pub fn fitted(mut self, formats: &[ExportFormat], source: PixelSize) -> Self {
        if let Some(first) = formats.first()
            && !formats.iter().any(|format| format.id == self.format)
        {
            self.format = first.id.to_owned();
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
    /// The file the user chose to write.
    pub path: PathBuf,
    /// The output size in pixels.
    pub size: PixelSize,
}

/// A file name safe on every platform: at most 80 characters, control characters and `< > : " / \ | ? *` replaced by
/// `_`, leading and trailing dots and spaces trimmed; empty becomes `fallback` ("layer" for per-layer exports, the
/// document name otherwise).
pub(crate) fn export_file_name(name: &str, fallback: &str) -> String {
    let name: String = name.chars().take(MAX_NAME_CHARS).map(|c| if c.is_control() || RESERVED.contains(c) { '_' } else { c }).collect();
    let name = name.trim().trim_matches('.');
    if name.is_empty() { fallback.to_owned() } else { name.to_owned() }
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
