//! `Thumbnails`: the one thumbnail contract. The app supplies pixels at once or as a job, keyed by a key and a
//! revision (`ThumbSource`); the library uploads, stores, evicts, asks for a redraw and paints (`Thumbnails::paint`).

use super::limits::MAX_TEXTURE;
use super::loader::{LoadFailure, LoadedPicture, LoaderOptions, Priority, TextureLoader};
use super::store::{Revision, TextureKey, TextureState, TextureStoreOptions};
use crate::primitives::{ContentPainterCx, IconSpec, Surface};
use crate::runtime::egui::{memory, pass, repaint, textures};
use crate::style::{Corners, DataColor, SurfaceKind, surface_style};
use craft_behavior::{IconId, PixelSize};
use craft_host::Jobs;
use craft_host::queue::{Cancel, QueueOptions};
use egui::emath::GuiRounding as _;
use egui::{Color32, ColorImage, Context, Rect, Vec2, pos2, vec2};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// What a thumbnail is asked at: the side in pixels, the screen's density and the part of the frame wanted.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ThumbRequest {
    /// The longest side of the picture, in pixels: a multiple of 16.
    pub side: u32,
    /// Pixels per point of the screen it shows on.
    pub pixels_per_point: f32,
    /// The part of the frame to render, in fractions of the frame (0 to 1 on both axes), or the whole frame. `side`
    /// is then the longest side of that part: a page larger than the texture side on screen asks for the part in
    /// view and shows its detail. The region is part of the picture's revision, so each region has its own
    /// texture.
    pub region: Option<Rect>,
}

/// Requested sides are multiples of this many pixels.
const SIZE_STEP: u32 = 16;

impl ThumbRequest {
    /// The request for a cell `points` wide on a screen of `pixels_per_point`: the side in pixels rounded up to a
    /// multiple of 16, at least 16 and at most `MAX_TEXTURE`. 28 points at 2x is 64 pixels. Neighbouring cell sizes
    /// share one picture. [`Thumbnails::state`] lowers a side past its [`ThumbnailOptions::max_side`].
    pub fn for_cell(points: f32, pixels_per_point: f32) -> ThumbRequest {
        let pixels = points * pixels_per_point;
        let steps = if pixels.is_finite() { (pixels / SIZE_STEP as f32).ceil().max(1.0) as u32 } else { 1 };
        let side = steps.saturating_mul(SIZE_STEP).min(MAX_TEXTURE / SIZE_STEP * SIZE_STEP);
        ThumbRequest { side, pixels_per_point, region: None }
    }

    /// The request for the part `region` of the frame (fractions of the frame, 0 to 1).
    #[must_use]
    pub fn region(self, region: Rect) -> ThumbRequest {
        ThumbRequest { region: Some(region), ..self }
    }
}

/// The request's region as hashable bits.
fn region_bits(region: Option<Rect>) -> Option<[u32; 4]> {
    region.map(|r| [r.min.x.to_bits(), r.min.y.to_bits(), r.max.x.to_bits(), r.max.y.to_bits()])
}

/// A background render: runs on a queue lane, checks the [`Cancel`] between steps, and returns the picture, the
/// answer that the key has none (`LoadedPicture::Blank`, state `Empty`), or the `LoadFailure` the tooltip words. It
/// owns what it reads (an `Arc` of the node, a path), never a borrow of the document.
pub type ThumbJob = Box<dyn FnOnce(&Cancel) -> Result<LoadedPicture, LoadFailure> + Send>;

/// What a [`ThumbSource`] renders for a key.
#[non_exhaustive]
pub enum ThumbRender {
    /// The picture, made now on the UI thread (a light layer, a fill colour).
    Ready(ColorImage),
    /// A job that makes it on a queue lane (a heavy node, a PDF page, a decoded file).
    Later(ThumbJob),
    /// No picture: state `Empty`, and the row paints its own tile (a group's folder, an adjustment's glyph).
    Blank,
}

