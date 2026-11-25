//! `NewDocumentDialog`: File > New. Category tabs over a grid of preset cards, a details column, saved presets and the
//! document settings the app creates the document from.

mod cards;
mod details;
mod presets;
mod settings;
#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::form::{read_bind, write_bind};
use super::{FormCx, FormDialog, HostRequests};
use crate::contract::{Bind, NoValue, Outcome, Remember as _, SaveKey, Scroll, ShowContainer as _};
use crate::runtime::requests::{ColorRequest, HostAnswer};

use crate::dialog::{
    DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, HoldsOk, StackDialog, dialog_id, serde_fields, set_serde_field,
};

use crate::contract::Show as _;
use crate::display::Divider;
use crate::layout_components::{Grid, Stack};

use crate::patterns::presets::{GroupPath, NamedPresetEvent, PresetLibrary, PresetWords as LibraryWords};

use crate::runtime::env::{Env, Label, Units};
use crate::runtime::overlay::CloseReason;
use craft_behavior::{ColumnWidth, PixelSize};

use craft_theme::Srgba;
use egui::{Context, Response, Ui};
use std::cell::RefCell;

pub use cards::PresetCardLook;
use cards::{Left, TabId};

use details::Right;

pub(crate) use presets::push_recent;
pub use presets::{
    BackgroundContents, BitDepth, ColorMode, ColorProfiles, DocumentOptions, DocumentPreset, NewDocumentPresetWords, Orientation, PIXEL_ASPECTS,
    PresetCategory, SQUARE_PIXELS,
};
pub use settings::{CardCx, CardSource, DetailsCx, DocumentCard, DocumentSettings, RasterMemory, RasterSetup};

/// The key of the colour request Custom makes.
pub const BACKGROUND_COLOR_KEY: &str = "newDocument.backgroundColor";
/// The key of the saved presets.
const PRESETS_KEY: SaveKey = SaveKey::new("craft_ui.newDocument.presets");

craft_ui::words! {
    /// The words of New Document, beside the common OK and Cancel and the names of the built-in presets (`NewDocumentPresetWords`).
    pub struct NewDocumentWords {
        title: "newDocument.title" = "New Document";
        create: "newDocument.create" = "Create";
        close: "newDocument.close" = "Close";
        tabs: "newDocument.tabs" = "Preset Categories";
        cards: "newDocument.cards" = "Blank Document Presets";
        saved_list: "newDocument.savedList" = "Saved Presets";
        delete: "newDocument.delete" = "Delete";
        recent: "newDocument.recent" = "Recent";
        saved: "newDocument.saved" = "Saved";
        photo: "newDocument.photo" = "Photo";
        print: "newDocument.print" = "Print";
        art: "newDocument.art" = "Art & Illustration";
        web: "newDocument.web" = "Web";
        mobile: "newDocument.mobile" = "Mobile";
        film: "newDocument.film" = "Film & Video";
        clipboard: "newDocument.clipboard" = "Clipboard";
        custom: "newDocument.custom" = "Custom";
        untitled: "newDocument.untitled" = "Untitled-1";
        name: "newDocument.name" = "Name";
        width: "newDocument.width" = "Width";
        height: "newDocument.height" = "Height";
        width_unit: "newDocument.widthUnit" = "Width Unit";
        height_unit: "newDocument.heightUnit" = "Height Unit";
        resolution_unit: "newDocument.resolutionUnit" = "Resolution Unit";
        portrait: "newDocument.portrait" = "Portrait";
        landscape: "newDocument.landscape" = "Landscape";
        artboards: "newDocument.artboards" = "Artboards";
        resolution: "newDocument.resolution" = "Resolution";
        color_mode: "newDocument.colorMode" = "Color Mode";
        depth: "newDocument.depth" = "Depth";
        profile: "newDocument.profile" = "Color Profile";
        /// `{name}` is the working profile's name.
        working_profile: template "newDocument.workingProfile" = "Working: {name}";
        working: "newDocument.working" = "Working";
        background: "newDocument.background" = "Background Contents";
        white: "newDocument.white" = "White";
        black: "newDocument.black" = "Black";
        background_color: "newDocument.backgroundColor" = "Background Color";
        transparent: "newDocument.transparent" = "Transparent";
        custom_color: "newDocument.customColor" = "Custom…";
        custom_swatch: "newDocument.customSwatch" = "Custom background color";
        /// The icon right of Name that opens the naming pane, and the pane's header.
        save_preset_title: "newDocument.savePresetTitle" = "Save Document Preset";
        /// The naming pane's button that saves the preset.
        save_preset: "newDocument.savePreset" = "Save Preset";
        preset_name: "newDocument.presetName" = "Preset Name";
        /// The name the naming pane proposes: `{n}` is the first number no saved preset has.
        saved_name: template "newDocument.savedName" = "Saved {n}";
        no_saved: "newDocument.noSaved" = "No saved presets yet.";
        too_many: "newDocument.tooMany" = "At most 256 document presets can be saved";
        bitmap: "newDocument.bitmap" = "Bitmap";
        grayscale: "newDocument.grayscale" = "Grayscale";
        rgb_color: "newDocument.rgbColor" = "RGB Color";
        cmyk_color: "newDocument.cmykColor" = "CMYK Color";
        lab_color: "newDocument.labColor" = "Lab Color";
        depth_1: "newDocument.depth1" = "1 bit";
        depth_8: "newDocument.depth8" = "8 bit";
        depth_16: "newDocument.depth16" = "16 bit";
        depth_32: "newDocument.depth32" = "32 bit (float)";
        orientation: "newDocument.orientation" = "Orientation";
        pixel_aspect: "newDocument.pixelAspect" = "Pixel Aspect Ratio";
        square_pixels: "newDocument.pixelAspectSquare" = "Square Pixels";
        d1_dv_ntsc: "newDocument.pixelAspectD1DvNtsc" = "D1/DV NTSC (0.91)";
        d1_dv_pal: "newDocument.pixelAspectD1DvPal" = "D1/DV PAL (1.09)";
        d1_dv_ntsc_widescreen: "newDocument.pixelAspectD1DvNtscWidescreen" = "D1/DV NTSC Widescreen (1.21)";
        hdv_1080: "newDocument.pixelAspectHdv1080" = "HDV 1080/DVCPRO HD 720 (1.33)";
        d1_dv_pal_widescreen: "newDocument.pixelAspectD1DvPalWidescreen" = "D1/DV PAL Widescreen (1.46)";
        anamorphic_2_to_1: "newDocument.pixelAspectAnamorphic2To1" = "Anamorphic 2:1 (2)";
        dvcpro_hd_1080: "newDocument.pixelAspectDvcproHd1080" = "DVCPRO HD 1080 (1.5)";
        /// `{n}` is how many cards the open tab lists.
        presets_heading: template "newDocument.presetsHeading" = "Blank Document Presets ({n})";
        preset_details: "newDocument.presetDetails" = "Preset Details";
        default_size: "newDocument.defaultSizePlain" = "Default Size";
        /// `{app}` is the application's name.
        default_size_named: template "newDocument.defaultSize" = "Default {app} Size";
        /// A card's size line: `{width}`, `{height}`, `{unit}` and `{resolution}`.
        size_line: template "newDocument.sizeLine" = "{width} × {height} {unit} @ {resolution}";
        /// A recent card's hover text: `{title}`, `{size}`, `{mode}` and `{background}`.
        recent_detail: template "newDocument.recentDetail" = "{title} · {size} · {mode} · {background}";
        /// A saved preset's second line: `{size}`, `{mode}` and `{depth}`.
        saved_detail: template "newDocument.savedDetail" = "{size} · {mode}, {depth}";
    }
}

