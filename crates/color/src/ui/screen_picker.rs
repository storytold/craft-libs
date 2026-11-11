//! Screen colour picking (F04.05): one pick at a time, owned by the key that asked, with its state in an app-owned
//! [`ScreenPick`]. The press-and-drag from the picker's button inside the app's windows samples the app's last frame
//! through the runtime's screenshot slot; a click, or a release outside the app's windows, captures the screen
//! ([`craft_platform::screen`]). A release outside takes the colour under it; after a click the capture shows over
//! each monitor ([`ScreenColorPicker`]) and a click there takes the colour under it.
//!
//! ```ignore
//! let mut pick = ScreenPick::new();
//! // In the pass that got the picker's `ScreenPickGesture`:
//! if let Some(color) = pick.hold(ctx, "fill", gesture) { /* live sample while the button is held */ }
//! // Once per pass, after the app's UI:
//! pick.tick(ctx, window_generation);
//! ScreenColorPicker::new(&mut pick, "PhotoCraft").show(ui);
//! if let Some(color) = pick.take("fill") { /* the picked colour */ }
//! if let Some(toast) = pick.take_notice(&words) { /* a capture failure */ }
//! ```

use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use craft_cms::{Builtin, Intent, Profile, Transform};
use craft_host::Wake;
use craft_host::target::Caps;
use craft_platform::screen::{self, Capture, CaptureHandle, ScreenError, ScreenImage};
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use craft_ui::contract::{Pattern, Props as _, Show as _, ShowContainer as _};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::toast::Toast;
use craft_ui::runtime::windows::{ScreenshotSlot, WindowHost};
use egui::{Color32, Context, Pos2, Rect, Ui, ViewportId, pos2, vec2};

use super::ScreenPickGesture;

mod capture;
#[cfg(feature = "screen-testing")]
mod stories;

use capture::{CaptureView, CaptureWindow};

craft_ui::words! {
    /// The screen picker's words, English by default.
    #[non_exhaustive]
    pub struct ScreenPickerWords {
        window_title: template "screenPicker.windowTitle" = "{app} — Screen Color Picker";
        click_to_pick: "screenPicker.clickToPick" = "Click to pick · Esc to cancel";
        scroll_to_zoom: "screenPicker.scrollToZoom" = "Scroll to zoom";
        denied: "screenPicker.denied" = "Screen capture was not allowed. Allow screen recording for this app in the system settings.";
        unsupported: "screenPicker.unsupported" = "Screen capture is not available here.";
        invalid_capture: "screenPicker.invalidCapture" = "The screen capture is not valid.";
        texture_limit: "screenPicker.textureLimit" = "This display is larger than the graphics card can show.";
        not_rgb_profile: "screenPicker.notRgbProfile" = "The screen profile must describe RGB pixels.";
        nothing_captured: "screenPicker.nothingCaptured" = "No screen was captured.";
        failed: template "screenPicker.failed" = "Screen capture failed: {reason}";
    }
}

/// The loupe shows this many pixels on each side (odd, so one is in the middle).
pub const LOUPE_PIXELS: usize = 11;

/// The loupe's zoom in screen pixels per pixel: the least, the first and the most; the wheel steps between them.
pub const LOUPE_ZOOM: (usize, usize, usize) = (4, 12, 24);

/// How far one wheel notch steps the loupe's zoom.
const ZOOM_STEP: usize = 2;

/// The most bytes one frame of the app's own windows may take when sampled during a held drag.
const FRAME_BYTES: usize = 256 * 1024 * 1024;

/// What a capture ends in.
pub type CaptureResult = Result<Capture, ScreenError>;

/// Where a capture's result comes from; the platform capture in the app, a fixed one in stories and tests.
pub trait CaptureSource: Send {
    /// The result, once, when it is ready.
    fn poll(&self) -> Option<CaptureResult>;
}

impl CaptureSource for CaptureHandle {
    fn poll(&self) -> Option<CaptureResult> {
        self.try_take()
    }
}

/// A capture that is ready at once: a result given in advance.
pub struct StillCapture(Mutex<Option<CaptureResult>>);

impl StillCapture {
    /// A capture that ends in `result`.
    pub fn new(result: CaptureResult) -> StillCapture {
        StillCapture(Mutex::new(Some(result)))
    }