/// The app's side of the contract: how a key's picture is versioned and made.
///
/// - `revision` is cheap: a stored fingerprint or pointer identity with a `Weak` guard, never a pixel walk. It covers
///   the pixels only, so a step that changes only the selection keeps it and every thumbnail stays. The request's
///   side is added by [`Thumbnails`]; the revision need not include it.
/// - `render` is called only when the revision has no picture yet, within the pass's render budget. The picture has
///   the frame's aspect, longest side at most `request.side`, with no letterbox bars ([`thumb_image`] builds one from
///   a sampler); [`Thumbnails::paint`] fits it to the cell.
/// - Pictures are display-space sRGBA, unmultiplied (`ColorImage::from_rgba_unmultiplied`); colour management is the
///   app's and happens before the picture is returned.
pub trait ThumbSource<K> {
    /// The version of `key`'s picture.
    fn revision(&self, key: &K, request: &ThumbRequest) -> Revision;
    /// Makes `key`'s picture, now or as a job.
    fn render(&self, key: &K, request: &ThumbRequest) -> ThumbRender;
}

/// A [`Thumbnails`]' settings, read when it is created.
#[derive(Clone)]
#[non_exhaustive]
pub struct ThumbnailOptions {
    /// The longest side any request is lowered to, rounded down to 16: 512; a page grid sets more.
    pub max_side: u32,
    /// Budgets of the texture store: `TextureStoreOptions::thumbnails`.
    pub store: TextureStoreOptions,
    /// Lanes of the queue for `Later` renders; see `LoaderOptions::lanes`.
    pub lanes: usize,
    /// Where the lanes run; see `LoaderOptions::jobs`.
    pub jobs: Option<Arc<dyn Jobs>>,
    /// Renders on the UI thread per pass: 16.
    pub renders_per_pass: usize,
}

impl ThumbnailOptions {
    /// Sides up to 512, thumbnail budgets, the machine's lanes, the host's jobs, 16 renders per pass.
    pub fn new() -> Self {
        Self { max_side: 512, store: TextureStoreOptions::thumbnails(), lanes: QueueOptions::default_lanes(), jobs: None, renders_per_pass: 16 }
    }

    /// The longest side any request is lowered to.
    #[must_use]
    pub fn max_side(mut self, max_side: u32) -> Self {
        self.max_side = max_side;
        self
    }

    /// Budgets of the texture store.
    #[must_use]
    pub fn store(mut self, store: TextureStoreOptions) -> Self {
        self.store = store;
        self
    }

    /// Lanes of the queue for `Later` renders.
    #[must_use]
    pub fn lanes(mut self, lanes: usize) -> Self {
        self.lanes = lanes;
        self
    }

    /// Where the lanes run.
    #[must_use]
    pub fn jobs(mut self, jobs: Arc<dyn Jobs>) -> Self {
        self.jobs = Some(jobs);
        self
    }

    /// Renders on the UI thread per pass.
    #[must_use]
    pub fn renders_per_pass(mut self, renders: usize) -> Self {
        self.renders_per_pass = renders;
        self
    }
}

impl Default for ThumbnailOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// How [`Thumbnails::paint`] fits a picture into its cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ThumbFit {
    /// The whole picture, as large as fits, centred ([`fit_rect`]); the checkerboard is under the picture only, so
    /// the frame's empty sides stay the row's.
    #[default]
    Contain,
    /// The cell filled, the picture cut at its long sides around its centre (a square grid of photos).
    Cover,
}

/// How to paint one thumbnail.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ThumbPaint {
    /// How the picture fits its box.
    pub fit: ThumbFit,
    /// The transparency grid behind the picture (default on). Off for opaque pictures such as photos and pages.
    /// Ignored while `paper` is set.
    pub checker: bool,
    /// The paper the picture sits on, when the document gives one: a fill under the picture's rect in place of the
    /// transparency grid (default none). It wins over `checker`; the two never paint together.
    pub paper: Option<DataColor>,
    /// Multiplies the picture (default untinted): a dimmed row's faint thumbnail.
    pub tint: DataColor,
}

impl Default for ThumbPaint {
    fn default() -> Self {
        Self { fit: ThumbFit::Contain, checker: true, paper: None, tint: DataColor::new(super::UNTINTED) }
    }
}

