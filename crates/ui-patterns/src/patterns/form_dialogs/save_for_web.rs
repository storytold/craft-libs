//! `SaveForWebDialog`: the Save for Web dialog on `Dialog`, and `HostedSaveForWeb`, the same dialog owned by a
//! `DialogHost`. The app encodes: the dialog asks for the panes it shows with a revision, and the app answers each
//! with a [`WebVariant`].

use super::form::FormDialog;
use super::host_requests::{HostRequests, PreviewRequest};
use super::preview::PreviewView;
use super::size_model::LinkedSize;
use super::sizing::{Resample, SizingWords};
use crate::patterns::presets::PresetState;
use crate::runtime::requests::{HostAnswer, PathFilter, PathRequest};
use crate::sections::{DialogRole, FooterButton};

use crate::contract::{Bind, Held, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};

use crate::display::Text;
use crate::layout_components::{Row, Stack};
use crate::style::TypeRole;

use crate::runtime::env::{Env, Label};

use crate::runtime::memory::{Remember as _, SaveKey};

use crate::runtime::motion::Animate;
use crate::runtime::overlay::CloseReason;
use craft_behavior::{PixelSize, Selection};
use craft_keymap::{Chord, Key, Mods};

use craft_theme::{Rgba, variant::Tone};

use egui::load::SizedTexture;
use egui::{Response, Ui};

use serde::{Deserialize, Serialize};

use std::path::{Path, PathBuf};

mod options;
mod panes;

#[cfg(feature = "suite")]
pub(crate) mod stories;

pub use panes::{BrowserPreview, WebPane};

mod hosted;
mod model;

pub use hosted::HostedSaveForWeb;
pub use model::{
    Animation, ColorTable, ColorTableEdit, ColorTableEvent, Dither, OptimizeAnswer, OptimizeRequest, OptimizeScope, OptimizeStart, SaveForWebOptions,
    SaveForWebResult, TableMarks, TableOrder, WEB_SPEEDS, WantedVariant, WebAntiAlias, WebDocument, WebFormat, WebMetadata, WebPreset,
    WebPresetEvent, WebSlice, WebSlices, WebVariant, WebView,
};

/// The key of the matte colour request the dialog files.
pub const MATTE_COLOR_KEY: &str = "web.matte";

/// The key of the colour request the dialog files for an edited palette entry.
pub const PALETTE_COLOR_KEY: &str = "web.palette";

/// The key of the path request Save… files.
pub const SAVE_PATH_KEY: &str = "web.save";

/// The resample methods the size row lists: Photoshop's Save for Web offers these five.
const RESAMPLE_METHODS: [Resample; 5] =
    [Resample::NearestNeighbor, Resample::Bilinear, Resample::Bicubic, Resample::BicubicSmoother, Resample::BicubicSharper];

/// The Color Table list the dialog offers unless the host gives its own.
const COLOR_TABLES: [ColorTable; 7] = [
    ColorTable::Perceptual,
    ColorTable::Selective,
    ColorTable::Adaptive,
    ColorTable::Restrictive,
    ColorTable::Exact,
    ColorTable::BlackWhite,
    ColorTable::Grayscale,
];

/// The download speed the Download select opens at, in kilobits per second.
const OPENING_SPEED: f64 = WEB_SPEEDS[1];

/// The Desired File Size the Optimize rows open at, in K.
const DEFAULT_OPTIMIZE_K: f64 = 32.0;

/// The index `FormOutput::picked` gives Done: Save… is 0, Cancel 1, and the extra buttons start at 2.
const DONE_BUTTON: usize = 2;

/// The frame rate of an animation whose timeline gives none.
const DEFAULT_FPS: f32 = 30.0;

/// The longest the next pass of a playing animation waits, in seconds.
const LONGEST_WAIT: f64 = 1.0;

/// The widest percent the Percent row and the Percent unit allow.
const MAX_PERCENT: f64 = 1000.0;

/// The zoom range of the panes, as fractions of the picture's size.
const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.0001..=32.0;

