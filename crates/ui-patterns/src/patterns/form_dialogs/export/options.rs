//! `FormatOptionsDialog`: the format options asked after the file name is chosen (File, Save, Save As, Save a Copy).
//! JPEG, layered TIFF, Targa and DDS are one form over four parameter lists; OK and Cancel, Enter and Escape come from
//! `ParamDialog`, and Cancel abandons the save.

use super::{ExportWords, Revisions, SizeEstimate};
use crate::contract::{Bind, Entry, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use crate::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use crate::form::Field;
use crate::layout_components::{Row, Stack};
use crate::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind, PreviewRequest, open_fields, set_param_field};
use crate::pickers::Select;
use crate::runtime::env::{Env, Label, Units};
use crate::runtime::overlay::CloseReason;
use crate::text::NumberField;
use crate::value_drag::Slider;
use egui::{Response, Ui};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

craft_ui::words! {
    /// The words of the format options dialogs.
    pub struct FormatOptionsWords {
        jpeg_title: "formatOptions.jpegTitle" = "JPEG Options";
        tiff_title: "formatOptions.tiffTitle" = "TIFF Options";
        targa_title: "formatOptions.targaTitle" = "Targa Options";
        dds_title: "formatOptions.ddsTitle" = "DDS Options";
        bc1: "formatOptions.bc1" = "BC1 / DXT1";
        bc2: "formatOptions.bc2" = "BC2 / DXT3";
        bc3: "formatOptions.bc3" = "BC3 / DXT5";
        image_options: "formatOptions.imageOptions" = "Image Options";
        quality: "formatOptions.quality" = "Quality";
        low: "formatOptions.low" = "Low";
        medium: "formatOptions.medium" = "Medium";
        high: "formatOptions.high" = "High";
        maximum: "formatOptions.maximum" = "Maximum";
        format_options: "formatOptions.formatOptions" = "Format Options";
        baseline: "formatOptions.baseline" = "Baseline (\"Standard\")";
        baseline_optimized: "formatOptions.baselineOptimized" = "Baseline Optimized";
        progressive: "formatOptions.progressive" = "Progressive";
        discard_layers: "formatOptions.discardLayers" = "Discard Layers and Save a Copy";
        layers_note: "formatOptions.layersNote" = "Including layers will increase file size.";
        dont_show: "formatOptions.dontShow" = "Don't show again";
        resolution: "formatOptions.resolution" = "Resolution";
        bits_24: "formatOptions.bits24" = "24 bits/pixel";
        bits_32: "formatOptions.bits32" = "32 bits/pixel";
        compression: "formatOptions.compression" = "Compression";
        none: "formatOptions.none" = "None";
        lzw: "formatOptions.lzw" = "LZW";
        zip: "formatOptions.zip" = "ZIP";
        pyramid: "formatOptions.pyramid" = "Save Image Pyramid";
        transparency: "formatOptions.transparency" = "Save Transparency";
        mipmaps: "formatOptions.mipmaps" = "Generate Mipmaps";
        preset: "formatOptions.preset" = "Preset";
    }
}

/// Which format's options a [`FormatOptionsDialog`] asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ExportOptionsKind {
    /// JPEG: quality and encoding.
    Jpeg,
    /// Layered TIFF: keep or discard the layers.
    Tiff,
    /// Targa: bits per pixel.
    Targa,
    /// DDS: block compression and mipmaps.
    Dds,
}

/// JPEG's encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum JpegFormat {
    /// Standard sequential JPEG.
    #[default]
    Baseline,
    /// Sequential JPEG with optimized Huffman tables (smaller file).
    BaselineOptimized,
    /// Progressive JPEG that fills in over several scans.
    Progressive,
}

impl JpegFormat {
    const ALL: [JpegFormat; 3] = [JpegFormat::Baseline, JpegFormat::BaselineOptimized, JpegFormat::Progressive];

    fn id(self) -> &'static str {
        match self {
            JpegFormat::Baseline => "baseline",
            JpegFormat::BaselineOptimized => "baselineOptimized",
            JpegFormat::Progressive => "progressive",
        }
    }

    fn from_id(id: &str) -> Self {
        Self::ALL.into_iter().find(|format| format.id() == id).unwrap_or_default()
    }
}

/// The highest value of the JPEG quality scale of 0 to 12.
const MAX_QUALITY: u8 = 12;

