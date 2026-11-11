//! HostedSaveForWeb: the Save for Web dialog owned by a DialogHost.

use super::model::{Animation, SaveForWebOptions, SaveForWebResult, WantedVariant, WebPreset, WebVariant};
#[cfg(doc)]
use super::{MATTE_COLOR_KEY, SAVE_PATH_KEY};
use super::{SaveForWebDialog, WebPictures};
use crate::contract::Bind;
use crate::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use crate::patterns::form_dialogs::host_requests::HostRequests;
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
}

impl HostedSaveForWeb {
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

    /// Save… was pressed in the pass since the last call: the file dialog to open, keyed [`SAVE_PATH_KEY`].
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

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<SaveForWebResult>> {
        let dialog = SaveForWebDialog::new(self.name.clone(), self.size, self.pictures.clone(), Bind::bound(&mut self.options))
            .presets(self.presets.clone())
            .saving(self.saving)
            .answers(self.requests.take_answers());
        let dialog = SaveForWebDialog { animation: self.animation, remember: self.remember.clone(), ..dialog };
        let mut shown = dialog.show(ui);
        self.frame = shown.frame;
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