craft_ui::words! {
    /// The words of Save for Web, beside the common OK and Cancel.
    pub struct SaveForWebWords {
        title: "saveForWeb.title" = "Save for Web (Legacy)";
        view: "saveForWeb.view" = "Preview";
        settings: "saveForWeb.settings" = "Settings";
        format: "saveForWeb.format" = "Format";
        zoom: "saveForWeb.zoom" = "Zoom";
        unnamed: "saveForWeb.unnamed" = "[Unnamed]";
        save: "saveForWeb.save" = "Save…";
        done: "saveForWeb.done" = "Done";
        original: "saveForWeb.original" = "Original";
        optimized: "saveForWeb.optimized" = "Optimized";
        two_up: "saveForWeb.twoUp" = "2-Up";
        four_up: "saveForWeb.fourUp" = "4-Up";
        preset: "saveForWeb.preset" = "Preset";
        new_preset_name: "saveForWeb.newPresetName" = "New Preset";
        color_table: "saveForWeb.colorTable" = "Color Table";
        perceptual: "saveForWeb.perceptual" = "Perceptual";
        selective: "saveForWeb.selective" = "Selective";
        adaptive: "saveForWeb.adaptive" = "Adaptive";
        restrictive: "saveForWeb.restrictive" = "Restrictive (Web)";
        exact: "saveForWeb.exact" = "Exact";
        black_white: "saveForWeb.blackWhite" = "Black-White";
        grayscale: "saveForWeb.grayscale" = "Grayscale";
        colors: "saveForWeb.colors" = "Colors";
        dither: "saveForWeb.dither" = "Dither";
        no_dither: "saveForWeb.noDither" = "No Dither";
        diffusion: "saveForWeb.diffusion" = "Diffusion";
        pattern: "saveForWeb.pattern" = "Pattern";
        noise: "saveForWeb.noise" = "Noise";
        amount: "saveForWeb.amount" = "Amount";
        transparency: "saveForWeb.transparency" = "Transparency";
        interlaced: "saveForWeb.interlaced" = "Interlaced";
        web_snap: "saveForWeb.webSnap" = "Web Snap";
        lossy: "saveForWeb.lossy" = "Lossy";
        quality: "saveForWeb.quality" = "Quality";
        progressive: "saveForWeb.progressive" = "Progressive";
        optimized_box: "saveForWeb.optimizedBox" = "Optimized";
        lossless: "saveForWeb.lossless" = "Lossless";
        embed_profile: "saveForWeb.embedProfile" = "Embed Color Profile";
        matte: "saveForWeb.matte" = "Matte";
        matte_hex: "saveForWeb.matteHex" = "Matte color";
        convert_srgb: "saveForWeb.convertSrgb" = "Convert to sRGB";
        metadata: "saveForWeb.metadata" = "Metadata";
        metadata_none: "saveForWeb.metadataNone" = "None";
        metadata_copyright: "saveForWeb.metadataCopyright" = "Copyright";
        metadata_contact: "saveForWeb.metadataContact" = "Copyright and Contact Info";
        metadata_all: "saveForWeb.metadataAll" = "All";
        image_size: "saveForWeb.imageSize" = "Image Size";
        width: "saveForWeb.width" = "Width";
        height: "saveForWeb.height" = "Height";
        constrain: "saveForWeb.constrainProportions" = "Constrain proportions";
        size_unit: "saveForWeb.sizeUnit" = "Unit";
        percent: "saveForWeb.percent" = "Percent";
        anti_alias: "saveForWeb.antiAlias" = "Anti-aliasing";
        anti_alias_none: "saveForWeb.antiAliasNone" = "None";
        anti_alias_art: "saveForWeb.antiAliasArt" = "Art Optimized";
        anti_alias_type: "saveForWeb.antiAliasType" = "Type Optimized";
        clip_to_artboard: "saveForWeb.clipToArtboard" = "Clip to Artboard";
        artboard: "saveForWeb.artboard" = "Artboard";
        output: "saveForWeb.output" = "Output";
        output_images: "saveForWeb.outputImages" = "Images";
        output_html: "saveForWeb.outputHtml" = "HTML and Images";
        fit: "saveForWeb.fit" = "Fit";
        actual: "saveForWeb.actual" = "100%";
        download_speed: "saveForWeb.downloadSpeed" = "Download";
        /// `{n}`: a speed under a megabit.
        speed_kbps: template "saveForWeb.speedKbps" = "{n} Kbps";
        /// `{n}`: a speed of a megabit or more.
        speed_mbps: template "saveForWeb.speedMbps" = "{n} Mbps";
        preview_browser: "saveForWeb.previewBrowser" = "Preview in Browser";
        /// `{modifier}`: the key held with the wheel to zoom (Ctrl, or Cmd on macOS).
        pan_hint: template "saveForWeb.panHint" = "Drag to pan · {modifier}+scroll to zoom · Double-click to fit";
        play: "saveForWeb.play" = "Play";
        pause: "saveForWeb.pause" = "Pause";
        /// `{frame}` of `{total}` animation frames.
        frame: template "saveForWeb.frame" = "{frame} / {total}";
        /// `{name}`, `{width}` and `{height}`: the Original pane's second caption line.
        original_caption: template "saveForWeb.originalCaption" = "{name}  ·  {width} × {height} px";
        /// `{format}` and `{size}`: an encoded pane's first caption line ("JPG  48.2K").
        encoded_caption: template "saveForWeb.encodedCaption" = "{format}  {size}";
        /// `{seconds}` and `{speed}`: an encoded pane's second caption line.
        download: template "saveForWeb.download" = "{seconds} sec @ {speed}";
        /// `{count}`: appended to the second line for indexed formats.
        color_count: template "saveForWeb.colorCount" = "  ·  {count} colors";
        /// Appended to the first line when the pane shows a reduced proxy.
        proxy: "saveForWeb.proxy" = "  ·  Proxy preview";
        /// `{name}` and `{number}`: the name of the 4-Up variant panes after the first.
        optimized_variant: template "saveForWeb.optimizedVariant" = "{name} {number}";
        /// `{pane}`, `{first}` and `{second}`: the accessible name of a pane.
        pane_name: template "saveForWeb.paneName" = "{pane}: {first}, {second}";
        /// The pane waits for the app's encode.
        pane_pending: "saveForWeb.panePending" = "Encoding";
        slices: "saveForWeb.slices" = "Slices";
        all_slices: "saveForWeb.allSlices" = "All Slices";
        user_slices: "saveForWeb.userSlices" = "All User Slices";
        selected_slices: "saveForWeb.selectedSlices" = "Selected Slices";
        save_html: "saveForWeb.saveHtml" = "Save HTML and Images";
        web_shift: "saveForWeb.webShift" = "Shift to Web Palette";
        unshift: "saveForWeb.unshift" = "Unshift Colors";
        lock: "saveForWeb.lock" = "Lock Colors";
        make_transparent: "saveForWeb.makeTransparent" = "Map Colors to Transparent";
        delete_color: "saveForWeb.deleteColor" = "Delete Color";
        table_menu: "saveForWeb.tableMenu" = "Color Table Menu";
        unshift_all: "saveForWeb.unshiftAll" = "Unshift All Colors";
        order_unsorted: "saveForWeb.orderUnsorted" = "Order: Unsorted";
        order_hue: "saveForWeb.orderHue" = "Order: Hue";
        order_luminance: "saveForWeb.orderLuminance" = "Order: Luminance";
        order_popularity: "saveForWeb.orderPopularity" = "Order: Popularity";
        table_failed: "saveForWeb.tableFailed" = "The color table is not available because the preview could not be encoded.";
        restore_table: "saveForWeb.restoreTable" = "Restore Generated Table";
        optimize_open: "saveForWeb.optimizeOpen" = "Optimize to File Size…";
        optimize_size: "saveForWeb.optimizeSize" = "Desired File Size (K)";
        optimize_start: "saveForWeb.optimizeStart" = "Start With";
        start_current: "saveForWeb.startCurrent" = "Current Settings";
        start_auto: "saveForWeb.startAuto" = "Auto Select GIF/JPEG";
        optimize_use: "saveForWeb.optimizeUse" = "Use";
        scope_each: "saveForWeb.scopeEach" = "Each Slice";
        scope_current: "saveForWeb.scopeCurrent" = "Current Slice";
        optimize_ok: "saveForWeb.optimizeOk" = "OK";
        optimize_cancel: "saveForWeb.optimizeCancel" = "Cancel Optimize";
        /// `{size}` and `{target}`: the size the search reached and the size asked for.
        optimize_reached: template "saveForWeb.optimizeReached" = "Optimized to {size} for a target of {target}";
        /// `{size}` and `{target}`: the closest size the search found and the size asked for.
        optimize_missed: template "saveForWeb.optimizeMissed" = "Closest found: {size} for a target of {target}";
    }
}