/// What JPEG Options answers with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct JpegOptions {
    /// The scale of 0 to 12; [`jpeg_quality_percent`] gives the encoder's 1 to 100.
    pub quality: u8,
    /// The JPEG encoding.
    pub format: JpegFormat,
}

impl JpegOptions {
    /// Baseline JPEG at `quality` (0 to 12; a larger value counts as 12).
    pub fn new(quality: u8) -> Self {
        JpegOptions { quality: quality.min(MAX_QUALITY), format: JpegFormat::Baseline }
    }
}

/// The quality presets of the JPEG scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum JpegPreset {
    /// Qualities 0 to 4; the preset sets 3.
    Low,
    /// Qualities 5 to 7; the preset sets 5.
    Medium,
    /// Qualities 8 and 9; the preset sets 8.
    High,
    /// Qualities 10 to 12; the preset sets 10.
    Maximum,
}

impl JpegPreset {
    const ALL: [JpegPreset; 4] = [JpegPreset::Low, JpegPreset::Medium, JpegPreset::High, JpegPreset::Maximum];

    /// The quality the preset sets.
    pub fn quality(self) -> u8 {
        match self {
            JpegPreset::Low => 3,
            JpegPreset::Medium => 5,
            JpegPreset::High => 8,
            JpegPreset::Maximum => 10,
        }
    }

    fn label(self, words: &FormatOptionsWords) -> Label {
        match self {
            JpegPreset::Low => words.low.clone(),
            JpegPreset::Medium => words.medium.clone(),
            JpegPreset::High => words.high.clone(),
            JpegPreset::Maximum => words.maximum.clone(),
        }
    }
}

/// The preset a quality of 0 to 12 falls in: Low 0 to 4, Medium 5 to 7, High 8 and 9, Maximum 10 to 12.
pub(crate) fn jpeg_quality_preset(quality: u8) -> JpegPreset {
    match quality {
        0..=4 => JpegPreset::Low,
        5..=7 => JpegPreset::Medium,
        8..=9 => JpegPreset::High,
        _ => JpegPreset::Maximum,
    }
}

/// The encoder's 1 to 100 quality for the scale of 0 to 12: `round(clamp(q, 0, 12) / 12 * 99 + 1)`.
pub fn jpeg_quality_percent(quality: u8) -> u8 {
    (f64::from(quality.min(MAX_QUALITY)) / f64::from(MAX_QUALITY) * 99.0 + 1.0).round() as u8
}

/// What layered-TIFF Options answers with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TiffOptions {
    /// Save a flattened copy instead of the layers.
    pub discard_layers: bool,
    /// Don't ask again: the app clears its "ask before saving layered TIFF" preference.
    pub dont_show_again: bool,
    /// How the pixel data is compressed.
    pub compression: TiffCompression,
    /// Save the reduced-resolution levels of a pyramidal TIFF.
    pub pyramid: bool,
    /// Keep the transparency as an alpha channel.
    pub transparency: bool,
}

/// How a TIFF's pixel data is compressed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TiffCompression {
    /// No compression.
    #[default]
    None,
    /// LZW.
    Lzw,
    /// ZIP (Deflate).
    Zip,
}

impl TiffCompression {
    const ALL: [TiffCompression; 3] = [TiffCompression::None, TiffCompression::Lzw, TiffCompression::Zip];

    fn id(self) -> &'static str {
        match self {
            TiffCompression::None => "none",
            TiffCompression::Lzw => "lzw",
            TiffCompression::Zip => "zip",
        }
    }

    fn from_id(id: &str) -> Self {
        Self::ALL.into_iter().find(|compression| compression.id() == id).unwrap_or_default()
    }
}

/// What the TIFF rows know of the document: which rows the list has. The plain and the hosted dialog build the same
/// list from this one value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TiffDocument {
    /// The document has layers: Discard Layers shows.
    layered: bool,
    /// The document has transparency: Save Transparency shows.
    transparent: bool,
    /// The app asks before saving layered TIFF files: the note and Don't show again show, while layers are kept.
    layer_note: bool,
}

impl Default for TiffDocument {
    /// A layered, transparent document of an app that asks about layers: every row.
    fn default() -> Self {
        TiffDocument { layered: true, transparent: true, layer_note: true }
    }
}

/// What Targa Options answers with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TargaOptions {
    /// Bits per pixel, 24 or 32.
    pub bits: u8,
}