    /// A capture that picks `color` at once, with no capture windows (a platform that picks natively).
    pub fn color(color: Srgba) -> StillCapture {
        StillCapture::new(Ok(Capture::Color([color.r, color.g, color.b])))
    }
}

impl CaptureSource for StillCapture {
    fn poll(&self) -> Option<CaptureResult> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

/// Why a pick ended without a colour; worded when the app takes the notice.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Failure {
    NothingCaptured,
    InvalidCapture,
    NotRgb,
    Denied,
    Unsupported,
    Failed(String),
}

/// Why a captured image cannot be shown.
enum SnapshotError {
    Invalid,
    NotRgb,
    Profile(String),
}

/// One monitor's capture with the conversion from its profile to sRGB.
#[derive(Clone)]
pub(super) struct Snapshot {
    image: ScreenImage,
    conversion: Option<Arc<Transform>>,
}

impl Snapshot {
    fn new(image: ScreenImage) -> Result<Snapshot, SnapshotError> {
        let [w, h] = image.size.map(|n| n as usize);
        let pixels = w.checked_mul(h).filter(|&n| n > 0 && n <= screen::MAX_PIXELS);
        if pixels.and_then(|n| n.checked_mul(4)) != Some(image.rgba.len()) || !image.scale.is_finite() || image.scale <= 0.0 {
            return Err(SnapshotError::Invalid);
        }
        let conversion = match &image.icc_profile {
            None => None,
            Some(bytes) => {
                let source = Profile::parse(bytes).map_err(|e| SnapshotError::Profile(e.to_string()))?;
                if source.color_space != craft_cms::ColorSpace::Rgb {
                    return Err(SnapshotError::NotRgb);
                }
                Some(Arc::new(
                    Transform::new(&source, Builtin::Srgb.profile(), Intent::RelativeColorimetric, true)
                        .map_err(|e| SnapshotError::Profile(e.to_string()))?,
                ))
            }
        };
        Ok(Snapshot { image, conversion })
    }

    /// The pixel `x`, `y` in 8-bit RGB.
    pub(super) fn pixel(&self, x: usize, y: usize) -> Option<Color32> {
        let [w, h] = self.image.size.map(|n| n as usize);
        if x >= w || y >= h {
            return None;
        }
        let at = (y * w + x) * 4;
        let p = self.image.rgba.get(at..at + 3)?;
        Some(Color32::from_rgb(p[0], p[1], p[2]))
    }

    /// The pixel `x`, `y` converted from the monitor's profile to sRGB.
    pub(super) fn sample(&self, x: usize, y: usize) -> Result<Srgba, String> {
        let p = self.pixel(x, y).ok_or("The pixel is outside the capture")?;
        let mut rgb = [p.r(), p.g(), p.b()].map(|v| f32::from(v) / 255.0);
        if let Some(t) = &self.conversion {
            t.apply(&mut rgb, 3).map_err(|e| e.to_string())?;
        }
        Ok(Srgba::from_rgb_array(rgb.map(|v| v.clamp(0.0, 1.0))))
    }
}

/// The app's own frame, grabbed through the screenshot slot while the button is held.
struct Hold {
    owner: u64,
    slot: Option<ScreenshotSlot>,
    frame: Option<Arc<egui::ColorImage>>,
}

/// The state of the screen picker. The app keeps one, and ticks and [`show`](Self::show)s it once per pass after its UI.
///
/// - One pick runs at a time, owned by the key that asked: only the owner [`take`](Self::take)s its colour, even
///   after the popover that asked has closed.
/// - [`tick`](Self::tick) starts a requested capture and settles a finished one; [`cancel`](Self::cancel) ends a
///   running pick.
/// - After a click on the picker's button the capture of each monitor shows in a window over that monitor until the
///   pick ends.
#[derive(Default)]
pub struct ScreenPick {
    request: Option<u64>,
    owner: Option<u64>,
    source: Option<Box<dyn CaptureSource>>,
    images: Vec<Snapshot>,
    answer: Option<Result<Option<Srgba>, Failure>>,
    completed: Option<(u64, Srgba)>,
    notice: Option<Failure>,
    unsupported: bool,
    target: Option<u64>,
    hold: Option<Hold>,
    monitor: Option<(u64, Option<Arc<Transform>>)>,
    /// Where a drag released outside the app's windows, in physical pixels of the virtual desktop: the captured
    /// monitor under it gives the colour.
    release: Option<Pos2>,
    /// The loupe's zoom in the capture windows, in screen pixels per captured pixel.
    zoom: usize,
    /// Counts the picks, so each pick's capture windows are new windows.
    generation: u64,
}

