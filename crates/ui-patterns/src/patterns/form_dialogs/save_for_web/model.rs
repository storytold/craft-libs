//! Save for Web's values: formats, palettes, dither, metadata, views, options, presets, variants, the result and animations.

use super::panes::WebPane;
use super::{Last, percent_of};
#[cfg(doc)]
use super::{SaveForWebDialog, four_up_variants};
use crate::patterns::form_dialogs::host_requests::PreviewRequest;
use crate::patterns::form_dialogs::{LinkedSize, MAX_DOCUMENT_SIDE, Resample, resize_from_scale};
use crate::runtime::env::Label;
use crate::runtime::env::UnitKind;
use crate::runtime::memory::SaveKey;
use craft_behavior::{PixelSize, Selection, SelectionChange, SelectionModel};
use egui::Context;
use egui::load::SizedTexture;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Loads both sides in pixels without changing the unit shown in the fields.
pub(super) fn set_pixels(link: &mut LinkedSize, size: PixelSize) {
    let unit = link.unit();
    let linked = link.is_linked();
    link.set_unit(UnitKind::Pixels);
    link.lock(false);
    link.set_width(f64::from(size.width));
    link.set_height(f64::from(size.height));
    link.lock(linked);
    link.set_unit(unit);
}

/// The formats of Save for Web.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// AVIF, lossy. The list offers it only when the host says it can write it ([`SaveForWebDialog::formats`]).
    Avif,
    /// Wireless bitmap: one bit per pixel.
    Wbmp,
}

impl WebFormat {
    /// Every format, in the order of the format list.
    pub const ALL: [WebFormat; 7] =
        [WebFormat::Gif, WebFormat::Png8, WebFormat::Png24, WebFormat::Jpeg, WebFormat::WebP, WebFormat::Avif, WebFormat::Wbmp];

    /// The formats the list offers unless the host names its own: all but AVIF, which needs an encoder not every
    /// app builds with.
    pub const STANDARD: [WebFormat; 6] = [WebFormat::Gif, WebFormat::Png8, WebFormat::Png24, WebFormat::Jpeg, WebFormat::WebP, WebFormat::Wbmp];

    /// The formats a host that names `formats` offers: an empty list names none, so it is [`WebFormat::STANDARD`].
    pub(super) fn offered(formats: &[WebFormat]) -> &[WebFormat] {
        if formats.is_empty() { &Self::STANDARD } else { formats }
    }

    /// The extension of a file in this format, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            WebFormat::Gif => "gif",
            WebFormat::Png8 | WebFormat::Png24 => "png",
            WebFormat::Jpeg => "jpg",
            WebFormat::WebP => "webp",
            WebFormat::Avif => "avif",
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
            WebFormat::Avif => "AVIF",
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
#[serde(rename_all = "camelCase")]
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
    /// Black and white.
    BlackWhite,
    /// Evenly spaced greys.
    Grayscale,
}

/// How an indexed format spreads colour error.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum WebView {
    /// The document alone.
    Original,
    /// The current settings' encode alone.
    Optimized,
    /// Original beside the current settings' encode.
    #[default]
    TwoUp,
    /// Original, the current settings and two cheaper variants (`four_up_variants`).
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

/// The anti-aliasing of a vector document's raster output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum WebAntiAlias {
    /// No anti-aliasing.
    None,
    /// Art optimised (supersampled).
    #[default]
    Art,
    /// Type optimised.
    Type,
}

/// The Download select's speeds in Kbps, one list for every app; 56.6 is the opening speed.
pub const WEB_SPEEDS: [f64; 8] = [28.8, 56.6, 128.0, 256.0, 512.0, 1024.0, 2048.0, 8192.0];

/// One table edit on the selected colours: the host applies it to its table model.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ColorTableEdit {
    /// Locks the colours.
    Lock,
    /// Makes the colours transparent.
    Transparent,
    /// Shifts the colours to their web-safe colours.
    WebShift,
    /// Shifts the colours to the colour `to`, the colour the palette answered with.
    Shift([u8; 3]),
    /// Unshifts the colours.
    Unshift,
    /// Unshifts every colour.
    UnshiftAll,
    /// Deletes the colours.
    Delete,
    /// Sorts the table.
    Sort(TableOrder),
    /// Restores the generated table.
    Restore,
}