impl TargaOptions {
    /// 32 bits per pixel when `bits` is 32 (an image with alpha), otherwise 24.
    pub fn new(bits: u8) -> Self {
        TargaOptions { bits: if bits == TARGA_ALPHA_BITS { TARGA_ALPHA_BITS } else { TARGA_BITS } }
    }
}

const TARGA_BITS: u8 = 24;
const TARGA_ALPHA_BITS: u8 = 32;

/// DDS block compression.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DdsCompression {
    /// No compression.
    #[default]
    None,
    /// BC1 / DXT1.
    Bc1,
    /// BC2 / DXT3.
    Bc2,
    /// BC3 / DXT5.
    Bc3,
}

impl DdsCompression {
    const ALL: [DdsCompression; 4] = [DdsCompression::None, DdsCompression::Bc1, DdsCompression::Bc2, DdsCompression::Bc3];

    fn id(self) -> &'static str {
        match self {
            DdsCompression::None => "none",
            DdsCompression::Bc1 => "bc1",
            DdsCompression::Bc2 => "bc2",
            DdsCompression::Bc3 => "bc3",
        }
    }

    fn from_id(id: &str) -> Self {
        Self::ALL.into_iter().find(|compression| compression.id() == id).unwrap_or_default()
    }
}

/// What DDS Options answers with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DdsOptions {
    /// The block compression.
    pub compression: DdsCompression,
    /// Generates mipmaps.
    pub mipmaps: bool,
}

/// The options of one format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FormatOptions {
    /// JPEG options.
    Jpeg(JpegOptions),
    /// Layered TIFF options.
    Tiff(TiffOptions),
    /// Targa options.
    Targa(TargaOptions),
    /// DDS options.
    Dds(DdsOptions),
}

impl FormatOptions {
    /// The format these options belong to.
    pub fn kind(&self) -> ExportOptionsKind {
        match self {
            FormatOptions::Jpeg(_) => ExportOptionsKind::Jpeg,
            FormatOptions::Tiff(_) => ExportOptionsKind::Tiff,
            FormatOptions::Targa(_) => ExportOptionsKind::Targa,
            FormatOptions::Dds(_) => ExportOptionsKind::Dds,
        }
    }

    /// The options a format opens with: JPEG at 8, layers kept, 24 bits, no compression and no mipmaps.
    pub fn default_for(kind: ExportOptionsKind) -> Self {
        match kind {
            ExportOptionsKind::Jpeg => FormatOptions::Jpeg(JpegOptions::new(DEFAULT_QUALITY)),
            ExportOptionsKind::Tiff => FormatOptions::Tiff(TiffOptions::default()),
            ExportOptionsKind::Targa => FormatOptions::Targa(TargaOptions::new(TARGA_BITS)),
            ExportOptionsKind::Dds => FormatOptions::Dds(DdsOptions::default()),
        }
    }
}

/// The JPEG quality a dialog opens with when the app names none.
const DEFAULT_QUALITY: u8 = 8;

const QUALITY: &str = "quality";
const FORMAT: &str = "format";
const DISCARD_LAYERS: &str = "discardLayers";
const DONT_SHOW_AGAIN: &str = "dontShowAgain";
const BITS: &str = "bits";
const COMPRESSION: &str = "compression";
const PYRAMID: &str = "pyramid";
const TRANSPARENCY: &str = "transparency";
const MIPMAPS: &str = "mipmaps";

/// What a format options dialog reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct FormatOptionsOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The options, the pass the dialog closed as OK.
    pub result: Option<FormatOptions>,
}

impl Outcome for FormatOptionsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The options dialog of one export format: JPEG (the quality as a field, a slider and a preset list, then the
/// encoding), layered TIFF (keep or discard the layers, and not to ask again), Targa (24 or 32 bits per pixel) or DDS
/// (compression and mipmaps).
///
/// - Keys: Tab walks the fields, Enter is OK unless Cancel has focus, Escape cancels, which abandons the save.
///   Holding Alt turns Cancel into Reset, which restores the options the dialog opened with.
/// - Opens with `initial` when it is the options of the dialog's format (the quality a JPEG document was saved with,
///   32 bits for an image with alpha), else with [`FormatOptions::default_for`].
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct FormatOptionsDialog<'a> {
    kind: ExportOptionsKind,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// The options the dialog opens with.
    #[prop]
    initial: Option<FormatOptions>,
    /// The estimated size of the saved file, shown in the footer: `None` while no answer came ("…"), `Some(None)` when
    /// the app has no estimate ("—"), else the size.
    #[prop]
    estimate: Option<Option<SizeEstimate>>,
    /// The TIFF rows the document has.
    tiff: TiffDocument,
    /// The values the hosted dialog keeps; without them the parameter dialog keeps its own.
    values: Option<Bind<'a, DialogFields>>,
    /// The values the host opened, which Reset restores; with `values`, they are not opened again.
    opening: Option<DialogFields>,
}

