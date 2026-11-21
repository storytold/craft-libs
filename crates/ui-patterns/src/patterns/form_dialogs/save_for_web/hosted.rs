//! HostedSaveForWeb: the Save for Web dialog owned by a DialogHost.

#[cfg(doc)]
use super::MATTE_COLOR_KEY;
use super::model::{
    Animation, ColorTable, ColorTableEvent, OptimizeAnswer, OptimizeRequest, SaveForWebOptions, SaveForWebResult, TableMarks, WantedVariant,
    WebDocument, WebFormat, WebPreset, WebPresetEvent, WebVariant,
};
use super::{BrowserPreview, SaveForWebDialog, WebPictures, WebSlice, WebView, result_of, save_request};
use crate::contract::Bind;
use crate::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldOk, StackDialog, dialog_id, serde_fields, set_serde_field};
use crate::patterns::form_dialogs::host_requests::HostRequests;
use crate::runtime::env::Label;
use crate::runtime::memory::SaveKey;
use crate::runtime::requests::{ColorRequest, PathRequest};
use craft_behavior::PixelSize;
use craft_theme::Srgba;
use egui::Ui;
use egui::load::SizedTexture;
use std::path::PathBuf;

/// A [`SaveForWebDialog`] the [`DialogHost`](crate::dialog::DialogHost) owns. It keeps the settings and what the app
/// answers (the pictures), drops stale answers, keeps the colour and path requests until the app takes them, and
/// finishes with the result when the file dialog's path comes back.
///
/// ```ignore
/// let mut web = HostedSaveForWeb::new("poster.psd", PixelSize::new(1200, 800), original).animation(Animation::new(12, 24.0));
/// // each pass: for want in web.wanted_variants() { encode(want) -> web.set_variant(variant) }
/// // pick_color() -> picker -> answer_color(MATTE_COLOR_KEY, color); pick_path() -> file dialog -> answer_path(key, path)
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
    /// The view tab: bound to the dialog's `view` prop, so automation reads and writes it here.
    view: WebView,
    /// The path the app answered the save request with.
    path: Option<PathBuf>,
    /// The output size after the last pass.
    pixels: PixelSize,
}

impl HostedSaveForWeb {
    /// The document's slices, for the Slices choice.
    #[must_use]
    pub fn slices(mut self, slices: Vec<WebSlice>) -> Self {
        self.slices = slices;
        self
    }

    /// Picks the variant: a raster or a vector document.
    #[must_use]
    pub fn document(self, document: WebDocument) -> Self {
        let _ = (self, document);
        todo!("ex.W1")
    }

    /// The formats the host encodes, in list order.
    #[must_use]
    pub fn formats(self, formats: Vec<WebFormat>) -> Self {
        let _ = (self, formats);
        todo!("ex.W1")
    }

    /// The Color Table list the host offers.
    #[must_use]
    pub fn color_tables(self, tables: Vec<ColorTable>) -> Self {
        let _ = (self, tables);
        todo!("ex.W1")
    }

    /// Whether the zoom row has Preview in Browser.
    #[must_use]
    pub fn browser_preview(self, preview: BrowserPreview) -> Self {
        let _ = (self, preview);
        todo!("ex.W4")
    }

    /// Shows "Optimize to File Size…" under the Preset row.
    #[must_use]
    pub fn optimize(self) -> Self {
        let _ = self;
        todo!("ex.W8")
    }

    /// Preview in Browser was pressed since the last call, once; the host previews `current()`.
    pub fn take_browser_preview(&mut self) -> bool {
        let _ = self;
        todo!("ex.W4")
    }

    /// What the user did to the presets since the last call; the host stores them and gives the new list with
    /// `set_presets`.
    pub fn take_preset_events(&mut self) -> Vec<WebPresetEvent> {
        let _ = self;
        todo!("ex.W7")
    }

    /// Replaces the Preset list on an open dialog.
    pub fn set_presets(&mut self, presets: Vec<WebPreset>) {
        let _ = (self, presets);
        todo!("ex.W7")
    }

    /// What the host's colour table says, for the cell marks.
    pub fn set_marks(&mut self, marks: TableMarks) {
        let _ = (self, marks);
        todo!("ex.W10")
    }

    /// The colour table edits since the last call.
    pub fn take_table_events(&mut self) -> Vec<ColorTableEvent> {
        let _ = self;
        todo!("ex.W10")
    }

    /// The Optimize rows opened since the last call; the host answers the opening size.
    pub fn take_optimize_open(&mut self) -> bool {
        let _ = self;
        todo!("ex.W8")
    }