/// What the columns read besides the session: the setup, the categories, the remembered lists, the words and the units.
struct Read<'a, S: DocumentSettings> {
    setup: &'a S::Setup,
    categories: &'a [PresetCategory<S::Preset>],
    recents: &'a [S],
    library: &'a PresetLibrary<S>,
    words: &'a NewDocumentWords,
    preset_words: &'a NewDocumentPresetWords,
    library_words: &'a LibraryWords,
    units: &'a Units,
}

/// The values the app binds, as the body works on them during one frame. A frame can run the body twice; the cells
/// carry what the first pass did into the second, and `events` and `actions` collect those of both passes.
struct Bound<'a, S: DocumentSettings> {
    /// The bound saved presets; `None` keeps them in the form memory.
    saved: Option<&'a RefCell<PresetLibrary<S>>>,
    /// The bound tab id.
    tab: Option<&'a RefCell<String>>,
    /// The bound memory of the details rows; `None` keeps it in the session.
    memory: Option<&'a RefCell<S::Memory>>,
    /// Each save and delete of the frame.
    events: &'a RefCell<Vec<NamedPresetEvent<String>>>,
    /// The `request_action` ids of the frame.
    actions: &'a RefCell<Vec<String>>,
}

/// What the body reads besides its form: the setup, the categories, the recent documents, the words and the units.
struct Shared<'a, S: DocumentSettings> {
    bound: Bound<'a, S>,
    setup: &'a S::Setup,
    categories: &'a [PresetCategory<S::Preset>],
    recents: &'a [S],
    words: &'a NewDocumentWords,
    preset_words: &'a NewDocumentPresetWords,
    library_words: &'a LibraryWords,
    units: &'a Units,
    untitled: &'a str,
    /// The host opened the settings ([`NewDocumentDialog::opening`]).
    host_opened: bool,
}

/// What the dialog keeps while it is open besides the settings: the open tab and card, the orientation, the Save
/// Document Preset pane and the settings type's own memory. It lives in the dialog's body slot.
#[derive(Clone)]
struct Session<S: DocumentSettings> {
    tab: TabId,
    /// The card the user picked in the open tab (the saved row in Saved).
    card: Option<usize>,
    /// The orientation picked with the control; built-in cards turn with it.
    orientation: Option<Orientation>,
    /// The Save Document Preset pane's name, while the pane stands in for the details.
    naming: Option<String>,
    /// A save was refused and its reason shows under the name field.
    name_tried: bool,
    focus_name: bool,
    focus_preset_name: bool,
    /// What the settings type's details rows keep.
    memory: S::Memory,
}

impl<S: DocumentSettings> Default for Session<S> {
    fn default() -> Self {
        Session {
            tab: TabId::Recent,
            card: None,
            orientation: None,
            naming: None,
            name_tried: false,
            focus_name: true,
            focus_preset_name: false,
            memory: S::Memory::default(),
        }
    }
}

