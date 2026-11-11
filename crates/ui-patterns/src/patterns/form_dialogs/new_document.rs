//! `NewDocumentDialog`: File > New. Category tabs over a grid of preset cards, a details column, saved presets and the
//! document settings the app creates the document from.

mod cards;
mod details;
mod presets;
#[cfg(feature = "testing")]
mod stories;

use super::{FormCx, FormDialog, HostRequests, LinkedSize};
use crate::contract::{Bind, NoValue, Outcome, Remember as _, SaveKey, Scroll, ShowContainer as _};
use crate::runtime::requests::{ColorRequest, HostAnswer};

use crate::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};

use crate::layout_components::{Grid, Stack};

use crate::patterns::presets::{GroupPath, PresetLibrary, PresetWords as LibraryWords};

use crate::runtime::env::{Env, Label, UnitKind, Units};
use crate::runtime::memory::{MemoryStage, unique};
use crate::runtime::overlay::CloseReason;
use craft_behavior::{ColumnWidth, PixelSize};

use craft_theme::Srgba;
use egui::{Response, Ui};

use cards::{Left, TabId};

use details::Right;

pub(crate) use presets::push_recent;
pub use presets::{
    BackgroundContents, BitDepth, ColorMode, ColorProfiles, DocumentOptions, DocumentPreset, NewDocumentPresetWords, Orientation, PresetCategory,
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
        save_preset: "newDocument.savePreset" = "Save Preset…";
        preset_name: "newDocument.presetName" = "Preset Name";
        save: "newDocument.save" = "Save";
        no_saved: "newDocument.noSaved" = "No saved presets yet.";
        too_many: "newDocument.tooMany" = "At most 256 document presets can be saved";
        grayscale: "newDocument.grayscale" = "Grayscale";
        rgb_color: "newDocument.rgbColor" = "RGB Color";
        cmyk_color: "newDocument.cmykColor" = "CMYK Color";
        lab_color: "newDocument.labColor" = "Lab Color";
        depth_8: "newDocument.depth8" = "8 bit";
        depth_16: "newDocument.depth16" = "16 bit";
        depth_32: "newDocument.depth32" = "32 bit (float)";
        orientation: "newDocument.orientation" = "Orientation";
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

/// What the body reads besides its form: the setup, the words and the units.
#[derive(Clone, Copy)]
struct Shared<'a> {
    setup: &'a Setup,
    words: &'a NewDocumentWords,
    preset_words: &'a NewDocumentPresetWords,
    library_words: &'a LibraryWords,
    units: &'a Units,
    untitled: &'a str,
}

/// What the dialog keeps while it is open besides the settings: the fields that show the size, the open tab and card,
/// and the state of the Save Preset row. It lives in the dialog's body slot.
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
    /// The Save Preset name row, when open.
    naming: Option<String>,
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
            name_tried: false,
            focus_name: true,
            focus_preset_name: false,
        }
    }
}

impl Session {
    /// Drops what the user was doing with the settings: the picked card, the orientation, the Save Preset row and a
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
            let name = std::mem::take(&mut values.name);
            *values = DocumentOptions { name, ..card.settings.clone() };
        } else {
            values.size = card.settings.size;
            values.pixels_per_inch = card.settings.pixels_per_inch;
            values.unit = card.settings.unit;
        }
        self.size = values.linked(points_per_inch);
        self.gesture = None;
    }
}

/// The settings a Create answers with: the name trimmed, "Untitled" when empty.
fn finish(mut settings: DocumentOptions, untitled: &str) -> DocumentOptions {
    settings.name = settings.name.trim().to_owned();
    if settings.name.is_empty() {
        untitled.clone_into(&mut settings.name);
    }
    settings
}

/// Fills what the dialog adds on the pass it opens in: the clipboard's size (the dialog opens on its card) and the
/// default name.
fn open_values(values: &mut DocumentOptions, session: &mut Session, setup: &Setup, untitled: &str) {
    if let Some(size) = setup.clipboard {
        values.size = size;
        values.unit = UnitKind::Pixels;
        values.pixels_per_inch = setup.screen_ppi;
        session.card = Some(0);
    }
    if values.name.is_empty() {
        untitled.clone_into(&mut values.name);
    }
}

fn mode_label(words: &NewDocumentWords, mode: ColorMode) -> &Label {
    match mode {
        ColorMode::Grayscale => &words.grayscale,
        ColorMode::Rgb => &words.rgb_color,
        ColorMode::Cmyk => &words.cmyk_color,
        ColorMode::Lab => &words.lab_color,
    }
}