impl ThumbPaint {
    /// Sets how the picture fits its box.
    #[must_use]
    pub fn fit(mut self, fit: ThumbFit) -> Self {
        self.fit = fit;
        self
    }

    /// Sets whether the transparency grid shows behind the picture.
    #[must_use]
    pub fn checker(mut self, checker: bool) -> Self {
        self.checker = checker;
        self
    }

    /// Sets the paper under the picture: a fill in place of the transparency grid, whatever `checker` says (a page
    /// thumbnail's paper is document data).
    #[must_use]
    pub fn paper(mut self, paper: DataColor) -> Self {
        self.paper = Some(paper);
        self
    }

    /// Sets the tint that multiplies the picture.
    #[must_use]
    pub fn tint(mut self, tint: DataColor) -> Self {
        self.tint = tint;
        self
    }
}

/// The per-pass state of a [`Thumbnails`].
struct Inner {
    options: ThumbnailOptions,
    /// While held, shown pictures stay and no render starts.
    hold: bool,
    /// The frame the render counters are for, and the counters.
    frame: Option<u64>,
    renders: usize,
}

impl Inner {
    /// Whether this pass has used its count of renders on the UI thread.
    fn over_budget(&self) -> bool {
        self.renders >= self.options.renders_per_pass
    }
}

/// The shared state of the thumbnails of one context and key type, by the store's salt.
struct Hubs<K> {
    by_salt: HashMap<u64, Arc<Mutex<Inner>>>,
    key: std::marker::PhantomData<fn() -> K>,
}

impl<K> Default for Hubs<K> {
    fn default() -> Self {
        Self { by_salt: HashMap::new(), key: std::marker::PhantomData }
    }
}

/// Thumbnails of one kind (layers, channels, snapshots, a grid's files) over one store: what every list and grid
/// draws its pictures with.
///
/// - **Asking.** A widget calls [`state`](Self::state) for each row whose rect is visible at `Priority::VISIBLE`,
///   and for one row or one viewport beyond at `Priority::NEAR`, before the row's paint closure runs (the closure
///   only gets a painter and a rect). Rows asked for in neither this pass nor the previous one have their jobs
///   dropped.
/// - **Making.** A revision without a picture is rendered by the source: on the UI thread within
///   [`renders_per_pass`](ThumbnailOptions::renders_per_pass) (the rest wait with their stand-ins and the
///   pass asks for an immediate repaint), or as a `Later` job on the loader's queue. A new revision keeps the old
///   picture as the stand-in until the new one lands. A failure is remembered for its revision, so nothing retries
///   every pass.
/// - **Holding.** [`hold`](Self::hold)`(true)` while a drag or a stroke runs: shown pictures stay, no render starts.
/// - **Painting.** [`paint`](Self::paint) draws the transparency grid, the picture fitted to its aspect, the
///   placeholder while nothing has arrived, or the failure icon. No widget draws its own placeholder or failure
///   glyph. Pictures never mirror in a right-to-left interface.
/// - **AccessKit.** None: the row names the thing.
///
/// `Thumbnails<K>` is `Clone + Send + Sync`; its state lives in the context.
pub struct Thumbnails<K: TextureKey> {
    loader: TextureLoader<K>,
    inner: Arc<Mutex<Inner>>,
}

impl<K: TextureKey> Clone for Thumbnails<K> {
    fn clone(&self) -> Self {
        Self { loader: self.loader.clone(), inner: Arc::clone(&self.inner) }
    }
}

const _: () = {
    const fn send_sync<T: Send + Sync + Clone + 'static>() {}
    send_sync::<Thumbnails<u64>>();
};