/// What the dialog remembers under its key: the last view and the last confirmed settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Last {
    view: WebView,
    options: SaveForWebOptions,
}

/// The state of the Optimize to File Size… rows.
#[derive(Clone, Default)]
struct OptimizeRows {
    /// The rows are open.
    open: bool,
    /// The Desired File Size in K; `None` until the host answers the opening size or the user types one.
    k: Option<f64>,
    start: OptimizeStart,
    scope: OptimizeScope,
    /// The host's last answer, for the result line.
    answer: Option<OptimizeAnswer>,
    /// Why the host refused the last search; the rows stay open with it.
    refused: Option<Label>,
}

impl OptimizeRows {
    /// Opens the rows; the host answers the opening size.
    fn open(&mut self) {
        self.open = true;
        self.k = None;
        self.refused = None;
    }

    /// Closes the rows and forgets the size they opened at.
    fn close(&mut self) {
        self.open = false;
        self.k = None;
        self.refused = None;
    }

    /// The Desired File Size in K the rows show.
    fn desired_k(&self) -> f64 {
        self.k.unwrap_or(DEFAULT_OPTIMIZE_K)
    }

    /// The search the rows ask for over `options`.
    fn request(&self, options: &SaveForWebOptions) -> OptimizeRequest {
        OptimizeRequest { desired_k: self.desired_k(), start: self.start, scope: self.scope, options: options.clone() }
    }

    /// The host's answer: found options replace the settings and the rows close; a refusal keeps them open with its reason.
    fn answered(&mut self, answer: &Result<OptimizeAnswer, Label>, options: &mut SaveForWebOptions) {
        match answer {
            Ok(found) => {
                self.close();
                if let Some(taken) = &found.options {
                    *options = taken.clone();
                }
                self.answer = Some(found.clone());
            }
            Err(reason) => self.refused = Some(reason.clone()),
        }
    }

    /// The host's opening size in bytes: the rows open at it unless the user already typed a size. An answer to rows
    /// that are closed opens them, which is how a story starts them open.
    fn started(&mut self, bytes: u64) {
        if self.k.is_none() {
            self.open = true;
            self.k = Some(bytes as f64 / 1024.0);
        }
    }
}

/// A value automation sets on the dialog's own state, handed in once through the host.
#[derive(Clone, Debug, PartialEq)]
enum WebInput {
    Speed(f64),
    /// `None` fits the pictures.
    Zoom(Option<f32>),
    /// The colours of the Color Table's cells to select.
    Selection(Vec<[u8; 3]>),
    OptimizeOpen,
    OptimizeK(f64),
    OptimizeStart(OptimizeStart),
    OptimizeUse(OptimizeScope),
    OptimizeGo,
}

