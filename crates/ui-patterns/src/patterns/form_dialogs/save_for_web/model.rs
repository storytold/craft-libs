//! Save for Web's values: formats, palettes, dither, metadata, views, options, presets, variants, the result and animations.

use super::Last;
use super::panes::WebPane;
#[cfg(doc)]
use super::{SaveForWebDialog, four_up_variants};
use crate::patterns::form_dialogs::host_requests::PreviewRequest;
use crate::patterns::form_dialogs::sizing::Resample;
use crate::runtime::env::Label;
use crate::runtime::memory::{MemoryStage, SaveKey, unique};
use egui::Ui;
use egui::load::SizedTexture;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The formats of Save for Web.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum WebFormat {
    /// GIF, indexed colour.
    Gif,
    /// PNG with an indexed palette of at most 256 colours.
    Png8,
    /// PNG with 24-bit colour.
    Png24,
    /// JPEG.
    #[default]
    Jpeg,
    /// WebP, lossy or lossless.
    WebP,
    /// Wireless bitmap: one bit per pixel.
    Wbmp,
}

impl WebFormat {
    /// Every format, in the order of the format list.
    pub const ALL: [WebFormat; 6] = [WebFormat::Gif, WebFormat::Png8, WebFormat::Png24, WebFormat::Jpeg, WebFormat::WebP, WebFormat::Wbmp];

    /// The extension of a file in this format, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            WebFormat::Gif => "gif",
            WebFormat::Png8 | WebFormat::Png24 => "png",
            WebFormat::Jpeg => "jpg",
            WebFormat::WebP => "webp",
            WebFormat::Wbmp => "wbmp",
        }
    }

    /// The format list's entry.
    pub fn name(self) -> &'static str {
        match self {
            WebFormat::Gif => "GIF",
            WebFormat::Png8 => "PNG-8",
            WebFormat::Png24 => "PNG-24",
            WebFormat::Jpeg => "JPEG",
            WebFormat::WebP => "WebP",
            WebFormat::Wbmp => "WBMP",
        }
    }

    /// Whether the format has a palette.
    pub fn indexed(self) -> bool {
        matches!(self, WebFormat::Gif | WebFormat::Png8)
    }
}

/// How an indexed format's palette is chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ColorTable {
    /// A palette weighted toward colours the eye notices.
    Perceptual,
    /// As Perceptual, favouring broad areas of colour and web-safe colours.
    #[default]
    Selective,
    /// A palette sampled from the colours most common in the image.
    Adaptive,
    /// The 216 web-safe colours.
    Restrictive,
    /// The image's own colours, when it has at most 256.
    Exact,
}

/// How an indexed format spreads colour error.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Dither {
    /// Nearest palette colour, no dither.
    None,
    /// Error diffusion; the amount sets its strength.
    #[default]
    Diffusion,
    /// A regular halftone-like pattern.
    Pattern,
    /// Random noise.
    Noise,
}

/// What metadata goes into a web file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum WebMetadata {
    /// No metadata.
    None,
    /// The copyright notice only.
    #[default]
    Copyright,
    /// The copyright notice and contact details.
    CopyrightAndContact,
    /// All metadata.
    All,
}

/// The view tabs over the panes. 2-Up is the default; the last tab is remembered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum WebView {
    /// The document alone.
    Original,
    /// The current settings' encode alone.
    Optimized,
    /// Original beside the current settings' encode.
    #[default]
    TwoUp,
    /// Original, the current settings and two cheaper variants ([`four_up_variants`]).
    FourUp,
}

impl WebView {
    /// The panes this view shows, in reading order.
    pub fn panes(self) -> &'static [WebPane] {
        match self {
            WebView::Original => &[WebPane::Original],
            WebView::Optimized => &[WebPane::Encoded(0)],
            WebView::TwoUp => &[WebPane::Original, WebPane::Encoded(0)],
            WebView::FourUp => &[WebPane::Original, WebPane::Encoded(0), WebPane::Encoded(1), WebPane::Encoded(2)],
        }
    }
}

/// Save for Web's settings: the result, what is remembered, and what the app encodes each variant with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SaveForWebOptions {
    /// The file format.
    pub format: WebFormat,
    /// 0 to 100 (JPEG, lossy WebP).
    pub quality: u8,
    /// The palette method; used by indexed formats.
    pub color_table: ColorTable,
    /// 2 to 256 (GIF, PNG-8).
    pub colors: u16,
    /// How colour error is spread; used by indexed formats.
    pub dither: Dither,
    /// 0 to 100.
    pub dither_amount: u8,
    /// Keeps transparency where the format allows it; otherwise the matte fills it.
    pub transparency: bool,
    /// Writes an interlaced file (GIF, PNG).
    pub interlaced: bool,
    /// 0 to 100.
    pub web_snap: u8,
    /// Writes a progressive JPEG.
    pub progressive: bool,
    /// Optimizes the JPEG encoding for a smaller file.
    pub optimized: bool,
    /// Encodes WebP losslessly; the quality is then ignored.
    pub lossless: bool,
    /// Embeds the colour profile in the file.
    pub embed_profile: bool,
    /// The matte, red, green and blue.
    pub matte: [u8; 3],
    /// Converts the colours to sRGB before encoding.
    pub convert_srgb: bool,
    /// What metadata goes into the file.
    pub metadata: WebMetadata,
    /// The output size in percent of the document, 1 to 1000 with two decimals.
    pub percent: f64,
    /// How the pixels are made when the size differs from the document's.
    pub resample: Resample,
}