impl<'a> FormatOptionsDialog<'a> {
    /// The dialog for the options of `kind`.
    pub fn new(kind: ExportOptionsKind) -> Self {
        FormatOptionsDialog { kind, open: None, initial: None, estimate: None, tiff: TiffDocument::default(), values: None, opening: None }
    }

    /// Offers the TIFF rows of a document: Discard Layers when it has `layered`, Save Transparency when it is
    /// `transparent`, and the layered-file note with Don't show again when `layer_note` is set (while the layers are
    /// kept). Without this call the dialog offers every row.
    pub fn tiff_document(mut self, layered: bool, transparent: bool, layer_note: bool) -> Self {
        self.tiff = TiffDocument { layered, transparent, layer_note };
        self
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> FormatOptionsOutput {
        let FormatOptionsDialog { kind, open, initial, estimate, tiff, values, opening } = self;
        let env = Env::of(ui);
        let words = env.words::<FormatOptionsWords>();
        let start = start_of(kind, initial);
        let title = match kind {
            ExportOptionsKind::Tiff => words.tiff_title.clone(),
            ExportOptionsKind::Targa => words.targa_title.clone(),
            ExportOptionsKind::Dds => words.dds_title.clone(),
            _ => words.jpeg_title.clone(),
        };
        let key = options_key(kind);
        let note = estimate_note(&env.words::<ExportWords>(), &env.units(), estimate);
        let mut dialog = ParamDialog::new(key, title, params(&words, start, tiff)).footer_note(note);
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        if let Some(values) = values {
            dialog = dialog.values(values);
            if let Some(opening) = opening {
                dialog = dialog.values_opened(opening);
            }
        }
        let shown = dialog.show(ui);
        let result = shown.result.as_ref().map(|result| answer(start, &result.fields));
        FormatOptionsOutput { response: shown.response, closed: shown.closed, result }
    }
}

/// The footer note of an estimate: "…" before any answer, "—" when the app has none, else the size (with "≈" for one
/// scaled from a proxy).
fn estimate_note(words: &ExportWords, units: &Units, estimate: Option<Option<SizeEstimate>>) -> Label {
    match estimate {
        None => words.estimating.clone(),
        Some(None) => words.no_estimate.clone(),
        Some(Some(SizeEstimate::Proxy(bytes))) => Label::from(words.estimate.fill(&[("size", units.file_size(bytes).as_str())])),
        Some(Some(SizeEstimate::Exact(bytes))) => Label::from(units.file_size(bytes)),
    }
}

/// The options a dialog of `kind` starts from: `initial` when it is the options of that format, else the format's
/// defaults.
fn start_of(kind: ExportOptionsKind, initial: Option<FormatOptions>) -> FormatOptions {
    initial.filter(|options| options.kind() == kind).unwrap_or_else(|| FormatOptions::default_for(kind))
}

/// The key of the options dialog of `kind`: its parameter dialog's and its hosted entry's.
fn options_key(kind: ExportOptionsKind) -> &'static str {
    match kind {
        ExportOptionsKind::Tiff => "tiff-options",
        ExportOptionsKind::Targa => "targa-options",
        ExportOptionsKind::Dds => "dds-options",
        _ => "jpeg-options",
    }
}