impl<S: DocumentSettings> Session<S> {
    /// Drops what the user was doing with the settings: the picked card, the orientation, the Save Document Preset pane
    /// and what the details rows remembered.
    fn reset(&mut self) {
        self.card = None;
        self.orientation = None;
        self.naming = None;
        self.name_tried = false;
        self.memory = S::Memory::default();
    }
}

/// Sets everything of `values` but the name to `settings`: what a recent document, a saved preset and the hosted
/// `preset` field restore.
fn restore_all<S: DocumentSettings>(values: &mut S, settings: &S) {
    let name = values.name().to_owned();
    *values = settings.clone();
    values.set_name(name);
}

/// The settings a Create answers with: the name trimmed, "Untitled" when empty.
fn finish<S: DocumentSettings>(mut settings: S, untitled: &str) -> S {
    let name = settings.name().trim().to_owned();
    settings.set_name(if name.is_empty() { untitled.to_owned() } else { name });
    settings
}

fn mode_label(words: &NewDocumentWords, mode: ColorMode) -> &Label {
    match mode {
        ColorMode::Bitmap => &words.bitmap,
        ColorMode::Grayscale => &words.grayscale,
        ColorMode::Rgb => &words.rgb_color,
        ColorMode::Cmyk => &words.cmyk_color,
        ColorMode::Lab => &words.lab_color,
    }
}

fn depth_label(words: &NewDocumentWords, depth: BitDepth) -> &Label {
    match depth {
        BitDepth::One => &words.depth_1,
        BitDepth::Eight => &words.depth_8,
        BitDepth::Sixteen => &words.depth_16,
        BitDepth::ThirtyTwo => &words.depth_32,
    }
}

fn background_label(words: &NewDocumentWords, background: BackgroundContents) -> &Label {
    match background {
        BackgroundContents::White => &words.white,
        BackgroundContents::Black => &words.black,
        BackgroundContents::BackgroundColor => &words.background_color,
        BackgroundContents::Transparent => &words.transparent,
        BackgroundContents::Custom(_) => &words.custom_color,
    }
}

/// The colour the background shows as: `None` for a transparent one.
fn background_rgb(background: BackgroundContents, app_background: [f32; 3]) -> Option<[f32; 3]> {
    match background {
        BackgroundContents::White => Some([1.0; 3]),
        BackgroundContents::Black => Some([0.0; 3]),
        BackgroundContents::BackgroundColor => Some(app_background),
        BackgroundContents::Transparent => None,
        BackgroundContents::Custom(color) => Some(color),
    }
}

/// The settings a Create answers with, and the recent list to keep once the document exists.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct NewDocumentResult<S = DocumentOptions> {
    /// The settings to create the document from, under the name trimmed.
    pub options: S,
    remember: Option<(SaveKey, Vec<S>)>,
}

impl<S: DocumentSettings> NewDocumentResult<S> {
    /// Stores the recent list of a dialog built with [`NewDocumentDialog::remember_recent`], with these settings first,
    /// for the next open. The app calls it after its create command succeeds, so a failed command leaves the list as
    /// it was; without a key it does nothing.
    pub fn remember(&self, ctx: &Context) {
        if let Some((key, list)) = &self.remember {
            crate::runtime::memory::set_saved_value::<Vec<S>>(ctx, key, list.clone());
        }
    }
}

/// What the dialog reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct NewDocumentOutput<S = DocumentOptions> {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The settings to create the document from, the pass the dialog closed as Create. The app keeps `options` as the
    /// settings the next dialog opens on and calls [`NewDocumentResult::remember`] after its create command succeeds.
    pub result: Option<NewDocumentResult<S>>,
    /// What the body asked of the host in this pass: the background colour the app's picker is asked for (Custom was
    /// chosen or the colour button pressed), keyed `BACKGROUND_COLOR_KEY`. The app answers with
    /// [`NewDocumentDialog::answers`].
    pub requests: HostRequests,
    /// The saved-preset edits the user made this pass (create, delete), for the app to store, keyed by the preset's
    /// name before the event.
    pub presets: Vec<NamedPresetEvent<String>>,
    /// This pass's `request_action` ids of the details rows.
    pub actions: Vec<String>,
}

impl<S: DocumentSettings> Outcome for NewDocumentOutput<S> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Leave most of the dialog to the presets and keep the details column at a consistent share of the width.
const COLUMNS: [ColumnWidth; 2] = [ColumnWidth::fill(2.0), ColumnWidth::fill(1.0)];