impl<K: TextureKey> Thumbnails<K> {
    /// The thumbnails stored in `ctx` under `salt`, made with `options` the first time; the loader and its store
    /// live under the same salt.
    pub fn in_context(ctx: &Context, salt: impl std::hash::Hash + std::fmt::Debug, options: ThumbnailOptions) -> Self {
        let mut loader_options = LoaderOptions::new().store(options.store).lanes(options.lanes);
        if let Some(jobs) = &options.jobs {
            loader_options = loader_options.jobs(Arc::clone(jobs));
        }
        let loader = TextureLoader::in_context(ctx, &salt, loader_options);
        let id = loader.cache().salt();
        let inner = memory::with(ctx, |hubs: &mut Hubs<K>| {
            Arc::clone(hubs.by_salt.entry(id).or_insert_with(|| Arc::new(Mutex::new(Inner { options, hold: false, frame: None, renders: 0 }))))
        });
        Self { loader, inner }
    }

    /// What `key` shows now; renders or queues it when its revision has no picture yet.
    ///
    /// In order: the request's side is lowered to `max_side` (rounded down to 16); the revision is
    /// `source.revision(key, request)` combined with the side and the region; the loader is asked and its answer
    /// returned when it has one. Otherwise, while [held](Self::hold) or past the pass's render budget, the key's
    /// stand-in is returned as `Loading` (past the budget with a repaint request). Otherwise `source.render`:
    /// `Ready` is put in the store (counted against the budget), `Blank` is stored as `Empty`, and `Later` is
    /// submitted at `priority`.
    pub fn state(&self, ctx: &Context, source: &dyn ThumbSource<K>, key: &K, request: &ThumbRequest, priority: Priority) -> TextureState {
        let max_side = self.lock().options.max_side;
        let side = request.side.min((max_side / SIZE_STEP * SIZE_STEP).max(SIZE_STEP));
        let request = ThumbRequest { side, ..*request };
        let revision = Revision::of(&(source.revision(key, &request), side, region_bits(request.region)));
        if let Some(state) = self.loader.ask(ctx, key, revision, priority) {
            return state;
        }
        let stand_in = || TextureState::Loading { stand_in: self.loader.cache().state(ctx, key, revision).shown() };
        {
            let mut inner = self.lock();
            let frame = pass::root_frame_nr(ctx);
            if inner.frame != Some(frame) {
                inner.frame = Some(frame);
                inner.renders = 0;
            }
            if inner.hold {
                return stand_in();
            }
            if inner.over_budget() {
                repaint::request(ctx);
                return stand_in();
            }
        }
        match source.render(key, &request) {
            ThumbRender::Ready(image) => {
                let texture = self.loader.cache().put(ctx, key.clone(), revision, image);
                let mut inner = self.lock();
                inner.renders += 1;
                TextureState::Ready(texture)
            }
            ThumbRender::Blank => {
                self.loader.cache().put_empty(ctx, key.clone(), revision);
                TextureState::Empty
            }
            ThumbRender::Later(job) => self.loader.submit(ctx, key.clone(), revision, priority, job),
        }
    }

    /// Paints `state` into `rect` and returns whether it drew anything:
    /// - `Ready`, or `Loading` with a stand-in: the picture fitted by `paint.fit`, over the transparency grid when
    ///   `paint.checker` (or over `paint.paper`, which replaces the grid), tinted by `paint.tint`.
    /// - `Missing`, or `Loading` without a stand-in: the placeholder, a sunken surface over `rect` (filled with
    ///   `paint.paper` when set).
    /// - `Failed`: the placeholder with the warning icon centred. The message is the host's tooltip
    ///   (`ListRow::tooltip`).
    /// - `Empty`: nothing; returns false, and the row paints its own tile.
    ///
    /// Rects are rounded to whole pixels.
    pub fn paint(&self, cx: &ContentPainterCx<'_>, rect: Rect, state: &TextureState, paint: &ThumbPaint) -> bool {
        let rect = rect.round_to_pixels(cx.pixels_per_point());
        match state {
            TextureState::Empty => false,
            TextureState::Ready(texture) | TextureState::Loading { stand_in: Some(texture) } => {
                paint_picture(cx, rect, texture.id, texture.size_vec2(), paint);
                true
            }
            TextureState::Missing | TextureState::Loading { stand_in: None } => {
                paint_placeholder(cx, rect, paint.paper);
                true
            }
            TextureState::Failed { .. } => {
                paint_placeholder(cx, rect, paint.paper);
                paint_failure(cx, rect);
                true
            }
        }
    }

