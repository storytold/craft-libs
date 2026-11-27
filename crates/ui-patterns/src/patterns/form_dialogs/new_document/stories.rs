//! Stories of `NewDocumentDialog`.

use super::{CardCx, CardSource, DetailsCx, DocumentCard, DocumentOptions, DocumentSettings, NewDocumentDialog, PresetCardLook};
use crate::buttons::Button;
use crate::choice::Checkbox;
use crate::contract::{Bind, Entry, Outcome as _, Props as _, Show as _, ShowContainer as _, clicked};
use crate::form::Field;
use crate::patterns::presets::{GroupPath, PresetLibrary};
use crate::pickers::Select;
use crate::popover::Popover;
use crate::runtime::env::with_env;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::PixelSize;
use egui::Ui;
use serde::{Deserialize, Serialize};

#[story(NewDocumentDialog<'static>, covers = [], drive = Drive::Never)]
fn new_document_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let clipboard = *data.value("clipboard_image", false);
    let mut dialog = NewDocumentDialog::new(Bind::kept());
    if clipboard {
        dialog = dialog.clipboard(PixelSize::new(1920, 1080));
    }
    dialog.opened(data.bind("open", true)).show(ui).into()
}

/// The preset cards in `PresetCardLook::Page`, which an app sets at its root (VectorCraft's): blank paper on flat
/// cards, the selected one with the hover fill and an accent edge.
#[story(NewDocumentDialog<'static>, covers = [], drive = Drive::Never, title = "Page cards")]
fn new_document_dialog_page_cards(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let opened = data.bind("open", true);
    with_env(
        ui,
        |overrides| {
            overrides.values.set(PresetCardLook::Page);
        },
        |ui| NewDocumentDialog::new(Bind::kept()).clipboard(PixelSize::new(1920, 1080)).opened(opened).show(ui).into(),
    )
}

#[story(NewDocumentDialog<'static>, covers = [], drive = Drive::Never)]
fn new_document_dialog_with_app_presets(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let presets = (
        PresetLibrary::<DocumentOptions>::new(),
        vec![DocumentOptions { name: "Poster".to_owned(), ..DocumentOptions::default() }],
        "recent".to_owned(),
    );
    let (((library, recent, tab), presets_at), (open, open_at)) = data.pair_at(("presets", presets), ("open", true));
    NewDocumentDialog::new(Bind::kept())
        .saved(presets_at.bind(library))
        .recent(presets_at.bind(recent))
        .tab(presets_at.bind(tab))
        .opened(open_at.bind(open))
        .show(ui)
        .into()
}

/// The unit a host's settings show sizes in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
enum StoryUnit {
    #[default]
    Inches,
    Centimetres,
}

/// A host's own settings: a unit, an artboard count and a setting that lives in a More Settings popover.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct StorySettings {
    name: String,
    unit: StoryUnit,
    artboards: u32,
    snap: bool,
}

impl Default for StorySettings {
    fn default() -> Self {
        StorySettings { name: String::new(), unit: StoryUnit::default(), artboards: 1, snap: false }
    }
}

/// What the host's details rows keep in the session: whether the popover is open.
#[derive(Clone, Default)]
struct StoryMemory {
    more_open: bool,
}

impl DocumentSettings for StorySettings {
    type Preset = ();
    type Setup = ();
    type Memory = StoryMemory;

    fn name(&self) -> &str {
        &self.name
    }

    fn set_name(&mut self, name: String) {
        self.name = name;
    }

    fn page(&self) -> (f64, f64) {
        (f64::from(self.artboards.max(1)), 1.0)
    }

    fn same_document(&self, other: &Self) -> bool {
        self.unit == other.unit && self.artboards == other.artboards
    }

    fn fits(&self) -> bool {
        (1..=MOST_ARTBOARDS).contains(&self.artboards)
    }

    fn open(&mut self, _setup: &(), untitled: &str) {
        if self.name.is_empty() {
            untitled.clone_into(&mut self.name);
        }
    }

    fn preset_card(_preset: &(), _cx: &CardCx<'_, Self>) -> DocumentCard<Self> {
        DocumentCard::new("Preset".to_owned(), String::new(), StorySettings::default(), CardSource::preset(None))
    }