/// The body of the dialog: the columns, then what they asked for.
fn body<S: DocumentSettings>(ui: &mut Ui, form: &mut FormCx<'_, '_>, values: &mut S, shared: &Shared<'_, S>) -> bool {
    let bound = &shared.bound;
    let (setup, words, units) = (shared.setup, shared.words, shared.units);
    let mut session = form.memory::<Session<S>>().clone();
    if let Some(cell) = bound.memory {
        session.memory.clone_from(&cell.borrow());
    }
    // Settings the host opened already hold what the dialog adds to them.
    if form.first() && !shared.host_opened {
        values.open(setup, shared.untitled);
    }
    let mut answers = form.take_answers();
    if let Some(cell) = bound.tab
        && let Some(tab) = TabId::from_key(cell.borrow().as_str(), shared.categories)
        && tab != session.tab
    {
        session.tab = tab;
        session.card = None;
    }
    let recents: Vec<S> = shared.recents.iter().filter(|recent| recent.fits()).cloned().collect();
    let mut library = match bound.saved {
        Some(cell) => cell.borrow().clone(),
        None => form.saved::<PresetLibrary<S>>(&PRESETS_KEY).clone(),
    };
    let mut changed = false;

    let read = Read {
        setup,
        categories: shared.categories,
        recents: &recents,
        library: &library,
        words,
        preset_words: shared.preset_words,
        library_words: shared.library_words,
        units,
    };
    // The dialog opens on the clipboard's card.
    if form.first()
        && session.tab == TabId::Recent
        && let Some(at) = cards::clipboard_at(&session, values, &read)
    {
        session.card = Some(at);
    }
    let mut left = Left::default();
    let mut right = Right::default();
    cards::tabs(ui, &session, &read, &mut left);
    Divider::horizontal().show(ui);
    Grid::new(&COLUMNS).fill().show(ui, |ui, _| {
        Stack::new().fill().show(ui, |ui, _| {
            cards::show(ui, &session, values, &read, &mut left);
        });
        Stack::new().scroll(Scroll::vertical()).show(ui, |ui, _| {
            details::show(ui, &mut session, values, &read, &mut answers, &mut right);
        });
    });

    if let Some(tab) = left.tab.filter(|tab| *tab != session.tab) {
        session.tab = tab;
        session.card = None;
    }
    if let Some(picked) = left.picked {
        S::apply_card(values, &picked.card, setup, &mut session.memory);
        session.card = Some(picked.index);
        if picked.activated {
            form.request_close_as(CloseReason::Activated);
        }
    }
    for request in right.requests {
        form.requests().request_color(request);
    }
    bound.actions.borrow_mut().extend(right.actions);
    for id in left.delete {
        let name = library.get(id).map(|item| item.name.clone());
        if library.delete(id).is_ok() {
            changed = true;
            session.card = None;
            if let Some(name) = name {
                bound.events.borrow_mut().push(NamedPresetEvent::Delete(name));
            }
        }
    }
    if right.save {
        let name = session.naming.clone().unwrap_or_default();
        if details::refusal(&library.names(), &name).is_none() {
            let mut preset = values.clone();
            preset.set_name(name.trim().to_owned());
            if let Ok(id) = library.add(&GroupPath::root(), &name, preset) {
                changed = true;
                let stored = library.get(id).map_or_else(|| name.trim().to_owned(), |item| item.name.clone());
                bound.events.borrow_mut().push(NamedPresetEvent::Create { name: stored });
                session.tab = TabId::Saved;
                session.card = Some(library.items().iter().filter(|item| item.value.fits()).count().saturating_sub(1));
                session.naming = None;
                session.name_tried = false;
            }
        } else {
            session.name_tried = true;
        }
    }
    if changed {
        match bound.saved {
            Some(cell) => *cell.borrow_mut() = library,
            None => *form.saved::<PresetLibrary<S>>(&PRESETS_KEY) = library,
        }
    }
    if let Some(cell) = bound.tab {
        let key = session.tab.key(shared.categories);
        if *cell.borrow() != key {
            *cell.borrow_mut() = key;
        }
    }
    if let Some(cell) = bound.memory {
        cell.borrow_mut().clone_from(&session.memory);
    }
    *form.memory::<Session<S>>() = session;
    true
}

