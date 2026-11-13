//! `ExportAsDialog`: File, Export, Export As. The settings are on the left (the format, its options, metadata, colour
//! profile and image size); the preview with the size estimate is on the right. The dialog never encodes: it
//! reports the revision of the settings it wants a preview for and shows the answer the app gives back.

use super::{
    ExportAsResult, ExportFormat, ExportFormatDefaults, ExportMetadata, ExportOptions, ExportPreview, ExportProfile, ExportSize, ExportWords,
    SizeEstimate, export_file_name, size_error_label,
};
use crate::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use crate::runtime::requests::{HostAnswer, PathFilter, PathRequest};

use crate::choice::Checkbox;
use crate::contract::{Bind, Entry, NoValue, Outcome, Props as _, Remember as _, Show as _, ShowContainer as _};

use crate::display::{Shade, Text};

use crate::form::{Field, FieldGroup, SliderField, SliderFieldLayout};

use crate::layout_components::{Row, Stack};

use crate::patterns::form_dialogs::{
    FormDialog, FormOutput, HostRequests, LinkedSize, MAX_DOCUMENT_SIDE, PreviewPane, PreviewRequest, PreviewState, PreviewView, Resample,
    ResetFooter, ResetPick, SizeError, SizingWords, resize_from_scale,
};
use crate::pickers::Select;
use crate::runtime::env::{Env, Label, UnitKind, Units};

use crate::runtime::overlay::CloseReason;
use crate::sections::{HeaderStrength, Section};
use crate::style::TypeRole;
use crate::text::NumberField;

use craft_behavior::PixelSize;

use craft_theme::variant::Tone;
use egui::{Response, Ui};

use std::cell::RefCell;
use std::path::PathBuf;

/// The least and greatest scale, in percent.
const SCALE_RANGE: std::ops::RangeInclusive<f64> = 1.0..=1000.0;
/// The least and greatest quality.
const QUALITY_RANGE: std::ops::RangeInclusive<f64> = 1.0..=100.0;
/// The key of the save request the dialog files at Export.
const SAVE_KEY: &str = "export";

/// What the dialog keeps while it is open besides the settings: the preview request it has numbered, the answer the app
/// gave, the preview view and the scale edit the last pass refused.
#[derive(Clone)]
struct Held {
    /// The newest revision of the settings.
    revision: u64,
    /// The settings `revision` numbers.
    requested: Option<ExportOptions>,
    /// The newest revision reported in `wanted_preview`.
    taken: u64,
    shown: Option<ExportPreview>,
    view: PreviewView,
    error: Option<SizeError>,
}

impl Default for Held {
    fn default() -> Self {
        Held { revision: 0, requested: None, taken: 0, shown: None, view: PreviewView::fitting(), error: None }
    }
}

impl Held {
    /// Takes the app's answer when it is for the newest revision asked for or later, and not older than the one
    /// shown.
    fn adopt(&mut self, given: Option<&ExportPreview>) {
        let Some(given) = given else { return };
        let newest = self.shown.as_ref().map_or(0, |shown| shown.revision);
        if given.revision >= self.taken && given.revision >= newest {
            self.shown = Some(given.clone());
        }
    }

    /// Numbers the settings anew when they differ from the ones the newest revision stands for, and returns the request
    /// when the revision was not reported yet.
    fn request(&mut self, options: &ExportOptions) -> Option<PreviewRequest<ExportOptions>> {
        if self.requested.as_ref() != Some(options) {
            self.revision += 1;
            self.requested = Some(options.clone());
        }
        (self.revision != self.taken).then(|| {
            self.taken = self.revision;
            PreviewRequest::new(self.revision, options.clone())
        })
    }
}

/// What Export As reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ExportAsOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The answer, the pass the dialog closed with the path the app gave.
    pub result: Option<ExportAsResult>,
    /// The settings and revision of a preview the app should make, when the settings changed since the last pass. The
    /// app encodes a proxy and gives the answer back with [`ExportAsDialog::preview`].
    pub wanted_preview: Option<PreviewRequest<ExportOptions>>,
    /// The Export button pressed this pass files a save request here, with the file name and the format's filter; the
    /// app opens its file dialog for it, then answers with [`ExportAsDialog::answers`]. A cancelled pick leaves the
    /// dialog open.
    pub requests: HostRequests,
}

