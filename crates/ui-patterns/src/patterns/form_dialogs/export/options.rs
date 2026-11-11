//! `FormatOptionsDialog`: the format options asked after the file name is chosen (File, Save, Save As, Save a Copy).
//! JPEG, layered TIFF, Targa and DDS are one form over four parameter lists; OK and Cancel, Enter and Escape come from
//! `ParamDialog`, and Cancel abandons the save.

use crate::contract::{Bind, Entry, NoValue, Outcome, Show as _};
use crate::display::Text;
use crate::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind, ParamResult};
use crate::pickers::Select;
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::value_drag::SliderScale;
use egui::{Response, Ui};
use serde::{Deserialize, Serialize};

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
const DISCARD: &str = "discard";
const DONT_SHOW: &str = "dontShow";
const BITS: &str = "bits";
const COMPRESSION: &str = "compression";
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
}

impl<'a> FormatOptionsDialog<'a> {
    /// The dialog for the options of `kind`.
    pub fn new(kind: ExportOptionsKind) -> Self {
        FormatOptionsDialog { kind, open: None, initial: None }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> FormatOptionsOutput {
        let FormatOptionsDialog { kind, open, initial } = self;
        let words = Env::of(ui).words::<FormatOptionsWords>();
        let start = initial.filter(|options| options.kind() == kind).unwrap_or_else(|| FormatOptions::default_for(kind));
        let (key, title) = match kind {
            ExportOptionsKind::Tiff => ("tiff-options", words.tiff_title.clone()),
            ExportOptionsKind::Targa => ("targa-options", words.targa_title.clone()),
            ExportOptionsKind::Dds => ("dds-options", words.dds_title.clone()),
            _ => ("jpeg-options", words.jpeg_title.clone()),
        };
        let mut dialog = ParamDialog::new(key, title, params(&words, start));
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        let shown = dialog.show(ui);
        let result = shown.result.as_ref().map(|result| answer(kind, result));
        FormatOptionsOutput { response: shown.response, closed: shown.closed, result }
    }
}

/// The parameter list of `start`'s format.
fn params(words: &FormatOptionsWords, start: FormatOptions) -> Vec<Param> {
    match start {
        FormatOptions::Jpeg(options) => jpeg_params(words, options),
        FormatOptions::Tiff(options) => vec![
            Param::custom("note", words.layers_note.clone(), {
                let note = words.layers_note.clone();
                move |ui, _, _| Text::new(note.clone()).show(ui)
            })
            .full_width(),
            Param::new(DISCARD, words.discard_layers.clone(), ParamKind::bool(options.discard_layers)),
            Param::new(DONT_SHOW, words.dont_show.clone(), ParamKind::bool(options.dont_show_again)),
        ],
        FormatOptions::Targa(options) => vec![
            Param::section(words.resolution.clone()),
            Param::new(
                BITS,
                words.resolution.clone(),
                ParamKind::radio(
                    vec![(TARGA_BITS.to_string(), words.bits_24.clone()), (TARGA_ALPHA_BITS.to_string(), words.bits_32.clone())],
                    Some(options.bits.to_string()),
                ),
            ),
        ],
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

/// The JPEG rows: the quality field and slider with the preset list beside them, then the encoding.
fn jpeg_params(words: &FormatOptionsWords, options: JpegOptions) -> Vec<Param> {
    let presets: Vec<(u8, Label)> = JpegPreset::ALL.into_iter().map(|preset| (preset.quality(), preset.label(words))).collect();
    let preset_name = words.preset.clone();
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
        Param::new(QUALITY, words.quality.clone(), ParamKind::range(0.0, f64::from(MAX_QUALITY), f64::from(options.quality), SliderScale::Linear))
            .decimals(0),
        Param::custom("preset", preset_name.clone(), move |ui, fields, _| preset_row(ui, fields, &preset_name, &presets)),
        Param::section(words.format_options.clone()),
        Param::new(FORMAT, words.format_options.clone(), ParamKind::choice(formats, Some(options.format.id().to_owned()))),
    ]
}

/// The preset list: it shows the preset the quality falls in; only a pick changes the quality.
fn preset_row(ui: &mut Ui, fields: &mut DialogFields, name: &Label, presets: &[(u8, Label)]) -> Response {
    let quality = fields.int(QUALITY).map_or(DEFAULT_QUALITY, |quality| quality.clamp(0, i64::from(MAX_QUALITY)) as u8);
    let current = jpeg_quality_preset(quality).quality();
    let entries: Vec<Entry<u8>> = presets.iter().map(|(value, label)| Entry::new(*value, label.clone())).collect();
    let shown = Select::new(name.clone(), Bind::given(current), &entries).show(ui);
    if let Some(picked) = shown.value() {
        fields.set(QUALITY, FieldValue::Int(i64::from(*picked)));
    }
    shown.response
}

/// The options a dialog of `kind` confirmed.
fn answer(kind: ExportOptionsKind, result: &ParamResult) -> FormatOptions {
    let fields = &result.fields;
    match kind {
        ExportOptionsKind::Tiff => FormatOptions::Tiff(TiffOptions {
            discard_layers: fields.bool(DISCARD).unwrap_or_default(),
            dont_show_again: fields.bool(DONT_SHOW).unwrap_or_default(),
        }),
        ExportOptionsKind::Targa => {
            FormatOptions::Targa(TargaOptions::new(fields.text(BITS).and_then(|bits| bits.parse().ok()).unwrap_or(TARGA_BITS)))
        }
        ExportOptionsKind::Dds => FormatOptions::Dds(DdsOptions {
            compression: fields.text(COMPRESSION).map(DdsCompression::from_id).unwrap_or_default(),
            mipmaps: fields.bool(MIPMAPS).unwrap_or_default(),
        }),
        _ => {
            let quality = fields.int(QUALITY).map_or(DEFAULT_QUALITY, |quality| quality.clamp(0, i64::from(MAX_QUALITY)) as u8);
            let format = fields.text(FORMAT).map(JpegFormat::from_id).unwrap_or_default();
            FormatOptions::Jpeg(JpegOptions { quality, format })
        }
    }
}