fn key_of(owner: impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    owner.hash(&mut hasher);
    hasher.finish()
}

/// The capture windows of a running pick, one over each captured monitor. Show it once per pass, after the app's UI.
#[must_use = "show the windows with `show`"]
#[derive(Pattern)]
pub struct ScreenColorPicker<'a> {
    pick: &'a mut ScreenPick,
    app: &'a str,
}

impl<'a> ScreenColorPicker<'a> {
    /// The windows of `pick`; `app` is the app's name, for the windows' title.
    pub fn new(pick: &'a mut ScreenPick, app: &'a str) -> Self {
        ScreenColorPicker { pick, app }
    }

    /// Shows the capture windows of `pick`'s running pick, one over each captured monitor, borderless and above every other
    /// window; `app` is the app's name, for the windows' title. A click picks the pixel under it, as a release outside
    /// the app's windows does; Escape, a right-click or closing a window cancels. The wheel steps the loupe's zoom
    /// ([`LOUPE_ZOOM`]). Every window closes when the pick ends.
    pub fn show(self, ui: &mut Ui) {
        let ScreenColorPicker { pick, app } = self;
        if pick.images.is_empty() || pick.answer.is_some() {
            return;
        }
        let words = Env::of(ui).words::<ScreenPickerWords>();
        let title = Label::from(words.window_title.fill(&[("app", app)]));
        let hints = [words.click_to_pick.clone(), words.scroll_to_zoom.clone()];
        let (mut picked, mut cancelled, mut notches) = (None, false, 0);
        for (n, snapshot) in pick.images.iter().enumerate() {
            let [x, y, w, h] = capture::area(snapshot).cover();
            let key = key_of((pick.generation, n));
            let window = CaptureWindow::new(title.clone(), Rect::from_min_size(pos2(x, y), vec2(w, h))).id_salt((
                "craft-color screen pick",
                pick.generation,
                n,
            ));
            let mut shown = window.show(ui, |ui, _| {
                let rect = ui.max_rect();
                CaptureView::new(snapshot.image.name.clone(), snapshot, key, pick.zoom, hints.clone()).show_at(ui, rect)
            });
            cancelled |= shown.output.closed.is_some();
            if let Some((at, right_click, wheel)) = shown.settle(|_, view| (view.picked, view.cancelled, view.notches)) {
                picked = picked.or(at);
                cancelled |= right_click;
                notches += wheel;
            }
        }
        let step = notches.unsigned_abs() as usize * ZOOM_STEP;
        pick.zoom = if notches > 0 { pick.zoom + step } else { pick.zoom.saturating_sub(step) }.clamp(LOUPE_ZOOM.0, LOUPE_ZOOM.2);
        if let Some(at) = picked {
            pick.answer = Some(sample_desktop(&pick.images, at).map_err(Failure::Failed));
        } else if cancelled {
            pick.answer = Some(Ok(None));
        }
        pick.settle(ui.ctx());
    }
}

impl ScreenPick {
    /// No pick.
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks for a pick for `owner`; nothing happens while another pick runs.
    pub fn start(&mut self, owner: impl Hash) {
        if self.owner.is_none() && self.request.is_none() {
            self.request = Some(key_of(owner));
        }
    }

    /// Runs the pick for one pass: starts the requested capture through the platform, waking the app when it
    /// finishes, and settles the answer. `target` identifies the window the pick belongs to: a pick whose target
    /// changed is cancelled.
    pub fn tick(&mut self, ctx: &Context, target: u64) {
        let scale = ctx.native_pixels_per_point().unwrap_or(1.0);
        let repaint = ctx.clone();
        self.tick_with(ctx, target, &|| {
            let repaint = repaint.clone();
            Box::new(screen::capture(scale, Wake::new(move || repaint.request_repaint())))
        });
    }