/// File > New: category tabs over a grid of preset cards, a details column and Create and Close.
///
/// - Tabs: Recent, the categories and Saved. Recent lists the clipboard's image size first when the app gives one,
///   then the recent documents kept under [`remember_recent`](NewDocumentDialog::remember_recent) (the last 20), then the built-in defaults not yet among them. Saved lists the user's presets in a list
///   whose Delete key removes the picked ones, with "No saved presets yet." when there are none.
/// - Cards: a click or an arrow fills the details; Enter on a card or a double click creates. A built-in card turns with
///   the orientation control. Saved and recent cards restore every setting but the name. A Web or Mobile card turns
///   Artboards on; a card of another category turns it off again only when such a card turned it on.
/// - Details: Name with the Save Document Preset icon, Width and Height with their units, Orientation, Artboards,
///   Resolution, Color Mode, Depth, Color Profile, Pixel Aspect Ratio (the built-in ratios, then the app's
///   [`custom_pixel_aspects`](NewDocumentDialog::custom_pixel_aspects)) and Background Contents. Bitmap takes only 1
///   bit and offers no Transparent background ([`DocumentOptions::fitted`]).
/// - Save Document Preset: the icon opens a pane in place of the details with the first free `Saved N` selected;
///   Save Preset or Enter saves the preset and does not create the document, Cancel goes back. The name is checked by
///   the preset library (not blank, different from the saved names ignoring case), and at most 256 presets are kept.
/// - Keys: Tab walks the fields, Enter is Create unless a field owns it, Escape closes. Holding Alt turns Close into
///   Reset, which restores the settings from when the dialog opened and keeps it open. Ctrl+Z inside the dialog steps
///   the dialog's own settings.
/// - Settings: the dialog opens on the value in the bind (the app seeds it from the last created settings) and writes
///   its edits back. Create answers with `result`, the same settings under the name trimmed.
/// - Custom under Background Contents, or the colour button, asks the app for a colour: the output's `requests` carries
///   the colour request for one pass, and the app answers with `answers`.
/// - Memory: the saved presets are kept across runs. The recent documents are kept under `remember_recent`; the last created settings are the app's.
/// - Settings type: the dialog is generic over [`DocumentSettings`]; [`new`](NewDocumentDialog::new) is the raster
///   one over `DocumentOptions`, and [`over`](NewDocumentDialog::over) takes any other.
/// - Host details: after Name the details column shows the settings type's own rows (`DocumentSettings::details`), which
///   edit the settings, keep what they need in the session's memory and ask the app for actions that come back in
///   `NewDocumentOutput::actions`. The settings are made valid (`DocumentSettings::fitted`) after every edit.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
#[pattern(probe(NewDocumentDialog<'static, DocumentOptions>))]
pub struct NewDocumentDialog<'a, S: DocumentSettings = DocumentOptions> {
    bound: Bind<'a, S>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// The categories in tab order, in place of `PresetCategory::standard()`.
    #[prop]
    categories: Vec<PresetCategory<S::Preset>>,
    /// The cards Recent shows after the recent documents, in place of `PresetCategory::default_recent()`. A card with
    /// the id `"default"` is named "Default {app} Size".
    #[prop]
    remember_recent: Option<SaveKey>,
    /// Binds the saved presets of the Saved tab to `saved`, in place of the list the dialog keeps across runs. Each
    /// save and delete is also reported in [`NewDocumentOutput::presets`] for the app to store.
    #[prop]
    saved: Option<Bind<'a, PresetLibrary<S>>>,
    /// Binds the recent documents of the Recent tab to `recent`, in place of the list of `remember_recent`. The dialog
    /// only reads it: the app adds the created document itself.
    #[prop]
    recent: Option<Bind<'a, Vec<S>>>,
    /// Binds the shown tab's id (`recent`, `saved` or a category's id) to `tab`.
    #[prop]
    tab: Option<Bind<'a, String>>,
    /// Keeps what the details rows keep (`DocumentSettings::Memory`) in `memory`, in place of the dialog's session, so
    /// a host that sets the settings from outside the dialog (`DocumentSettings::note_set`) keeps it with them.
    #[prop]
    memory: Option<&'a mut S::Memory>,
    /// Keeps the dialog's place and size across runs under `key`.
    #[prop]
    remember: Option<SaveKey>,
    /// The host's answers to the requests of earlier passes.
    #[prop]
    answers: Vec<HostAnswer>,
    /// The settings the host opened the dialog with, the clipboard's size and the default name filled in (as
    /// `HostedNewDocument` does when it is made): the first pass keeps the bound settings as they are, and Reset
    /// restores these (a field edited before the dialog was first shown is not the Reset target).
    #[prop]
    opening: Option<S>,
    /// What the dialog is set up with besides the cards.
    #[prop]
    setup: S::Setup,
    /// The resolution print presets open at, in pixels per inch, over the setup's.
    #[prop(quantity = "resolution in pixels per inch")]
    print_resolution: Option<f64>,
    /// The resolution screen presets open at, in pixels per inch, over the setup's.
    #[prop(quantity = "resolution in pixels per inch")]
    screen_resolution: Option<f64>,
}

impl<'a, S: DocumentSettings> NewDocumentDialog<'a, S> {
    /// The dialog over any settings type: no categories and the default setup.
    pub fn over(options: Bind<'a, S>) -> Self {
        NewDocumentDialog {
            bound: options,
            open: None,
            categories: Vec::new(),
            remember_recent: None,
            saved: None,
            recent: None,
            tab: None,
            memory: None,
            remember: None,
            answers: Vec::new(),
            opening: None,
            setup: S::Setup::default(),
            print_resolution: None,
            screen_resolution: None,
        }
    }
}

impl<'a> NewDocumentDialog<'a, DocumentOptions> {
    /// The dialog over the settings in `options`, with `PresetCategory::standard()`, print resolution 300 and screen
    /// resolution 72.
    pub fn new(options: Bind<'a, DocumentOptions>) -> Self {
        let mut dialog = Self::over(options);
        dialog.categories = PresetCategory::standard();
        dialog
    }

    /// The cards Recent shows after the recent documents, in place of `PresetCategory::default_recent()`. A card with
    /// the id `"default"` is named "Default {app} Size".
    pub fn recent_defaults(mut self, presets: Vec<DocumentPreset>) -> Self {
        self.setup.recent_defaults = presets;
        self
    }

    /// The clipboard holds an image of this size: Recent shows a Clipboard card first and the dialog opens on it.
    pub fn clipboard(mut self, size: PixelSize) -> Self {
        self.setup.clipboard = Some(size);
        self
    }

    /// The app's background colour, offered as the Background Color choice.
    pub fn background_color(mut self, color: Srgba) -> Self {
        self.setup.background = color.to_rgb_array();
        self
    }

    /// The colour profiles each mode offers.
    pub fn profiles(mut self, profiles: Vec<ColorProfiles>) -> Self {
        self.setup.profiles = profiles;
        self
    }