    /// Stops the queue of `Later` renders and waits up to `timeout` for the running ones to end
    /// (`TextureLoader::shutdown`); returns whether they have. For an app about to exit the process.
    pub fn shutdown(&self, timeout: Duration) -> bool {
        self.loader.shutdown(timeout)
    }

    /// Freezes the shown pictures while `hold`: no render starts and no job is queued; `state` returns the current
    /// picture of each key (as `Loading` when its revision moved on). Results of jobs already running still land.
    pub fn hold(&self, hold: bool) {
        self.lock().hold = hold;
    }

    /// Keeps only the pictures whose key `keep` accepts (the open documents changed).
    pub fn retain(&self, ctx: &Context, keep: impl FnMut(&K) -> bool) {
        self.loader.cache().retain(ctx, keep);
    }

    /// The store under the thumbnails, for `forget`, `clear` and `statistics`.
    pub fn cache(&self) -> &super::TextureStore<K> {
        self.loader.cache()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The rect inside `cell` that shows a picture of `picture`'s aspect whole, as large as fits and centred; `cell`
/// itself for a zero-sized picture.
pub fn fit_rect(cell: Rect, picture: Vec2) -> Rect {
    if !(picture.x > 0.0 && picture.y > 0.0) {
        return cell;
    }
    let scale = (cell.width() / picture.x).min(cell.height() / picture.y);
    Rect::from_center_size(cell.center(), picture * scale)
}

/// A picture of the document area `frame` (pixels) with that exact aspect, its longest side `longest_side`, each
/// side at least 1: texel `(tx, ty)` is `sample(x, y)` at the document pixel under the texel's centre, `x` and `y`
/// counted from the frame's top left (the caller adds the frame's origin). `sample` returns sRGBA, unmultiplied. A
/// frame smaller than the side is sampled nearest, larger ones point-sampled at texel centres.
pub fn thumb_image(frame: PixelSize, longest_side: u32, mut sample: impl FnMut(u32, u32) -> [u8; 4]) -> ColorImage {
    let PixelSize { width, height, .. } = thumb_size(frame, longest_side);
    let (frame_width, frame_height) = (u64::from(frame.width.max(1)), u64::from(frame.height.max(1)));
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for ty in 0..height {
        let y = ((2 * u64::from(ty) + 1) * frame_height / (2 * u64::from(height))).min(frame_height - 1) as u32;
        for tx in 0..width {
            let x = ((2 * u64::from(tx) + 1) * frame_width / (2 * u64::from(width))).min(frame_width - 1) as u32;
            let [r, g, b, a] = sample(x, y);
            pixels.push(Color32::from_rgba_unmultiplied(r, g, b, a));
        }
    }
    ColorImage::new([width as usize, height as usize], pixels)
}

/// The pixel size of a picture of `frame`'s aspect with its longest side `longest_side`.
fn thumb_size(frame: PixelSize, longest_side: u32) -> PixelSize {
    let (frame_width, frame_height) = (f64::from(frame.width.max(1)), f64::from(frame.height.max(1)));
    let scale = f64::from(longest_side.max(1)) / frame_width.max(frame_height);
    PixelSize::new((frame_width * scale).round().max(1.0) as u32, (frame_height * scale).round().max(1.0) as u32)
}

/// Draws the transparency grid and the picture for `paint.fit`.
fn paint_picture(cx: &ContentPainterCx<'_>, cell: Rect, texture: egui::TextureId, size: Vec2, paint: &ThumbPaint) {
    let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let (target, uv) = match paint.fit {
        ThumbFit::Contain => (fit_rect(cell, size).round_to_pixels(cx.pixels_per_point()), full),
        ThumbFit::Cover => (cell, cover_uv(cell, size)),
    };
    if let Some(paper) = paint.paper {
        cx.painter.rect_filled(target, 0.0, paper.0);
    } else if paint.checker {
        cx.checker(target);
    }
    textures::paint(&cx.painter, texture, target, uv, paint.tint.0, egui::CornerRadius::ZERO);
}

/// The part of a picture a `Cover` fit shows in `cell`: centred, cut at the long sides.
fn cover_uv(cell: Rect, picture: Vec2) -> Rect {
    if !(picture.x > 0.0 && picture.y > 0.0 && cell.width() > 0.0 && cell.height() > 0.0) {
        return Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    }
    let scale = (cell.width() / picture.x).max(cell.height() / picture.y);
    let shown = vec2(cell.width() / (picture.x * scale), cell.height() / (picture.y * scale));
    Rect::from_center_size(pos2(0.5, 0.5), shown)
}

/// The cell while no picture has arrived, and behind the failure icon: a sunken surface, in the caller's `paper`
/// when it gives one.
fn paint_placeholder(cx: &ContentPainterCx<'_>, rect: Rect, paper: Option<DataColor>) {
    let mut style = surface_style(cx.tokens, SurfaceKind::Sunken);
    if let Some(paper) = paper {
        style.fill = paper.0;
    }
    cx.painter.extend(Surface::shapes(&style, None, rect, Corners::None, cx.direction, cx.pixels_per_point()));
}

/// The warning icon centred in `rect`, no larger than the small icon size.
fn paint_failure(cx: &ContentPainterCx<'_>, rect: Rect) {
    let side = cx.tokens.size.icon.sm.min(rect.width().min(rect.height()));
    let icon = Rect::from_center_size(rect.center(), vec2(side, side));
    crate::primitives::icon::paint(&cx.painter, cx.tokens, cx.direction, icon, &IconSpec::svg(IconId::new("warning")));
}

/// A [`ThumbSource`] for stories and tests in every crate: pictures made from the key and a revision the test sets,
/// so every state shows without a document.
///
/// - The picture: a diagonal gradient between two colours whose hues come from the key's hash (start hue
///   `hash % 360`, end hue 120 degrees on), fully opaque except a transparent top-left quarter (so the checkerboard
///   shows), with `revision % 8` white bars of one texel along the bottom edge; [`aspect`](Self::aspect) shapes the
///   frame and [`opaque`](Self::opaque) fills the quarter, as a story's sample photos are.
/// - [`later`](Self::later)`(true)` renders as `Later` jobs, so with a `ManualJobs` in [`ThumbnailOptions::jobs`]
///   pictures arrive one per `run_next`.
/// - Keys given to [`failing`](Self::failing) fail with the message "Fake failure" (always as a `Later` job, since a
///   synchronous render cannot fail); keys given to [`blank`](Self::blank) render `Blank`.
/// - [`set_revision`](Self::set_revision) and [`bump`](Self::bump) change every key's revision, so stand-ins show.
#[cfg(feature = "testing")]
pub struct FakeThumbSource<K> {
    revision: u64,
    later: bool,
    opaque: bool,
    aspect: PixelSize,
    failing: Vec<K>,
    blank: Vec<K>,
}

#[cfg(feature = "testing")]
impl<K: std::hash::Hash + PartialEq + Clone + Send + 'static> FakeThumbSource<K> {
    /// Synchronous square pictures at revision 0.
    pub fn new() -> Self {
        Self { revision: 0, later: false, opaque: false, aspect: PixelSize::new(1, 1), failing: Vec::new(), blank: Vec::new() }
    }

    /// Renders as `Later` jobs.
    #[must_use]
    pub fn later(mut self, later: bool) -> Self {
        self.later = later;
        self
    }

    /// The picture has no transparent quarter.
    #[must_use]
    pub fn opaque(mut self, opaque: bool) -> Self {
        self.opaque = opaque;
        self
    }

    /// The frame's width to height (4:3 landscape, 3:4 portrait).
    #[must_use]
    pub fn aspect(mut self, width: u32, height: u32) -> Self {
        self.aspect = PixelSize::new(width.max(1), height.max(1));
        self
    }

    /// These keys fail.
    #[must_use]
    pub fn failing(mut self, keys: impl IntoIterator<Item = K>) -> Self {
        self.failing.extend(keys);
        self
    }

    /// These keys have no picture.
    #[must_use]
    pub fn blank(mut self, keys: impl IntoIterator<Item = K>) -> Self {
        self.blank.extend(keys);
        self
    }

    /// Every key's revision.
    pub fn set_revision(&mut self, revision: u64) {
        self.revision = revision;
    }

    /// Moves every key's revision on by one.
    pub fn bump(&mut self) {
        self.revision += 1;
    }
}

#[cfg(feature = "testing")]
impl<K: std::hash::Hash + PartialEq + Clone + Send + 'static> Default for FakeThumbSource<K> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "testing")]
impl<K: std::hash::Hash + PartialEq + Clone + Send + 'static> ThumbSource<K> for FakeThumbSource<K> {
    /// `Revision::of(&(key, revision))`.
    fn revision(&self, key: &K, _request: &ThumbRequest) -> Revision {
        Revision::of(&(key, self.revision))
    }

