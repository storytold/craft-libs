//! `NewDocumentDialog`: File > New. Category tabs over a grid of preset cards, a details column, saved presets and the
//! document settings the app creates the document from.

mod cards;
mod details;
mod presets;
#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::form::{read_bind, write_bind};
use super::{FormCx, FormDialog, HostRequests, LinkedSize};
use crate::contract::{Bind, NoValue, Outcome, Remember as _, SaveKey, Scroll, ShowContainer as _};
use crate::runtime::requests::{ColorRequest, HostAnswer};

use crate::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id, serde_fields, set_serde_field};

use crate::layout_components::{Grid, Stack};

use crate::patterns::presets::{GroupPath, NamedPresetEvent, PresetLibrary, PresetWords as LibraryWords};

use crate::runtime::env::{Env, Label, UnitKind, Units};
use crate::runtime::overlay::CloseReason;
use craft_behavior::{ColumnWidth, PixelSize};

use craft_theme::Srgba;
use egui::{Context, Response, Ui};
use std::cell::RefCell;

use cards::{Left, TabId};

use details::Right;

pub(crate) use presets::push_recent;
pub use presets::{
    BackgroundContents, BitDepth, ColorMode, ColorProfiles, DocumentOptions, DocumentPreset, NewDocumentPresetWords, Orientation, PIXEL_ASPECTS,
    PresetCategory, SQUARE_PIXELS,
};

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

/// What the dialog is set up with: the cards, the clipboard, the profiles and the resolutions.
struct Setup {
    categories: Vec<PresetCategory>,
    recent_defaults: Vec<DocumentPreset>,
    recent: Vec<DocumentOptions>,
    clipboard: Option<PixelSize>,
    background: [f32; 3],
    print_ppi: f64,
    screen_ppi: f64,
    profiles: Vec<ColorProfiles>,
    app_name: Option<Label>,
    custom_pixel_aspects: Vec<(String, String)>,
}

/// What the columns read besides the session: the setup, the remembered lists, the words and the units.
#[derive(Clone, Copy)]
struct Read<'a> {
    setup: &'a Setup,
    recents: &'a [DocumentOptions],
    library: &'a PresetLibrary<DocumentOptions>,
    words: &'a NewDocumentWords,
    preset_words: &'a NewDocumentPresetWords,
    library_words: &'a LibraryWords,
    units: &'a Units,
}

/// The values the app binds, as the body works on them during one frame. A frame can run the body twice; the cells
/// carry what the first pass did into the second, and `events` collects the edits of both passes.
#[derive(Clone, Copy)]
struct Bound<'a> {
    /// The bound saved presets; `None` keeps them in the form memory.
    saved: Option<&'a RefCell<PresetLibrary<DocumentOptions>>>,
    /// The bound tab id.
    tab: Option<&'a RefCell<String>>,
    /// Each save and delete of the frame.
    events: &'a RefCell<Vec<NamedPresetEvent<String>>>,
}

/// What the body reads besides its form: the setup, the words and the units.
#[derive(Clone, Copy)]
struct Shared<'a> {
    bound: Bound<'a>,
    setup: &'a Setup,
    words: &'a NewDocumentWords,
    preset_words: &'a NewDocumentPresetWords,
    library_words: &'a LibraryWords,
    units: &'a Units,
    untitled: &'a str,
    /// The host opened the settings ([`NewDocumentDialog::opening`]).
    host_opened: bool,
}

/// What the dialog keeps while it is open besides the settings: the fields that show the size, the open tab and card,
/// the Save Document Preset pane and whether a card turned Artboards on. It lives in the dialog's body slot.
#[derive(Clone)]
struct Session {
    /// The width, height and resolution fields of the settings.
    size: LinkedSize,
    /// The size before a running edit of a size field, for putting it back when the edit is cancelled.
    gesture: Option<LinkedSize>,
    tab: TabId,
    /// The card the user picked in the open tab (the saved row in Saved).
    card: Option<usize>,
    /// The orientation picked with the control; built-in cards turn with it.
    orientation: Option<Orientation>,
    /// The Save Document Preset pane's name, while the pane stands in for the details.
    naming: Option<String>,
    /// A Web or Mobile card turned Artboards on, so a card of another tab turns it off again; the user's own tick ends
    /// it.
    preset_artboard: bool,
    /// A save was refused and its reason shows under the name field.
    name_tried: bool,
    focus_name: bool,
    focus_preset_name: bool,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            size: LinkedSize::new(PixelSize::new(1, 1)),
            gesture: None,
            tab: TabId::Recent,
            card: None,
            orientation: None,
            naming: None,
            preset_artboard: false,
            name_tried: false,
            focus_name: true,
            focus_preset_name: false,
        }
    }
}

