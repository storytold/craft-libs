//! What New Document needs of an app's settings: [`DocumentSettings`], the cards and the details rows built from it,
//! and the raster implementation's setup.

use super::NewDocumentWords;
use super::presets::{ColorProfiles, DocumentOptions, DocumentPreset, NewDocumentPresetWords, Orientation, PresetCategory};
use crate::patterns::presets::PresetId;
use crate::runtime::env::{Label, Units};
use crate::runtime::requests::{ColorRequest, HostAnswer};
use craft_behavior::PixelSize;
use egui::Ui;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt::Debug;
use std::marker::PhantomData;

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
        let _ = (title, line, settings, source);
        todo!("fl.F23")
    }

    /// The card's hover text.
    #[must_use]
    pub fn tooltip(self, text: impl Into<String>) -> Self {
        let _ = (self, text);
        todo!("fl.F23")
    }
}

/// Where a card comes from.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CardSource {
    /// A built-in preset; `artboard` is whether it turns Artboards on (`None`: it leaves them).
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
    _marker: PhantomData<&'a S>,
}

impl<S: DocumentSettings> DetailsCx<'_, S> {
    /// What the dialog is set up with.
    pub fn setup(&self) -> &S::Setup {
        let _ = self;
        todo!("fl.F25")
    }

    /// What the session keeps for the rows.
    pub fn memory(&mut self) -> &mut S::Memory {
        let _ = self;
        todo!("fl.F25")
    }

    /// The units sizes are shown in.
    pub fn units(&self) -> &Units {
        let _ = self;
        todo!("fl.F25")
    }

    /// The dialog's words.
    pub fn words(&self) -> &NewDocumentWords {
        let _ = self;
        todo!("fl.F25")
    }

    /// The orientation the control picked.
    pub fn orientation(&self) -> Option<Orientation> {
        let _ = self;
        todo!("fl.F25")
    }

    /// Sets the orientation, which built-in cards turn with.
    pub fn set_orientation(&mut self, orientation: Orientation) {
        let _ = (self, orientation);
        todo!("fl.F25")
    }

    /// Asks the host's colour picker for a colour.
    pub fn request_color(&mut self, request: ColorRequest) {
        let _ = (self, request);
        todo!("fl.F25")
    }

    /// Asks the host to run the action `id`; it comes back in `NewDocumentOutput::actions`.
    pub fn request_action(&mut self, id: impl Into<String>) {
        let _ = (self, id);
        todo!("fl.F25")
    }

    /// This pass's host answers, once, as `FormCx::take_answers`.
    pub fn take_answers(&mut self) -> Vec<HostAnswer> {
        let _ = self;
        todo!("fl.F25")
    }

    /// Drops the card the settings were last filled from, as an edit of the details does.
    pub fn drop_card(&mut self) {
        let _ = self;
        todo!("fl.F25")
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
            app_name: None,
            custom_pixel_aspects: Vec::new(),
        }
    }
}

impl DocumentSettings for DocumentOptions {
    type Preset = DocumentPreset;
    type Setup = RasterSetup;
    type Memory = ();

    fn name(&self) -> &str {
        todo!("fl.F22")
    }

    fn set_name(&mut self, name: String) {
        let _ = name;
        todo!("fl.F22")
    }

    fn page(&self) -> (f64, f64) {
        todo!("fl.F22")
    }

    fn same_document(&self, other: &Self) -> bool {
        let _ = other;
        todo!("fl.F22")
    }

    fn fits(&self) -> bool {
        todo!("fl.F22")
    }

    fn open(&mut self, setup: &Self::Setup, untitled: &str) {
        let _ = (setup, untitled);
        todo!("fl.F22")
    }

    fn preset_card(preset: &Self::Preset, cx: &CardCx<'_, Self>) -> DocumentCard<Self> {
        let _ = (preset, cx);
        todo!("fl.F22")
    }

    fn recent_cards(cx: &CardCx<'_, Self>) -> Vec<DocumentCard<Self>> {
        let _ = cx;
        todo!("fl.F22")
    }

    fn saved_card(name: &str, saved: &Self, cx: &CardCx<'_, Self>) -> DocumentCard<Self> {
        let _ = (name, saved, cx);
        todo!("fl.F22")
    }

    fn apply_card(&mut self, card: &DocumentCard<Self>, setup: &Self::Setup, memory: &mut Self::Memory) {
        let _ = (card, setup, memory);
        todo!("fl.F22")
    }

    fn details(ui: &mut Ui, values: &mut Self, cx: &mut DetailsCx<'_, Self>) {
        let _ = (ui, values, cx);
        todo!("fl.F22")
    }
}
