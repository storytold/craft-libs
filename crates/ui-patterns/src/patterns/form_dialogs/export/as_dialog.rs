//! `ExportAsDialog`: File, Export, Export As. The settings are on the left (the format, its options, metadata, colour
//! profile and image size); the preview with the size estimate is on the right. The dialog never encodes: it
//! reports the revision of the settings it wants a preview for and shows the answer the app gives back.

use super::{
    ExportAsAsset, ExportAsResult, ExportAsRow, ExportAsRowEstimate, ExportAsRowResult, ExportAsRows, ExportFormat, ExportFormatDefaults,
    ExportMetadata, ExportOptions, ExportPreview, ExportProfile, ExportSize, ExportWords, Revisions, ScaleRow, ScaleSize, SizeEstimate,
    export_file_name, size_error_label,
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
use crate::contract::{
    Bind, CollectionEvent, ColumnSpec, Entry, NoValue, Outcome, Props as _, Remember as _, SaveKey, SelectMode, Show as _, ShowCollection as _,
    ShowContainer as _, Sizing, SpaceAction, ToggleColumn,
};

use crate::display::{Shade, Text};

use crate::form::{Field, FieldGroup, SliderField, SliderFieldLayout};

use crate::layout_components::{Row, Stack};
use crate::list::{List, ListRow, ThumbSize, ToggleCell, ToggleState};

use crate::patterns::form_dialogs::{
    FormDialog, FormOutput, HostRequests, LinkedSize, MAX_DOCUMENT_SIDE, PreviewPane, PreviewRequest, PreviewState, PreviewView, Resample,
    ResetFooter, ResetPick, SizeError, SizingWords, resize_from_scale,
};
use crate::pickers::Select;
use crate::primitives::IconSpec;
use crate::runtime::env::{Env, Label, UnitKind, Units};

use crate::runtime::overlay::CloseReason;
use crate::sections::Section;
use crate::style::TypeRole;
use crate::text::NumberField;

use craft_behavior::{ColumnKey, ColumnWidth, PixelSize, Selection, keyed};

use craft_theme::variant::Tone;
use egui::{Pos2, Rect, Response, Ui};

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;

/// The least and greatest scale, in percent.
const SCALE_RANGE: std::ops::RangeInclusive<f64> = 1.0..=1000.0;
/// The least and greatest quality.
const QUALITY_RANGE: std::ops::RangeInclusive<f64> = 1.0..=100.0;
/// The key of the save request the dialog files at Export.
const SAVE_KEY: &str = "export";
/// The most rows an asset list shows at once before it scrolls them in a well of its own.
const DEFAULT_ASSET_ROWS: u16 = 10;

/// The column of an asset row's tick box.
const TICK: ColumnKey = ColumnKey::new("tick");
/// The column of an asset row's name, which is the row's label.
const ROW_NAME: ColumnKey = ColumnKey::new("rowName");
/// The column of an asset row's format.
const ROW_FORMAT: ColumnKey = ColumnKey::new("rowFormat");
/// The column of an asset row's width and height.
const ROW_SIZE: ColumnKey = ColumnKey::new("rowSize");
/// The column of an asset row's size estimate.
const ROW_ESTIMATE: ColumnKey = ColumnKey::new("rowEstimate");
/// The whole of a thumbnail's texture.
const WHOLE: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));

/// What one row of the asset list shows as text: the asset's name, and for an asset with pixels the name of the
/// format its row exports with, the size those settings give that asset and the estimate the app answered for them.
/// An asset with no pixels shows nothing beside its name.
#[derive(Clone, Debug, Default, Hash)]
struct RowText {
    name: String,
    format: String,
    dimensions: String,
    estimate: String,
}

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
    /// Which row's settings the form's value holds, counted from 0; unused without an asset list.
    value_row: usize,
}