/// The order a colour table sorts in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TableOrder {
    /// The order the table was generated in.
    #[default]
    Unsorted,
    /// By hue.
    Hue,
    /// By luminance.
    Luminance,
    /// By how many pixels use the colour.
    Popularity,
}

/// A table edit and the colours it applies to.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ColorTableEvent {
    /// The edit.
    pub edit: ColorTableEdit,
    /// The selected colours, by colour, so the host skips those the shown table no longer has.
    pub colors: Vec<[u8; 3]>,
}

/// What the host's table says, for the cell marks (web-safe is computed).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct TableMarks {
    /// The locked colours.
    pub locked: Vec<[u8; 3]>,
    /// The transparent colours.
    pub transparent: Vec<[u8; 3]>,
    /// The shifted colours, from and to.
    pub shifted: Vec<([u8; 3], [u8; 3])>,
}

/// What the dialog is for: a raster document or a vector one.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum WebDocument {
    /// A raster document: Image Size shows the resample method.
    #[default]
    Raster,
    /// A vector document: Image Size shows Anti-aliasing, Clip to Artboard and the Artboard select in place of
    /// Resample; an Output row follows Metadata; Slices has no HTML check.
    #[non_exhaustive]
    Vector {
        /// The artboards' names.
        artboards: Vec<String>,
    },
}

impl WebDocument {
    /// A vector document with the artboards `artboards`.
    pub fn vector(artboards: Vec<String>) -> Self {
        WebDocument::Vector { artboards }
    }
}

/// The search Optimize to File Size… asks the host to run.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct OptimizeRequest {
    /// The desired file size in K.
    pub desired_k: f64,
    /// Where the search starts.
    pub start: OptimizeStart,
    /// Which slices it covers.
    pub scope: OptimizeScope,
    /// The options the search starts from.
    pub options: SaveForWebOptions,
}

/// Where an optimize search starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum OptimizeStart {
    /// The current settings.
    #[default]
    Current,
    /// An automatically selected GIF or JPEG.
    Auto,
}

/// Which slices an optimize search covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum OptimizeScope {
    /// Every slice, together.
    #[default]
    AllSlices,
    /// Every slice by itself.
    EachSlice,
    /// The current slice.
    CurrentSlice,
}

/// The host's answer to an [`OptimizeRequest`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct OptimizeAnswer {
    /// The options found, taken when given.
    pub options: Option<SaveForWebOptions>,
    /// The size the found options give, in bytes.
    pub bytes: u64,
    /// The size asked for, in bytes.
    pub target_bytes: u64,
    /// Whether the found options fit the target.
    pub achieved: bool,
}

impl OptimizeAnswer {
    /// An answer with the found `options`, their size `bytes`, the `target_bytes` and whether it was `achieved`.
    pub fn new(options: Option<SaveForWebOptions>, bytes: u64, target_bytes: u64, achieved: bool) -> Self {
        OptimizeAnswer { options, bytes, target_bytes, achieved }
    }
}

/// What a preset row asked of the host; the host stores the presets and gives the new list back.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum WebPresetEvent {
    /// The current options were saved under a new name.
    Saved(WebPreset),
    /// A saved preset was renamed.
    #[non_exhaustive]
    Renamed {
        /// The old name.
        from: String,
        /// The new name.
        to: String,
    },
    /// A saved preset was deleted.
    Deleted(String),
}

/// Save for Web's settings: the result, what is remembered, and what the app encodes each variant with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SaveForWebOptions {
    /// The file format.
    pub format: WebFormat,
    /// 0 to 100 (JPEG, lossy WebP, AVIF).
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
    /// How much a GIF is allowed to lose to shrink, 0 to 100.
    #[serde(default)]
    pub lossy: u8,
    /// The anti-aliasing of a vector document's output.
    #[serde(default)]
    pub anti_alias: WebAntiAlias,
    /// A vector document's output is clipped to the artboard.
    #[serde(default)]
    pub clip_to_artboard: bool,
    /// The artboard a clipped vector document is written for.
    #[serde(default)]
    pub artboard: Option<usize>,
    /// The matte, red, green and blue.
    pub matte: [u8; 3],
    /// Converts the colours to sRGB before encoding.
    pub convert_srgb: bool,
    /// What metadata goes into the file.
    pub metadata: WebMetadata,
    /// The output size in percent of the document, 1 to 1000 with two decimals.
    pub percent: f64,
    /// Exact output dimensions after editing either side, when those take precedence over percent.
    #[serde(default)]
    pub size: Option<PixelSize>,
    /// Makes the other side follow the document's proportions when one side is edited.
    #[serde(default = "constrain_default")]
    pub constrain_proportions: bool,
    /// How the pixels are made when the size differs from the document's.
    pub resample: Resample,
    /// Which slices of the document the file holds.
    #[serde(default)]
    pub slices: WebSlices,
    /// Whether the export writes an HTML file beside the images.
    #[serde(default = "html_default")]
    pub html: bool,
}