impl Session {
    /// Drops what the user was doing with the settings: the picked card, the orientation, the Save Document Preset pane and a
    /// running size edit. The fields that show the size follow the settings by themselves.
    fn reset(&mut self) {
        self.card = None;
        self.orientation = None;
        self.naming = None;
        self.name_tried = false;
        self.gesture = None;
    }

    /// Applies a card: a recent document or saved preset restores everything but the name, a built-in card sets the
    /// size and unit.
    fn apply_card(&mut self, values: &mut DocumentOptions, card: &cards::Card, points_per_inch: f64) {
        if card.restores_all {
            restore_all(values, &card.settings);
            self.preset_artboard = false;
        } else {
            values.size = card.settings.size;
            values.pixels_per_inch = card.settings.pixels_per_inch;
            values.unit = card.settings.unit;
            match card.artboard {
                Some(true) if !values.artboard => {
                    values.artboard = true;
                    self.preset_artboard = true;
                }
                Some(false) if self.preset_artboard => {
                    values.artboard = false;
                    self.preset_artboard = false;
                }
                _ => {}
            }
        }
        self.size = values.linked(points_per_inch);
        self.gesture = None;
    }
}

/// Sets everything of `values` but the name to `settings`: what a recent document, a saved preset and the hosted
/// `preset` field restore. The fields that show the size follow on the next pass.
fn restore_all(values: &mut DocumentOptions, settings: &DocumentOptions) {
    let name = std::mem::take(&mut values.name);
    *values = DocumentOptions { name, ..settings.clone() };
}

/// The settings a Create answers with: the name trimmed, "Untitled" when empty.
fn finish(mut settings: DocumentOptions, untitled: &str) -> DocumentOptions {
    settings.name = settings.name.trim().to_owned();
    if settings.name.is_empty() {
        untitled.clone_into(&mut settings.name);
    }
    settings
}

/// Fills what the dialog adds to the settings it opens on: the clipboard's size at the screen resolution and the
/// default name.
fn open_options(values: &mut DocumentOptions, clipboard: Option<PixelSize>, screen_ppi: f64, untitled: &str) {
    if let Some(size) = clipboard {
        values.size = size;
        values.unit = UnitKind::Pixels;
        values.pixels_per_inch = screen_ppi;
    }
    if values.name.is_empty() {
        untitled.clone_into(&mut values.name);
    }
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
pub struct NewDocumentResult {
    /// The settings to create the document from, under the name trimmed.
    pub options: DocumentOptions,
    remember: Option<(SaveKey, Vec<DocumentOptions>)>,
}

impl NewDocumentResult {
    /// Stores the recent list of a dialog built with [`NewDocumentDialog::remember_recent`], with these settings first,
    /// for the next open. The app calls it after its create command succeeds, so a failed command leaves the list as
    /// it was; without a key it does nothing.
    pub fn remember(&self, ctx: &Context) {
        if let Some((key, list)) = &self.remember {
            crate::runtime::memory::set_saved_value::<Vec<DocumentOptions>>(ctx, key, list.clone());
        }
    }
}

/// What the dialog reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct NewDocumentOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The settings to create the document from, the pass the dialog closed as Create. The app keeps `options` as the
    /// settings the next dialog opens on and calls [`NewDocumentResult::remember`] after its create command succeeds.
    pub result: Option<NewDocumentResult>,
    /// What the body asked of the host in this pass: the background colour the app's picker is asked for (Custom was
    /// chosen or the colour button pressed), keyed `BACKGROUND_COLOR_KEY`. The app answers with
    /// [`NewDocumentDialog::answers`].
    pub requests: HostRequests,
    /// The saved-preset edits the user made this pass (create, delete), for the app to store, keyed by the preset's
    /// name before the event.
    pub presets: Vec<NamedPresetEvent<String>>,
}