impl Default for Held {
    fn default() -> Self {
        Held { revisions: Revisions::default(), shown: None, view: PreviewView::fitting(), error: None, picker: PresetState::default(), value_row: 0 }
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
    /// The rows whose size estimate the app should measure, when the dialog shows an asset list
    /// ([`rows`](ExportAsDialog::rows)): each entry names the asset and the settings to measure, and the dialog asks
    /// for a row once per change of its settings. An asset with no pixels is asked for nothing. The app answers with
    /// [`HostedExportAs::set_row_estimate`].
    pub wanted_estimates: Vec<ExportAsRowEstimate>,
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
///   turns Cancel into Reset, which restores the selected row's own opening settings when the dialog draws an
///   [`asset list`](ExportAsDialog::rows) and the value the dialog opened with when it draws none. Export asks the app
///   for a file
///   to write (`ExportAsOutput::requests`), or a folder with more than one Scale All row, and the dialog stays open
///   until the app answers with the path. Without [`folder_export`](ExportAsDialog::folder_export), more than one row
///   refuses Export with a note. An export of several files ([`files`](ExportAsDialog::files)) asks for a folder
///   too where the app can write one.
/// - Settings: the dialog opens on the value in the bind, fitted to the document, and writes its edits back.
/// - Previews: whenever the settings change, `wanted_preview` carries a new revision. The app encodes a proxy and
///   gives it back with `preview`; a size estimate shows at once ("≈ 1.2 MB"), and the app may follow with an exact
///   one for the same revision.
/// - Several assets ([`rows`](ExportAsDialog::rows)): the dialog draws a list on its left, one row per asset with its
///   tick box, thumbnail, name, format, size and estimate. The arrow keys move which row is selected and Space flips
///   that row's tick box; the settings and the preview are the selected row's, and the row shows them as they change.
///   An asset with no pixels says so where the preview is, and a row that is not ticked carries a warning mark that
///   says the image will not be exported. Export asks for a folder when more than one row is ticked, and the answer
///   carries each ticked row with the settings it exports with.
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
    /// pass (a field edited before the dialog was first shown is not the Reset target). With an
    /// [`asset list`](ExportAsDialog::rows) the rows carry their own opening settings, and Reset restores the selected
    /// row's.
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
    /// The assets of an export of several assets: one row each, and which row the settings edit. `None` draws no list,
    /// which is how every caller of [`new`](ExportAsDialog::new) works today.
    #[prop]
    rows: Option<Bind<'a, ExportAsRows>>,
    /// The most rows the asset list shows at once: past this it is as tall as its first rows and scrolls the rest in a
    /// well of its own, so a long list stays in the dialog and its last row is reached by the wheel. A list of no rows
    /// is not a shape this dialog has: unset, or a count below one, ten rows.
    #[prop]
    max_rows: Option<u16>,
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
            rows: None,
            max_rows: None,
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
            mut rows,
            max_rows,
        } = self;
        let env = Env::of(ui);
        let words = env.words::<ExportWords>();
        // A frame can run the body twice: the library lives in a cell the passes share, and `events` collects the edits
        // of both passes (the first pass alone gets the frame's input).
        let library = presets.as_mut().map(|bind| RefCell::new(read_bind(bind).unwrap_or_default()));
        let listed = rows.as_mut().map(|bind| RefCell::new(read_bind(bind).unwrap_or_default()));
        let events = RefCell::new(Vec::new());
        // The estimates the list asked the app for in this frame, in the passes it runs.
        let asked = RefCell::new(Vec::new());
        let pass = Pass {
            formats: &formats,
            methods: &methods,
            source: Cell::new(source),
            words: &words,
            sizing: env.words::<SizingWords>(),
            units: env.units(),
            defaults,
            library: library.as_ref(),
            events: &events,
            asked: &asked,
            list_id: ui.scope_id().with("export-as-rows"),
            max_rows: max_rows.filter(|rows| *rows > 0).unwrap_or(DEFAULT_ASSET_ROWS),
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
            // With an asset list the settings belong to the rows, so what Reset restores belongs to a row too: the
            // settings the selected row opened with, fitted to that row's own asset. The caller's opening settings
            // stand for a dialog over one file and for a list with no rows in it.
            let row_opening = listed.as_ref().and_then(|cell| {
                let held = cell.borrow();
                held.rows.get(held.selected.min(held.rows.len().saturating_sub(1))).map(|row| (row.asset.options.clone(), row.asset.source))
            });
            let opening = match row_opening {
                Some((options, asset)) => options.fitted(&formats, &methods, asset),
                None => opened_options(opening, last, &formats, &methods, source),
            };
            dialog = dialog.opening(opening);
        }
        let shown = dialog.show(ui, |ui, form, values| {
            let first = form.first();
            if first {
                let last = remember.as_ref().and_then(|key| form.saved::<Option<ExportOptions>>(key).clone());
                *values = opened_options(values.clone(), last, &formats, &methods, source);
                if let Some(cell) = listed.as_ref() {
                    // Each row opens with the settings the app gave it, fitted to the row's own asset.
                    let mut held_rows = cell.borrow_mut();
                    for row in held_rows.rows.iter_mut() {
                        row.options = row.options.clone().fitted(&formats, &methods, row.asset.source);
                    }
                }
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
            // The row of the asset list whose settings this pass edits; `None` without a list.
            let mut editing = None;
            Row::new().show(ui, |ui, _| {
                if let Some(cell) = listed.as_ref()
                    && !cell.borrow().rows.is_empty()
                {
                    Stack::new().show(ui, |ui, _| pass.row_list(ui, cell));
                    // The list ran first, so the settings, the preview and the answer follow the row it selected. The
                    // value is read anew every pass, including a measuring one: a pass that edits nothing still writes
                    // its value back into the row, and loading only on a change would write one row's settings into
                    // another.
                    let at = pass.row_of(cell);
                    pass.source.set(cell.borrow().rows[at].asset.source);
                    // The row's settings are the value the form edits. They are read again when the row changes rather
                    // than in every pass: settings a host or a Reset put into the value stay, and the pass that follows
                    // writes them into the row.
                    if first || held.value_row != at {
                        *values = cell.borrow().rows[at].options.clone();
                    }
                    held.value_row = at;
                    editing = Some(at);
                }
                Stack::new().show(ui, |ui, _| {
                    FieldGroup::new(words.title.clone()).show(ui, |ui, _| pass.settings(ui, values, &mut held));
                });
                let empty = editing.is_some_and(|at| listed.as_ref().is_some_and(|cell| cell.borrow().rows[at].asset.empty));
                // A measuring pass edits nothing, so it writes nothing back into the row either.
                if let (Some(cell), Some(at)) = (listed.as_ref(), editing)
                    && !crate::runtime::measure::active(ui.ctx())
                    && let Some(row) = cell.borrow_mut().rows.get_mut(at)
                {
                    row.options.clone_from(values);
                }
                Stack::new().show(ui, |ui, _| pass.preview(ui, values, &mut held, empty));
            });
            let several = listed.as_ref().is_some_and(|cell| cell.borrow().ticked().count() > 1);
            let exportable = folder_export || (!several && values.scale_all.len() <= 1);
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
        let several_files = folder_export && (files > 1 || listed.as_ref().is_some_and(|cell| cell.borrow().ticked().count() > 1));
        if reset.pick(picked) == Some(ResetPick::Accept)
            && let Some(options) = &latest
        {
            requests.request_path(save_request(&formats, options, &file_name, several_files, &words));
        }
        let listed_rows = listed.as_ref().map(|cell| cell.borrow());
        let result = accepted.zip(answered.into_inner()).map(|(options, path)| answer_of(options, source, path, remember, listed_rows.as_deref()));
        drop(listed_rows);
        let events = events.into_inner();
        if !events.is_empty()
            && let (Some(bind), Some(cell)) = (presets.as_mut(), library)
        {
            write_bind(bind, cell.into_inner());
        }
        if let (Some(bind), Some(cell)) = (rows.as_mut(), listed) {
            write_bind(bind, cell.into_inner());
        }
        ExportAsOutput {
            response,
            closed,
            result,
            wanted_preview: wanted.into_inner(),
            requests,
            presets: events,
            wanted_estimates: asked.into_inner(),
        }
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

/// What Export answers with: the confirmed options, the chosen path, the output size, the rows of an asset-list
/// export, and the options to remember when the dialog has a key. The rows are the ticked ones, in the list's order,
/// each with the settings the user confirmed for it and its size from its own asset; an export over one file has none.
/// The single-file size and outputs describe the row the settings edit — with an asset list that row's own asset, the
/// same asset its entry in the rows is measured from — and the document's size only for a dialog over one file.
fn answer_of(options: ExportOptions, source: PixelSize, path: PathBuf, remember: Option<SaveKey>, listed: Option<&ExportAsRows>) -> ExportAsResult {
    let rows = listed.map_or_else(Vec::new, |rows| {
        rows.ticked()
            .map(|row| ExportAsRowResult {
                id: row.asset.id.clone(),
                size: row.options.output_size(row.asset.source),
                outputs: row.options.outputs(row.asset.source),
                options: row.options.clone(),
            })
            .collect()
    });
    let source = listed.and_then(|rows| rows.rows.get(rows.selected.min(rows.rows.len().saturating_sub(1)))).map_or(source, |row| row.asset.source);
    ExportAsResult {
        size: options.output_size(source),
        outputs: options.outputs(source),
        rows,
        remember: remember.map(|key| (key, options.clone())),
        options,
        path,
    }
}

/// An [`ExportAsDialog`] the [`DialogHost`](crate::dialog::DialogHost) owns. It keeps the settings and the app's
/// preview, keeps the preview and save requests until the app takes them, and finishes with the result when the save
/// dialog's path comes back. With [`assets`](HostedExportAs::assets) it also keeps the asset list: one row per asset,
/// the selected row's settings, the tick boxes and the size estimates the app has been asked for.
///
/// ```ignore
/// host.open(HostedExportAs::new("poster", PixelSize::new(1200, 800), ExportOptions::default()));
/// // each pass: wanted_preview() -> encode -> set_preview(preview);
/// // pick_path() -> file dialog -> answer_path(key, path), or cancel_requests() when the user cancels it
/// ```
///
/// ```ignore
/// host.open(HostedExportAs::new("poster", PixelSize::new(1200, 800), options).assets(rows));
/// // each pass: wanted_preview() and wanted_estimates() -> the app's answers;
/// // rows() reads which rows are ticked and what each row exports with
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
    /// The asset list, as the user left it; `None` over one file.
    rows: Option<ExportAsRows>,
    /// How many rows of that list the dialog shows at once before it scrolls them
    /// ([`ExportAsDialog::max_rows`]).
    max_rows: Option<u16>,
    /// The row estimates the app has not taken yet.
    estimates: Vec<ExportAsRowEstimate>,
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
            rows: None,
            max_rows: None,
            estimates: Vec::new(),
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

    /// Opens with one row per asset ([`ExportAsRows`]) in a list on the dialog's left: the settings and the preview
    /// follow the selected row, a tick box decides which rows Export writes, and Export asks for a folder where more
    /// than one row is ticked. Each row starts ticked unless its asset has no pixels, and with the settings the asset
    /// carries, fitted to that asset; holding Alt and pressing Reset puts those settings back into the row that is
    /// selected, and into no other. The asset ids must be unique ([`ExportAsAsset::id`]): what an app calls its assets
    /// may repeat — a layers panel holds layers of the same name — so the host passes ids its names do not share. The
    /// list shows ten rows at a time and scrolls the rest in a well of its own; [`max_rows`](Self::max_rows) changes
    /// that.
    #[must_use]
    pub fn assets(mut self, assets: impl IntoIterator<Item = ExportAsAsset>) -> Self {
        self.rows = Some(ExportAsRows::new(assets));
        self
    }

    /// Shows at most `rows` of the asset list at once; past that the list is as tall as its first `rows` rows and
    /// scrolls the rest itself ([`ExportAsDialog::max_rows`]). A count below one reads as the default of ten, and
    /// without [`assets`](Self::assets) there is no list to bound.
    #[must_use]
    pub fn max_rows(mut self, rows: u16) -> Self {
        self.max_rows = Some(rows);
        self
    }

    /// The asset list as the user left it, `None` when the dialog was opened over one file: which rows are ticked,
    /// what each row exports with, and which row the settings edit.
    pub fn rows(&self) -> Option<&ExportAsRows> {
        self.rows.as_ref()
    }

    /// The rows whose size estimate the app should measure: the asset's id and the settings to measure them for.
    /// Returned once per change of a row's settings; an asset with no pixels is asked for nothing. Answer with
    /// [`set_row_estimate`](Self::set_row_estimate).
    pub fn wanted_estimates(&mut self) -> Vec<ExportAsRowEstimate> {
        std::mem::take(&mut self.estimates)
    }

    /// The app's estimate for row `id`, measured for `options`: the row shows it while it still holds those settings,
    /// and waits ("…") while it holds settings the app has been asked for and has not answered. An answer for settings
    /// the row no longer holds does nothing.
    pub fn set_row_estimate(&mut self, id: &str, options: &ExportOptions, estimate: Option<SizeEstimate>) {
        if let Some(rows) = self.rows.as_mut()
            && let Some(asked) = rows.estimates.iter_mut().find(|asked| asked.id == id && &asked.asked == options)
        {
            asked.estimate = estimate;
        }
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

    /// The settings the dialog shows: with an asset list, the selected row's.
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
        let ticked = self.rows.as_ref().map_or(0, |rows| rows.ticked().count());
        if !self.folder_export && (self.options.scale_all.len() > 1 || ticked > 1) {
            return None;
        }
        let Some(path) = self.path.clone() else {
            if !self.saving {
                let words = Env::of_root(ctx).words::<ExportWords>();
                let several_files = self.folder_export && (self.files > 1 || ticked > 1);
                self.requests.request_path(save_request(&self.formats, &self.options, &self.file_name, several_files, &words));
                self.saving = true;
            }
            return None;
        };
        Some(answer_of(self.options.clone(), self.source, path, self.remember.clone(), self.rows.as_ref()))
    }

    /// Waiting while the save dialog Export asked for is open.
    fn held_state(&self) -> Option<HeldOk> {
        self.saving.then_some(HeldOk::Waiting)
    }

    /// The options by their serde keys, and the path (`path`, null until one was answered or set). With an asset list
    /// the selected row's id is `row` (null without one).
    fn json_fields(&self) -> Option<serde_json::Value> {
        let mut fields = serde_fields(&self.options)?;
        if let serde_json::Value::Object(map) = &mut fields {
            let path = self.path.as_ref().map_or(serde_json::Value::Null, |path| serde_json::Value::String(path.to_string_lossy().into_owned()));
            map.insert("path".to_owned(), path);
            let row = self
                .rows
                .as_ref()
                .and_then(|rows| rows.rows.get(rows.selected))
                .map_or(serde_json::Value::Null, |row| serde_json::Value::String(row.asset.id.clone()));
            map.insert("row".to_owned(), row);
        }
        Some(fields)
    }

    /// Sets an option as the serde round trip takes it, or the path to a string. A `resample` method the list does not
    /// offer is refused. With an asset list the option fields edit the selected row, and `row` selects the row with
    /// that id; an id the list has no row for is refused.
    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        if field == "path" {
            let path = value.as_str().filter(|path| !path.is_empty()).ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
            self.path = Some(PathBuf::from(path));
            return Ok(());
        }
        if field == "row" {
            let id = value.as_str().filter(|id| !id.is_empty()).ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
            let refusal = || FieldRefusal::Rejects(&value).message(field);
            let Some(rows) = self.rows.as_mut() else { return Err(refusal()) };
            let Some(at) = rows.rows.iter().position(|row| row.asset.id == id) else { return Err(refusal()) };
            rows.selected = at;
            if let Some(row) = rows.rows.get(at) {
                self.options = row.options.clone();
            }
            return Ok(());
        }
        if field == "resample"
            && !self.methods.is_empty()
            && serde_json::from_value::<Resample>(value.clone()).is_ok_and(|method| !self.methods.contains(&method))
        {
            return Err(FieldRefusal::Rejects(&value).message(field));
        }
        set_serde_field(&mut self.options, field, value)?;
        // The selected row owns the settings the pass reads back, so an answer set here reaches that row and no other.
        if let Some(rows) = self.rows.as_mut()
            && let Some(row) = rows.rows.get_mut(rows.selected)
        {
            row.options.clone_from(&self.options);
        }
        Ok(())
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
            rows: self.rows.as_mut().map(Bind::bound),
            max_rows: self.max_rows,
            ..ExportAsDialog::new(self.file_name.clone(), self.source, Bind::bound(&mut self.options))
        };
        let mut shown = dialog.show(ui);
        #[cfg(feature = "testing")]
        {
            self.facts = crate::contract::Outcome::story_facts(&shown);
        }
        self.events.extend(std::mem::take(&mut shown.presets));
        self.estimates.extend(std::mem::take(&mut shown.wanted_estimates));
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
    /// The size the settings, the preview and the answer are for: the document's, or the selected asset's once the
    /// asset list has run in this pass.
    source: Cell<PixelSize>,
    words: &'p ExportWords,
    sizing: std::sync::Arc<SizingWords>,
    units: Units,
    defaults: ExportFormatDefaults,
    /// The bound saved presets; `None` shows no preset row.
    library: Option<&'p RefCell<PresetLibrary<ExportOptions>>>,
    /// Each preset edit of the frame.
    events: &'p RefCell<Vec<NamedPresetEvent<String>>>,
    /// The row estimates the list asked the app for in the frame.
    asked: &'p RefCell<Vec<ExportAsRowEstimate>>,
    /// The id of the asset list, so the collection keeps its lead and its layout from pass to pass.
    list_id: egui::Id,
    /// The most rows the asset list shows at once before it scrolls them in a well of its own.
    max_rows: u16,
}

impl Pass<'_> {
    /// The row the asset list has selected, kept inside the rows the app gave.
    fn row_of(&self, cell: &RefCell<ExportAsRows>) -> usize {
        let rows = cell.borrow();
        rows.selected.min(rows.rows.len().saturating_sub(1))
    }

    /// The text `row` shows: its asset's name, and for an asset with pixels the name of the format its settings pick,
    /// the size those settings give that asset and the estimate the app answered for the settings the row holds.
    /// `asked_by_id` holds the list's estimate entries, grouped by the asset id they belong to.
    fn row_text(&self, row: &ExportAsRow, asked_by_id: &HashMap<&str, Vec<&ExportAsRowEstimate>>) -> RowText {
        let words = self.words;
        let asset = &row.asset;
        if asset.empty {
            return RowText { name: asset.name.clone(), ..RowText::default() };
        }
        let format = self
            .formats
            .iter()
            .find(|format| format.id == row.options.format)
            .or(self.formats.first())
            .map_or_else(String::new, |format| format.name.to_uppercase());
        let size = row.options.output_size(asset.source);
        let dimensions = words.dimensions.fill(&[
            ("width", self.units.number(f64::from(size.width), 0..=0).as_str()),
            ("height", self.units.number(f64::from(size.height), 0..=0).as_str()),
        ]);
        let asked = asked_by_id.get(asset.id.as_str()).and_then(|entries| entries.iter().rev().find(|asked| asked.asked == row.options));
        let estimate = match asked.and_then(|asked| asked.estimate) {
            Some(SizeEstimate::Proxy(bytes)) => words.estimate.fill(&[("size", self.units.file_size(bytes).as_str())]),
            Some(SizeEstimate::Exact(bytes)) => self.units.file_size(bytes),
            // Asked for and not answered yet: the mark the footer note shows while it waits. An empty asset is asked
            // for nothing, and its row shows no estimate at all.
            None => words.estimating.as_str().to_owned(),
        };
        RowText { name: asset.name.clone(), format, dimensions, estimate }
    }

    /// The asset list on the dialog's left: one row per asset, each with its tick box, picture, name, format, size and
    /// estimate. The arrow keys move which row is selected — the row the settings beside the list edit — and Space
    /// flips the selected row's tick box, as does a press on the box. A row that is not ticked carries a warning mark
    /// that says the image will not be exported; an asset with no pixels has no picture, format, size or estimate.
    /// The list shows [`max_rows`](ExportAsDialog::max_rows) rows at a time and scrolls the rest in a well of its own.
    /// Each row whose settings have no estimate yet is asked of the app, once.
    fn row_list(&self, ui: &mut Ui, cell: &RefCell<ExportAsRows>) {
        let words = self.words;
        let (rows, estimates) = {
            let held = cell.borrow();
            (held.rows.clone(), held.estimates.clone())
        };
        // The id is the only handle a row, its estimate answer and its result share (`ExportAsAsset::id`), so the
        // entries the app has been asked for are grouped by it: a panel of a few hundred layers asks for its
        // estimates, keeps them and shows them in a pass over its rows, not one over every pair of row and entry.
        let mut asked_by_id: HashMap<&str, Vec<&ExportAsRowEstimate>> = HashMap::new();
        for asked in &estimates {
            asked_by_id.entry(asked.id.as_str()).or_default().push(asked);
        }
        let texts: Vec<RowText> = rows.iter().map(|row| self.row_text(row, &asked_by_id)).collect();
        // The list measures its Content columns from the text its rows hold and keeps that measure while its rows are
        // the same set — so the text is what says the set is the same. An estimate the app answers after the dialog
        // opened, a format the user picked while another row was selected and a unit the app switched to all change
        // what a row holds, and the column holding it has to be measured again. A stamp that changes starts the lead
        // over, which this list survives: its selection is bound below, and the lead comes back from that selection.
        let stamp = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(&rows.len(), &mut hasher);
            for (row, text) in rows.iter().zip(&texts) {
                std::hash::Hash::hash(&row.ticked, &mut hasher);
                std::hash::Hash::hash(text, &mut hasher);
            }
            std::hash::Hasher::finish(&hasher)
        };
        let columns = vec![
            ColumnSpec::toggle(TICK, Label::Empty, ToggleColumn::sweepable()),
            ColumnSpec::new(ROW_NAME, Label::Empty, ColumnWidth::fill(1.0)),
            ColumnSpec::new(ROW_FORMAT, Label::Empty, ColumnWidth::Content),
            ColumnSpec::new(ROW_SIZE, Label::Empty, ColumnWidth::Content),
            ColumnSpec::new(ROW_ESTIMATE, Label::Empty, ColumnWidth::Content),
        ];
        let order: Vec<usize> = (0..rows.len()).collect();
        let names: Vec<String> = texts.iter().map(|text| text.name.clone()).collect();
        let ticked: Vec<bool> = rows.iter().map(|row| row.ticked).collect();
        let source = keyed(&order[..], |at: &usize| *at)
            .with_text(move |at: &usize| Cow::Owned(names[*at].clone()))
            .with_toggled(move |at: &usize, column: &ColumnKey| *column == TICK && ticked[*at]);
        let selected = cell.borrow().selected.min(rows.len().saturating_sub(1));
        let mut selection = if rows.is_empty() { Selection::default() } else { Selection::single(selected) };
        let out = List::<usize>::new(words.assets.clone())
            .id(self.list_id)
            .select(SelectMode::Single)
            .allow_empty(false)
            .arrows_select(true)
            .header(false)
            .space(SpaceAction::ToggleCell)
            .sizing(Sizing::FillWidth)
            .max_rows(self.max_rows)
            .rows_stamp(stamp)
            .columns(columns)
            .selection(Bind::bound(&mut selection))
            .show_items(ui, &source, &mut |at, icx| {
                let row = &rows[*at];
                let text = &texts[*at];
                let asset = &row.asset;
                let mut item = ListRow::new(text.name.clone()).cells(icx.cells()).state(icx.row.clone()).toggle(ToggleCell::check(
                    TICK,
                    ToggleState::of(row.ticked),
                    asset.name.clone(),
                ));
                if let (false, Some(texture)) = (asset.empty, asset.thumbnail) {
                    item = item.thumb(ThumbSize::Small, move |paint, rect| paint.picture(texture.id, rect, WHOLE, 0.0));
                }
                if !asset.empty {
                    item =
                        item.cell(ROW_FORMAT, text.format.clone()).cell(ROW_SIZE, text.dimensions.clone()).cell(ROW_ESTIMATE, text.estimate.clone());
                }
                if !row.ticked {
                    item = item.trailing_icon(IconSpec::svg(IconId::new("warning")), words.row_not_exported.clone());
                }
                item
            });
        let mut held = cell.borrow_mut();
        if let Some(at) = selection.items.first().copied() {
            held.selected = at;
        }
        for event in out.events {
            if let CollectionEvent::Toggle(edit) = event
                && let Some(toggle) = edit.value()
                && let Some(row) = held.rows.get_mut(toggle.key)
            {
                row.ticked = toggle.on;
            }
        }
        // An entry lives while the row of its id still holds the settings it was asked for; a row whose settings have
        // no entry yet is asked for one, once. Both questions go by the id rather than across the other list.
        let mut options_of: HashMap<&str, Vec<&ExportOptions>> = HashMap::new();
        for row in &rows {
            options_of.entry(row.asset.id.as_str()).or_default().push(&row.options);
        }
        held.estimates.retain(|asked| options_of.get(asked.id.as_str()).is_some_and(|options| options.contains(&&asked.asked)));
        let missing: Vec<(String, ExportOptions)> = held
            .rows
            .iter()
            .filter(|row| !row.asset.empty)
            .filter(|row| !asked_by_id.get(row.asset.id.as_str()).is_some_and(|asked| asked.iter().any(|entry| entry.asked == row.options)))
            .map(|row| (row.asset.id.clone(), row.options.clone()))
            .collect();
        for (id, options) in missing {
            let asked = ExportAsRowEstimate { id, asked: options, estimate: None };
            self.asked.borrow_mut().push(asked.clone());
            held.estimates.push(asked);
        }
    }

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
                        *options = item.value.clone().fitted(self.formats, self.methods, self.source.get());
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
        let source = self.source.get();
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

    /// The preview pane with the format and size line under it. For the row of an asset with no pixels, the pane shows
    /// that in the picture's place instead of a picture.
    fn preview(&self, ui: &mut Ui, options: &ExportOptions, held: &mut Held, empty: bool) {
        let words = self.words;
        let state = match (empty, held.shown.as_ref()) {
            (true, _) => PreviewState::Failed(words.image_empty.clone()),
            (false, Some(shown)) => shown.state.clone(),
            (false, None) => PreviewState::Pending(None),
        };
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