/// A [`FormatOptionsDialog`] the [`DialogHost`](crate::dialog::DialogHost) owns: the format and the options it opens
/// with. It answers with the [`FormatOptions`] at OK; Cancel abandons the save.
///
/// ```ignore
/// host.open(HostedFormatOptions::new(ctx, ExportOptionsKind::Jpeg).initial(FormatOptions::Jpeg(JpegOptions::new(10))));
/// if let Some(DialogResult::Ok(options)) = host.take::<HostedFormatOptions>() {
///     save_with(options);
/// }
/// ```
pub struct HostedFormatOptions {
    id: DialogId,
    kind: ExportOptionsKind,
    initial: Option<FormatOptions>,
    tiff: TiffDocument,
    open: bool,
    /// The words of the rows, from the root when the dialog was made.
    words: Arc<FormatOptionsWords>,
    /// The values of the rows, open from construction: the dialog edits them in place, and automation reads and writes
    /// the same ones.
    fields: DialogFields,
    /// The values right after the fields were opened (defaults filled): what Reset restores.
    opening: DialogFields,
    /// The numbering of the options the estimate is asked for.
    revisions: Revisions<FormatOptions>,
    /// The request the app has not taken yet.
    wanted: Option<PreviewRequest<FormatOptions>>,
    /// The newest answer and the revision it answered; `None` before the first.
    estimate: Option<(u64, Option<SizeEstimate>)>,
    #[cfg(feature = "testing")]
    facts: crate::testing::StoryOutput,
}

impl HostedFormatOptions {
    /// The settings the app should estimate the file size for, once, when they changed since the last call.
    pub fn wanted_estimate(&mut self) -> Option<PreviewRequest<FormatOptions>> {
        self.wanted.take()
    }

    /// The app's estimate for the settings of `revision`; an answer for an older revision than the newest asked for or
    /// than the one shown is dropped. While a newer revision is pending, the last answer stays shown.
    pub fn set_estimate(&mut self, revision: u64, estimate: Option<SizeEstimate>) {
        let newest = self.estimate.map_or(0, |(answered, _)| answered);
        if self.revisions.adopts(revision) && revision >= newest {
            self.estimate = Some((revision, estimate));
        }
    }

    /// The options dialog of `kind`, its fields open at once, so automation lists and sets them before the first
    /// pass.
    pub fn new(ctx: &egui::Context, kind: ExportOptionsKind) -> Self {
        let mut dialog = HostedFormatOptions {
            id: dialog_id(options_key(kind)),
            kind,
            initial: None,
            tiff: TiffDocument::default(),
            open: true,
            words: Env::of_root(ctx).words::<FormatOptionsWords>(),
            fields: DialogFields::new(),
            opening: DialogFields::new(),
            revisions: Revisions::default(),
            wanted: None,
            estimate: None,
            #[cfg(feature = "testing")]
            facts: crate::testing::StoryOutput::default(),
        };
        dialog.open_fields();
        dialog
    }

    /// The options the dialog starts from.
    fn start(&self) -> FormatOptions {
        start_of(self.kind, self.initial)
    }

    /// The parameter list the dialog shows.
    fn params(&self) -> Vec<Param> {
        params(&self.words, self.start(), self.tiff)
    }

    /// Opens the fields from the start, with the defaults filled; the opening values Reset restores are these.
    fn open_fields(&mut self) {
        self.fields = DialogFields::new();
        open_fields(&self.params(), &mut self.fields);
        self.opening = self.fields.clone();
    }

    /// The options the dialog opens with, when they are the options of its format.
    #[must_use]
    pub fn initial(mut self, options: FormatOptions) -> Self {
        self.initial = Some(options);
        self.open_fields();
        self
    }

    /// Offers the TIFF rows of a document, as [`FormatOptionsDialog::tiff_document`]: the hosted dialog keeps the
    /// three flags and passes them to the dialog it builds.
    #[must_use]
    pub fn tiff_document(mut self, layered: bool, transparent: bool, layer_note: bool) -> Self {
        self.tiff = TiffDocument { layered, transparent, layer_note };
        self.open_fields();
        self
    }

    /// The format whose options the dialog edits.
    pub fn kind(&self) -> ExportOptionsKind {
        self.kind
    }
}

impl StackDialog for HostedFormatOptions {
    type Result = FormatOptions;

    /// Answers OK as the button does, from the values the rows show.
    fn confirm(&mut self, _: &egui::Context) -> Option<Self::Result> {
        Some(answer(self.start(), &self.fields))
    }

    /// The values by parameter key, as [`DialogFields`] serializes them.
    fn json_fields(&self) -> Option<serde_json::Value> {
        serde_json::to_value(&self.fields).ok()
    }

