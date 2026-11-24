//! What New Document needs of an app's settings: [`DocumentSettings`], the cards and the details rows built from it,
//! and the raster implementation's setup and session memory.

use super::presets::{ColorProfiles, DocumentOptions, DocumentPreset, NewDocumentPresetWords, Orientation, PresetCategory};
use super::{NewDocumentWords, cards, details, restore_all};
use crate::patterns::form_dialogs::LinkedSize;
use crate::patterns::presets::PresetId;
use crate::runtime::env::{Label, UnitKind, Units};
use crate::runtime::requests::{ColorRequest, HostAnswer};
use craft_behavior::PixelSize;
use egui::Ui;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt::Debug;

/// What New Document needs of an app's settings: its cards, its details rows and how a card fills it. `fitted` runs
/// after every edit and every `set_json_field`; `Memory` lives in the dialog's session.
pub trait DocumentSettings: Clone + PartialEq + Debug + Default + Serialize + DeserializeOwned + Send + Sync + 'static {
    /// What a built-in card stands for.
    type Preset: Clone + Debug + PartialEq + Send + Sync + 'static;
    /// What the dialog is set up with besides the cards (the clipboard, the profiles, the resolutions).
    type Setup: Clone + Default + Send + Sync + 'static;
    /// What the dialog's session keeps for the details rows (the linked size fields, a running gesture).
    type Memory: Clone + Default + Send + Sync + 'static;

    /// The document's name.
    fn name(&self) -> &str;
    /// Sets the document's name.
    fn set_name(&mut self, name: String);
    /// The page's width and height, for a card's aspect.
    fn page(&self) -> (f64, f64);
    /// Whether `other` is the same document as `self`, for the recent list.
    fn same_document(&self, other: &Self) -> bool;
    /// Whether the settings are valid.
    fn fits(&self) -> bool;
    /// The settings made valid; they are unchanged by default.
    fn fitted(self) -> Self {
        self
    }
    /// Notes that the settings key `field` was set from outside the dialog (automation), before `fitted` runs; nothing
    /// by default.
    fn note_set(&mut self, field: &str, memory: &mut Self::Memory) {
        let _ = (field, memory);
    }
    /// Sets the resolutions print and screen presets open at in `setup`, in pixels per inch, each the host gave
    /// ([`NewDocumentDialog::print_resolution`](super::NewDocumentDialog::print_resolution)); nothing by default.
    fn set_resolutions(setup: &mut Self::Setup, print: Option<f64>, screen: Option<f64>) {
        let _ = (setup, print, screen);
    }
    /// Fills what the dialog adds to the settings it opens on: the clipboard's size and the default name `untitled`.
    fn open(&mut self, setup: &Self::Setup, untitled: &str);
    /// The card of a built-in `preset`.
    fn preset_card(preset: &Self::Preset, cx: &CardCx<'_, Self>) -> DocumentCard<Self>;
    /// The cards of the Recent tab.
    fn recent_cards(cx: &CardCx<'_, Self>) -> Vec<DocumentCard<Self>>;
    /// The card of the saved preset `name` with the settings `saved`.
    fn saved_card(name: &str, saved: &Self, cx: &CardCx<'_, Self>) -> DocumentCard<Self>;
    /// Applies `card` to the settings.
    fn apply_card(&mut self, card: &DocumentCard<Self>, setup: &Self::Setup, memory: &mut Self::Memory);
    /// Draws the details rows over `values`.
    fn details(ui: &mut Ui, values: &mut Self, cx: &mut DetailsCx<'_, Self>);
}

/// One card or Saved row; the aspect comes from `settings.page()`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DocumentCard<S> {
    /// The card's title.
    pub title: Label,
    /// The card's second line.
    pub line: String,
    /// The card's hover text.
    pub tooltip: Option<String>,
    /// The settings the card applies.
    pub settings: S,
    /// Where the card comes from.
    pub source: CardSource,
}

impl<S> DocumentCard<S> {
    /// A card titled `title` with the second `line`, applying `settings`.
    pub fn new(title: impl Into<Label>, line: impl Into<String>, settings: S, source: CardSource) -> Self {
        DocumentCard { title: title.into(), line: line.into(), tooltip: None, settings, source }
    }

    /// The card's hover text.
    #[must_use]
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
}

/// Where a card comes from.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CardSource {
    /// A built-in preset; `artboard` is whether it turns Artboards on (`None`: it leaves them).
    #[non_exhaustive]
    Preset {
        /// Whether the card turns Artboards on.
        artboard: Option<bool>,
    },
    /// A recent document.
    Recent,
    /// A saved preset.
    Saved(PresetId),
    /// The clipboard's image size.
    Clipboard,
}

impl CardSource {
    /// A built-in preset that turns Artboards on or off, or leaves them (`None`).
    pub fn preset(artboard: Option<bool>) -> Self {
        CardSource::Preset { artboard }
    }
}

/// What a card is built from.
#[non_exhaustive]
pub struct CardCx<'a, S: DocumentSettings> {
    /// The settings the dialog shows now.
    pub values: &'a S,
    /// What the dialog is set up with.
    pub setup: &'a S::Setup,
    /// The orientation the control picked.
    pub orientation: Option<Orientation>,
    /// The recent documents.
    pub recents: &'a [S],
    /// The categories of built-in presets.
    pub presets: &'a [PresetCategory<S::Preset>],
    /// The units the cards word sizes in.
    pub units: &'a Units,
    /// The dialog's words.
    pub words: &'a NewDocumentWords,
    /// The names of the built-in presets.
    pub preset_words: &'a NewDocumentPresetWords,
}

