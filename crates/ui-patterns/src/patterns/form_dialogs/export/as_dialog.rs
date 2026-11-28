//! `ExportAsDialog`: File, Export, Export As. The settings are on the left (the format, its options, metadata, colour
//! profile and image size); the preview with the size estimate is on the right. The dialog never encodes: it
//! reports the revision of the settings it wants a preview for and shows the answer the app gives back.

use super::{
    ExportAsResult, ExportFormat, ExportFormatDefaults, ExportMetadata, ExportOptions, ExportPreview, ExportProfile, ExportSize, ExportWords,
    Revisions, ScaleRow, ScaleSize, SizeEstimate, export_file_name, size_error_label,
};
use crate::buttons::IconButton;
use crate::contract::clicked;
use crate::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldOk, StackDialog, dialog_id, serde_fields, set_serde_field};
use crate::patterns::form_dialogs::form::{read_bind, write_bind};
use crate::patterns::presets::{
    GroupPath, NameCheck, NamedPresetEvent, NamedPresetRow, PickerField, PresetFns, PresetLibrary, PresetPicker, PresetRow, PresetSource, PresetState,
};
use crate::runtime::requests::{HostAnswer, PathFilter, PathKind, PathRequest};
use crate::text::TextField;
use craft_behavior::IconId;

use crate::choice::Checkbox;
use crate::contract::{Bind, Entry, NoValue, Outcome, Props as _, Remember as _, SaveKey, Show as _, ShowContainer as _};

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
use crate::sections::Section;
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
    /// The numbering of the settings and the newest revision reported in `wanted_preview`.
    revisions: Revisions<ExportOptions>,
    shown: Option<ExportPreview>,
    view: PreviewView,
    error: Option<SizeError>,
    /// The view of the preset picker.
    picker: PresetState<String>,
}

impl Default for Held {
    fn default() -> Self {
        Held { revisions: Revisions::default(), shown: None, view: PreviewView::fitting(), error: None, picker: PresetState::default() }
    }
}