fn constrain_default() -> bool {
    true
}

/// The serde default of [`SaveForWebOptions::html`]: on.
fn html_default() -> bool {
    true
}

/// Which slices of the document Save for Web writes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum WebSlices {
    /// The whole image as one file.
    #[default]
    All,
    /// The user slices.
    User,
    /// The slices with these numbers.
    Selected(Vec<u32>),
}

impl WebSlices {
    /// The numbers of the selected slices; none unless the choice is `Selected`.
    pub(super) fn selected(&self) -> &[u32] {
        match self {
            WebSlices::Selected(numbers) => numbers,
            _ => &[],
        }
    }

    /// A click on slice `number`: it becomes the one selected slice, or with `add` (Shift) it is added to the
    /// selection or, when already selected, removed from it.
    pub(super) fn select(&mut self, number: u32, add: bool) {
        let items = self.selected().to_vec();
        let last = items.last().copied();
        let mut selection = Selection::new(items, last, last);
        selection.apply(if add { SelectionChange::Toggle(number) } else { SelectionChange::Set(vec![number]) });
        *self = WebSlices::Selected(selection.items);
    }
}

/// A slice of the document: its number and its rectangle in document pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WebSlice {
    /// The slice's number.
    pub number: u32,
    /// The slice's rectangle in document pixels.
    pub rect: egui::Rect,
}

impl WebSlice {
    /// The slice numbered `number` over `rect`.
    pub const fn new(number: u32, rect: egui::Rect) -> Self {
        Self { number, rect }
    }
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
            lossy: 0,
            anti_alias: WebAntiAlias::default(),
            clip_to_artboard: false,
            artboard: None,
            matte: [255, 255, 255],
            convert_srgb: true,
            metadata: WebMetadata::Copyright,
            percent: 100.0,
            size: None,
            constrain_proportions: true,
            resample: Resample::Bicubic,
            slices: WebSlices::All,
            html: true,
        }
    }
}

impl SaveForWebOptions {
    /// The output size of the current document, including percent-only settings from existing callers.
    pub(super) fn output_size(&self, original: PixelSize) -> PixelSize {
        self.size.or_else(|| resize_from_scale(original, self.percent).ok()).unwrap_or(original)
    }

    /// Keeps supplied exact dimensions within the same bounds as the width and height fields.
    pub(super) fn clamp_exact_size(&mut self, original: PixelSize) {
        let Some(size) = self.size else { return };
        let clamp = |side: u32| side.clamp(1, MAX_DOCUMENT_SIDE as u32);
        let clamped = PixelSize::new(clamp(size.width), clamp(size.height));
        if clamped != size {
            self.size = Some(clamped);
            self.percent = percent_of(clamped, original);
        }
    }

    pub(super) fn apply_size(&mut self, link: &mut LinkedSize) {
        self.clamp_exact_size(link.original());
        let size = self.output_size(link.original());
        if size != link.pixels() {
            set_pixels(link, size);
        }
        link.lock(self.constrain_proportions);
    }

    pub(super) fn set_exact_size(&mut self, link: &LinkedSize) {
        self.size = Some(link.pixels());
        self.percent = percent_of(link.pixels(), link.original());
    }

    pub(super) fn constrain(&mut self, link: &mut LinkedSize, constrain: bool) {
        self.constrain_proportions = constrain;
        link.lock(constrain);
        if constrain {
            let unit = link.unit();
            link.set_unit(UnitKind::Pixels);
            let width = f64::from(link.pixels().width);
            link.set_width(width);
            link.set_unit(unit);
            self.set_exact_size(link);
        }
    }