/// What the details rows can do.
pub struct DetailsCx<'a, S: DocumentSettings> {
    setup: &'a S::Setup,
    memory: &'a mut S::Memory,
    units: &'a Units,
    words: &'a NewDocumentWords,
    orientation: &'a mut Option<Orientation>,
    card: &'a mut Option<usize>,
    answers: Vec<HostAnswer>,
    colors: Vec<ColorRequest>,
    actions: Vec<String>,
}

impl<'a, S: DocumentSettings> DetailsCx<'a, S> {
    pub(super) fn new(
        setup: &'a S::Setup,
        memory: &'a mut S::Memory,
        units: &'a Units,
        words: &'a NewDocumentWords,
        orientation: &'a mut Option<Orientation>,
        card: &'a mut Option<usize>,
        answers: Vec<HostAnswer>,
    ) -> Self {
        DetailsCx { setup, memory, units, words, orientation, card, answers, colors: Vec::new(), actions: Vec::new() }
    }

    /// What the rows asked of the host: the colour requests and the action ids.
    pub(super) fn into_requests(self) -> (Vec<ColorRequest>, Vec<String>) {
        (self.colors, self.actions)
    }

    /// What the dialog is set up with.
    pub fn setup(&self) -> &'a S::Setup {
        self.setup
    }

    /// What the session keeps for the rows.
    pub fn memory(&mut self) -> &mut S::Memory {
        self.memory
    }

    /// The units sizes are shown in.
    pub fn units(&self) -> &'a Units {
        self.units
    }

    /// The dialog's words.
    pub fn words(&self) -> &'a NewDocumentWords {
        self.words
    }

    /// The orientation the control picked.
    pub fn orientation(&self) -> Option<Orientation> {
        *self.orientation
    }

    /// Sets the orientation, which built-in cards turn with.
    pub fn set_orientation(&mut self, orientation: Orientation) {
        *self.orientation = Some(orientation);
    }

    /// Asks the host's colour picker for a colour.
    pub fn request_color(&mut self, request: ColorRequest) {
        self.colors.push(request);
    }

    /// Asks the host to run the action `id`; it comes back in `NewDocumentOutput::actions`.
    pub fn request_action(&mut self, id: impl Into<String>) {
        self.actions.push(id.into());
    }

    /// This pass's host answers, once, as `FormCx::take_answers`.
    pub fn take_answers(&mut self) -> Vec<HostAnswer> {
        std::mem::take(&mut self.answers)
    }

    /// Drops the card the settings were last filled from, as an edit of the details does.
    pub fn drop_card(&mut self) {
        *self.card = None;
    }
}

/// The raster implementation's setup: the dialog's set-up values besides the cards.
#[derive(Clone)]
#[non_exhaustive]
pub struct RasterSetup {
    /// The cards Recent shows after the recent documents.
    pub recent_defaults: Vec<DocumentPreset>,
    /// The clipboard holds an image of this size.
    pub clipboard: Option<PixelSize>,
    /// The app's background colour.
    pub background: [f32; 3],
    /// The resolution print presets open at, in pixels per inch.
    pub print_ppi: f64,
    /// The resolution screen presets open at, in pixels per inch.
    pub screen_ppi: f64,
    /// The colour profiles each mode offers.
    pub profiles: Vec<ColorProfiles>,
    /// The profile id of the Don't Color Manage entry, when the app offers one.
    pub unmanaged_profile: Option<String>,
    /// The application's name, for the "Default {app} Size" card.
    pub app_name: Option<Label>,
    /// The app's own pixel aspect ratios as `(id, name)`.
    pub custom_pixel_aspects: Vec<(String, String)>,
}