    /// The picture described on [`FakeThumbSource`], at `thumb_image(aspect scaled, request.side, ..)`.
    fn render(&self, key: &K, request: &ThumbRequest) -> ThumbRender {
        if self.blank.contains(key) {
            return ThumbRender::Blank;
        }
        if self.failing.contains(key) {
            return ThumbRender::Later(Box::new(|_| Err(LoadFailure::Message("Fake failure".to_owned()))));
        }
        let frame = thumb_size(self.aspect, request.side);
        let image = fake_picture(frame, fake_hue(key), self.revision, self.opaque);
        if self.later { ThumbRender::Later(Box::new(move |_| Ok(LoadedPicture::Image(image)))) } else { ThumbRender::Ready(image) }
    }
}

/// Hue in degrees from the key's hash.
#[cfg(feature = "testing")]
fn fake_hue(key: &impl std::hash::Hash) -> f32 {
    use std::hash::Hasher as _;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    (hasher.finish() % 360) as f32
}

/// The opaque picture of a [`FakeThumbSource`] for `key` at revision 0, `frame` pixels: a sample picture for a
/// component that takes an image rather than a source.
#[cfg(feature = "testing")]
pub(crate) fn fake_thumb_image(key: &impl std::hash::Hash, frame: PixelSize) -> ColorImage {
    fake_picture(frame, fake_hue(key), 0, true)
}

