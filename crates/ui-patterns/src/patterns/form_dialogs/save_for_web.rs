//! `SaveForWebDialog`: the Save for Web dialog on `Dialog`, and `HostedSaveForWeb`, the same dialog owned by a
//! `DialogHost`. The app encodes: the dialog asks for the panes it shows with a revision, and the app answers each
//! with a [`WebVariant`].

use super::form::FormDialog;
use super::host_requests::{HostRequests, PreviewRequest};
use super::preview::PreviewView;
use super::size_model::LinkedSize;
use super::sizing::{Resample, SizingWords};
use crate::runtime::requests::{HostAnswer, PathFilter, PathRequest};

use crate::contract::{Bind, NoValue, Outcome, Props as _, ShowContainer as _};

use crate::layout_components::{Row, Stack};

use crate::runtime::env::{Env, Label};

use crate::runtime::memory::{Remember as _, SaveKey};

use crate::runtime::motion::Animate;
use crate::runtime::overlay::CloseReason;
use craft_behavior::PixelSize;
use craft_keymap::{Chord, Key, Mods};

use craft_theme::Rgba;

use egui::load::SizedTexture;
use egui::{Response, Ui};

use serde::{Deserialize, Serialize};

use std::path::Path;

mod options;
mod panes;

#[cfg(feature = "testing")]
mod stories;

pub use panes::WebPane;

mod hosted;
mod model;

pub use hosted::HostedSaveForWeb;
pub use model::{
    Animation, ColorTable, Dither, SaveForWebOptions, SaveForWebResult, WantedVariant, WebFormat, WebMetadata, WebPreset, WebVariant, WebView,
};

/// The key of the matte colour request the dialog files.
pub const MATTE_COLOR_KEY: &str = "web.matte";

/// The key of the path request Save… files.
pub const SAVE_PATH_KEY: &str = "web.save";

/// The resample methods the size row lists: Photoshop's Save for Web offers these five.
const RESAMPLE_METHODS: [Resample; 5] =
    [Resample::NearestNeighbor, Resample::Bilinear, Resample::Bicubic, Resample::BicubicSmoother, Resample::BicubicSharper];

/// The download speed of the time estimate in the captions, in kilobits per second.
const MODEM_KBPS: f64 = 56.6;

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
        original: "saveForWeb.original" = "Original";
        optimized: "saveForWeb.optimized" = "Optimized";
        two_up: "saveForWeb.twoUp" = "2-Up";
        four_up: "saveForWeb.fourUp" = "4-Up";
        preset: "saveForWeb.preset" = "Preset";
        color_table: "saveForWeb.colorTable" = "Color Table";
        perceptual: "saveForWeb.perceptual" = "Perceptual";
        selective: "saveForWeb.selective" = "Selective";
        adaptive: "saveForWeb.adaptive" = "Adaptive";
        restrictive: "saveForWeb.restrictive" = "Restrictive (Web)";
        exact: "saveForWeb.exact" = "Exact";
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
        size_unit: "saveForWeb.sizeUnit" = "Unit";
        percent: "saveForWeb.percent" = "Percent";
        fit: "saveForWeb.fit" = "Fit";
        actual: "saveForWeb.actual" = "100%";
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
        /// `{seconds}`: an encoded pane's second caption line.
        download: template "saveForWeb.download" = "{seconds} sec @ 56.6 Kbps";
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
    }
}