impl Outcome for NewDocumentOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The columns of the body: the cards take the room the details column leaves.
static COLUMNS: [ColumnWidth; 2] = [ColumnWidth::fill(1.0), ColumnWidth::Content];

/// The body of the dialog: the columns, then what they asked for.
fn body(ui: &mut Ui, form: &mut FormCx<'_, '_>, values: &mut DocumentOptions, shared: &Shared<'_>) -> bool {
    let Shared { bound, setup, words, preset_words, library_words, units, untitled, host_opened } = *shared;
    let points = units.points_per_inch();
    let mut session = form.memory::<Session>().clone();
    // The dialog opens on the clipboard's card; settings the host opened already hold its size.
    if form.first() {
        if !host_opened {
            open_options(values, setup.clipboard, setup.screen_ppi, untitled);
        }
        if setup.clipboard.is_some() {
            session.card = Some(0);
        }
    }
    for answer in form.take_answers() {
        if let HostAnswer::Color { key, color } = answer
            && key == BACKGROUND_COLOR_KEY
        {
            values.background = BackgroundContents::Custom(color.to_rgb_array());
        }
    }
    if !values.shown_by(&session.size) {
        session.size = values.linked(points);
        session.gesture = None;
    }
    if let Some(cell) = bound.tab
        && let Some(tab) = TabId::from_key(cell.borrow().as_str(), &setup.categories)
        && tab != session.tab
    {
        session.tab = tab;
        session.card = None;
    }
    let recents: Vec<DocumentOptions> = setup.recent.iter().filter(|recent| recent.fits()).cloned().collect();
    let mut library = match bound.saved {
        Some(cell) => cell.borrow().clone(),
        None => form.saved::<PresetLibrary<DocumentOptions>>(&PRESETS_KEY).clone(),
    };
    let mut changed = false;

    let read = Read { setup, recents: &recents, library: &library, words, preset_words, library_words, units };
    let mut left = Left::default();
    let mut right = Right::default();
    Grid::new(&COLUMNS).fill().show(ui, |ui, _| {
        Stack::new().fill().show(ui, |ui, _| {
            cards::show(ui, &session, values, &read, &mut left);
        });
        Stack::new().scroll(Scroll::vertical()).show(ui, |ui, _| {
            details::show(ui, &mut session, values, &read, &mut right);
        });
    });

    if let Some(tab) = left.tab.filter(|tab| *tab != session.tab) {
        session.tab = tab;
        session.card = None;
    }
    if let Some(picked) = left.picked {
        session.apply_card(values, &picked.card, points);
        session.card = Some(picked.index);
        if picked.activated {
            form.request_close_as(CloseReason::Activated);
        }
    }
    if right.request_color {
        let current = background_rgb(values.background, setup.background).unwrap_or([1.0; 3]);
        form.requests().request_color(ColorRequest::new(BACKGROUND_COLOR_KEY, Srgba::from_rgb_array(current)).title(words.custom_swatch.clone()));
    }
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
        if details::refusal(&library, &name).is_none() {
            let preset = DocumentOptions { name: name.trim().to_owned(), ..values.clone() };
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
            None => *form.saved::<PresetLibrary<DocumentOptions>>(&PRESETS_KEY) = library,
        }
    }
    if let Some(cell) = bound.tab {
        let key = session.tab.key(&setup.categories);
        if *cell.borrow() != key {
            *cell.borrow_mut() = key;
        }
    }
    *form.memory::<Session>() = session;
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
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct NewDocumentDialog<'a> {
    bound: Bind<'a, DocumentOptions>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// The categories in tab order, in place of `PresetCategory::standard()`.
    #[prop]
    categories: Vec<PresetCategory>,
    /// The cards Recent shows after the recent documents, in place of `PresetCategory::default_recent()`. A card with
    /// the id `"default"` is named "Default {app} Size".
    #[prop]
    recent_defaults: Vec<DocumentPreset>,
    /// Keeps the documents created lately, newest first, under `key`, and lists them in Recent. The result's
    /// [`NewDocumentResult::remember`] stores the list with the created settings added.
    #[prop]
    remember_recent: Option<SaveKey>,
    /// The clipboard holds an image of this size: Recent shows a Clipboard card first and the dialog opens on it.
    #[prop]
    clipboard: Option<PixelSize>,
    /// The app's background colour, offered as the Background Color choice.
    #[prop]
    background_color: Option<Srgba>,
    /// The resolution print presets open at, in pixels per inch.
    #[prop(quantity = "resolution in pixels per inch")]
    print_resolution: f64,
    /// The resolution screen presets open at, in pixels per inch.
    #[prop(quantity = "resolution in pixels per inch")]
    screen_resolution: f64,
    /// The colour profiles each mode offers.
    #[prop]
    profiles: Vec<ColorProfiles>,
    /// The app's own pixel aspect ratios as `(id, name)`, listed after the built-in ones ([`PIXEL_ASPECTS`]); the
    /// names are the user's and are not translated.
    #[prop]
    custom_pixel_aspects: Vec<(String, String)>,
    /// Binds the saved presets of the Saved tab to `saved`, in place of the list the dialog keeps across runs. Each
    /// save and delete is also reported in [`NewDocumentOutput::presets`] for the app to store.
    #[prop]
    saved: Option<Bind<'a, PresetLibrary<DocumentOptions>>>,
    /// Binds the recent documents of the Recent tab to `recent`, in place of the list of `remember_recent`. The dialog
    /// only reads it: the app adds the created document itself.
    #[prop]
    recent: Option<Bind<'a, Vec<DocumentOptions>>>,
    /// Binds the shown tab's id (`recent`, `saved` or a category's id) to `tab`.
    #[prop]
    tab: Option<Bind<'a, String>>,
    /// The application's name, for the "Default {app} Size" card.
    #[prop(into)]
    app_name: Option<Label>,
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
    opening: Option<DocumentOptions>,
}

