//! HostedSaveForWeb: the Save for Web dialog owned by a DialogHost.

#[cfg(doc)]
use super::MATTE_COLOR_KEY;
use super::model::{
    Animation, ColorTable, ColorTableEdit, ColorTableEvent, OptimizeAnswer, OptimizeRequest, OptimizeScope, OptimizeStart, SaveForWebOptions,
    SaveForWebResult, TableMarks, WantedVariant, WebDocument, WebFormat, WebPreset, WebPresetEvent, WebVariant,
};
use super::options::apply_preset;
use super::{
    BrowserPreview, COLOR_TABLES, DEFAULT_OPTIMIZE_K, OPENING_SPEED, SaveForWebDialog, WEB_SPEEDS, WebInput, WebPictures, WebSlice, WebView,
    ZOOM_RANGE, hex, parse_hex, result_of, save_request,
};
use crate::contract::Bind;
use crate::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldOk, StackDialog, dialog_id, serde_fields, set_serde_field};
use crate::patterns::form_dialogs::LinkedSize;
use crate::patterns::form_dialogs::host_requests::HostRequests;
use crate::runtime::env::Label;
use crate::runtime::memory::SaveKey;
use crate::runtime::requests::{ColorRequest, PathRequest};
use craft_behavior::PixelSize;
use craft_theme::Srgba;
use egui::Ui;
use egui::load::SizedTexture;
use serde_json::Value;
use std::path::PathBuf;

/// A [`SaveForWebDialog`] the [`DialogHost`](crate::dialog::DialogHost) owns. It keeps the settings and what the app
/// answers (the pictures), drops stale answers, keeps the colour and path requests until the app takes them, and
/// finishes with the result when the file dialog's path comes back or the user presses Done.
///
/// ```ignore
/// let mut web = HostedSaveForWeb::new("poster.psd", PixelSize::new(1200, 800), original).animation(Animation::new(12, 24.0));
/// // each pass: for want in web.wanted_variants() { encode(want) -> web.set_variant(variant) }
/// // host.take_color_request() -> picker -> host.answer_color(id, MATTE_COLOR_KEY, color)
/// // web.pick_path() -> file dialog -> web.answer_path(key, path)
/// ```
pub struct HostedSaveForWeb {
    id: DialogId,
    name: String,
    size: PixelSize,
    options: SaveForWebOptions,
    presets: Vec<WebPreset>,
    animation: Option<Animation>,
    remember: Option<SaveKey>,
    pictures: WebPictures,
    taken: u64,
    wanted: Vec<WantedVariant>,
    frame: u32,
    requests: HostRequests,
    saving: bool,
    /// Whether the dialog is open: bound to the dialog's `opened` prop.
    open: bool,
    /// The document's slices.
    slices: Vec<WebSlice>,
    slice_warning: Option<Label>,
    /// The view tab: bound to the dialog's `view` prop, so automation reads and writes it here.
    view: WebView,
    /// The path the app answered the save request with.
    path: Option<PathBuf>,
    document: WebDocument,
    formats: Vec<WebFormat>,
    color_tables: Vec<ColorTable>,
    marks: TableMarks,
    browser: BrowserPreview,
    optimize: bool,
    ok_label: Option<Label>,
    /// The next confirm answers Done: the settings with no path.
    done: bool,
    /// What automation set on the dialog's own state, handed to the next pass once.
    inputs: Vec<WebInput>,
    browser_pressed: bool,
    preset_events: Vec<WebPresetEvent>,
    table_events: Vec<ColorTableEvent>,
    optimize_opened: bool,
    optimize_start: Option<u64>,
    optimize_request: Option<OptimizeRequest>,
    optimize_answer: Option<Result<OptimizeAnswer, Label>>,
    optimize_forget: bool,
    /// What the last pass showed, for `json_fields`.
    speed: f64,
    zoom: Option<f32>,
    selection: Vec<[u8; 3]>,
    optimize_state: Option<(f64, OptimizeStart, OptimizeScope)>,
}

impl HostedSaveForWeb {
    /// The document's slices, for the Slices choice.
    #[must_use]
    pub fn slices(mut self, slices: Vec<WebSlice>) -> Self {
        self.slices = slices;
        self
    }

    /// A notice placed below the preview captions when the host omits slices.
    #[must_use]
    pub fn slice_warning(mut self, warning: Option<Label>) -> Self {
        self.slice_warning = warning;
        self
    }