    fn recent_cards(_cx: &CardCx<'_, Self>) -> Vec<DocumentCard<Self>> {
        Vec::new()
    }

    fn saved_card(name: &str, saved: &Self, _cx: &CardCx<'_, Self>) -> DocumentCard<Self> {
        let line = format!("{} artboards, {}", saved.artboards, if saved.unit == StoryUnit::Inches { "in" } else { "cm" });
        DocumentCard::new(name.to_owned(), line, saved.clone(), CardSource::Recent)
    }

    fn apply_card(&mut self, card: &DocumentCard<Self>, _setup: &(), _memory: &mut StoryMemory) {
        self.unit = card.settings.unit;
        self.artboards = card.settings.artboards;
        self.snap = card.settings.snap;
    }

    fn details(ui: &mut Ui, values: &mut Self, cx: &mut DetailsCx<'_, Self>) {
        let units = [Entry::new(StoryUnit::Inches, "Inches".to_owned()), Entry::new(StoryUnit::Centimetres, "Centimetres".to_owned())];
        Field::new("Units".to_owned()).id_salt("units").show(ui, |ui, _| {
            let shown = Select::new("Units".to_owned(), Bind::given(values.unit), &units).show(ui);
            if let Some(unit) = shown.value().cloned() {
                values.unit = unit;
            }
        });
        let counts: Vec<Entry<u32>> = (1..=MOST_ARTBOARDS).map(|count| Entry::new(count, count.to_string())).collect();
        Field::new("Artboards".to_owned()).id_salt("artboards").show(ui, |ui, _| {
            let shown = Select::new("Artboards".to_owned(), Bind::given(values.artboards), &counts).show(ui);
            if let Some(count) = shown.value().cloned() {
                values.artboards = count;
            }
        });
        let more = Button::new("More Settings".to_owned()).id_salt("more").show(ui);
        if clicked(&more) {
            cx.request_action("more-settings");
        }
        let open = &mut cx.memory().more_open;
        if clicked(&more) {
            *open = !*open;
        }
        Popover::new("More Settings".to_owned(), Bind::bound(open)).opened_by(more.response()).show(ui, |ui, _| {
            let shown = Checkbox::new(Bind::given(values.snap)).label("Snap to Grid".to_owned()).show(ui);
            if let Some(on) = shown.value().cloned() {
                values.snap = on;
            }
        });
    }
}

/// The most artboards the story's settings offer.
const MOST_ARTBOARDS: u32 = 4;

// PLAN: the registry matches a story to the probe instantiation (`DocumentOptions`), so the owner names that and the
// dialog is built over `StorySettings` in the body.
#[story(NewDocumentDialog<'static>, covers = [], drive = Drive::Never)]
fn new_document_dialog_with_host_details(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut settings = data.value("settings", StorySettings::default()).clone();
    let output = NewDocumentDialog::over(Bind::bound(&mut settings)).opened(data.bind("open", true)).show(ui);
    *data.value("settings", StorySettings::default()) = settings;
    output.into()
}

/// A library holding one preset of the host's settings.
fn seeded_library() -> PresetLibrary<StorySettings> {
    let mut library = PresetLibrary::new();
    let _ = library.add(&GroupPath::root(), "Wide Spread", StorySettings { artboards: 3, ..StorySettings::default() });
    library
}

// PLAN: see `new_document_dialog_with_host_details`.
#[story(NewDocumentDialog<'static>, covers = [], drive = Drive::Never)]
fn new_document_dialog_saving_host_details(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut settings = data.value("settings", StorySettings::default()).clone();
    let mut library = data.value("library", seeded_library()).clone();
    let mut tab = data.value("tab", "saved".to_owned()).clone();
    let output = NewDocumentDialog::over(Bind::bound(&mut settings))
        .saved(Bind::bound(&mut library))
        .tab(Bind::bound(&mut tab))
        .opened(data.bind("open", true))
        .show(ui);
    *data.value("settings", StorySettings::default()) = settings;
    *data.value("library", seeded_library()) = library;
    *data.value("tab", "saved".to_owned()) = tab;
    output.into()
}