    /// The app's own pixel aspect ratios as `(id, name)`, listed after the built-in ones ([`PIXEL_ASPECTS`]); the
    /// names are the user's and are not translated.
    pub fn custom_pixel_aspects(mut self, aspects: Vec<(String, String)>) -> Self {
        self.setup.custom_pixel_aspects = aspects;
        self
    }

    /// The application's name, for the "Default {app} Size" card.
    pub fn app_name(mut self, name: impl Into<Label>) -> Self {
        self.setup.app_name = Some(name.into());
        self
    }
}

impl<'a, S: DocumentSettings> NewDocumentDialog<'a, S> {
    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> NewDocumentOutput<S> {
        let NewDocumentDialog {
            bound,
            open,
            categories,
            remember_recent,
            mut saved,
            recent: mut recent_bound,
            mut tab,
            memory,
            remember,
            answers,
            opening,
            mut setup,
            print_resolution,
            screen_resolution,
        } = self;
        S::set_resolutions(&mut setup, print_resolution, screen_resolution);
        let recent_is_bound = recent_bound.is_some();
        let recent: Vec<S> = match recent_bound.as_mut() {
            Some(bind) => read_bind(bind).unwrap_or_default(),
            None => remember_recent.as_ref().map_or_else(Vec::new, |key| crate::runtime::memory::saved_value::<Vec<S>>(ui.ctx(), key)),
        };
        let saved_cell = saved.as_mut().map(|bind| RefCell::new(read_bind(bind).unwrap_or_default()));
        let tab_cell = tab.as_mut().map(|bind| RefCell::new(read_bind(bind).unwrap_or_default()));
        let memory_cell = memory.as_deref().map(|memory| RefCell::new(memory.clone()));
        let events = RefCell::new(Vec::new());
        let actions = RefCell::new(Vec::new());
        let env = Env::of(ui);
        let words = env.words::<NewDocumentWords>();
        let preset_words = env.words::<NewDocumentPresetWords>();
        let library_words = env.words::<LibraryWords>();
        let units = env.units();
        let untitled = words.untitled.as_str().to_owned();
        let shared = Shared {
            bound: Bound { saved: saved_cell.as_ref(), tab: tab_cell.as_ref(), memory: memory_cell.as_ref(), events: &events, actions: &actions },
            setup: &setup,
            categories: &categories,
            recents: &recent,
            words: &words,
            preset_words: &preset_words,
            library_words: &library_words,
            units: &units,
            untitled: &untitled,
            host_opened: opening.is_some(),
        };

        let mut dialog = FormDialog::new(words.title.clone(), bound)
            .ok_label(words.create.clone())
            .cancel_label(words.close.clone())
            .collection_body(true)
            .outer_gesture(true)
            .answers(answers)
            .on_reset(|cx| {
                cx.memory::<Session<S>>().reset();
                if let Some(cell) = &memory_cell {
                    *cell.borrow_mut() = S::Memory::default();
                }
            });
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        if let Some(opening) = opening {
            dialog = dialog.opening(opening);
        }
        if let Some(key) = remember {
            dialog = dialog.shaped(move |dialog| dialog.remember(key));
        }
        let shown = dialog.show(ui, |ui, form, values| body(ui, form, values, &shared));
        let events = events.into_inner();
        if !events.is_empty()
            && let (Some(bind), Some(cell)) = (saved.as_mut(), saved_cell)
        {
            write_bind(bind, cell.into_inner());
        }
        if let (Some(bind), Some(cell)) = (tab.as_mut(), tab_cell) {
            let now = cell.into_inner();
            if read_bind(bind).as_ref() != Some(&now) {
                write_bind(bind, now);
            }
        }
        if let (Some(memory), Some(cell)) = (memory, memory_cell) {
            *memory = cell.into_inner();
        }
        NewDocumentOutput {
            response: shown.response,
            closed: shown.closed,
            result: shown.accepted.map(|options| {
                let options = finish(options, &untitled);
                let remember = remember_recent.filter(|_| !recent_is_bound).map(|key| {
                    let mut list = recent.clone();
                    let mut entry = options.clone();
                    entry.set_name(String::new());
                    push_recent(&mut list, entry, presets::MAX_RECENT);
                    (key, list)
                });
                NewDocumentResult { options, remember }
            }),
            requests: shown.requests,
            presets: events,
            actions: actions.into_inner(),
        }
    }
}

type Build<S> = dyn for<'b> Fn(&'b mut S) -> NewDocumentDialog<'b, S> + Send;