    /// [`tick`](Self::tick) with the capture `start` makes, for stories and tests.
    pub fn tick_with(&mut self, ctx: &Context, target: u64, start: &dyn Fn() -> Box<dyn CaptureSource>) {
        if self.completed.is_some() && self.target != Some(target) {
            self.completed = None;
        }
        if let Some(owner) = self.request.take() {
            self.completed = None;
            self.notice = None;
            self.source = Some(start());
            self.owner = Some(owner);
            self.target = Some(target);
            self.zoom = LOUPE_ZOOM.1;
            self.generation = self.generation.wrapping_add(1);
        }
        if self.owner.is_none() {
            return;
        }
        if self.target != Some(target) {
            self.answer = Some(Ok(None));
        }
        if self.answer.is_none()
            && let Some(result) = self.source.as_ref().and_then(|source| source.poll())
        {
            self.source = None;
            self.answer = Some(match result {
                Ok(Capture::Color([r, g, b])) => Ok(Some(Srgba::rgb(r, g, b))),
                Ok(Capture::Images(images)) => match images.into_iter().map(Snapshot::new).collect::<Result<Vec<_>, _>>() {
                    Ok(images) if !images.is_empty() => match self.release.take() {
                        Some(at) => sample_desktop(&images, at).map_err(Failure::Failed),
                        None => {
                            self.images = images;
                            return;
                        }
                    },
                    Ok(_) => Err(Failure::NothingCaptured),
                    Err(SnapshotError::Invalid) => Err(Failure::InvalidCapture),
                    Err(SnapshotError::NotRgb) => Err(Failure::NotRgb),
                    Err(SnapshotError::Profile(reason)) => Err(Failure::Failed(reason)),
                },
                Err(ScreenError::Cancelled) => Ok(None),
                Err(ScreenError::Denied) => Err(Failure::Denied),
                Err(ScreenError::Unsupported) => {
                    self.unsupported = true;
                    Err(Failure::Unsupported)
                }
                Err(ScreenError::Failed(reason)) => Err(Failure::Failed(reason)),
            });
        }
        self.settle(ctx);
    }

    /// Ends the pick with the answer it has.
    fn settle(&mut self, ctx: &Context) {
        let Some(answer) = self.answer.take() else { return };
        let owner = self.owner.take();
        self.source = None;
        self.release = None;
        if !self.images.is_empty() {
            capture::store(ctx).clear(ctx);
        }
        self.images.clear();
        match answer {
            Ok(Some(color)) => self.completed = owner.map(|owner| (owner, color)),
            Ok(None) => {}
            Err(notice) => self.notice = Some(notice),
        }
        ctx.request_repaint();
    }

    /// Whether a pick runs or waits for its capture; the picker's button is disabled meanwhile.
    pub fn busy(&self) -> bool {
        self.owner.is_some() || self.request.is_some()
    }

    /// Whether the platform can capture the screen (`caps` are the shell's) and has not said it cannot.
    pub fn available(&self, caps: Caps) -> bool {
        caps.contains(Caps::SCREEN_CAPTURE) && !self.unsupported
    }

    /// Cancels a running or requested pick: Esc, a closed capture window and a right-click all end here.
    pub fn cancel(&mut self) {
        if self.owner.is_some() {
            self.answer = Some(Ok(None));
        } else {
            self.request = None;
            self.release = None;
        }
    }

    /// The colour `owner` picked, once. Only the owner takes it.
    pub fn take(&mut self, owner: impl Hash) -> Option<Srgba> {
        let key = key_of(owner);
        if self.completed.as_ref().is_some_and(|(id, _)| *id == key) { self.completed.take().map(|(_, color)| color) } else { None }
    }

    /// A capture failure, once, as a toast worded with `words`.
    pub fn take_notice(&mut self, words: &ScreenPickerWords) -> Option<Toast> {
        let text = match self.notice.take()? {
            Failure::NothingCaptured => words.nothing_captured.clone(),
            Failure::InvalidCapture => words.invalid_capture.clone(),
            Failure::NotRgb => words.not_rgb_profile.clone(),
            Failure::Denied => words.denied.clone(),
            Failure::Unsupported => words.unsupported.clone(),
            Failure::Failed(reason) => Label::from(words.failed.fill(&[("reason", reason.as_str())])),
        };
        Some(Toast::new(text).tone(Tone::Bad))
    }