    /// The Desired File Size the rows open at, in bytes (32 K for `None`).
    pub fn answer_optimize_start(&mut self, bytes: Option<u64>) {
        let _ = (self, bytes);
        todo!("ex.W8")
    }

    /// The search to run, once.
    pub fn take_optimize_request(&mut self) -> Option<OptimizeRequest> {
        let _ = self;
        todo!("ex.W8")
    }

    /// The host's answer to the search: `Ok` closes the rows and shows the result line; `Err` keeps them open with the
    /// reason.
    pub fn answer_optimize(&mut self, answer: Result<OptimizeAnswer, Label>) {
        let _ = (self, answer);
        todo!("ex.W8")
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
            view: WebView::default(),
            path: None,
            pixels: size,
        }
    }

    /// The settings the dialog opens with.
    #[must_use]
    pub fn options(mut self, options: SaveForWebOptions) -> Self {
        self.options = options;
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

    /// An encoded pane; one for an older revision than the last asked for is dropped.
    pub fn set_variant(&mut self, variant: WebVariant) {
        if variant.revision >= self.taken {
            self.pictures.set(variant);
        }
    }

    /// The Matte button was pressed in the pass since the last call: the colour to ask the app's picker for, keyed
    /// [`MATTE_COLOR_KEY`]. Returned once.
    pub fn pick_color(&mut self) -> Option<ColorRequest> {
        self.requests.take_color()
    }

    /// Save… was pressed in the pass since the last call: the file dialog to open, keyed [`SAVE_PATH_KEY`](super::SAVE_PATH_KEY).
    /// Returned once; answer with [`answer_path`](Self::answer_path).
    pub fn pick_path(&mut self) -> Option<PathRequest> {
        self.requests.take_path()
    }

    /// Answers the colour request `key`: the matte, without alpha. Other keys are ignored.
    pub fn answer_color(&mut self, key: impl Into<String>, color: Srgba) {
        self.requests.answer_color(key, color);
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
}

impl StackDialog for HostedSaveForWeb {
    type Result = SaveForWebResult;

    /// Answers only once the app answered the save request with a path, as the dialog's own Save… does: without a path
    /// it asks for the file dialog as Save… does and answers none.
    fn confirm(&mut self, _: &egui::Context) -> Option<Self::Result> {
        let Some(path) = self.path.clone() else {
            if !self.saving {
                self.requests.request_path(save_request(&self.name, self.options.format));
                self.saving = true;
            }
            return None;
        };
        Some(result_of(self.options.clone(), path, self.pixels, self.view, self.remember.clone()))
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
        map.insert("view".to_owned(), serde_json::to_value(self.view).ok()?);
        map.insert("frame".to_owned(), serde_json::Value::from(self.frame));
        let path = self.path.as_ref().map_or(serde_json::Value::Null, |path| serde_json::Value::String(path.to_string_lossy().into_owned()));
        map.insert("path".to_owned(), path);
        Some(fields)
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        match field {
            "view" => {
                self.view = serde_json::from_value(value.clone()).map_err(|_| FieldRefusal::Rejects(&value).message(field))?;
                Ok(())
            }
            "path" => {
                let serde_json::Value::String(path) = &value else { return Err(FieldRefusal::Rejects(&value).message(field)) };
                self.path = Some(PathBuf::from(path));
                Ok(())
            }
            // The panes show the frame from the next pass; one past the last frame is the last, as Play keeps it.
            "frame" => {
                let frame = value.as_u64().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                let last = self.animation.map_or(0, |animation| animation.frames.saturating_sub(1));
                self.frame = u32::try_from(frame).unwrap_or(u32::MAX).min(last);
                Ok(())
            }
            _ => set_serde_field(&mut self.options, field, value),
        }
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<SaveForWebResult>> {
        let dialog = SaveForWebDialog::new(self.name.clone(), self.size, self.pictures.clone(), Bind::bound(&mut self.options))
            .presets(self.presets.clone())
            .saving(self.saving)
            .answers(self.requests.take_answers());
        let dialog = SaveForWebDialog {
            animation: self.animation,
            remember: self.remember.clone(),
            slices: &self.slices,
            view: Some(Bind::bound(&mut self.view)),
            frame: Some(Bind::bound(&mut self.frame)),
            open: Some(Bind::bound(&mut self.open)),
            ..dialog
        };
        let mut shown = dialog.show(ui);
        self.pixels = shown.size;
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