    /// Puts a format not among `formats` (as [`WebFormat::offered`] gives them) at the default one, or at the first
    /// offered where the default is not.
    pub(super) fn keep_to(&mut self, formats: &[WebFormat]) {
        if !formats.contains(&self.format) {
            let default = SaveForWebOptions::default().format;
            self.format = if formats.contains(&default) { default } else { formats.first().copied().unwrap_or(default) };
        }
    }

    /// The options as the next open remembers them: the slices and the artboard belong to one document, so they are
    /// left at their defaults, and the next document starts at its full size with proportions linked.
    pub(super) fn without_document_fields(&self) -> Self {
        SaveForWebOptions { percent: 100.0, size: None, constrain_proportions: true, slices: WebSlices::default(), artboard: None, ..self.clone() }
    }

    /// What decides the palette the encoder generates: a change of it drops the host's edits of the generated table.
    pub(super) fn reduction(&self) -> (WebFormat, ColorTable, u16) {
        (self.format, self.color_table, self.colors)
    }

    /// Whether the matte row shows: the matte fills the transparency of formats that cannot keep it.
    pub fn has_matte(&self) -> bool {
        !matches!(self.format, WebFormat::Png24 | WebFormat::WebP | WebFormat::Avif) || !self.transparency
    }

    /// Whether the Metadata row shows: the AVIF writer embeds none, so AVIF offers no choice of it.
    pub fn has_metadata(&self) -> bool {
        self.format != WebFormat::Avif
    }

    /// Whether the quality row shows.
    pub fn has_quality(&self) -> bool {
        match self.format {
            WebFormat::Jpeg | WebFormat::Avif => true,
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
    /// A built-in preset is not renamed or deleted.
    pub built_in: bool,
}

impl WebPreset {
    /// A preset named `name`.
    pub fn new(name: impl Into<String>, options: SaveForWebOptions) -> Self {
        WebPreset { name: name.into(), options, built_in: false }
    }

    /// Whether the preset is what `current` shows apart from what belongs to the document: the size and its link, the
    /// resample method, the slices and the artboard.
    pub fn is_set_in(&self, current: &SaveForWebOptions) -> bool {
        let own = &self.options;
        own == &SaveForWebOptions {
            percent: own.percent,
            size: own.size,
            constrain_proportions: own.constrain_proportions,
            resample: own.resample,
            slices: own.slices.clone(),
            artboard: own.artboard,
            ..current.clone()
        }
    }
}

/// One encoded pane the app hands back for a variant the dialog asked for.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct WebVariant {
    /// Which pane: 0 is the current settings, 1 and 2 the 4-Up variants.
    pub index: usize,
    /// The revision `WantedVariant::revision` gave; older answers are dropped.
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
    /// The palette the encode used, as red, green and blue, when it is an indexed format.
    pub palette: Option<Vec<[u8; 3]>>,
}

impl WebVariant {
    /// The encoded pane `index` of `revision`, shown from `texture`.
    pub fn new(index: usize, revision: u64, texture: SizedTexture, bytes: u64) -> Self {
        WebVariant { index, revision, picture: Ok(texture), bytes, colors: None, proxy: false, palette: None }
    }

    /// The pane `index` of `revision` whose encode failed with `message`.
    pub fn failed(index: usize, revision: u64, message: impl Into<Label>) -> Self {
        WebVariant { index, revision, picture: Err(message.into()), bytes: 0, colors: None, proxy: false, palette: None }
    }

    /// The palette size of an indexed format.
    #[must_use]
    pub fn colors(mut self, colors: u16) -> Self {
        self.colors = Some(colors);
        self
    }

    /// The palette the encode used.
    #[must_use]
    pub fn palette(mut self, palette: Vec<[u8; 3]>) -> Self {
        self.palette = Some(palette);
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
    /// The file the user chose to write; `None` when the dialog closed with Done (keep the settings, write nothing).
    pub path: Option<PathBuf>,
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
    pub fn remember(&self, ctx: &Context) {
        if let Some((key, last)) = &self.remember {
            crate::runtime::memory::set_saved_value(ctx, key, Some(last.clone()));
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