impl Held {
    /// Takes the app's answer when it is for the newest revision asked for or later, and not older than the one
    /// shown.
    fn adopt(&mut self, given: Option<&ExportPreview>) {
        let Some(given) = given else { return };
        let newest = self.shown.as_ref().map_or(0, |shown| shown.revision);
        if self.revisions.adopts(given.revision) && given.revision >= newest {
            self.shown = Some(given.clone());
        }
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
    /// The saved-preset edits the user made this pass (create, rename, delete), for the app to store, keyed by the
    /// preset's name before the event.
    pub presets: Vec<NamedPresetEvent<String>>,
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
///   Color Space (Convert to sRGB, on by default, and Embed Color Profile), Image Size (Scale, Width, Height,
///   Constrain Proportions, Resample) and Scale All: rows of a size and a file-name suffix, + adding the next size
///   ([`ScaleRow::next`]) and each row's trash button removing it while more than one is left.
/// - Keys: Tab walks the fields and Escape cancels; Enter does not export, only the Export button does. Holding Alt
///   turns Cancel into Reset, which restores the settings the dialog opened with. Export asks the app for a file
///   to write (`ExportAsOutput::requests`), or a folder with more than one Scale All row, and the dialog stays open
///   until the app answers with the path. Without [`folder_export`](ExportAsDialog::folder_export), more than one row
///   refuses Export with a note. An export of several files ([`files`](ExportAsDialog::files)) asks for a folder
///   too where the app can write one.
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
    /// The resample methods the list offers, in the order given; all of them when empty. A method the list lacks,
    /// remembered or from a preset, opens as Automatic when listed, else as the first listed one.
    #[prop]
    methods: Vec<Resample>,
    /// The dialog's title in place of "Export As" ("Export As: Layer 1").
    #[prop(into)]
    title: Option<Label>,
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
    /// Binds the saved presets of the settings to `presets`.
    #[prop]
    presets: Option<Bind<'a, PresetLibrary<ExportOptions>>>,
    /// Saves the settings under `key` when the app calls [`ExportAsResult::remember`], and opens with them.
    #[prop]
    remember: Option<SaveKey>,
    /// The settings the host opened the dialog with: what Reset restores, in place of the bound settings of the first
    /// pass (a field edited before the dialog was first shown is not the Reset target).
    #[prop]
    opening: Option<ExportOptions>,
    /// Whether the app can write a folder of files, for more than one Scale All row; the default is true. A build
    /// whose file dialog answers no folder (the browser) sets it off.
    #[prop]
    folder_export: bool,
    /// How many files the export writes before Scale All, one per exportable layer of a layer export; the default is
    /// 1. With more than one and [`folder_export`](ExportAsDialog::folder_export), Export asks for a folder.
    #[prop]
    files: usize,
}

impl<'a> ExportAsDialog<'a> {
    /// The dialog for the document named `file_name` (no extension) of size `source`, over the settings in `options`.
    pub fn new(file_name: impl Into<String>, source: PixelSize, options: Bind<'a, ExportOptions>) -> Self {
        ExportAsDialog {
            file_name: file_name.into(),
            source,
            bound: options,
            formats: ExportFormat::standard(),
            methods: Vec::new(),
            title: None,
            open: None,
            defaults: ExportFormatDefaults::default(),
            preview: None,
            answers: Vec::new(),
            saving: false,
            presets: None,
            remember: None,
            opening: None,
            folder_export: true,
            files: 1,
        }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> ExportAsOutput {
        let ExportAsDialog {
            file_name,
            source,
            bound,
            formats,
            methods,
            title,
            open,
            defaults,
            preview,
            answers,
            saving,
            mut presets,
            remember,
            opening,
            folder_export,
            files,
        } = self;
        let env = Env::of(ui);
        let words = env.words::<ExportWords>();
        // A frame can run the body twice: the library lives in a cell the passes share, and `events` collects the edits
        // of both passes (the first pass alone gets the frame's input).
        let library = presets.as_mut().map(|bind| RefCell::new(read_bind(bind).unwrap_or_default()));
        let events = RefCell::new(Vec::new());
        let pass = Pass {
            formats: &formats,
            methods: &methods,
            source,
            words: &words,
            sizing: env.words::<SizingWords>(),
            units: env.units(),
            defaults,
            library: library.as_ref(),
            events: &events,
        };

        let reset = ResetFooter::new(ui, None, true);
        let wanted = RefCell::new(None::<PreviewRequest<ExportOptions>>);
        let latest = RefCell::new(None::<ExportOptions>);
        let answered = RefCell::new(None::<PathBuf>);
        let mut dialog = FormDialog::new(title.unwrap_or_else(|| words.title.clone()), bound)
            .ok_label(words.export.clone())
            .ok_enabled(!saving)
            .accept_stays_open(true)
            .enter_confirms(false)
            .answers(answers);
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        if let Some(opening) = opening {
            let last = remember.as_ref().and_then(|key| crate::runtime::memory::saved_value::<Option<ExportOptions>>(ui.ctx(), key));
            dialog = dialog.opening(opened_options(opening, last, &formats, &methods, source));
        }
        let shown = dialog.show(ui, |ui, form, values| {
            if form.first() {
                let last = remember.as_ref().and_then(|key| form.saved::<Option<ExportOptions>>(key).clone());
                *values = opened_options(values.clone(), last, &formats, &methods, source);
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
            let exportable = folder_export || values.scale_all.len() <= 1;
            if !exportable {
                Text::new(words.folder_needed.clone()).role(TypeRole::Caption).tone(Tone::Bad).show(ui);
            }
            *wanted.borrow_mut() = held.revisions.request(values);
            *form.memory::<Held>() = held;
            *latest.borrow_mut() = Some(values.clone());
            exportable
        });

        let FormOutput { response, closed, picked, accepted, mut requests, .. } = shown;
        let latest = latest.into_inner();
        if reset.pick(picked) == Some(ResetPick::Accept)
            && let Some(options) = &latest
        {
            requests.request_path(save_request(&formats, options, &file_name, folder_export && files > 1, &words));
        }
        let result = accepted.zip(answered.into_inner()).map(|(options, path)| answer_of(options, source, path, remember));
        let events = events.into_inner();
        if !events.is_empty()
            && let (Some(bind), Some(cell)) = (presets.as_mut(), library)
        {
            write_bind(bind, cell.into_inner());
        }
        ExportAsOutput { response, closed, result, wanted_preview: wanted.into_inner(), requests, presets: events }
    }
}

/// The save dialog Export asks for: the document's name with the format's extension and filter; a folder for more than
/// one Scale All row, or when `several_files` (a layer export of more than one layer, where the app writes a folder).
fn save_request(formats: &[ExportFormat], options: &ExportOptions, file_name: &str, several_files: bool, words: &ExportWords) -> PathRequest {
    if options.scale_all.len() > 1 || several_files {
        return PathRequest::new(SAVE_KEY, PathKind::Folder);
    }
    let format = formats.iter().find(|format| format.id == options.format);
    let extension = format.map_or("png", |format| format.extension);
    let name = format!("{}.{extension}", export_file_name(file_name, words.untitled.as_str()));
    let filters = format.map(|format| PathFilter::new(format.name, &[format.extension]));
    PathRequest::save(SAVE_KEY, name, filters)
}

/// What Export answers with: the confirmed options, the chosen path, the output size, and the options to remember when
/// the dialog has a key.
fn answer_of(options: ExportOptions, source: PixelSize, path: PathBuf, remember: Option<SaveKey>) -> ExportAsResult {
    ExportAsResult {
        size: options.output_size(source),
        outputs: options.outputs(source),
        remember: remember.map(|key| (key, options.clone())),
        options,
        path,
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
    /// The settings the dialog was made with: what Reset restores, whatever automation sets before the first show.
    opening: ExportOptions,
    formats: Vec<ExportFormat>,
    methods: Vec<Resample>,
    title: Option<Label>,
    defaults: ExportFormatDefaults,
    preview: Option<ExportPreview>,
    wanted: Option<PreviewRequest<ExportOptions>>,
    requests: HostRequests,
    saving: bool,
    open: bool,
    /// The saved presets the app gave, as the user left them.
    presets: PresetLibrary<ExportOptions>,
    /// Whether the app gave a library: the preset row shows only then.
    with_presets: bool,
    /// The saved-preset edits the app has not taken yet.
    events: Vec<NamedPresetEvent<String>>,
    remember: Option<SaveKey>,
    /// The path the host's save dialog answered, or automation set; Export finishes with it.
    path: Option<PathBuf>,
    folder_export: bool,
    files: usize,
    #[cfg(feature = "testing")]
    facts: crate::testing::StoryOutput,
}

impl HostedExportAs {
    /// Opens with `library` as the saved presets, which shows the preset row under Image Size.
    #[must_use]
    pub fn presets(mut self, library: PresetLibrary<ExportOptions>) -> Self {
        self.presets = library;
        self.with_presets = true;
        self
    }

    /// Saves the settings under `key` when the app calls [`ExportAsResult::remember`], and opens with them.
    #[must_use]
    pub fn remember(mut self, key: SaveKey) -> Self {
        self.remember = Some(key);
        self
    }

    /// The saved presets as the user left them.
    pub fn saved_presets(&self) -> &PresetLibrary<ExportOptions> {
        &self.presets
    }

    /// The saved-preset edits since the last call, for the app to store. A created preset's value is in
    /// [`saved_presets`](Self::saved_presets) under its name.
    pub fn take_preset_events(&mut self) -> Vec<NamedPresetEvent<String>> {
        std::mem::take(&mut self.events)
    }

    /// A hosted dialog for the document named `file_name` (no extension) of size `source`, opening with `options`.
    pub fn new(file_name: impl Into<String>, source: PixelSize, options: ExportOptions) -> Self {
        HostedExportAs {
            id: dialog_id(EXPORT_KEY),
            file_name: file_name.into(),
            source,
            opening: options.clone(),
            options,
            formats: ExportFormat::standard(),
            methods: Vec::new(),
            title: None,
            defaults: ExportFormatDefaults::default(),
            preview: None,
            wanted: None,
            requests: HostRequests::new(),
            saving: false,
            open: true,
            presets: PresetLibrary::new(),
            with_presets: false,
            events: Vec::new(),
            remember: None,
            path: None,
            folder_export: true,
            files: 1,
            #[cfg(feature = "testing")]
            facts: crate::testing::StoryOutput::default(),
        }
    }

    /// Whether the app can write a folder of files ([`ExportAsDialog::folder_export`]).
    #[must_use]
    pub fn folder_export(mut self, on: bool) -> Self {
        self.folder_export = on;
        self
    }

    /// How many files the export writes before Scale All ([`ExportAsDialog::files`]).
    #[must_use]
    pub fn files(mut self, count: usize) -> Self {
        self.files = count;
        self
    }

    /// The formats the list offers.
    #[must_use]
    pub fn formats(mut self, formats: Vec<ExportFormat>) -> Self {
        self.formats = formats;
        self
    }

    /// The resample methods the list offers, in the order given; all of them when empty
    /// ([`ExportAsDialog::methods`]).
    #[must_use]
    pub fn methods(mut self, methods: Vec<Resample>) -> Self {
        self.methods = methods;
        self
    }

    /// The dialog's title in place of "Export As" ([`ExportAsDialog::title`]).
    #[must_use]
    pub fn title(mut self, title: impl Into<Label>) -> Self {
        self.title = Some(title.into());
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
        self.path = Some(path.clone());
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

    /// Answers what the save answer answers. Without a path it asks for the save dialog as Export does and answers
    /// none until the app answers the path.
    fn confirm(&mut self, ctx: &egui::Context) -> Option<Self::Result> {
        if !self.folder_export && self.options.scale_all.len() > 1 {
            return None;
        }
        let Some(path) = self.path.clone() else {
            if !self.saving {
                let words = Env::of_root(ctx).words::<ExportWords>();
                let several_files = self.folder_export && self.files > 1;
                self.requests.request_path(save_request(&self.formats, &self.options, &self.file_name, several_files, &words));
                self.saving = true;
            }
            return None;
        };
        Some(answer_of(self.options.clone(), self.source, path, self.remember.clone()))
    }

    /// Waiting while the save dialog Export asked for is open.
    fn held_state(&self) -> Option<HeldOk> {
        self.saving.then_some(HeldOk::Waiting)
    }

    /// The options by their serde keys, and the path (`path`, null until one was answered or set).
    fn json_fields(&self) -> Option<serde_json::Value> {
        let mut fields = serde_fields(&self.options)?;
        if let serde_json::Value::Object(map) = &mut fields {
            let path = self.path.as_ref().map_or(serde_json::Value::Null, |path| serde_json::Value::String(path.to_string_lossy().into_owned()));
            map.insert("path".to_owned(), path);
        }
        Some(fields)
    }

    /// Sets an option as the serde round trip takes it, or the path to a string. A `resample` method the list does not
    /// offer is refused.
    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        if field == "path" {
            let path = value.as_str().filter(|path| !path.is_empty()).ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
            self.path = Some(PathBuf::from(path));
            return Ok(());
        }
        if field == "resample"
            && !self.methods.is_empty()
            && serde_json::from_value::<Resample>(value.clone()).is_ok_and(|method| !self.methods.contains(&method))
        {
            return Err(FieldRefusal::Rejects(&value).message(field));
        }
        set_serde_field(&mut self.options, field, value)
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<ExportAsResult>> {
        let measuring = crate::runtime::measure::active(ui.ctx());
        let answers = if measuring { Vec::new() } else { self.requests.take_answers() };
        let dialog = ExportAsDialog {
            formats: self.formats.clone(),
            methods: self.methods.clone(),
            title: self.title.clone(),
            defaults: self.defaults,
            preview: self.preview.clone(),
            answers,
            saving: self.saving,
            open: Some(Bind::bound(&mut self.open)),
            presets: self.with_presets.then(|| Bind::bound(&mut self.presets)),
            remember: self.remember.clone(),
            opening: Some(self.opening.clone()),
            folder_export: self.folder_export,
            files: self.files,
            ..ExportAsDialog::new(self.file_name.clone(), self.source, Bind::bound(&mut self.options))
        };
        let mut shown = dialog.show(ui);
        #[cfg(feature = "testing")]
        {
            self.facts = crate::contract::Outcome::story_facts(&shown);
        }
        self.events.extend(std::mem::take(&mut shown.presets));
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

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.facts.clone()
    }

    fn cancelled(&mut self, _: &egui::Context) {
        self.cancel_requests();
    }
}

/// The settings the dialog opens with: the remembered ones when there are any, else `base`, fitted to the formats and
/// the source.
fn opened_options(
    base: ExportOptions,
    last: Option<ExportOptions>,
    formats: &[ExportFormat],
    methods: &[Resample],
    source: PixelSize,
) -> ExportOptions {
    last.unwrap_or(base).fitted(formats, methods, source)
}

/// What the rows read besides the dialog's state.
struct Pass<'p> {
    formats: &'p [ExportFormat],
    /// The resample methods the list offers; all when empty.
    methods: &'p [Resample],
    source: PixelSize,
    words: &'p ExportWords,
    sizing: std::sync::Arc<SizingWords>,
    units: Units,
    defaults: ExportFormatDefaults,
    /// The bound saved presets; `None` shows no preset row.
    library: Option<&'p RefCell<PresetLibrary<ExportOptions>>>,
    /// Each preset edit of the frame.
    events: &'p RefCell<Vec<NamedPresetEvent<String>>>,
}

impl Pass<'_> {
    fn settings(&self, ui: &mut Ui, options: &mut ExportOptions, held: &mut Held) {
        let words = self.words;
        let mut format = None;
        Section::fixed(words.file_settings.clone()).id_salt("file").show(ui, |ui, _| format = self.file_settings(ui, options));
        let Some(format) = format else { return };
        Section::fixed(words.color_space.clone()).id_salt("color-space").show(ui, |ui, _| {
            let mut srgb = options.profile == ExportProfile::Srgb;
            check(ui, &words.convert_srgb, "profile", &mut srgb);
            options.profile = if srgb { ExportProfile::Srgb } else { ExportProfile::Document };
            check(ui, &words.embed_profile, "embed", &mut options.embed_profile);
        });

        Section::fixed(words.image_size.clone()).id_salt("size").show(ui, |ui, _| self.size(ui, options, held, format));
        Section::fixed(words.scale_all.clone()).id_salt("scale-all").show(ui, |ui, _| self.scale_all(ui, options));
        if let Some(library) = self.library {
            self.preset_row(ui, options, held, library);
        }
    }

    /// File Settings: the format, its options and the metadata; the format shown, `None` when the list has none.
    fn file_settings(&self, ui: &mut Ui, options: &mut ExportOptions) -> Option<&ExportFormat> {
        let words = self.words;
        let chosen = self.formats.iter().position(|format| format.id == options.format).unwrap_or(0);
        let entries: Vec<Entry<usize>> = self.formats.iter().enumerate().map(|(at, format)| Entry::new(at, format.name)).collect();
        let mut now = chosen;
        choose(ui, &words.format, "format", &entries, &mut now);
        if now != chosen {
            self.switch(options, now);
        }
        let format = self.formats.get(now)?;

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
        Some(format)
    }

    /// The preset row under Image Size: a pick applies the preset's options, and each new, renamed or deleted preset
    /// is made in the library and reported in `events`, keyed by the preset's name before the event.
    fn preset_row(&self, ui: &mut Ui, options: &mut ExportOptions, held: &mut Held, library: &RefCell<PresetLibrary<ExportOptions>>) {
        let words = self.words;
        let shown = {
            let presets = library.borrow();
            let names: Vec<String> = presets.items().iter().map(|item| item.name.clone()).collect();
            let rows = PresetSource::new(names.as_slice(), PresetFns::new(name_rows));
            let current = presets.items().iter().find(|item| item.value == *options).map(|item| item.name.clone());
            let picker = PresetPicker::new(words.preset.clone(), Bind::given(current), &rows, Bind::bound(&mut held.picker)).field(PickerField::Name);
            NamedPresetRow::new(words.preset.clone(), picker)
                .suggest_name(words.new_preset_name.as_str().to_owned())
                .check_name(NameCheck::new(presets.names(), |names, typed| names.name_error(typed)))
                .show(ui)
        };
        for event in shown.events {
            let mut presets = library.borrow_mut();
            let id_of =
                |presets: &PresetLibrary<ExportOptions>, name: &str| presets.items().iter().find(|item| item.name == name).map(|item| item.id);
            match event {
                NamedPresetEvent::Apply(name) => {
                    if let Some(item) = presets.items().into_iter().find(|item| item.name == name) {
                        *options = item.value.clone().fitted(self.formats, self.methods, self.source);
                        held.error = None;
                    }
                }
                NamedPresetEvent::Create { name } => {
                    if let Ok(id) = presets.add(&GroupPath::root(), &name, options.clone()) {
                        let stored = presets.get(id).map_or(name, |item| item.name.clone());
                        self.events.borrow_mut().push(NamedPresetEvent::Create { name: stored });
                    }
                }
                NamedPresetEvent::Rename { key, text } => {
                    if let Some(id) = id_of(&presets, &key)
                        && presets.rename(id, &text).is_ok()
                    {
                        self.events.borrow_mut().push(NamedPresetEvent::Rename { key, text });
                    }
                }
                NamedPresetEvent::Delete(name) => {
                    if let Some(id) = id_of(&presets, &name)
                        && presets.delete(id).is_ok()
                    {
                        self.events.borrow_mut().push(NamedPresetEvent::Delete(name));
                    }
                }
                _ => {}
            }
        }
    }

    /// Scale All: a row per size, each a size list, a suffix field and a trash button (disabled while one row is left),
    /// and + adding the next size. The rows are kept only while there is more than one, so a single 1x row with no
    /// suffix is no rows.
    fn scale_all(&self, ui: &mut Ui, options: &mut ExportOptions) {
        let words = self.words;
        let mut rows = options.scale_rows();
        let sizes: Vec<Entry<ScaleSize>> = ScaleSize::ALL.iter().map(|size| Entry::new(*size, size.name())).collect();
        let next = ScaleRow::next(&rows);
        let add = IconButton::new(IconId::new("plus"))
            .label(words.add_size.clone())
            .tooltip(words.add_size.clone())
            .enabled(next.is_some())
            .id_salt("add-size")
            .show(ui);
        if clicked(&add)
            && let Some(row) = next
        {
            rows.push(row);
        }
        let single = rows.len() == 1;
        let mut remove = None;
        for (at, row) in rows.iter_mut().enumerate() {
            let n = (at + 1).to_string();
            Row::new().id_salt(("scale-row", at)).show(ui, |ui, _| {
                let name = words.size_n.fill(&[("n", n.as_str())]);
                if let Some(size) = Select::new(name, Bind::given(row.size), &sizes).show(ui).value().copied() {
                    row.size = size;
                }
                let name = Label::from(words.suffix_n.fill(&[("n", n.as_str())]));
                let typed =
                    TextField::new(name, Bind::given(row.suffix.clone())).limit(ScaleRow::MAX_SUFFIX).placeholder(words.suffix.clone()).show(ui);
                if let Some(suffix) = typed.value().cloned() {
                    row.suffix = suffix;
                }
                let name = Label::from(words.remove_size.fill(&[("n", n.as_str())]));
                if clicked(&IconButton::new(IconId::new("trash")).label(name.clone()).tooltip(name).enabled(!single).id_salt("remove").show(ui)) {
                    remove = Some(at);
                }
            });
        }
        if let Some(at) = remove {
            rows.remove(at);
        }
        let rows = if rows.len() == 1 && rows[0] == ScaleRow::default() { Vec::new() } else { rows };
        if rows != options.scale_all {
            options.scale_all = rows;
        }
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
            let entries = Resample::entries(&self.sizing, self.methods);
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

/// One preset row per name, keyed by it.
fn name_rows(names: &&[String], _: &Env) -> Vec<PresetRow<String>> {
    names.iter().map(|name| PresetRow::item(name.clone(), name.clone(), 0)).collect()
}