fn depth_label(words: &NewDocumentWords, depth: BitDepth) -> &Label {
    match depth {
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
    pub fn remember(&self, ui: &Ui) {
        if let Some((key, list)) = &self.remember {
            let mut stage = MemoryStage::open(ui.ctx(), unique(("craft_ui::new_document_recent", key.as_str())));
            *stage.saved::<Vec<DocumentOptions>>(key) = list.clone();
            stage.write_back();
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
    let Shared { setup, words, preset_words, library_words, units, untitled } = *shared;
    let points = units.points_per_inch();
    let mut session = form.memory::<Session>().clone();
    if form.first() {
        open_values(values, &mut session, setup, untitled);
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
    let recents: Vec<DocumentOptions> = setup.recent.iter().filter(|recent| recent.fits()).cloned().collect();
    let mut library = form.saved::<PresetLibrary<DocumentOptions>>(&PRESETS_KEY).clone();
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
        if library.delete(id).is_ok() {
            changed = true;
            session.card = None;
        }
    }
    if right.save {
        let name = session.naming.clone().unwrap_or_default();
        if details::refusal(&library, &name).is_none() {
            let preset = DocumentOptions { name: name.trim().to_owned(), ..values.clone() };
            if library.add(&GroupPath::root(), &name, preset).is_ok() {
                changed = true;
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
        *form.saved::<PresetLibrary<DocumentOptions>>(&PRESETS_KEY) = library;
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
///   the orientation control. Saved and recent cards restore every setting but the name.
/// - Details: Name, Width and Height with their units, Orientation, Artboards, Resolution, Color Mode, Depth, Color
///   Profile and Background Contents. Save Preset opens a name row under the column; Enter there saves the preset and
///   does not create the document. The name is checked by the preset library (not blank, different from the saved
///   names ignoring case), and at most 256 presets are kept.
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
    /// The application's name, for the "Default {app} Size" card.
    #[prop(into)]
    app_name: Option<Label>,
    /// Keeps the dialog's place and size across runs under `key`.
    #[prop]
    remember: Option<SaveKey>,
    /// The host's answers to the requests of earlier passes.
    #[prop]
    answers: Vec<HostAnswer>,
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
            app_name: None,
            remember: None,
            answers: Vec::new(),
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
            app_name,
            remember,
            answers,
        } = self;
        let recent: Vec<DocumentOptions> = remember_recent.as_ref().map_or_else(Vec::new, |key| MemoryStage::peek_saved(ui.ctx(), key));
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
        };
        let untitled = words.untitled.as_str().to_owned();
        let shared =
            Shared { setup: &setup, words: &words, preset_words: &preset_words, library_words: &library_words, units: &units, untitled: &untitled };

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
        if let Some(key) = remember {
            dialog = dialog.shaped(move |dialog| dialog.remember(key));
        }
        let shown = dialog.show(ui, |ui, form, values| body(ui, form, values, &shared));
        NewDocumentOutput {
            response: shown.response,
            closed: shown.closed,
            result: shown.accepted.map(|options| {
                let options = finish(options, &untitled);
                let remember = remember_recent.map(|key| {
                    let mut list = setup.recent.clone();
                    push_recent(&mut list, DocumentOptions { name: String::new(), ..options.clone() }, presets::MAX_RECENT);
                    (key, list)
                });
                NewDocumentResult { options, remember }
            }),
            requests: shown.requests,
        }
    }
}

type Build = dyn for<'b> Fn(&'b mut DocumentOptions) -> NewDocumentDialog<'b> + Send;

/// A [`NewDocumentDialog`] the [`DialogHost`](crate::dialog::DialogHost) owns: the host keeps the settings and the colour request, and `build` makes the dialog over the settings each pass.
///
/// ```ignore
/// host.open(HostedNewDocument::new(last_created, |options| {
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
}

impl HostedNewDocument {
    /// A hosted dialog opening on `options`; `build` makes the dialog over them (the clipboard, the profiles, the
    /// app's name).
    pub fn new(options: DocumentOptions, build: impl for<'b> Fn(&'b mut DocumentOptions) -> NewDocumentDialog<'b> + Send + 'static) -> Self {
        HostedNewDocument { id: dialog_id("new-document"), options, build: Box::new(build), requests: HostRequests::new() }
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

    /// The picker was cancelled: the request is dropped and Background Contents stays as it was.
    pub fn cancel_color(&mut self) {
        self.requests.cancel_color();
    }
}

impl StackDialog for HostedNewDocument {
    type Result = NewDocumentResult;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<NewDocumentResult>> {
        let shown = (self.build)(&mut self.options).answers(self.requests.take_answers()).show(ui);
        self.requests.extend(shown.requests);
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