/// What the dialog keeps in its root container's slot besides the settings.
#[derive(Clone)]
struct Live {
    started: bool,
    link: LinkedSize,
    view: WebView,
    shared: PreviewView,
    selected: WebPane,
    frame: u32,
    playing: bool,
    clock: f64,
    revision: u64,
    reported: u64,
    /// The selection of the Color Table's cells.
    table: Selection<usize>,
    /// The colour of the palette entry whose colour request is out.
    editing: Option<[u8; 3]>,
    /// The Download speed of the captions in Kbps; not remembered.
    speed: f64,
    /// The Preset row's picker.
    preset: PresetState<String>,
    optimize: OptimizeRows,
}

impl Default for Live {
    fn default() -> Self {
        Live {
            started: false,
            link: LinkedSize::new(PixelSize::new(1, 1)),
            view: WebView::default(),
            shared: PreviewView::fitting().zoom_range(ZOOM_RANGE),
            selected: WebPane::Encoded(0),
            frame: 0,
            playing: false,
            clock: 0.0,
            revision: 1,
            reported: 0,
            table: Selection::default(),
            editing: None,
            speed: OPENING_SPEED,
            preset: PresetState::new(),
            optimize: OptimizeRows::default(),
        }
    }
}

impl Live {
    /// The state of a dialog that opens now: the document `size` and, when saved, the last view.
    fn open(size: PixelSize, saved: Option<&Last>) -> Self {
        Live { started: true, link: LinkedSize::new(size), view: saved.map_or_else(WebView::default, |last| last.view), ..Live::default() }
    }

    /// Puts back the size from when the dialog opened and asks for new encodes.
    fn reset(&mut self) {
        self.link = LinkedSize::new(self.link.original());
        self.revision += 1;
    }

    /// Moves the playing animation on by the time that has passed and asks for the pass of the next frame.
    fn advance(&mut self, cx: &impl Animate, animation: Option<Animation>) {
        let Some(Animation { frames, fps }) = animation.filter(|animation| animation.frames >= 2) else { return };
        let now = cx.now();
        self.frame = self.frame.min(frames - 1);
        if !self.playing {
            self.clock = now;
            return;
        }
        let (frame, clock) = advance_frame(self.frame, frames, fps, self.clock, now);
        self.frame = frame;
        self.clock = clock;
        let rate = f64::from(frame_rate(fps));
        cx.wake_at(now + (1.0 / rate - (now - clock)).clamp(0.0, LONGEST_WAIT));
    }

    /// Applies what automation set; `palette` resolves a colour selection to the shown table's cells.
    fn apply(&mut self, inputs: &[WebInput], palette: Option<&[[u8; 3]]>, options: &SaveForWebOptions, asked: &mut options::Asked) {
        for input in inputs {
            match input {
                WebInput::Speed(kbps) => self.speed = *kbps,
                WebInput::Zoom(None) => self.shared = PreviewView::fitting().zoom_range(ZOOM_RANGE),
                WebInput::Zoom(Some(scale)) => self.shared = self.shared.zoomed_to(*scale),
                WebInput::Selection(colors) => {
                    let cells: Vec<usize> = palette
                        .map(|palette| colors.iter().filter_map(|color| palette.iter().position(|shown| shown == color)).collect())
                        .unwrap_or_default();
                    let last = cells.last().copied();
                    self.table = Selection::new(cells, last, last);
                }
                WebInput::OptimizeOpen => {
                    self.optimize.open();
                    asked.optimize_open = true;
                }
                WebInput::OptimizeK(k) => self.optimize.k = Some(*k),
                WebInput::OptimizeStart(start) => self.optimize.start = *start,
                WebInput::OptimizeUse(scope) => self.optimize.scope = *scope,
                WebInput::OptimizeGo => asked.optimize_request = Some(self.optimize.request(options)),
            }
        }
    }

    /// The colours of the selected cells of the shown table.
    fn selected_colors(&self, palette: Option<&[[u8; 3]]>) -> Vec<[u8; 3]> {
        let Some(palette) = palette else { return Vec::new() };
        self.table.items.iter().filter_map(|at| palette.get(*at).copied()).collect()
    }
}

/// What the dialog reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct SaveForWebOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open. A path answer closes the dialog as
    /// `CloseReason::Activated`.
    pub closed: Option<CloseReason>,
    /// The answer, in the pass the host answered the save request with a path or the user pressed Done (`path` is then
    /// `None`; the owner closes the dialog).
    pub result: Option<SaveForWebResult>,
    /// The panes to encode, when the settings, the size, the view or the frame changed: every pane the view shows,
    /// with the revision to answer with.
    pub wanted_variants: Vec<WantedVariant>,
    /// What the dialog asks of the host: the colour picker for the matte (keyed [`MATTE_COLOR_KEY`]) or for a
    /// palette entry (keyed [`PALETTE_COLOR_KEY`]) and, after Save…, the file dialog (keyed [`SAVE_PATH_KEY`]). The
    /// host answers with
    /// [`HostRequests::answer_color`] and [`HostRequests::answer_path`] and gives the answers back through
    /// [`SaveForWebDialog::answers`].
    pub requests: HostRequests,
    /// The settings after the pass.
    pub options: SaveForWebOptions,
    /// The output size after the pass.
    pub size: PixelSize,
    /// The frame the panes show; the app encodes this frame.
    pub frame: u32,
    /// The pane last clicked.
    pub selected: WebPane,
    /// This pass's colour table edits.
    pub table: Vec<ColorTableEvent>,
    /// This pass's preset row events.
    pub preset_events: Vec<WebPresetEvent>,
    /// Preview in Browser was pressed in this pass.
    pub browser_preview: bool,
    /// The Optimize rows opened in this pass: the host answers the opening size.
    pub optimize_open: bool,
    /// The search the Optimize rows filed in this pass.
    pub optimize_request: Option<OptimizeRequest>,
    /// While the Optimize rows are open: the Desired File Size in K, the start and the scope.
    pub optimize_state: Option<(f64, OptimizeStart, OptimizeScope)>,
    /// The Download speed of the captions, in Kbps.
    pub speed: f64,
    /// The panes' zoom as a fraction of the picture's size; `None` while they fit.
    pub zoom: Option<f32>,
    /// The colours of the selected Color Table cells.
    pub selection: Vec<[u8; 3]>,
}