impl<'a> NewDocumentDialog<'a> {
    /// The dialog over the settings in `options`, with `PresetCategory::standard()`, print resolution 300 and screen
    /// resolution 72.
    pub fn new(options: Bind<'a, DocumentOptions>) -> Self {
        NewDocumentDialog {
            bound: options,
            open: None,
            categories: PresetCategory::standard(),
            recent_defaults: PresetCategory::default_recent(),
            remember_recent: None,
            clipboard: None,
            background_color: None,
            print_resolution: 300.0,
            screen_resolution: 72.0,
            profiles: Vec::new(),
            custom_pixel_aspects: Vec::new(),
            saved: None,
            recent: None,
            tab: None,
            app_name: None,
            remember: None,
            answers: Vec::new(),
            opening: None,
        }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> NewDocumentOutput {
        let NewDocumentDialog {
            bound,
            open,
            categories,
            recent_defaults,
            remember_recent,
            clipboard,
            background_color,
            print_resolution,
            screen_resolution,
            profiles,
            custom_pixel_aspects,
            mut saved,
            recent: mut recent_bound,
            mut tab,
            app_name,
            remember,
            answers,
            opening,
        } = self;
        let recent_is_bound = recent_bound.is_some();
        let recent: Vec<DocumentOptions> = match recent_bound.as_mut() {
            Some(bind) => read_bind(bind).unwrap_or_default(),
            None => remember_recent.as_ref().map_or_else(Vec::new, |key| crate::runtime::memory::saved_value::<Vec<DocumentOptions>>(ui.ctx(), key)),
        };
        let saved_cell = saved.as_mut().map(|bind| RefCell::new(read_bind(bind).unwrap_or_default()));
        let tab_cell = tab.as_mut().map(|bind| RefCell::new(read_bind(bind).unwrap_or_default()));
        let events = RefCell::new(Vec::new());
        let env = Env::of(ui);
        let words = env.words::<NewDocumentWords>();
        let preset_words = env.words::<NewDocumentPresetWords>();
        let library_words = env.words::<LibraryWords>();
        let units = env.units();
        let setup = Setup {
            categories,
            recent_defaults,
            recent,
            clipboard,
            background: background_color.map_or([1.0; 3], |color| color.to_rgb_array()),
            print_ppi: print_resolution,
            screen_ppi: screen_resolution,
            profiles,
            app_name,
            custom_pixel_aspects,
        };
        let untitled = words.untitled.as_str().to_owned();
        let shared = Shared {
            bound: Bound { saved: saved_cell.as_ref(), tab: tab_cell.as_ref(), events: &events },
            setup: &setup,
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
            .on_reset(|cx| cx.memory::<Session>().reset());
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
        NewDocumentOutput {
            response: shown.response,
            closed: shown.closed,
            result: shown.accepted.map(|options| {
                let options = finish(options, &untitled);
                let remember = remember_recent.filter(|_| !recent_is_bound).map(|key| {
                    let mut list = setup.recent.clone();
                    push_recent(&mut list, DocumentOptions { name: String::new(), ..options.clone() }, presets::MAX_RECENT);
                    (key, list)
                });
                NewDocumentResult { options, remember }
            }),
            requests: shown.requests,
            presets: events,
        }
    }
}

type Build = dyn for<'b> Fn(&'b mut DocumentOptions) -> NewDocumentDialog<'b> + Send;

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
pub struct HostedNewDocument {
    id: DialogId,
    options: DocumentOptions,
    build: Box<Build>,
    requests: HostRequests,
    /// Whether the dialog is open: bound to the dialog's `opened` prop.
    open: bool,
    /// The saved presets the app gave, as the user left them.
    saved: PresetLibrary<DocumentOptions>,
    /// The recent documents the app gave; the app adds the created one itself.
    recent: Vec<DocumentOptions>,
    /// The shown tab's id.
    tab: String,
    /// The saved-preset edits the app has not taken yet.
    events: Vec<NamedPresetEvent<String>>,
    /// The settings the dialog opened on, the clipboard's size and the default name filled in: what Reset restores,
    /// whatever automation sets before the first show.
    opening: DocumentOptions,
}

impl HostedNewDocument {
    /// A hosted dialog opening on `options`; `build` makes the dialog over them (the clipboard, the profiles, the
    /// app's name). The settings open at once (the clipboard's size, the default name from the root's words), so
    /// automation lists and sets them before the first pass.
    pub fn new(
        ctx: &egui::Context,
        mut options: DocumentOptions,
        build: impl for<'b> Fn(&'b mut DocumentOptions) -> NewDocumentDialog<'b> + Send + 'static,
    ) -> Self {
        let built = build(&mut options);
        let (clipboard, screen_ppi) = (built.clipboard, built.screen_resolution);
        drop(built);
        open_options(&mut options, clipboard, screen_ppi, Env::of_root(ctx).words::<NewDocumentWords>().untitled.as_str());
        HostedNewDocument {
            id: dialog_id("new-document"),
            opening: options.clone(),
            options,
            build: Box::new(build),
            requests: HostRequests::new(),
            open: true,
            saved: PresetLibrary::new(),
            recent: Vec::new(),
            tab: TabId::Recent.key(&[]),
            events: Vec::new(),
        }
    }