/// A [`NewDocumentDialog`] the [`DialogHost`](crate::dialog::DialogHost) owns: the host keeps the settings and the colour request, and `build` makes the dialog over the settings each pass.
///
/// ```ignore
/// host.open(HostedNewDocument::new(ctx, last_created, |options| {
///     NewDocumentDialog::new(Bind::bound(options)).clipboard(size).app_name("PhotoCraft")
/// }));
/// // each pass, for a colour request:
/// if let Some(dialog) = host.get_mut::<HostedNewDocument>() {
///     if let Some(request) = dialog.pick_color() { /* open the colour picker, then */ dialog.set_color(&request.key, color); }
/// }
/// ```
pub struct HostedNewDocument<S: DocumentSettings = DocumentOptions> {
    id: DialogId,
    options: S,
    build: Box<Build<S>>,
    requests: HostRequests,
    /// Whether the dialog is open: bound to the dialog's `opened` prop.
    open: bool,
    /// The saved presets the app gave, as the user left them.
    saved: PresetLibrary<S>,
    /// The recent documents the app gave; the app adds the created one itself.
    recent: Vec<S>,
    /// The shown tab's id.
    tab: String,
    /// The saved-preset edits the app has not taken yet.
    events: Vec<NamedPresetEvent<String>>,
    /// The settings the dialog opened on, the clipboard's size and the default name filled in: what Reset restores,
    /// whatever automation sets before the first show.
    opening: S,
    /// Whether OK files its answer for the host instead of closing ([`held_ok`](HostedNewDocument::held_ok)).
    hold: bool,
    /// The OK the host holds back until it answers `answer_ok`, and its refusal.
    held: HeldAnswer<NewDocumentResult<S>>,
    /// What the dialog is set up with when the host set it ([`set_setup`](HostedNewDocument::set_setup)); otherwise the
    /// built dialog's own.
    setup: Option<S::Setup>,
    /// The `request_action` ids the app has not taken yet.
    actions: Vec<String>,
    /// What the details rows keep, bound to the dialog, so a field automation sets is noted where the rows read it.
    memory: S::Memory,
}

impl<S: DocumentSettings> HostedNewDocument<S> {
    /// Names the tab the dialog opens on.
    #[must_use]
    pub fn tab(mut self, key: impl Into<String>) -> Self {
        self.tab = key.into();
        self
    }

    /// The `request_action` ids of the details rows since the last call.
    pub fn take_actions(&mut self) -> Vec<String> {
        std::mem::take(&mut self.actions)
    }

    /// Sets what the dialog is set up with, from the next pass on.
    pub fn set_setup(&mut self, setup: S::Setup) {
        self.setup = Some(setup);
    }

    /// Replaces the settings the dialog shows.
    pub fn set_options(&mut self, options: S) {
        self.options = options;
    }

    /// Makes OK file its answer for the host to take instead of closing: the host checks it and answers with
    /// [`answer_ok`](HostedNewDocument::answer_ok). OK stays disabled until then.
    #[must_use]
    pub fn held_ok(mut self) -> Self {
        self.hold = true;
        self
    }

    /// The OK filed by a held dialog, once.
    pub fn take_ok(&mut self) -> Option<NewDocumentResult<S>> {
        self.held.take()
    }

    /// The host's answer to the held OK: `Ok` closes the dialog with the held answer in the next pass; `Err` enables
    /// OK again and shows the label until a value changes.
    pub fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }
}

impl<S: DocumentSettings> HostedNewDocument<S> {
    /// A hosted dialog opening on `options`; `build` makes the dialog over them (the clipboard, the profiles, the
    /// app's name). The settings open at once (the clipboard's size, the default name from the root's words), so
    /// automation lists and sets them before the first pass.
    pub fn new(ctx: &egui::Context, options: S, build: impl for<'b> Fn(&'b mut S) -> NewDocumentDialog<'b, S> + Send + 'static) -> Self {
        let mut hosted = HostedNewDocument {
            id: dialog_id("new-document"),
            opening: S::default(),
            options,
            build: Box::new(build),
            requests: HostRequests::new(),
            open: true,
            saved: PresetLibrary::new(),
            recent: Vec::new(),
            tab: TabId::Recent.key::<S::Preset>(&[]),
            events: Vec::new(),
            hold: false,
            held: HeldAnswer::new(),
            setup: None,
            actions: Vec::new(),
            memory: S::Memory::default(),
        };
        let setup = hosted.setup_now();
        hosted.options.open(&setup, Env::of_root(ctx).words::<NewDocumentWords>().untitled.as_str());
        hosted.opening = hosted.options.clone();
        hosted
    }

    /// What the dialog is set up with: the host's setup or the built dialog's, under the built dialog's resolutions.
    fn setup_now(&mut self) -> S::Setup {
        let built = (self.build)(&mut self.options);
        let mut setup = self.setup.clone().unwrap_or(built.setup);
        S::set_resolutions(&mut setup, built.print_resolution, built.screen_resolution);
        setup
    }

    /// The settings the details column shows.
    pub fn options(&self) -> &S {
        &self.options
    }

    /// The colour the dialog asks the app's picker for, once: Custom was chosen or the colour button pressed.
    pub fn pick_color(&mut self) -> Option<ColorRequest> {
        self.requests.take_color()
    }

    /// The colour the app's picker returned for `key`: the dialog sets Background Contents to
    /// `BackgroundContents::Custom` in its next pass. Other keys are ignored.
    pub fn set_color(&mut self, key: &str, color: Srgba) {
        self.requests.answer_color(key, color);
    }

    /// Opens with `library` as the saved presets.
    #[must_use]
    pub fn saved(mut self, library: PresetLibrary<S>) -> Self {
        self.saved = library;
        self
    }

    /// Opens with `list` as the recent documents.
    #[must_use]
    pub fn recent(mut self, list: Vec<S>) -> Self {
        self.recent = list;
        self
    }

    /// The saved presets as the user left them.
    pub fn saved_presets(&self) -> &PresetLibrary<S> {
        &self.saved
    }

    /// The saved-preset edits since the last call, for the app to store. A created preset's value is in
    /// [`saved_presets`](Self::saved_presets) under its name.
    pub fn take_preset_events(&mut self) -> Vec<NamedPresetEvent<String>> {
        std::mem::take(&mut self.events)
    }

    /// The picker was cancelled: the request is dropped and Background Contents stays as it was.
    pub fn cancel_color(&mut self) {
        self.requests.cancel_color();
    }
}