    /// Picks the variant: a raster or a vector document.
    #[must_use]
    pub fn document(mut self, document: WebDocument) -> Self {
        self.document = document;
        self
    }

    /// The formats the host encodes, in list order: [`WebFormat::STANDARD`] unless set or when empty. Add
    /// [`WebFormat::Avif`] only when the app can write it. The dialog asks for no others: see
    /// [`SaveForWebDialog::formats`].
    #[must_use]
    pub fn formats(mut self, formats: Vec<WebFormat>) -> Self {
        self.formats = formats;
        self
    }

    /// The Color Table list the host offers.
    #[must_use]
    pub fn color_tables(mut self, tables: Vec<ColorTable>) -> Self {
        self.color_tables = tables;
        self
    }

    /// Whether the zoom row has Preview in Browser.
    #[must_use]
    pub fn browser_preview(mut self, preview: BrowserPreview) -> Self {
        self.browser = preview;
        self
    }

    /// Shows "Optimize to File Size…" under the Preset row.
    #[must_use]
    pub fn optimize(mut self) -> Self {
        self.optimize = true;
        self
    }

    /// The label of the confirming button; Save… unless named.
    #[must_use]
    pub fn ok_label(mut self, label: impl Into<Label>) -> Self {
        self.ok_label = Some(label.into());
        self
    }

    /// Preview in Browser was pressed since the last call, once; the host previews `current()`.
    pub fn take_browser_preview(&mut self) -> bool {
        std::mem::take(&mut self.browser_pressed)
    }

    /// What the user did to the presets since the last call; the host stores them and gives the new list with
    /// `set_presets`.
    pub fn take_preset_events(&mut self) -> Vec<WebPresetEvent> {
        std::mem::take(&mut self.preset_events)
    }

    /// Replaces the Preset list on an open dialog.
    pub fn set_presets(&mut self, presets: Vec<WebPreset>) {
        self.presets = presets;
    }

    /// What the host's colour table says, for the cell marks.
    pub fn set_marks(&mut self, marks: TableMarks) {
        self.marks = marks;
    }

    /// The colour table edits since the last call.
    pub fn take_table_events(&mut self) -> Vec<ColorTableEvent> {
        std::mem::take(&mut self.table_events)
    }

    /// The Optimize rows opened since the last call; the host answers the opening size.
    pub fn take_optimize_open(&mut self) -> bool {
        std::mem::take(&mut self.optimize_opened)
    }

    /// The Desired File Size the rows open at, in bytes (32 K for `None`).
    pub fn answer_optimize_start(&mut self, bytes: Option<u64>) {
        self.optimize_start = Some(bytes.unwrap_or((DEFAULT_OPTIMIZE_K * 1024.0) as u64));
    }

    /// The search to run, once.
    pub fn take_optimize_request(&mut self) -> Option<OptimizeRequest> {
        self.optimize_request.take()
    }

    /// The host's answer to the search: `Ok` closes the rows and shows the result line; `Err` keeps them open with the
    /// reason.
    pub fn answer_optimize(&mut self, answer: Result<OptimizeAnswer, Label>) {
        self.optimize_answer = Some(answer);
    }

    /// Drops the shown Optimize result: the art changed.
    pub fn forget_optimized(&mut self) {
        self.optimize_forget = true;
    }

    /// The options as the user left them.
    pub fn current(&self) -> &SaveForWebOptions {
        &self.options
    }

    /// A hosted dialog for the document `name` of `size` pixels showing `original` in the Original pane.
    pub fn new(name: impl Into<String>, size: PixelSize, original: SizedTexture) -> Self {
        HostedSaveForWeb {
            id: dialog_id("save-for-web"),
            name: name.into(),
            size,
            options: SaveForWebOptions::default(),
            presets: Vec::new(),
            formats: WebFormat::STANDARD.to_vec(),
            animation: None,
            remember: None,
            pictures: WebPictures::new().original(original),
            taken: 0,
            wanted: Vec::new(),
            frame: 0,
            requests: HostRequests::new(),
            saving: false,
            open: true,
            slices: Vec::new(),
            slice_warning: None,
            view: WebView::default(),
            path: None,
            document: WebDocument::default(),
            color_tables: COLOR_TABLES.to_vec(),
            marks: TableMarks::default(),
            browser: BrowserPreview::default(),
            optimize: false,
            ok_label: None,
            done: false,
            inputs: Vec::new(),
            browser_pressed: false,
            preset_events: Vec::new(),
            table_events: Vec::new(),
            optimize_opened: false,
            optimize_start: None,
            optimize_request: None,
            optimize_answer: None,
            optimize_forget: false,
            speed: OPENING_SPEED,
            zoom: None,
            selection: Vec::new(),
            optimize_state: None,
        }
    }