    /// The profile of the monitor the app's own windows show on: a frame sampled during a held drag converts from it
    /// to sRGB.
    pub fn set_monitor(&mut self, monitor: &Profile) {
        let hash = monitor.content_hash();
        if self.monitor.as_ref().is_some_and(|(known, _)| *known == hash) {
            return;
        }
        let back = if monitor.same_colors(Builtin::Srgb.profile()) {
            None
        } else {
            Transform::new(monitor, Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).ok().map(Arc::new)
        };
        self.monitor = Some((hash, back));
    }

    /// Follows the picker button's press for `owner`: a click starts the capture; a held drag inside the app's windows
    /// samples the app's last frame each pass (the colour returned); a release inside takes the colour under it, and
    /// a release outside the app's windows starts the capture.
    pub fn hold(&mut self, ctx: &Context, owner: impl Hash, gesture: ScreenPickGesture) -> Option<Srgba> {
        let key = key_of(&owner);
        if self.busy() {
            self.hold = None;
            return None;
        }
        let (at, released) = match gesture {
            ScreenPickGesture::Held { at } => (at, false),
            ScreenPickGesture::Released { at } => (at, true),
            _ => {
                self.hold = None;
                self.start(owner);
                return None;
            }
        };
        let windows = WindowHost::of(ctx);
        let size = windows.screenshot_facts();
        let inside = at.x >= 0.0 && at.y >= 0.0 && at.x < size.size_points.x && at.y < size.size_points.y;
        let back = self.monitor.as_ref().and_then(|(_, back)| back.clone());
        let hold = self.hold.get_or_insert_with(|| Hold { owner: key, slot: None, frame: None });
        if hold.owner != key {
            *hold = Hold { owner: key, slot: None, frame: None };
        }
        if let Some(Ok(frame)) = hold.slot.as_ref().and_then(ScreenshotSlot::take) {
            hold.frame = Some(frame);
            hold.slot = None;
        }
        let sampled = hold.frame.as_ref().filter(|_| inside).and_then(|frame| sample_frame(frame, size.pixels_per_point, at, back.as_deref()));
        if !released {
            if hold.slot.is_none() {
                hold.slot = Some(windows.screenshot(ViewportId::ROOT, FRAME_BYTES));
            }
            ctx.request_repaint();
            return sampled;
        }
        self.hold = None;
        if !inside {
            // A window with no reported position (a host with no desktop) sits at the desktop's origin.
            let window = ctx.input(|input| input.viewport().inner_rect).map_or(Pos2::ZERO, |inner| inner.min);
            self.start(owner);
            if self.request.is_some() {
                let native = size.pixels_per_point / size.zoom;
                self.release = Some(Pos2::new(
                    (window.x * native + at.x * size.pixels_per_point).round(),
                    (window.y * native + at.y * size.pixels_per_point).round(),
                ));
            }
        }
        sampled
    }
}

/// The colour under the desktop pixel `at` (physical pixels): the pixel of the monitor that holds it, converted from
/// the monitor's profile to sRGB. `Ok(None)` when no captured monitor holds it.
fn sample_desktop(images: &[Snapshot], at: Pos2) -> Result<Option<Srgba>, String> {
    for snapshot in images {
        let [w, h] = snapshot.image.size.map(|n| n as f32);
        let (dx, dy) = (at.x - snapshot.image.origin[0] as f32, at.y - snapshot.image.origin[1] as f32);
        if dx >= 0.0 && dy >= 0.0 && dx < w && dy < h {
            return snapshot.sample(dx as usize, dy as usize).map(Some);
        }
    }
    Ok(None)
}

/// The colour of `image` under `at` (points of the viewport), converted with `back` when the monitor is not sRGB.
fn sample_frame(image: &egui::ColorImage, pixels_per_point: f32, at: Pos2, back: Option<&Transform>) -> Option<Srgba> {
    let [w, h] = image.size;
    let (x, y) = ((at.x * pixels_per_point) as usize, (at.y * pixels_per_point) as usize);
    if x >= w || y >= h {
        return None;
    }
    let c = image.pixels.get(y * w + x)?;
    let mut rgb = [c.r(), c.g(), c.b()].map(|v| f32::from(v) / 255.0);
    if let Some(back) = back {
        back.apply(&mut rgb, 3).ok()?;
    }
    Some(Srgba::from_rgb_array(rgb.map(|v| v.clamp(0.0, 1.0))))
}