impl Outcome for ExportAsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// File, Export, Export As: format, size, quality, metadata, colour profile, a preview and a size estimate the app
/// computes.
///
/// - The settings: the format, then that format's rows (Lossless, Quality, Bits/Pixel, Transparency), Metadata,
///   Color Space with Embed Color Profile, and Image Size (Scale, Width, Height, Constrain Proportions, Resample).
/// - Keys: Tab walks the fields, Enter is Export and Escape cancels. Holding Alt
///   turns Cancel into Reset, which restores the settings the dialog opened with. Export asks the app for a file
///   to write (`ExportAsOutput::requests`) and the dialog stays open until the app answers with the path.
/// - Settings: the dialog opens on the value in the bind, fitted to the document, and writes its edits back.
/// - Previews: whenever the settings change, `wanted_preview` carries a new revision. The app encodes a proxy and
///   gives it back with `preview`; a size estimate shows at once ("≈ 1.2 MB"), and the app may follow with an exact
///   one for the same revision.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct ExportAsDialog<'a> {
    file_name: String,
    source: PixelSize,
    bound: Bind<'a, ExportOptions>,
    /// The formats the list offers.
    #[prop(into)]
    formats: Vec<ExportFormat>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// The quality and WebP mode a format takes when the user switches to it.
    #[prop]
    defaults: ExportFormatDefaults,
    /// The app's preview and estimate.
    #[prop]
    preview: Option<ExportPreview>,
    /// The host's answers to the requests of earlier passes: the path its save dialog picked.
    #[prop]
    answers: Vec<HostAnswer>,
    /// The app's save dialog is open: Export waits.
    #[prop]
    saving: bool,
}

impl<'a> ExportAsDialog<'a> {
    /// The dialog for the document named `file_name` (no extension) of size `source`, over the settings in `options`.
    pub fn new(file_name: impl Into<String>, source: PixelSize, options: Bind<'a, ExportOptions>) -> Self {
        ExportAsDialog {
            file_name: file_name.into(),
            source,
            bound: options,
            formats: ExportFormat::standard(),
            open: None,
            defaults: ExportFormatDefaults::default(),
            preview: None,
            answers: Vec::new(),
            saving: false,
        }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> ExportAsOutput {
        let ExportAsDialog { file_name, source, bound, formats, open, defaults, preview, answers, saving } = self;
        let env = Env::of(ui);
        let words = env.words::<ExportWords>();
        let pass = Pass { formats: &formats, source, words: &words, sizing: env.words::<SizingWords>(), units: env.units(), defaults };

        let reset = ResetFooter::new(ui, None, true);
        let wanted = RefCell::new(None::<PreviewRequest<ExportOptions>>);
        let latest = RefCell::new(None::<ExportOptions>);
        let answered = RefCell::new(None::<PathBuf>);
        let mut dialog =
            FormDialog::new(words.title.clone(), bound).ok_label(words.export.clone()).ok_enabled(!saving).accept_stays_open(true).answers(answers);
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        let shown = dialog.show(ui, |ui, form, values| {
            if form.first() {
                *values = values.clone().fitted(&formats, source);
            }
            for answer in form.take_answers() {
                if let HostAnswer::Path { key, path } = answer
                    && key == SAVE_KEY
                {
                    *answered.borrow_mut() = Some(path);
                    form.request_close_as(CloseReason::Activated);
                }
            }
            let mut held = form.memory::<Held>().clone();
            held.adopt(preview.as_ref());
            Row::new().show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| {
                    FieldGroup::new(words.title.clone()).show(ui, |ui, _| pass.settings(ui, values, &mut held));
                });
                Stack::new().show(ui, |ui, _| pass.preview(ui, values, &mut held));
            });
            *wanted.borrow_mut() = held.request(values);
            *form.memory::<Held>() = held;
            *latest.borrow_mut() = Some(values.clone());
            true
        });

        let FormOutput { response, closed, picked, accepted, mut requests, .. } = shown;
        let latest = latest.into_inner();
        if reset.pick(picked) == Some(ResetPick::Accept)
            && let Some(options) = &latest
        {
            let format = formats.iter().find(|format| format.id == options.format);
            let extension = format.map_or("png", |format| format.extension);
            let name = format!("{}.{extension}", export_file_name(&file_name, words.untitled.as_str()));
            let filters = format.map(|format| PathFilter::new(format.name, &[format.extension]));
            requests.request_path(PathRequest::save(SAVE_KEY, name, filters));
        }
        let result = accepted.zip(answered.into_inner()).map(|(options, path)| ExportAsResult { size: options.output_size(source), options, path });
        ExportAsOutput { response, closed, result, wanted_preview: wanted.into_inner(), requests }
    }
}