    /// The settings the dialog opens with.
    #[must_use]
    pub fn options(mut self, options: SaveForWebOptions) -> Self {
        self.options = options;
        self.options.clamp_exact_size(self.size);
        self
    }

    /// The Preset list.
    #[must_use]
    pub fn presets(mut self, presets: Vec<WebPreset>) -> Self {
        self.presets = presets;
        self
    }

    /// The document is an animation of `animation.frames` frames.
    #[must_use]
    pub fn animation(mut self, animation: Animation) -> Self {
        self.animation = Some(animation);
        self
    }

    /// Remembers the last view and settings under `key`.
    #[must_use]
    pub fn remember(mut self, key: SaveKey) -> Self {
        self.remember = Some(key);
        self
    }

    /// The frame the panes show; the app encodes this frame.
    pub fn frame(&self) -> u32 {
        self.frame
    }

    /// The panes to encode when they changed since the last call.
    pub fn wanted_variants(&mut self) -> Vec<WantedVariant> {
        std::mem::take(&mut self.wanted)
    }

    /// The Original pane's picture from the next pass, for a document the app has rendered anew.
    pub fn set_original(&mut self, original: SizedTexture) {
        self.pictures.original = Some(original);
    }

    /// An encoded pane; one for an older revision than the last asked for is dropped.
    pub fn set_variant(&mut self, variant: WebVariant) {
        if variant.revision >= self.taken {
            self.pictures.set(variant);
        }
    }

    /// Save… was pressed in the pass since the last call: the file dialog to open, keyed [`SAVE_PATH_KEY`](super::SAVE_PATH_KEY).
    /// Returned once; answer with [`answer_path`](Self::answer_path).
    pub fn pick_path(&mut self) -> Option<PathRequest> {
        self.requests.take_path()
    }

    /// Answers the file dialog request `key` with the chosen path, which finishes the dialog.
    pub fn answer_path(&mut self, key: impl Into<String>, path: PathBuf) {
        self.saving = false;
        self.path = Some(path.clone());
        self.requests.answer_path(key, path);
    }

    /// The host cancelled its picker or file dialog: the requests not taken yet are dropped and Save… works again.
    pub fn cancel_requests(&mut self) {
        self.saving = false;
        self.requests.cancel();
    }

    /// Drops the edits of the generated table when the field's change moved the palette's reduction.
    fn restore_if_reduced(&mut self, before: (WebFormat, ColorTable, u16)) {
        if self.options.reduction() != before {
            self.table_events.push(ColorTableEvent { edit: ColorTableEdit::Restore, colors: Vec::new() });
        }
    }

    /// The Optimize rows' fields only take a value while the rows are open.
    fn rows_open(&self, field: &str) -> Result<(), String> {
        if self.optimize_state.is_some() { Ok(()) } else { Err(format!("{field}: the Optimize rows are not open")) }
    }
}

/// The value of `field` parsed as `T`, or the refusal message.
fn parsed<T: serde::de::DeserializeOwned>(field: &str, value: &Value) -> Result<T, String> {
    serde_json::from_value(value.clone()).map_err(|_| FieldRefusal::Rejects(value).message(field))
}

impl StackDialog for HostedSaveForWeb {
    type Result = SaveForWebResult;

    /// Done (the `choice` field) answers at once with the settings and no path. Otherwise it answers only once the app
    /// answered the save request with a path, as the dialog's own Save… does: without a path it asks for the file
    /// dialog as Save… does and answers none.
    fn confirm(&mut self, _: &egui::Context) -> Option<Self::Result> {
        self.options.keep_to(WebFormat::offered(&self.formats));
        if std::mem::take(&mut self.done) {
            return Some(result_of(self.options.clone(), None, self.options.output_size(self.size), self.view, self.remember.clone()));
        }
        let Some(path) = self.path.clone() else {
            if !self.saving {
                self.requests.request_path(save_request(&self.name, self.options.format));
                self.saving = true;
            }
            return None;
        };
        Some(result_of(self.options.clone(), Some(path), self.options.output_size(self.size), self.view, self.remember.clone()))
    }

    fn take_color_request(&mut self) -> Option<ColorRequest> {
        self.requests.take_color()
    }

    fn answer_color(&mut self, key: &str, color: Srgba) {
        self.requests.answer_color(key, color);
    }