impl Outcome for SaveForWebOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The picture the Original pane shows and the answers of the app for the encoded panes.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct WebPictures {
    /// The Original pane's picture, drawn at the document's size in pixels.
    pub original: Option<SizedTexture>,
    /// The encoded panes by index.
    pub variants: [Option<WebVariant>; 3],
}

impl WebPictures {
    /// No pictures yet: every pane waits.
    pub fn new() -> Self {
        Self::default()
    }

    /// The picture of the Original pane.
    #[must_use]
    pub fn original(mut self, original: SizedTexture) -> Self {
        self.original = Some(original);
        self
    }

    /// Takes `variant` for its pane; an index past the third pane is ignored.
    pub fn set(&mut self, variant: WebVariant) {
        if let Some(slot) = self.variants.get_mut(variant.index) {
            *slot = Some(variant);
        }
    }
}

/// The Save for Web dialog: the view tabs over a row of preview panes that share one view, the zoom row, the
/// animation row, and the settings column. It is a [`FormDialog`] over [`SaveForWebOptions`].
///
/// - Keys: Tab walks the controls, Enter is Save… unless a field owns it, Escape cancels. Holding Alt turns Cancel
///   into Reset, which restores the settings and the size from when the dialog opened. Ctrl+Z steps the dialog's own
///   settings. While the Optimize rows are open Save… cannot be accepted, so Enter is the rows' OK.
/// - Panes: a drag pans every pane, Ctrl with the wheel zooms about the pointer, a double click fits, a click selects
///   the pane. The zoom row steps through the preview's zoom levels, has Fit and 100%, the Download speed of the
///   captions and, when the host offers it, Preview in Browser. A click on a slice of the document selects it, and
///   Shift adds it to or removes it from the selection.
/// - Color Table: an indexed format whose shown pane carries a palette lists it in a multi-select table with the
///   host's [`TableMarks`]; the icon buttons, the table menu and the Delete key file [`ColorTableEvent`]s on the
///   selected colours, and opening an entry files a colour request keyed [`PALETTE_COLOR_KEY`], whose answer becomes a
///   `Shift` event. A change of format, colour count, palette method or preset files a `Restore` event. A host answer
///   applies in the pass that takes it; a second pass of the same frame sees none.
/// - Encoding is the app's: [`SaveForWebOutput::wanted_variants`] lists the panes to encode; the app answers with
///   [`WebVariant`]s in `pictures`, and the dialog drops none: the owner of the pictures drops stale ones
///   ([`HostedSaveForWeb::set_variant`]).
/// - Save… files a path request in [`SaveForWebOutput::requests`] (keyed [`SAVE_PATH_KEY`]); the dialog stays open
///   while the host's file dialog is up (`saving` disables Save…). The host's path answer, given through `answers`,
///   closes the dialog as `CloseReason::Activated` and makes `result` the answer. Done makes `result` the settings with
///   no path.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct SaveForWebDialog<'a> {
    name: String,
    size: PixelSize,
    pictures: WebPictures,
    options: Bind<'a, SaveForWebOptions>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// The Preset list.
    #[prop]
    presets: Vec<WebPreset>,
    /// The document is an animation: the play row shows and the frame advances while playing.
    #[prop]
    animation: Option<Animation>,
    /// Remembers the last view and, through [`SaveForWebResult::remember`], the last settings under `key`. A
    /// remembered set replaces the bound settings when the dialog opens.
    #[prop]
    remember: Option<SaveKey>,
    /// Whether the host's file dialog is open: Save… waits.
    #[prop]
    saving: bool,
    /// The host's answers to earlier requests: the matte colour and the path of the file to write.
    #[prop]
    answers: Vec<HostAnswer>,
    /// The document's slices, for the Slices choice.
    #[prop]
    slices: &'a [WebSlice],
    /// A notice from the host about slices omitted from the preview and export.
    #[prop]
    slice_warning: Option<Label>,
    /// Binds the view tab to `view`: the dialog shows the bound view and writes the tab the user picks, so the host
    /// and the dialog share one value.
    #[prop]
    view: Option<Bind<'a, WebView>>,
    /// Binds the animation frame the panes show to `frame`: the dialog shows the bound frame and writes the frame Play
    /// moves to, so the host and the dialog share one value.
    #[prop]
    frame: Option<Bind<'a, u32>>,
    /// Picks the variant: a raster or a vector document.
    #[prop]
    document: WebDocument,
    /// The formats the host encodes, in list order.
    #[prop]
    formats: &'a [WebFormat],
    /// The Color Table list the host offers.
    #[prop]
    color_tables: &'a [ColorTable],
    /// What the host's colour table says, for the cell marks.
    #[prop]
    marks: Option<&'a TableMarks>,
    /// Shows "Optimize to File Size…" under the Preset row.
    #[prop(flag)]
    optimize: bool,
    /// Whether the zoom row has Preview in Browser.
    #[prop]
    browser_preview: BrowserPreview,
    /// The Desired File Size in bytes the host answered the Optimize rows' opening with; the rows open at it.
    #[prop]
    optimize_start: Option<u64>,
    /// The host's answer to the Optimize search.
    #[prop]
    optimize_answer: Option<Result<OptimizeAnswer, Label>>,
    /// Drops the Optimize result line: the art changed.
    #[prop(flag)]
    optimize_forget: bool,
    /// The label of the confirming button; Save… unless the host names another.
    #[prop]
    ok_label: Option<Label>,
    /// What automation sets on the dialog's own state, applied once.
    inputs: Vec<WebInput>,
}