    /// Sets the value of the row `field` names when the row is shown, takes input and takes the value, as editing it
    /// would.
    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        set_param_field(&self.params(), &mut self.fields, field, value)
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<FormatOptions>> {
        let shown = FormatOptionsDialog {
            kind: self.kind,
            open: Some(Bind::bound(&mut self.open)),
            initial: self.initial,
            estimate: self.estimate.map(|(_, estimate)| estimate),
            tiff: self.tiff,
            values: Some(Bind::bound(&mut self.fields)),
            opening: Some(self.opening.clone()),
        }
        .show(ui);
        #[cfg(feature = "testing")]
        {
            self.facts = crate::contract::Outcome::story_facts(&shown);
        }
        if let Some(request) = self.revisions.request(&answer(self.start(), &self.fields)) {
            self.wanted = Some(request);
        }
        match (shown.result, shown.closed) {
            (Some(options), _) => Some(DialogResult::Ok(options)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn key(&self) -> Option<&str> {
        Some(options_key(self.kind))
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.facts.clone()
    }
}

/// The parameter list of `start`'s format.
fn params(words: &FormatOptionsWords, start: FormatOptions, tiff: TiffDocument) -> Vec<Param> {
    match start {
        FormatOptions::Jpeg(options) => jpeg_params(words, options),
        FormatOptions::Tiff(options) => tiff_params(words, options, tiff),
        FormatOptions::Targa(options) => vec![Param::new(
            BITS,
            words.resolution.clone(),
            ParamKind::radio(
                vec![(TARGA_BITS.to_string(), words.bits_24.clone()), (TARGA_ALPHA_BITS.to_string(), words.bits_32.clone())],
                Some(options.bits.to_string()),
            ),
        )],
        FormatOptions::Dds(options) => vec![
            Param::new(
                COMPRESSION,
                words.compression.clone(),
                ParamKind::radio(
                    DdsCompression::ALL
                        .into_iter()
                        .map(|compression| {
                            let label = match compression {
                                DdsCompression::Bc1 => words.bc1.clone(),
                                DdsCompression::Bc2 => words.bc2.clone(),
                                DdsCompression::Bc3 => words.bc3.clone(),
                                _ => words.none.clone(),
                            };
                            (compression.id().to_owned(), label)
                        })
                        .collect(),
                    Some(options.compression.id().to_owned()),
                ),
            ),
            Param::new(MIPMAPS, words.mipmaps.clone(), ParamKind::bool(options.mipmaps)),
        ],
    }
}

/// The TIFF rows of `document`: for a layered one the note, Discard Layers and Don't show again (the note and Don't
/// show again only when the app asks about layers, and only while the layers are kept), then Compression and Save Image
/// Pyramid, and Save Transparency for a transparent one.
fn tiff_params(words: &FormatOptionsWords, options: TiffOptions, document: TiffDocument) -> Vec<Param> {
    let mut rows = Vec::new();
    if document.layered {
        if document.layer_note {
            rows.push(
                Param::read_only_text("note", words.layers_note.clone(), words.layers_note.clone()).full_width().visible_when(DISCARD_LAYERS, false),
            );
        }
        rows.push(Param::new(DISCARD_LAYERS, words.discard_layers.clone(), ParamKind::bool(options.discard_layers)));
        if document.layer_note {
            rows.push(
                Param::new(DONT_SHOW_AGAIN, words.dont_show.clone(), ParamKind::bool(options.dont_show_again)).visible_when(DISCARD_LAYERS, false),
            );
        }
    }
    let compressions = TiffCompression::ALL
        .into_iter()
        .map(|compression| {
            let label = match compression {
                TiffCompression::Lzw => words.lzw.clone(),
                TiffCompression::Zip => words.zip.clone(),
                _ => words.none.clone(),
            };
            (compression.id().to_owned(), label)
        })
        .collect();
    rows.push(Param::new(COMPRESSION, words.compression.clone(), ParamKind::radio(compressions, Some(options.compression.id().to_owned()))));
    rows.push(Param::new(PYRAMID, words.pyramid.clone(), ParamKind::bool(options.pyramid)));
    if document.transparent {
        rows.push(Param::new(TRANSPARENCY, words.transparency.clone(), ParamKind::bool(options.transparency)));
    }
    rows
}

/// The JPEG rows: the quality field with the preset list beside it and the slider across the form under them, then
/// the encoding.
fn jpeg_params(words: &FormatOptionsWords, options: JpegOptions) -> Vec<Param> {
    let presets: Vec<(u8, Label)> = JpegPreset::ALL.into_iter().map(|preset| (preset.quality(), preset.label(words))).collect();
    let (quality_name, preset_name) = (words.quality.clone(), words.preset.clone());
    let start = options.quality;
    let formats = JpegFormat::ALL
        .into_iter()
        .map(|format| {
            let label = match format {
                JpegFormat::BaselineOptimized => words.baseline_optimized.clone(),
                JpegFormat::Progressive => words.progressive.clone(),
                _ => words.baseline.clone(),
            };
            (format.id().to_owned(), label)
        })
        .collect();
    vec![
        Param::section(words.image_options.clone()),
        Param::custom(QUALITY, words.quality.clone(), move |ui, fields, _| quality_rows(ui, fields, &quality_name, &preset_name, &presets, start))
            .opens_with(FieldValue::Int(i64::from(start)))
            .full_width(),
        Param::new(FORMAT, words.format_options.clone(), ParamKind::radio(formats, Some(options.format.id().to_owned()))),
    ]
}

/// The quality in `fields`, in whatever form it was set, else `start`, the quality the dialog opened with.
fn quality_of(fields: &DialogFields, start: u8) -> u8 {
    fields.number(QUALITY).filter(|quality| quality.is_finite()).map_or(start, |quality| quality.round().clamp(0.0, f64::from(MAX_QUALITY)) as u8)
}

/// The quality rows: the field named `name` and the preset list on one line, the slider across the form under them.
/// The list shows the preset the quality falls in; only a pick changes the quality.
fn quality_rows(ui: &mut Ui, fields: &mut DialogFields, name: &Label, preset: &Label, presets: &[(u8, Label)], start: u8) -> Response {
    let quality = quality_of(fields, start);
    let max = f64::from(MAX_QUALITY);
    let current = jpeg_quality_preset(quality).quality();
    let entries: Vec<Entry<u8>> = presets.iter().map(|(value, label)| Entry::new(*value, label.clone())).collect();
    let mut set = None;
    let shown = Stack::new().fill_width().show(ui, |ui, _| {
        Field::new(name.clone()).id_salt("quality").show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                let typed = NumberField::new(name.clone(), Bind::given(Some(f64::from(quality))))
                    .range(0.0..=max)
                    .step(1.0)
                    .fixed_decimals(0)
                    .commit_decimals(0)
                    .id_salt("field")
                    .show(ui);
                if let Some(Some(value)) = typed.edit.value() {
                    set = Some(value.round() as u8);
                }
                if let Some(picked) = Select::new(preset.clone(), Bind::given(current), &entries).id_salt("preset").show(ui).value() {
                    set = Some(*picked);
                }
            });
        });
        let slid = Slider::new(Bind::given(f64::from(quality)), 0.0..=max).step(1.0).label(name.clone()).id_salt("slider").show(ui);
        if let Some(value) = slid.value() {
            set = Some(value.round() as u8);
        }
    });
    if let Some(quality) = set {
        fields.set(QUALITY, FieldValue::Int(i64::from(quality.min(MAX_QUALITY))));
    }
    shown.output
}