    /// Waiting while the file dialog Save… asked for is open.
    fn held_state(&self) -> Option<HeldOk> {
        self.saving.then_some(HeldOk::Waiting)
    }

    fn json_fields(&self) -> Option<serde_json::Value> {
        let mut fields = serde_fields(&self.options)?;
        let map = fields.as_object_mut()?;
        let size = self.options.output_size(self.size);
        map.insert("width".into(), size.width.into());
        map.insert("height".into(), size.height.into());
        map.insert("__constrain".into(), self.options.constrain_proportions.into());
        map.insert("view".to_owned(), serde_json::to_value(self.view).ok()?);
        map.insert("frame".to_owned(), Value::from(self.frame));
        let path = self.path.as_ref().map_or(Value::Null, |path| Value::String(path.to_string_lossy().into_owned()));
        map.insert("path".to_owned(), path);
        map.insert("kbps".to_owned(), Value::from(self.speed));
        map.insert("zoom".to_owned(), self.zoom.map_or_else(|| Value::from("fit"), Value::from));
        map.insert("choice".to_owned(), Value::from(if self.done { "done" } else { "save" }));
        map.insert("colorSelection".to_owned(), self.selection.iter().map(|color| Value::String(hex(*color))).collect());
        if let Some((k, start, scope)) = self.optimize_state {
            map.insert("optimizeK".to_owned(), Value::from(k));
            map.insert("optimizeStart".to_owned(), serde_json::to_value(start).ok()?);
            map.insert("optimizeUse".to_owned(), serde_json::to_value(scope).ok()?);
        }
        Some(fields)
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        let before = self.options.reduction();
        let rejected = || FieldRefusal::Rejects(&value).message(field);
        match field {
            "view" => {
                self.view = parsed(field, &value)?;
                Ok(())
            }
            "path" => {
                let Value::String(path) = &value else { return Err(rejected()) };
                self.path = Some(PathBuf::from(path));
                Ok(())
            }
            // The panes show the frame from the next pass; one past the last frame is the last, as Play keeps it.
            "frame" => {
                let frame = value.as_u64().ok_or_else(rejected)?;
                let last = self.animation.map_or(0, |animation| animation.frames.saturating_sub(1));
                self.frame = u32::try_from(frame).unwrap_or(u32::MAX).min(last);
                Ok(())
            }
            "kbps" => {
                let kbps = value.as_f64().filter(|kbps| WEB_SPEEDS.contains(kbps)).ok_or_else(rejected)?;
                self.speed = kbps;
                self.inputs.push(WebInput::Speed(kbps));
                Ok(())
            }
            "zoom" => {
                let zoom = match &value {
                    Value::String(text) if text == "fit" => None,
                    other => Some(other.as_f64().map(|scale| scale as f32).filter(|scale| ZOOM_RANGE.contains(scale)).ok_or_else(rejected)?),
                };
                self.zoom = zoom;
                self.inputs.push(WebInput::Zoom(zoom));
                Ok(())
            }
            "choice" => {
                self.done = match value.as_str() {
                    Some("done") => true,
                    Some("save") => false,
                    _ => return Err(rejected()),
                };
                Ok(())
            }
            "colorSelection" => {
                let colors = value
                    .as_array()
                    .and_then(|items| items.iter().map(|item| item.as_str().and_then(parse_hex)).collect::<Option<Vec<_>>>())
                    .ok_or_else(rejected)?;
                self.inputs.push(WebInput::Selection(colors));
                Ok(())
            }
            "preset" => {
                let name = value.as_str().ok_or_else(rejected)?;
                let offered = WebFormat::offered(&self.formats);
                let listed = |preset: &&WebPreset| preset.name == name && offered.contains(&preset.options.format);
                let preset = self.presets.iter().find(listed).ok_or_else(rejected)?;
                apply_preset(&mut self.options, preset);
                self.restore_if_reduced(before);
                Ok(())
            }
            "optimizeOpen" => {
                if value != Value::Bool(true) {
                    return Err(rejected());
                }
                self.inputs.push(WebInput::OptimizeOpen);
                Ok(())
            }
            "optimizeK" => {
                self.rows_open(field)?;
                let k = value.as_f64().filter(|k| k.is_finite() && *k > 0.0).ok_or_else(rejected)?;
                self.inputs.push(WebInput::OptimizeK(k));
                Ok(())
            }
            "optimizeStart" => {
                self.rows_open(field)?;
                self.inputs.push(WebInput::OptimizeStart(parsed(field, &value)?));
                Ok(())
            }
            "optimizeUse" => {
                self.rows_open(field)?;
                self.inputs.push(WebInput::OptimizeUse(parsed(field, &value)?));
                Ok(())
            }
            "optimizeGo" => {
                self.rows_open(field)?;
                if value != Value::Bool(true) {
                    return Err(rejected());
                }
                self.inputs.push(WebInput::OptimizeGo);
                Ok(())
            }
            "width" | "height" => {
                let pixels = value.as_f64().filter(|v| v.is_finite() && *v > 0.0).ok_or_else(rejected)?;
                let mut link = LinkedSize::new(self.size);
                self.options.apply_size(&mut link);
                if field == "width" {
                    link.set_width(pixels);
                } else {
                    link.set_height(pixels);
                }
                self.options.set_exact_size(&link);
                Ok(())
            }
            "__constrain" => {
                let constrain = value.as_bool().ok_or_else(rejected)?;
                let mut link = LinkedSize::new(self.size);
                self.options.apply_size(&mut link);
                self.options.constrain(&mut link, constrain);
                Ok(())
            }
            // Only a format the list offers.
            "format" => {
                let format = parsed(field, &value)?;
                if !WebFormat::offered(&self.formats).contains(&format) {
                    return Err(rejected());
                }
                self.options.format = format;
                self.restore_if_reduced(before);
                Ok(())
            }
            "percent" => {
                set_serde_field(&mut self.options, field, value.clone())?;
                self.options.size = None;
                Ok(())
            }
            _ => {
                set_serde_field(&mut self.options, field, value.clone())?;
                self.options.clamp_exact_size(self.size);
                self.restore_if_reduced(before);
                Ok(())
            }
        }
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<SaveForWebResult>> {
        // A measuring run keeps the host's answers and the inputs: the real run of the pass takes them.
        let measuring = crate::runtime::measure::active(ui.ctx());
        let answers = if measuring { Vec::new() } else { self.requests.take_answers() };
        let dialog = SaveForWebDialog::new(self.name.clone(), self.size, self.pictures.clone(), Bind::bound(&mut self.options))
            .presets(self.presets.clone())
            .saving(self.saving)
            .answers(answers);
        let dialog = SaveForWebDialog {
            animation: self.animation,
            remember: self.remember.clone(),
            slices: &self.slices,
            slice_warning: self.slice_warning.clone(),
            view: Some(Bind::bound(&mut self.view)),
            frame: Some(Bind::bound(&mut self.frame)),
            open: Some(Bind::bound(&mut self.open)),
            document: self.document.clone(),
            formats: &self.formats,
            color_tables: &self.color_tables,
            marks: Some(&self.marks),
            optimize: self.optimize,
            browser_preview: self.browser.clone(),
            optimize_start: if measuring { None } else { self.optimize_start.take() },
            optimize_answer: if measuring { None } else { self.optimize_answer.take() },
            optimize_forget: !measuring && std::mem::take(&mut self.optimize_forget),
            ok_label: self.ok_label.clone(),
            inputs: if measuring { Vec::new() } else { std::mem::take(&mut self.inputs) },
            ..dialog
        };
        let mut shown = dialog.show(ui);
        if let Some(last) = shown.wanted_variants.first() {
            self.taken = last.request.revision;
            self.wanted = shown.wanted_variants;
        }
        if let Some(request) = shown.requests.take_color() {
            self.requests.request_color(request);
        }
        if let Some(request) = shown.requests.take_path() {
            self.requests.request_path(request);
            self.saving = true;
        }
        self.preset_events.extend(shown.preset_events);
        self.table_events.extend(shown.table);
        self.browser_pressed |= shown.browser_preview;
        self.optimize_opened |= shown.optimize_open;
        if shown.optimize_request.is_some() {
            self.optimize_request = shown.optimize_request;
        }
        self.speed = shown.speed;
        self.zoom = shown.zoom;
        self.selection = shown.selection;
        self.optimize_state = shown.optimize_state;
        match (shown.result, shown.closed) {
            (Some(result), _) => Some(DialogResult::Ok(result)),
            (None, Some(_)) => {
                self.saving = false;
                self.requests.cancel();
                Some(DialogResult::Cancel)
            }
            (None, None) => None,
        }
    }

    fn key(&self) -> Option<&str> {
        Some("save-for-web")
    }

    fn cancelled(&mut self, _: &egui::Context) {
        self.cancel_requests();
    }
}