/// What the dialog remembers under its key: the last view and the last confirmed settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Last {
    view: WebView,
    options: SaveForWebOptions,
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
    /// The answer, in the pass the host answered the save request with a path.
    pub result: Option<SaveForWebResult>,
    /// The panes to encode, when the settings, the size, the view or the frame changed: every pane the view shows,
    /// with the revision to answer with.
    pub wanted_variants: Vec<WantedVariant>,
    /// What the dialog asks of the host: the colour picker for the matte (keyed [`MATTE_COLOR_KEY`]) and, after
    /// Save…, the file dialog (keyed [`SAVE_PATH_KEY`]). The host answers with
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
///   settings.
/// - Panes: a drag pans every pane, Ctrl with the wheel zooms about the pointer, a double click fits, a click selects
///   the pane. The zoom row steps through the preview's zoom levels and has Fit and 100%.
/// - Encoding is the app's: [`SaveForWebOutput::wanted_variants`] lists the panes to encode; the app answers with
///   [`WebVariant`]s in `pictures`, and the dialog drops none: the owner of the pictures drops stale ones
///   ([`HostedSaveForWeb::set_variant`]).
/// - Save… files a path request in [`SaveForWebOutput::requests`] (keyed [`SAVE_PATH_KEY`]); the dialog stays open
///   while the host's file dialog is up (`saving` disables Save…). The host's path answer, given through `answers`,
///   closes the dialog as `CloseReason::Activated` and makes `result` the answer.
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
        }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> SaveForWebOutput {
        let SaveForWebDialog { name, size, pictures, options, open, presets, animation, remember, saving, answers } = self;
        let env = Env::of(ui);
        let words = env.words::<SaveForWebWords>();
        let sizing = env.words::<SizingWords>();
        let units = env.units();
        let hint = zoom_hint(&env, &words);

        let mut form = FormDialog::new(words.title.clone(), options)
            .ok_label(words.save.clone())
            .ok_enabled(!saving)
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
        let shown = form.show(ui, |ui, cx, options| {
            let mut live = cx.memory::<Live>().clone();
            if !live.started {
                let saved = remember.as_ref().and_then(|key| cx.saved::<Option<Last>>(key).clone());
                live = Live::open(size, saved.as_ref());
                if let Some(last) = &saved {
                    *options = SaveForWebOptions { percent: 100.0, ..last.options.clone() };
                }
            }
            for answer in cx.take_answers() {
                match answer {
                    HostAnswer::Color { key, color } if key == MATTE_COLOR_KEY => {
                        let Rgba([r, g, b, _]) = color.to_rgba();
                        options.matte = [r, g, b];
                    }
                    HostAnswer::Path { key, path } if key == SAVE_PATH_KEY => {
                        chosen = Some(path);
                        cx.request_close_as(CloseReason::Activated);
                    }
                    _ => {}
                }
            }
            let before = (options.clone(), live.link.pixels(), live.view, live.frame);
            live.advance(&**cx, animation);
            let view_before = live.view;

            let mut ask = None;
            Row::new().show(ui, |ui, _| {
                Stack::new().id_salt("preview").show(ui, |ui, _| {
                    let reads = panes::Panes {
                        words: &words,
                        units: &units,
                        name: &name,
                        size,
                        pictures: &pictures,
                        animation,
                        hint: &hint,
                        options: &*options,
                    };
                    panes::show(ui, &reads, &mut live)
                });
                Stack::new().id_salt("settings").show(ui, |ui, _| {
                    let kit = options::Kit { words: &words, sizing: &sizing, units: &units, presets: &presets };
                    options::show(ui, &kit, options, &mut live.link, &mut ask)
                });
            });

            if live.link.pixels() != before.1 {
                options.percent = percent_of(live.link.pixels(), live.link.original());
            }
            if (options.clone(), live.link.pixels(), live.view, live.frame) != before {
                live.revision += 1;
            }
            if let Some(key) = &remember
                && live.view != view_before
            {
                let saved = cx.saved::<Option<Last>>(key);
                let kept = saved.as_ref().map_or_else(|| options.clone(), |last| last.options.clone());
                *saved = Some(Last { view: live.view, options: kept });
            }
            if let Some(request) = ask {
                cx.requests().request_color(request);
            }
            wanted = Self::wanted(&live, options);
            live.reported = live.revision;
            *cx.memory::<Live>() = live.clone();
            after = live;
            finished = options.clone();
            true
        });

        let live = after;
        let pixels = live.link.pixels();
        let mut requests = shown.requests;
        if shown.picked == Some(0) {
            requests.request_path(save_request(&name, finished.format));
        }
        let result = shown.accepted.zip(chosen).map(|(accepted, path)| SaveForWebResult {
            options: accepted.clone(),
            path,
            width: pixels.width,
            height: pixels.height,
            remember: remember.map(|key| (key, Last { view: live.view, options: accepted })),
        });
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
        }
    }
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

/// Seconds to download `bytes` at `kbps` kilobits per second, rounded up: the caption uses 56.6, the modem estimate.
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