impl Default for RasterSetup {
    fn default() -> Self {
        RasterSetup {
            recent_defaults: PresetCategory::default_recent(),
            clipboard: None,
            background: [1.0; 3],
            print_ppi: 300.0,
            screen_ppi: 72.0,
            profiles: Vec::new(),
            unmanaged_profile: None,
            app_name: None,
            custom_pixel_aspects: Vec::new(),
        }
    }
}

/// What the raster details rows keep in the dialog's session: the fields that show the size, the start of a running
/// edit of them, and whether a Web or Mobile card turned Artboards on.
#[derive(Clone)]
pub struct RasterMemory {
    /// The width, height and resolution fields of the settings.
    pub(super) size: LinkedSize,
    /// The size before a running edit of a size field, for putting it back when the edit is cancelled.
    pub(super) gesture: Option<LinkedSize>,
    /// A Web or Mobile card turned Artboards on, so a card of another tab turns it off again; the user's own tick ends
    /// it.
    pub(super) preset_artboard: bool,
    /// The fields must be made again from the settings: a card replaced them.
    pub(super) relink: bool,
}

impl Default for RasterMemory {
    fn default() -> Self {
        RasterMemory { size: LinkedSize::new(PixelSize::new(1, 1)), gesture: None, preset_artboard: false, relink: true }
    }
}

impl DocumentSettings for DocumentOptions {
    type Preset = DocumentPreset;
    type Setup = RasterSetup;
    type Memory = RasterMemory;

    fn name(&self) -> &str {
        &self.name
    }

    fn set_name(&mut self, name: String) {
        self.name = name;
    }

    fn page(&self) -> (f64, f64) {
        (f64::from(self.size.width), f64::from(self.size.height))
    }

    fn same_document(&self, other: &Self) -> bool {
        DocumentOptions::same_document(self, other)
    }

    fn fits(&self) -> bool {
        DocumentOptions::fits(self)
    }

    fn fitted(self) -> Self {
        DocumentOptions::fitted(self)
    }

    fn set_resolutions(setup: &mut Self::Setup, print: Option<f64>, screen: Option<f64>) {
        setup.print_ppi = print.unwrap_or(setup.print_ppi);
        setup.screen_ppi = screen.unwrap_or(setup.screen_ppi);
    }

    fn open(&mut self, setup: &Self::Setup, untitled: &str) {
        if let Some(size) = setup.clipboard {
            self.size = size;
            self.unit = UnitKind::Pixels;
            self.pixels_per_inch = setup.screen_ppi;
        }
        if self.name.is_empty() {
            untitled.clone_into(&mut self.name);
        }
    }

    fn preset_card(preset: &Self::Preset, cx: &CardCx<'_, Self>) -> DocumentCard<Self> {
        cards::raster_preset_card(preset, cx)
    }

    fn recent_cards(cx: &CardCx<'_, Self>) -> Vec<DocumentCard<Self>> {
        cards::raster_recent_cards(cx)
    }

    fn saved_card(name: &str, saved: &Self, cx: &CardCx<'_, Self>) -> DocumentCard<Self> {
        cards::raster_saved_card(name, saved, cx)
    }

    /// A recent document or saved preset restores everything but the name, a built-in card sets the size and unit.
    fn apply_card(&mut self, card: &DocumentCard<Self>, _setup: &Self::Setup, memory: &mut Self::Memory) {
        match &card.source {
            CardSource::Recent | CardSource::Saved(_) => {
                restore_all(self, &card.settings);
                memory.preset_artboard = false;
            }
            source => {
                self.size = card.settings.size;
                self.pixels_per_inch = card.settings.pixels_per_inch;
                self.unit = card.settings.unit;
                let artboard = match source {
                    CardSource::Preset { artboard } => *artboard,
                    _ => None,
                };
                match artboard {
                    Some(true) if !self.artboard => {
                        self.artboard = true;
                        memory.preset_artboard = true;
                    }
                    Some(false) if memory.preset_artboard => {
                        self.artboard = false;
                        memory.preset_artboard = false;
                    }
                    _ => {}
                }
            }
        }
        memory.relink = true;
        memory.gesture = None;
    }

    fn details(ui: &mut Ui, values: &mut Self, cx: &mut DetailsCx<'_, Self>) {
        details::raster_details(ui, values, cx);
    }
}