/// An [`ExportAsDialog`] the [`DialogHost`](crate::dialog::DialogHost) owns. It keeps the settings and the app's
/// preview, keeps the preview and save requests until the app takes them, and finishes with the result when the save
/// dialog's path comes back.
///
/// ```ignore
/// host.open(HostedExportAs::new("poster", PixelSize::new(1200, 800), ExportOptions::default()));
/// // each pass: wanted_preview() -> encode -> set_preview(preview);
/// // pick_path() -> file dialog -> answer_path(key, path), or cancel_requests() when the user cancels it
/// ```
pub struct HostedExportAs {
    id: DialogId,
    file_name: String,
    source: PixelSize,
    options: ExportOptions,
    formats: Vec<ExportFormat>,
    defaults: ExportFormatDefaults,
    preview: Option<ExportPreview>,
    wanted: Option<PreviewRequest<ExportOptions>>,
    requests: HostRequests,
    saving: bool,
    open: bool,
}

impl HostedExportAs {
    /// A hosted dialog for the document named `file_name` (no extension) of size `source`, opening with `options`.
    pub fn new(file_name: impl Into<String>, source: PixelSize, options: ExportOptions) -> Self {
        HostedExportAs {
            id: dialog_id(EXPORT_KEY),
            file_name: file_name.into(),
            source,
            options,
            formats: ExportFormat::standard(),
            defaults: ExportFormatDefaults::default(),
            preview: None,
            wanted: None,
            requests: HostRequests::new(),
            saving: false,
            open: true,
        }
    }

    /// The formats the list offers.
    #[must_use]
    pub fn formats(mut self, formats: Vec<ExportFormat>) -> Self {
        self.formats = formats;
        self
    }

    /// The quality and WebP mode a format takes when the user switches to it.
    #[must_use]
    pub fn defaults(mut self, defaults: ExportFormatDefaults) -> Self {
        self.defaults = defaults;
        self
    }

    /// The settings the dialog shows.
    pub fn options(&self) -> &ExportOptions {
        &self.options
    }

    /// The preview to make, when the settings changed since the last call. Returned once.
    pub fn wanted_preview(&mut self) -> Option<PreviewRequest<ExportOptions>> {
        self.wanted.take()
    }

    /// The app's preview and estimate; the dialog shows it unless a newer revision was asked for.
    pub fn set_preview(&mut self, preview: ExportPreview) {
        self.preview = Some(preview);
    }

    /// Export was pressed in the pass since the last call: the save dialog to open. Returned once; answer with
    /// [`answer_path`](Self::answer_path).
    pub fn pick_path(&mut self) -> Option<PathRequest> {
        self.requests.take_path()
    }

    /// Answers the save request `key` with the chosen path, which finishes the dialog.
    pub fn answer_path(&mut self, key: impl Into<String>, path: PathBuf) {
        self.saving = false;
        self.requests.answer_path(key, path);
    }

    /// The host cancelled its save dialog: the request not taken yet is dropped and Export works again.
    pub fn cancel_requests(&mut self) {
        self.saving = false;
        self.requests.cancel();
    }
}

/// The key of the hosted Export As dialog.
const EXPORT_KEY: &str = "file.exportAs";

impl StackDialog for HostedExportAs {
    type Result = ExportAsResult;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<ExportAsResult>> {
        let dialog = ExportAsDialog {
            formats: self.formats.clone(),
            defaults: self.defaults,
            preview: self.preview.clone(),
            answers: self.requests.take_answers(),
            saving: self.saving,
            open: Some(Bind::bound(&mut self.open)),
            ..ExportAsDialog::new(self.file_name.clone(), self.source, Bind::bound(&mut self.options))
        };
        let mut shown = dialog.show(ui);
        if let Some(wanted) = shown.wanted_preview {
            self.wanted = Some(wanted);
        }
        if let Some(request) = shown.requests.take_path() {
            self.requests.request_path(request);
            self.saving = true;
        }
        match (shown.result, shown.closed) {
            (Some(result), _) => Some(DialogResult::Ok(result)),
            (None, Some(_)) => {
                self.cancel_requests();
                Some(DialogResult::Cancel)
            }
            (None, None) => None,
        }
    }

    fn key(&self) -> Option<&str> {
        Some(EXPORT_KEY)
    }

    fn cancelled(&mut self, _: &egui::Context) {
        self.cancel_requests();
    }
}