impl Default for SaveForWebOptions {
    /// The opening values: JPEG quality 60, Selective 128 colours, Diffusion 88 %, transparency, white matte,
    /// Optimized, Convert to sRGB, Copyright, 100 %.
    fn default() -> Self {
        Self {
            format: WebFormat::Jpeg,
            quality: 60,
            color_table: ColorTable::Selective,
            colors: 128,
            dither: Dither::Diffusion,
            dither_amount: 88,
            transparency: true,
            interlaced: false,
            web_snap: 0,
            progressive: false,
            optimized: true,
            lossless: false,
            embed_profile: false,
            matte: [255, 255, 255],
            convert_srgb: true,
            metadata: WebMetadata::Copyright,
            percent: 100.0,
            resample: Resample::Bicubic,
        }
    }
}

impl SaveForWebOptions {
    /// Whether the matte row shows: the matte fills the transparency of formats that cannot keep it.
    pub fn has_matte(&self) -> bool {
        !matches!(self.format, WebFormat::Png24 | WebFormat::WebP) || !self.transparency
    }

    /// Whether the quality row shows.
    pub fn has_quality(&self) -> bool {
        match self.format {
            WebFormat::Jpeg => true,
            WebFormat::WebP => !self.lossless,
            _ => false,
        }
    }
}

/// A named set of options in the Preset list.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct WebPreset {
    /// The name in the list.
    pub name: String,
    /// The options the preset sets.
    pub options: SaveForWebOptions,
}

impl WebPreset {
    /// A preset named `name`.
    pub fn new(name: impl Into<String>, options: SaveForWebOptions) -> Self {
        WebPreset { name: name.into(), options }
    }

    /// Whether the preset is what `current` shows apart from the size.
    pub fn is_set_in(&self, current: &SaveForWebOptions) -> bool {
        self.options == SaveForWebOptions { percent: self.options.percent, ..current.clone() }
    }
}

/// One encoded pane the app hands back for a variant the dialog asked for.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct WebVariant {
    /// Which pane: 0 is the current settings, 1 and 2 the 4-Up variants.
    pub index: usize,
    /// The revision [`WantedVariant::revision`] gave; older answers are dropped.
    pub revision: u64,
    /// The encoded picture, drawn at `size` in document pixels (a reduced proxy texture is drawn at the full size),
    /// or why the encode failed.
    pub picture: Result<SizedTexture, Label>,
    /// The encoded file size in bytes.
    pub bytes: u64,
    /// The palette size of an indexed format.
    pub colors: Option<u16>,
    /// The texture is a reduced proxy (the long side is capped at 2048 px).
    pub proxy: bool,
}

impl WebVariant {
    /// The encoded pane `index` of `revision`, shown from `texture`.
    pub fn new(index: usize, revision: u64, texture: SizedTexture, bytes: u64) -> Self {
        WebVariant { index, revision, picture: Ok(texture), bytes, colors: None, proxy: false }
    }

    /// The pane `index` of `revision` whose encode failed with `message`.
    pub fn failed(index: usize, revision: u64, message: impl Into<Label>) -> Self {
        WebVariant { index, revision, picture: Err(message.into()), bytes: 0, colors: None, proxy: false }
    }

    /// The palette size of an indexed format.
    #[must_use]
    pub fn colors(mut self, colors: u16) -> Self {
        self.colors = Some(colors);
        self
    }

    /// Marks the texture as a reduced proxy.
    #[must_use]
    pub fn proxy(mut self, proxy: bool) -> Self {
        self.proxy = proxy;
        self
    }
}

/// A pane the dialog wants encoded.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct WantedVariant {
    /// Which pane: 0 is the current settings, 1 and 2 the 4-Up variants.
    pub index: usize,
    /// The options to encode with and the revision to answer with.
    pub request: PreviewRequest<SaveForWebOptions>,
}

/// What Save for Web returns.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SaveForWebResult {
    /// The settings the user confirmed.
    pub options: SaveForWebOptions,
    /// The file the user chose to write.
    pub path: PathBuf,
    /// The output width in pixels.
    pub width: u32,
    /// The output height in pixels.
    pub height: u32,
    pub(super) remember: Option<(SaveKey, Last)>,
}

impl SaveForWebResult {
    /// Stores the settings and the view for the next open of a dialog built with
    /// [`SaveForWebDialog::remember`]. The app calls it after the file is written; without a remember key it does
    /// nothing.
    pub fn remember(&self, ui: &Ui) {
        if let Some((key, last)) = &self.remember {
            let mut stage = MemoryStage::open(ui.ctx(), unique(("craft_ui::save_for_web_result", key.as_str())));
            *stage.saved::<Option<Last>>(key) = Some(last.clone());
            stage.write_back();
        }
    }
}

/// An animation: the panes show one frame, and Play steps through them.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Animation {
    /// The number of frames; fewer than two shows no play row.
    pub frames: u32,
    /// Frames per second; 30 when it is not positive.
    pub fps: f32,
}

impl Animation {
    /// An animation of `frames` frames at `fps`.
    pub fn new(frames: u32, fps: f32) -> Self {
        Animation { frames, fps }
    }
}