/// The options a dialog that opened with `start` confirmed.
fn answer(start: FormatOptions, fields: &DialogFields) -> FormatOptions {
    match start.kind() {
        ExportOptionsKind::Tiff => FormatOptions::Tiff(TiffOptions {
            discard_layers: fields.bool(DISCARD_LAYERS).unwrap_or_default(),
            dont_show_again: fields.bool(DONT_SHOW_AGAIN).unwrap_or_default(),
            compression: fields.text(COMPRESSION).map(TiffCompression::from_id).unwrap_or_default(),
            pyramid: fields.bool(PYRAMID).unwrap_or_default(),
            transparency: fields.bool(TRANSPARENCY).unwrap_or_default(),
        }),
        ExportOptionsKind::Targa => {
            FormatOptions::Targa(TargaOptions::new(fields.text(BITS).and_then(|bits| bits.parse().ok()).unwrap_or(TARGA_BITS)))
        }
        ExportOptionsKind::Dds => FormatOptions::Dds(DdsOptions {
            compression: fields.text(COMPRESSION).map(DdsCompression::from_id).unwrap_or_default(),
            mipmaps: fields.bool(MIPMAPS).unwrap_or_default(),
        }),
        _ => {
            let quality = quality_of(fields, if let FormatOptions::Jpeg(options) = start { options.quality } else { DEFAULT_QUALITY });
            let format = fields.text(FORMAT).map(JpegFormat::from_id).unwrap_or_default();
            FormatOptions::Jpeg(JpegOptions { quality, format })
        }
    }
}