/// The fake source's picture: a gradient from `hue` to `hue + 120` degrees, a transparent top left quarter unless
/// `opaque`, and `revision % 8` white texels along the bottom row. Every test context renders its pictures anew, so
/// each picture is made once per process.
#[cfg(feature = "testing")]
fn fake_picture(frame: PixelSize, hue: f32, revision: u64, opaque: bool) -> ColorImage {
    type Key = (u32, u32, u32, u32, bool);
    static MADE: Mutex<Option<HashMap<Key, ColorImage>>> = Mutex::new(None);
    let bars = (revision % 8) as u32;
    let key = (frame.width, frame.height, hue.to_bits(), bars, opaque);
    let mut made = MADE.lock().unwrap_or_else(PoisonError::into_inner);
    made.get_or_insert_default().entry(key).or_insert_with(|| make_fake_picture(frame, hue, bars, opaque)).clone()
}

#[cfg(feature = "testing")]
fn make_fake_picture(frame: PixelSize, hue: f32, bars: u32, opaque: bool) -> ColorImage {
    let PixelSize { width, height, .. } = frame;
    let end = |degrees: f32| egui::ecolor::Hsva::new((degrees % 360.0) / 360.0, 0.6, 0.85, 1.0).to_srgba_unmultiplied();
    let (from, to) = (end(hue), end(hue + 120.0));
    thumb_image(frame, width.max(height), |x, y| {
        if y + 1 == height && x % 2 == 0 && x / 2 < bars {
            return [255, 255, 255, 255];
        }
        if !opaque && x < width / 2 && y < height / 2 {
            return [0, 0, 0, 0];
        }
        let t = ((x as f32 + 0.5) / width as f32 + (y as f32 + 0.5) / height as f32) / 2.0;
        let mix = |i: usize| (f32::from(from[i]) + (f32::from(to[i]) - f32::from(from[i])) * t).round() as u8;
        [mix(0), mix(1), mix(2), 255]
    })
}