impl<S: DocumentSettings> StackDialog for HostedNewDocument<S> {
    type Result = NewDocumentResult<S>;

    /// Answers what Create answers. The recent list is bound, so the app adds the document itself (`remember` is
    /// empty, as in a pass). A held dialog files the answer for the host, as its button does, and answers none; once
    /// the host accepted the held answer, it answers that.
    fn confirm(&mut self, ctx: &egui::Context) -> Option<Self::Result> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if self.held.is_waiting() {
            return None;
        }
        let words = Env::of_root(ctx).words::<NewDocumentWords>();
        let result = NewDocumentResult { options: finish(self.options.clone(), words.untitled.as_str()), remember: None };
        if self.hold {
            self.held.file(result);
            return None;
        }
        Some(result)
    }

    /// The settings by their serde keys, the shown tab's id (`tab`) and the name of the saved preset the settings
    /// equal but for the name (`preset`, null when none).
    fn json_fields(&self) -> Option<serde_json::Value> {
        let mut fields = serde_fields(&self.options)?;
        let preset = self.saved.items().into_iter().find(|item| {
            let mut restored = self.options.clone();
            restore_all(&mut restored, &item.value);
            restored == self.options
        });
        if let serde_json::Value::Object(map) = &mut fields {
            map.insert("tab".to_owned(), serde_json::Value::String(self.tab.clone()));
            map.insert("preset".to_owned(), preset.map_or(serde_json::Value::Null, |item| serde_json::Value::String(item.name.clone())));
        }
        Some(fields)
    }

    /// Sets a settings key as the serde round trip takes it, `tab` to a tab the dialog has, or `preset` to a saved
    /// preset's name, which applies the preset's card as picking it does.
    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        match field {
            "tab" => {
                let key = value.as_str().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                let categories = (self.build)(&mut self.options).categories;
                if TabId::from_key(key, &categories).is_none() {
                    return Err(FieldRefusal::Rejects(&value).message(field));
                }
                self.tab = key.to_owned();
                Ok(())
            }
            "preset" => {
                let name = value.as_str().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                let item =
                    self.saved.items().into_iter().find(|item| item.name == name).ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                let card = DocumentCard::new(item.name.clone(), String::new(), item.value.clone(), CardSource::Saved(item.id));
                let setup = self.setup_now();
                S::apply_card(&mut self.options, &card, &setup, &mut self.memory);
                Ok(())
            }
            _ => {
                set_serde_field(&mut self.options, field, value)?;
                self.options.note_set(field, &mut self.memory);
                self.options = std::mem::take(&mut self.options).fitted();
                Ok(())
            }
        }
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<NewDocumentResult<S>>> {
        let measuring = crate::runtime::measure::active(ui.ctx());
        // The host accepted the held answer: this pass's real run closes the dialog with it.
        if !measuring && self.held.is_accepted() {
            return Some(self.held.finish().map_or(DialogResult::Cancel, DialogResult::Ok));
        }
        let answers = if measuring { Vec::new() } else { self.requests.take_answers() };
        let mut dialog = (self.build)(&mut self.options)
            .opened(Bind::bound(&mut self.open))
            .saved(Bind::bound(&mut self.saved))
            .recent(Bind::bound(&mut self.recent))
            .tab(Bind::bound(&mut self.tab))
            .memory(&mut self.memory)
            .answers(answers)
            .opening(self.opening.clone());
        if let Some(setup) = &self.setup {
            dialog = dialog.setup(setup.clone());
        }
        let shown = dialog.show(ui);
        self.requests.extend(shown.requests);
        self.events.extend(shown.presets);
        self.actions.extend(shown.actions);
        match (shown.result, shown.closed) {
            // A held dialog files its answer and stays open until the host answers it.
            (Some(result), _) if self.hold => {
                if !self.held.is_waiting() {
                    self.held.file(result);
                }
                self.open = true;
                None
            }
            (Some(result), _) => Some(DialogResult::Ok(result)),
            (None, Some(_)) => {
                self.requests.cancel();
                Some(DialogResult::Cancel)
            }
            (None, None) => None,
        }
    }

    fn key(&self) -> Option<&str> {
        Some("new-document")
    }

    fn cancelled(&mut self, _: &egui::Context) {
        self.requests.cancel();
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }
}

impl<S: DocumentSettings> HoldsOk for HostedNewDocument<S> {
    type Answer = NewDocumentResult<S>;

    fn held_ok(self) -> Self {
        HostedNewDocument::held_ok(self)
    }

    fn take_ok(&mut self) -> Option<NewDocumentResult<S>> {
        HostedNewDocument::take_ok(self)
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        HostedNewDocument::answer_ok(self, answer);
    }
}