    /// The settings the details column shows.
    pub fn options(&self) -> &DocumentOptions {
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
    pub fn saved(mut self, library: PresetLibrary<DocumentOptions>) -> Self {
        self.saved = library;
        self
    }

    /// Opens with `list` as the recent documents.
    #[must_use]
    pub fn recent(mut self, list: Vec<DocumentOptions>) -> Self {
        self.recent = list;
        self
    }

    /// The saved presets as the user left them.
    pub fn saved_presets(&self) -> &PresetLibrary<DocumentOptions> {
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

impl StackDialog for HostedNewDocument {
    type Result = NewDocumentResult;

    /// Answers what Create answers. The recent list is bound, so the app adds the document itself (`remember` is
    /// empty, as in a pass).
    fn confirm(&mut self, ctx: &egui::Context) -> Option<Self::Result> {
        let words = Env::of_root(ctx).words::<NewDocumentWords>();
        Some(NewDocumentResult { options: finish(self.options.clone(), words.untitled.as_str()), remember: None })
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
    /// preset's name, which restores everything but the name as picking its card does.
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
                restore_all(&mut self.options, &item.value);
                Ok(())
            }
            _ => {
                set_serde_field(&mut self.options, field, value)?;
                self.options = self.options.clone().fitted();
                Ok(())
            }
        }
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<NewDocumentResult>> {
        let shown = (self.build)(&mut self.options)
            .opened(Bind::bound(&mut self.open))
            .saved(Bind::bound(&mut self.saved))
            .recent(Bind::bound(&mut self.recent))
            .tab(Bind::bound(&mut self.tab))
            .answers(self.requests.take_answers())
            .opening(self.opening.clone())
            .show(ui);
        self.requests.extend(shown.requests);
        self.events.extend(shown.presets);
        match (shown.result, shown.closed) {
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
}