impl<'a> SaveForWebDialog<'a> {
    /// A dialog for the document `name` of `size` pixels with the app's `pictures`, editing the settings in `options`.
    pub fn new(name: impl Into<String>, size: PixelSize, pictures: WebPictures, options: Bind<'a, SaveForWebOptions>) -> Self {
        SaveForWebDialog {
            name: name.into(),
            size,
            pictures,
            options,
            open: None,
            presets: Vec::new(),
            animation: None,
            remember: None,
            saving: false,
            answers: Vec::new(),
            slices: &[],
            slice_warning: None,
            view: None,
            frame: None,
            document: WebDocument::default(),
            formats: &WebFormat::ALL,
            color_tables: &COLOR_TABLES,
            marks: None,
            optimize: false,
            browser_preview: BrowserPreview::default(),
            optimize_start: None,
            optimize_answer: None,
            optimize_forget: false,
            ok_label: None,
            inputs: Vec::new(),
        }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> SaveForWebOutput {
        let SaveForWebDialog {
            name,
            size,
            pictures,
            options,
            open,
            presets,
            animation,
            remember,
            saving,
            answers,
            slices,
            slice_warning,
            view,
            frame,
            document,
            formats,
            color_tables,
            marks,
            optimize,
            browser_preview,
            optimize_start,
            optimize_answer,
            optimize_forget,
            ok_label,
            inputs,
        } = self;
        let mut view_held = view.map(Held::new);
        let mut frame_held = frame.map(Held::new);
        let env = Env::of(ui);
        let words = env.words::<SaveForWebWords>();
        let sizing = env.words::<SizingWords>();
        let units = env.units();
        let hint = zoom_hint(&env, &words);

        // PLAN: `FormDialog::enter_confirms` is fixed before the body runs, so it cannot follow the Optimize rows' state,
        // which lives in the body's memory slot; the body's validity gives the same result: while the rows are open the
        // dialog cannot be accepted and Enter is the rows' OK.
        let mut form = FormDialog::new(words.title.clone(), options)
            .ok_label(ok_label.unwrap_or_else(|| words.save.clone()))
            .ok_enabled(!saving)
            .extra_buttons([FooterButton::new(words.done.clone(), DialogRole::Other)])
            .outer_gesture(true)
            .answers(answers)
            .on_reset(|cx| cx.memory::<Live>().reset())
            .accept_stays_open(true);
        if let Some(open) = open {
            form = form.opened(open);
        }

        let mut after = Live::default();
        let mut wanted = Vec::new();
        let mut chosen = None;
        let mut finished = SaveForWebOptions::default();
        let mut reported = options::Asked::default();
        let mut browser_pressed = false;
        let shown = form.show(ui, |ui, cx, options| {
            let mut live = cx.memory::<Live>().clone();
            let mut restored = false;
            let mut asked = options::Asked::default();
            if !live.started {
                let saved = remember.as_ref().and_then(|key| cx.saved::<Option<Last>>(key).clone());
                live = Live::open(size, saved.as_ref());
                if let Some(last) = &saved {
                    *options = last.options.without_document_fields();
                    restored = true;
                }
            }
            // A measuring run reads the view but never writes it, as `FormDialog` does for its own bind.
            let measuring = crate::runtime::measure::active(ui.ctx());
            if let Some(held) = view_held.as_mut()
                && let Some(bound) = held.read(&mut **cx)
            {
                if restored {
                    if !measuring {
                        held.write(&mut **cx, &bound, live.view, "view");
                    }
                } else {
                    live.view = bound;
                }
            }
            for answer in cx.take_answers() {
                match answer {
                    HostAnswer::Color { key, color } if key == MATTE_COLOR_KEY => {
                        let Rgba([r, g, b, _]) = color.to_rgba();
                        options.matte = [r, g, b];
                    }
                    HostAnswer::Color { key, color } if key == PALETTE_COLOR_KEY => {
                        if let Some(from) = live.editing.take() {
                            let Rgba([r, g, b, _]) = color.to_rgba();
                            asked.table.push(ColorTableEvent { edit: ColorTableEdit::Shift([r, g, b]), colors: vec![from] });
                        }
                    }
                    HostAnswer::Path { key, path } if key == SAVE_PATH_KEY => {
                        chosen = Some(path);
                        cx.request_close_as(CloseReason::Activated);
                    }
                    _ => {}
                }
            }
            let before = (options.clone(), live.link.pixels(), live.view, live.frame);
            options.apply_size(&mut live.link);
            // A frame the host set is a change from the one the last pass showed: the panes are encoded again.
            if let Some(bound) = frame_held.as_mut().and_then(|held| held.read(&mut **cx)) {
                live.frame = bound;
            }
            let frame_before = live.frame;
            live.advance(&**cx, animation);
            let view_before = live.view;

            if let Some(answer) = &optimize_answer {
                live.optimize.answered(answer, options);
            }
            if let Some(bytes) = optimize_start {
                live.optimize.started(bytes);
            }
            if optimize_forget {
                live.optimize.answer = None;
            }
            let palette = palette_of(&pictures, live.selected);
            live.apply(&inputs, palette, options, &mut asked);

            let mut slice_click = None;
            let failed = failed_of(&pictures, live.selected);
            Row::new().show(ui, |ui, _| {
                Stack::new().fill().id_salt("preview").show(ui, |ui, _| {
                    let reads = panes::Panes {
                        words: &words,
                        units: &units,
                        name: &name,
                        size,
                        pictures: &pictures,
                        animation,
                        hint: &hint,
                        options: &*options,
                        slices,
                        chosen: options.slices.selected(),
                        browser: &browser_preview,
                        speed: live.speed,
                    };
                    panes::show(ui, &reads, &mut live, &mut slice_click, &mut browser_pressed);
                    if let Some(warning) = &slice_warning {
                        Text::new(warning.clone()).role(TypeRole::Caption).tone(Tone::Warn).show(ui);
                    }
                });
                Stack::new().id_salt("settings").show(ui, |ui, _| {
                    let kit = options::Kit {
                        words: &words,
                        sizing: &sizing,
                        units: &units,
                        presets: &presets,
                        palette,
                        failed,
                        marks,
                        formats,
                        color_tables,
                        document: &document,
                        optimize,
                        has_slices: !slices.is_empty(),
                    };
                    options::show(ui, &kit, options, &mut live, &mut asked)
                });
            });
            if let Some(panes::SliceClick { number, add }) = slice_click {
                options.slices.select(number, add);
            }

            // A change of the palette's reduction drops the host's edits of the generated table.
            if options.reduction() != before.0.reduction() {
                asked.table.push(ColorTableEvent { edit: ColorTableEdit::Restore, colors: Vec::new() });
            }
            if (options.clone(), live.link.pixels(), live.view, live.frame) != before {
                live.revision += 1;
            }
            if live.frame != frame_before
                && !measuring
                && let Some(held) = frame_held.as_mut()
            {
                held.write(&mut **cx, &frame_before, live.frame, "frame");
            }
            if live.view != view_before {
                if !measuring && let Some(held) = view_held.as_mut() {
                    held.write(&mut **cx, &view_before, live.view, "view");
                }
                if let Some(key) = &remember {
                    let saved = cx.saved::<Option<Last>>(key);
                    let kept = saved.as_ref().map_or_else(|| options.without_document_fields(), |last| last.options.clone());
                    *saved = Some(Last { view: live.view, options: kept });
                }
            }
            if let Some(request) = asked.color.take() {
                cx.requests().request_color(request);
            }
            wanted = Self::wanted(&live, options);
            live.reported = live.revision;
            *cx.memory::<Live>() = live.clone();
            let accepts = !live.optimize.open;
            after = live;
            finished = options.clone();
            reported = asked;
            accepts
        });

        let live = after;
        let pixels = live.link.pixels();
        let mut requests = shown.requests;
        if shown.picked == Some(0) {
            requests.request_path(save_request(&name, finished.format));
        }
        let finish = if let (Some(accepted), Some(path)) = (shown.accepted, chosen) {
            Some((accepted, Some(path)))
        } else if shown.picked == Some(DONE_BUTTON) {
            Some((finished.clone(), None))
        } else {
            None
        };
        let result = finish.map(|(accepted, path)| result_of(accepted, path, pixels, live.view, remember));
        let palette = palette_of(&pictures, live.selected);
        SaveForWebOutput {
            response: shown.response,
            closed: shown.closed,
            result,
            wanted_variants: wanted,
            requests,
            options: finished,
            size: pixels,
            frame: live.frame,
            selected: live.selected,
            table: reported.table,
            preset_events: reported.presets,
            browser_preview: browser_pressed,
            optimize_open: reported.optimize_open,
            optimize_request: reported.optimize_request,
            optimize_state: live.optimize.open.then(|| (live.optimize.desired_k(), live.optimize.start, live.optimize.scope)),
            speed: live.speed,
            zoom: (!live.shared.fit).then_some(live.shared.transform.scale),
            selection: live.selected_colors(palette),
        }
    }
}

/// The answer of a confirmed dialog: the settings, the file (none for Done), the output size, and what the next open
/// remembers (the settings without the slices and the artboard, which belong to one document).
fn result_of(options: SaveForWebOptions, path: Option<PathBuf>, size: PixelSize, view: WebView, remember: Option<SaveKey>) -> SaveForWebResult {
    SaveForWebResult {
        options: options.clone(),
        path,
        width: size.width,
        height: size.height,
        remember: remember.map(|key| (key, Last { view, options: options.without_document_fields() })),
    }
}

/// The palette of the encoded pane `pane`; an Original pane has none.
fn palette_of(pictures: &WebPictures, pane: WebPane) -> Option<&[[u8; 3]]> {
    let WebPane::Encoded(index) = pane else { return None };
    pictures.variants.get(index)?.as_ref()?.palette.as_deref()
}

/// Whether the encode of the pane `pane` failed.
fn failed_of(pictures: &WebPictures, pane: WebPane) -> bool {
    let WebPane::Encoded(index) = pane else { return false };
    matches!(pictures.variants.get(index), Some(Some(WebVariant { picture: Err(_), .. })))
}

/// `color` as `#rrggbb`.
fn hex(color: [u8; 3]) -> String {
    let [r, g, b] = color;
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The colour `#rrggbb` names.
fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 || !digits.chars().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// The percent `size` is of `original`, rounded to two decimals.
fn percent_of(size: PixelSize, original: PixelSize) -> f64 {
    (f64::from(size.width) / f64::from(original.width.max(1)) * 10_000.0).round() / 100.0
}

/// The file dialog request for `name` in `format`: the document's name without its extension, with the format's.
fn save_request(name: &str, format: WebFormat) -> PathRequest {
    let stem = Path::new(name).file_stem().map_or_else(|| name.to_owned(), |stem| stem.to_string_lossy().into_owned());
    let extension = format.extension();
    PathRequest::save(SAVE_PATH_KEY, format!("{stem}.{extension}"), [PathFilter::new(format.name(), &[extension])])
}

/// The pan and zoom hint, naming the key the wheel zooms with on this platform.
fn zoom_hint(env: &Env, words: &SaveForWebWords) -> Label {
    let view = env.view();
    let mut keys = Chord::new(Mods::CMD, Key::Plus).parts_for(view.shell().os, view.keyboard_layout());
    keys.pop();
    Label::from(words.pan_hint.fill(&[("modifier", keys.join("+").as_str())]))
}

/// The frame rate to use: `fps`, or the default for one that is not a positive number.
fn frame_rate(fps: f32) -> f32 {
    if fps.is_finite() && fps > 0.0 { fps } else { DEFAULT_FPS }
}

/// `frame` after the time from `last` to `now`, and the clock to carry on from; the fraction of a frame that has not
/// yet passed is kept.
pub(crate) fn advance_frame(frame: u32, frames: u32, fps: f32, last: f64, now: f64) -> (u32, f64) {
    let frames = frames.max(1);
    let frame = frame.min(frames - 1);
    if !now.is_finite() {
        return (frame, last);
    }
    if !last.is_finite() || now < last {
        return (frame, now);
    }
    let fps = f64::from(frame_rate(fps));
    let elapsed = (now - last) * fps;
    if !elapsed.is_finite() {
        return (frame, now);
    }
    let steps = elapsed.floor();
    if steps < 1.0 {
        return (frame, last);
    }
    let delta = ((steps % f64::from(frames)) as u32) % frames;
    ((frame + delta) % frames, now - (elapsed - steps) / fps)
}

/// 4-Up's two extra panes: JPEG and lossy WebP at half and a quarter of the quality; GIF and PNG-8 at half and a
/// quarter of the colours; any other format as PNG-8 at 128 colours and GIF at 64 colours.
pub(crate) fn four_up_variants(settings: &SaveForWebOptions) -> [SaveForWebOptions; 2] {
    [2u16, 4].map(|divisor| {
        let mut variant = settings.clone();
        match settings.format {
            WebFormat::Jpeg | WebFormat::WebP if !(settings.format == WebFormat::WebP && settings.lossless) => {
                variant.quality = (f64::from(settings.quality) / f64::from(divisor)).round().max(1.0) as u8;
            }
            WebFormat::Gif | WebFormat::Png8 => {
                variant.colors = (f64::from(settings.colors) / f64::from(divisor)).round().max(2.0) as u16;
            }
            _ => {
                variant.format = if divisor == 2 { WebFormat::Png8 } else { WebFormat::Gif };
                variant.colors = 256 / divisor;
            }
        }
        variant
    })
}

/// Seconds to download `bytes` at `kbps` kilobits per second, rounded up.
pub(crate) fn download_seconds(bytes: u64, kbps: f64) -> u64 {
    (bytes as f64 * 8.0 / (kbps * 1000.0)).ceil() as u64
}

impl SaveForWebDialog<'_> {
    /// The panes to encode when the revision changed since the last report.
    fn wanted(live: &Live, options: &SaveForWebOptions) -> Vec<WantedVariant> {
        if live.revision == live.reported {
            return Vec::new();
        }
        let revision = live.revision;
        let mut wanted = Vec::new();
        if live.view != WebView::Original {
            wanted.push(WantedVariant { index: 0, request: PreviewRequest::new(revision, options.clone()) });
        }
        if live.view == WebView::FourUp {
            let [half, quarter] = four_up_variants(options);
            wanted.push(WantedVariant { index: 1, request: PreviewRequest::new(revision, half) });
            wanted.push(WantedVariant { index: 2, request: PreviewRequest::new(revision, quarter) });
        }
        wanted
    }
}