/// What the rows read besides the dialog's state.
struct Pass<'p> {
    formats: &'p [ExportFormat],
    source: PixelSize,
    words: &'p ExportWords,
    sizing: std::sync::Arc<SizingWords>,
    units: Units,
    defaults: ExportFormatDefaults,
}

impl Pass<'_> {
    fn settings(&self, ui: &mut Ui, options: &mut ExportOptions, held: &mut Held) {
        let words = self.words;
        let chosen = self.formats.iter().position(|format| format.id == options.format).unwrap_or(0);
        let entries: Vec<Entry<usize>> = self.formats.iter().enumerate().map(|(at, format)| Entry::new(at, format.name)).collect();
        let mut now = chosen;
        choose(ui, &words.format, "format", &entries, &mut now);
        if now != chosen {
            self.switch(options, now);
        }
        let Some(format) = self.formats.get(now) else { return };

        if format.lossless {
            check(ui, &words.lossless, "lossless", &mut options.lossless);
        }
        if format.quality && !(format.lossless && options.lossless) {
            row(ui, &words.quality, "quality", |ui| {
                let field = SliderField::new(words.quality.clone(), Bind::given(f64::from(options.quality)), QUALITY_RANGE)
                    .step(1.0)
                    .layout(SliderFieldLayout::Inline)
                    .show(ui);
                if let Some(value) = field.value() {
                    options.quality = value.round().clamp(*QUALITY_RANGE.start(), *QUALITY_RANGE.end()) as u8;
                }
            });
        }
        if !format.bits.is_empty() {
            let entries: Vec<Entry<u8>> = std::iter::once(Entry::new(0, words.auto.clone()))
                .chain(format.bits.iter().map(|bits| Entry::new(*bits, bits.to_string())))
                .collect();
            choose(ui, &words.bits, "bits", &entries, &mut options.bits);
        }
        if format.transparency {
            check(ui, &words.transparency, "transparency", &mut options.transparency);
        }
        let metadata = [
            Entry::new(ExportMetadata::None, words.metadata_none.clone()),
            Entry::new(ExportMetadata::Exif, words.metadata_exif.clone()),
            Entry::new(ExportMetadata::All, words.metadata_all.clone()),
        ];
        choose(ui, &words.metadata, "metadata", &metadata, &mut options.metadata);
        let profiles =
            [Entry::new(ExportProfile::Document, words.document_profile.clone()), Entry::new(ExportProfile::Srgb, words.convert_srgb.clone())];
        choose(ui, &words.color_space, "profile", &profiles, &mut options.profile);
        check(ui, &words.embed_profile, "embed", &mut options.embed_profile);

        Section::fixed(words.image_size.clone())
            .strength(HeaderStrength::Caption)
            .id_salt("size")
            .show(ui, |ui, _| self.size(ui, options, held, format));
    }

    /// The format list picked `index`: the format takes the app's default quality (and WebP mode).
    fn switch(&self, options: &mut ExportOptions, index: usize) {
        let Some(format) = self.formats.get(index) else { return };
        options.format = format.id.to_owned();
        let clamp = |quality: u8| quality.clamp(*QUALITY_RANGE.start() as u8, *QUALITY_RANGE.end() as u8);
        match format.id {
            "jpg" => options.quality = clamp(self.defaults.jpeg_quality),
            "webp" => {
                options.quality = clamp(self.defaults.webp_quality);
                options.lossless = self.defaults.webp_lossless;
            }
            _ => {}
        }
    }

    /// Scale, Width, Height, Constrain Proportions, Resample and the notes under them. The sides are edited through
    /// a [`LinkedSize`], the one model of linked width and height.
    fn size(&self, ui: &mut Ui, options: &mut ExportOptions, held: &mut Held, format: &ExportFormat) {
        let words = self.words;
        let source = self.source;
        let size = options.output_size(source);
        let scale = match options.size {
            ExportSize::Scale(scale) => scale,
            ExportSize::Pixels { width, .. } => f64::from(width) / f64::from(source.width.max(1)) * 100.0,
        };
        let mut linked = LinkedSize::new(source);
        linked.lock(false);
        linked.set_width(f64::from(size.width));
        linked.set_height(f64::from(size.height));
        linked.lock(options.lock_aspect);
        let mut scaled = None;
        row(ui, &words.scale, "scale", |ui| {
            let field = NumberField::new(words.scale.clone(), Bind::given(Some(scale.round())))
                .unit_kind(UnitKind::Percent)
                .step(UnitKind::Percent.step())
                .range(SCALE_RANGE)
                .show(ui);
            scaled = field.edit.value().copied().flatten();
        });
        let mut resized = false;
        for (salt, name, value, is_width) in [("width", &words.width, size.width, true), ("height", &words.height, size.height, false)] {
            row(ui, name, salt, |ui| {
                let field = NumberField::new(name.clone(), Bind::given(Some(f64::from(value))))
                    .unit_kind(UnitKind::Pixels)
                    .step(UnitKind::Pixels.step())
                    .range(1.0..=MAX_DOCUMENT_SIDE)
                    .show(ui);
                if let Some(value) = field.edit.value().copied().flatten() {
                    if is_width {
                        linked.set_width(value);
                    } else {
                        linked.set_height(value);
                    }
                    resized = true;
                }
            });
        }
        let mut constrain = options.lock_aspect;
        check(ui, &words.constrain, "constrain", &mut constrain);
        if constrain != options.lock_aspect {
            linked.lock(constrain);
            options.lock_aspect = constrain;
            if constrain {
                linked.set_width(f64::from(size.width));
                resized = true;
            }
        }
        match scaled {
            Some(scale) => match resize_from_scale(source, scale) {
                Ok(_) => {
                    options.size = ExportSize::Scale(scale);
                    held.error = None;
                }
                Err(error) => held.error = Some(error),
            },
            None if resized => {
                let pixels = linked.pixels();
                options.size = ExportSize::pixels(pixels.width, pixels.height);
                held.error = None;
            }
            None => {}
        }

        row(ui, &self.sizing.resample, "resample", |ui| {
            let entries = Resample::entries(&self.sizing, &[]);
            if let Some(method) = Select::new(self.sizing.resample.clone(), Bind::given(options.resample), &entries).show(ui).value().copied() {
                options.resample = method;
            }
        });

        let size = options.output_size(source);
        let dimensions = words.dimensions.fill(&[
            ("width", self.units.number(f64::from(size.width), 0..=0).as_str()),
            ("height", self.units.number(f64::from(size.height), 0..=0).as_str()),
        ]);
        Text::new(dimensions).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
        if let Some(error) = held.error {
            Text::new(size_error_label(words, error)).role(TypeRole::Caption).tone(Tone::Bad).show(ui);
        }
        if let Some(limit) = format.max_side
            && (size.width > limit || size.height > limit)
        {
            Text::new(words.ico_limit.clone()).role(TypeRole::Caption).tone(Tone::Warn).show(ui);
        }
    }

    /// The preview pane with the format and size line under it.
    fn preview(&self, ui: &mut Ui, options: &ExportOptions, held: &mut Held) {
        let words = self.words;
        let state = held.shown.as_ref().map_or(PreviewState::Pending(None), |shown| shown.state.clone());
        let pane = PreviewPane::state(words.preview.clone(), state, &held.view).click_centres(true).aspect(1.0).show(ui);
        held.view = pane.view.unwrap_or(pane.shown);
        let name = self.formats.iter().find(|format| format.id == options.format).map_or("", |format| format.name).to_uppercase();
        let estimate = match held.shown.as_ref().and_then(|shown| shown.estimate) {
            Some(SizeEstimate::Proxy(bytes)) => words.estimate.fill(&[("size", self.units.file_size(bytes).as_str())]),
            Some(SizeEstimate::Exact(bytes)) => self.units.file_size(bytes),
            None => words.no_estimate.as_str().to_owned(),
        };
        Text::new(words.preview_line.fill(&[("format", name.as_str()), ("estimate", estimate.as_str())]))
            .role(TypeRole::Caption)
            .shade(Shade::Muted)
            .show(ui);
    }
}

/// Shows `body` in a field named `name`.
fn row(ui: &mut Ui, name: &Label, salt: &str, mut body: impl FnMut(&mut Ui)) {
    Field::new(name.clone()).id_salt(salt).show(ui, |ui, _| body(ui));
}

/// A check box named `name` over `value`.
fn check(ui: &mut Ui, name: &Label, salt: &str, value: &mut bool) {
    if let Some(checked) = Checkbox::new(Bind::given(*value)).label(name.clone()).id_salt(salt).show(ui).value().copied() {
        *value = checked;
    }
}

/// A select of `entries` in a field named `name`, over `value`.
fn choose<T: craft_behavior::Value>(ui: &mut Ui, name: &Label, salt: &str, entries: &[Entry<T>], value: &mut T) {
    row(ui, name, salt, |ui| {
        if let Some(picked) = Select::new(name.clone(), Bind::given(value.clone()), entries).show(ui).value().cloned() {
            *value = picked;
        }
    });
}
