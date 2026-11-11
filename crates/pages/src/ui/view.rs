//! `PageView`: a scrolling view of a document's pages, laid out by [`crate::layout`], drawn on a craft-ui [`Canvas`]
//! through its [`Pasteboard`] page frames, with the page pictures the app renders through craft-ui's texture
//! contract ([`ThumbSource`]).
//!
//! The view is a canvas: the app runs its [`ToolHost`](craft_ui::canvas::ToolHost) over [`PageViewOutput::canvas`]
//! with the page tools (Select Text, `HandTool` and `ZoomTool`), then hands the view the host reports back with
//! [`PageViewOutput::adopt`].
//!
//! ```ignore
//! let out = PageView::new("Pages", &doc_pages, &mut view_state, paper).options(LayoutOptions::ACROBAT).show(ui, |ui, body| {
//!     TextMarks::new(&body.pages, &marks).show_at(ui, body.rect);
//! });
//! let tools = ToolHost::new().run(ui, &out.canvas, &mut tool);
//! if let Some(view) = tools.view { out.adopt(&mut view_state, &view); }
//! ```

mod pictures;
#[cfg(feature = "screen-testing")]
mod stories;

use self::pictures::{PageMetrics, PagePictures};
use super::access::AccessDescriber;
use crate::layout::{
    Fit, LayoutOptions, PageDisplay, PageSpacing, PagedLayout, SCREEN_POINTS_PER_POINT, ViewDefaults, ZOOM_RANGE, fit_scale, place_pages, shown_row,
};
use craft_behavior::kurbo::{Point, Rect as DocRect, Size};
use craft_behavior::{CanvasView, PressPhase, WheelNotch, ZoomLadder};
use craft_ui::canvas::{Canvas, CanvasOutput, CanvasScrollbars, NavigationOptions, PageFrame, Pasteboard, PasteboardOutput, canvas_space};
use craft_ui::contract::{Bind, BodyCx, Held, Props as _, Show as _, ShowContainer as _};
use craft_ui::primitives::textures::ThumbSource;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::memory::Remember as _;
use craft_ui::style::DataColor;
use egui::{Rect, Ui, Vec2, pos2};
use std::cell::RefCell;
use std::ops::Range;
use std::sync::Arc;

/// The document's pages as a page view, thumbnails and previews read them. The app implements it over its document;
/// the view never owns pages.
///
/// Pictures go through craft-ui's one texture contract, [`ThumbSource<usize>`] keyed by page index (`revision` a
/// cheap stamp of the page's content, `render` the page at `request.side` pixels on its longest side, usually as a
/// `ThumbRender::Later` job on the loader's queue). The library uploads, caches, evicts, keeps a stale picture as
/// the stand-in until the new one lands and asks for the redraw; the app holds no texture and polls nothing.
pub trait PageSource: ThumbSource<usize> {
    /// The number of pages in the document.
    fn page_count(&self) -> usize;

    /// Page `page`'s size in points as shown (its rotation applied).
    fn page_size(&self, page: usize) -> Size;

    /// The page's label ("iv", "A-3"; PDF page labels, section numbering): what the page box, thumbnails and the
    /// accessibility tree call it. 1-based number by default.
    fn page_label(&self, page: usize) -> String {
        (page + 1).to_string()
    }
}

/// The view's state, kept by the app per document view (one per tab).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct PageViewState {
    /// The current page: the one the page box shows and paging moves from.
    pub current: usize,
    /// Single pages, two-up spreads or columns.
    pub display: PageDisplay,
    /// Every page scrolls past; off shows one row (a page or a spread) at a time.
    pub continuous: bool,
    /// Two-page spreads start with the first page alone.
    pub cover: bool,
    /// Separate pages with a gap and shadow; off joins adjacent page edges.
    pub page_gaps: bool,
    /// [`Fit::Width`] or [`Fit::Page`] refit as the view resizes; [`Fit::None`] keeps `zoom`.
    pub fit: Fit,
    /// The zoom (1.0 = 100 %), kept equal to what a fit shows so zoom steps start from it.
    pub zoom: f64,
    /// Word's Hide White Space.
    pub hide_white_space: bool,
    /// The scroll offset (screen points).
    pub offset: (f64, f64),
    /// The view's rotation in degrees clockwise (0, 90, 180 or 270). The app applies it in [`PageSource::page_size`]
    /// and its renders; the view keeps it so views that follow each other ([`SyncedViews`](super::SyncedViews))
    /// agree on it.
    pub rotation: u16,
    /// A page the next pass brings to the top of the view, with a fraction of its height to put at the top; that
    /// pass clears it.
    #[serde(skip)]
    pub go_to: Option<(usize, f64)>,
}

impl Default for PageViewState {
    fn default() -> Self {
        PageViewState::from_defaults(&ViewDefaults::default())
    }
}

impl PageViewState {
    /// A new view as the user's view defaults say.
    pub fn from_defaults(defaults: &ViewDefaults) -> Self {
        PageViewState {
            current: 0,
            display: defaults.display,
            continuous: defaults.continuous,
            cover: false,
            page_gaps: defaults.page_gaps,
            fit: defaults.fit,
            zoom: defaults.zoom,
            hide_white_space: false,
            offset: (0.0, 0.0),
            rotation: 0,
            go_to: None,
        }
    }

    /// Ask the next pass to bring `page` to the top of the view (page box, outline, thumbnails, find), and make it
    /// current now.
    pub fn go_to(&mut self, page: usize) {
        self.go_to = Some((page, 0.0));
        self.current = page;
    }

    /// Zoom In / Zoom Out: the next rung of craft-ui's [`ZoomLadder`] above (or below) the shown zoom, within
    /// [`ZOOM_RANGE`] (Photoshop's rungs, the suite's one zoom ladder); a fit ends. The zoom a fit showed is the
    /// start.
    pub fn zoom_step(&mut self, larger: bool) {
        self.fit = Fit::None;
        self.zoom = ZoomLadder::step_in_range(self.zoom, if larger { 1 } else { -1 }, ZOOM_RANGE);
    }
}

/// One page on screen this pass, for overlays: its index, screen rectangle and scale.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PlacedPage {
    /// Page index, 0-based.
    pub page: usize,
    /// The whole page on screen (part of it may be clipped or, with Hide White Space, hidden).
    pub rect: Rect,
    /// Screen points per page point.
    pub scale: f64,
    /// The part of the page's height that shows (screen points from its top).
    pub band: (f32, f32),
}

impl PlacedPage {
    /// Page `page` at `rect` on screen, `scale` screen points to a page point, with `band` the visible part of its height in
    /// screen points.
    pub const fn new(page: usize, rect: Rect, scale: f64, band: (f32, f32)) -> Self {
        PlacedPage { page, rect, scale, band }
    }

    /// Page point (origin top-left, y down) to screen.
    pub fn to_screen(&self, p: Point) -> egui::Pos2 {
        pos2(self.rect.min.x + (p.x * self.scale) as f32, self.rect.min.y + (p.y * self.scale) as f32)
    }

    /// Screen to page point.
    pub fn to_page(&self, p: egui::Pos2) -> Point {
        Point::new(f64::from(p.x - self.rect.min.x) / self.scale, f64::from(p.y - self.rect.min.y) / self.scale)
    }
}

/// What the page view's canvas body hands the app's canvas controls (margins, zones, marks, grids, cell overlay,
/// text marks).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct PageBody<'b> {
    /// The canvas body's rect.
    pub rect: Rect,
    /// The pages on screen this pass.
    pub pages: &'b [PlacedPage],
    /// The view the canvas shows this pass.
    pub view: CanvasView,
}

/// What the view did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct PageViewOutput {
    /// The canvas this pass, for the app's `ToolHost`, `Rulers` and key handling.
    pub canvas: CanvasOutput,
    /// The pages drawn this pass, in order: the page rectangles overlays and the access page window read.
    pub pages: Vec<PlacedPage>,
    /// The whole layout, in screen points from the content's top-left.
    pub layout: PagedLayout,
    /// The pages at least partly in view.
    pub visible: Range<usize>,
    /// The current page changed (scrolling, the wheel turning a page, a tool's view).
    pub current_changed: bool,
    /// A double-click on the gap between pages: the app toggles Hide White Space (Word).
    pub toggle_white_space: bool,
    /// The pasteboard outside the pages (`None` when the pass drew nothing): a secondary press or the Menu key there asks the app for Photoshop's
    /// pasteboard colour menu at [`PasteboardOutput::context_at`]; a page name click is not used here.
    pub pasteboard: Option<PasteboardOutput>,
    frame: Frame,
}

impl PageViewOutput {
    /// Takes the view a tool host reported (`ToolHostResponse::view`, from the Hand and Zoom tools) into `state`:
    /// the scroll offset, and the zoom when it changed (a fit ends). In a continuous view the current page follows
    /// the offset.
    pub fn adopt(&self, state: &mut PageViewState, view: &CanvasView) {
        let frame = self.frame;
        if (view.zoom - frame.zoom).abs() > frame.zoom * 1e-9 {
            state.fit = Fit::None;
            state.zoom = (view.zoom).clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
        }
        let size = view.rect.size();
        state.offset = frame.offset_of(view.center, f64::from(size.x), f64::from(size.y));
        if state.continuous && !self.layout.rects.is_empty() {
            state.current = page_on_line(&self.layout, state.offset.1 + f64::from(size.y) / 2.0);
        }
    }
}

/// The page view, built per pass over the app's [`PageSource`] and [`PageViewState`].
///
/// - Layout: [`place_pages`] at the state's zoom, or the fit's ([`fit_scale`]), with the [`PageSpacing`] of the
///   view's style, recomputed when the viewport or the pages change; the zoom is written back so it equals what
///   shows. Pages draw as [`PageFrame`]s on a [`Pasteboard`], the page picture over each through `Thumbnails` at
///   `Priority::VISIBLE`, one row either side at `Priority::NEAR`; a page larger than the texture side adds a
///   second picture of the part in view. Sharp renders wait while a zoom gesture or ease runs.
/// - Layout: a scale the fit or the app changes keeps the page at the top of the view in place.
/// - Scrolling: the canvas folds in the wheel (the wheel scrolls, Shift or Ctrl/Cmd + wheel scrolls sideways, Alt +
///   wheel and a pinch zoom about the pointer, as [`NavigationOptions`] say) and the view commands; the view
///   reads the result back into its state. [`CanvasScrollbars`] land in their own pass. Not continuous, a page
///   that fits turns with the wheel instead, and one zoomed in far enough to pan pans and turns the page at its
///   edge. Space-drag and the Hand and Zoom tools run in the app's `ToolHost`.
/// - Current page: the page whose rectangle holds the view's middle line (ties at half a point go to the earlier
///   page).
/// - AccessKit: a canvas named by the app; pages-b's describer adds the text under it.
#[derive(craft_ui::contract::Pattern)]
pub struct PageView<'a> {
    label: Label,
    source: &'a dyn PageSource,
    state: &'a mut PageViewState,
    options: LayoutOptions,
    navigation: NavigationOptions,
    bodies: Option<&'a [(f64, f64)]>,
    describer: Option<&'a dyn AccessDescriber>,
    paper: DataColor,
}

impl<'a> PageView<'a> {
    /// A view named `label` of the pages of `source`, with `state` kept by the app between passes, on pages of the
    /// document's `paper` colour.
    pub fn new(label: impl Into<Label>, source: &'a dyn PageSource, state: &'a mut PageViewState, paper: DataColor) -> Self {
        PageView {
            label: label.into(),
            source,
            state,
            options: LayoutOptions::ACROBAT,
            navigation: NavigationOptions::default(),
            bodies: None,
            describer: None,
            paper,
        }
    }

    /// The layout constants; the state's display, continuous, cover and page gaps override theirs.
    #[must_use]
    pub fn options(mut self, options: LayoutOptions) -> Self {
        self.options = options;
        self
    }

    /// The user's navigation options: how the wheel and a pinch move the view.
    #[must_use]
    pub fn navigation(mut self, navigation: NavigationOptions) -> Self {
        self.navigation = navigation;
        self
    }

    /// Each page's body (top and bottom of its text area, points), for Hide White Space.
    #[must_use]
    pub fn bodies(mut self, bodies: &'a [(f64, f64)]) -> Self {
        self.bodies = Some(bodies);
        self
    }

    /// The describer of the text under the pages for assistive technology.
    #[must_use]
    pub fn access(mut self, describer: &'a dyn AccessDescriber) -> Self {
        self.describer = Some(describer);
        self
    }

    /// Shows the view in the room left in `ui`, and runs `body` for the app's canvas controls (margins, zones,
    /// marks, grids), which it shows with `show_at(ui, body.rect)`. `body` runs on every run of the canvas body (the measuring run and
    /// the real one), so it builds what it shows each time and consumes nothing.
    pub fn show(self, ui: &mut Ui, body: impl FnMut(&mut Ui, &PageBody<'_>)) -> PageViewOutput {
        let rect = ui.available_rect_before_wrap();
        self.run(ui, rect, true, body)
    }

    /// Shows the view at `rect` without allocating (a pane of split views).
    pub fn show_at(self, ui: &mut Ui, rect: Rect, body: impl FnMut(&mut Ui, &PageBody<'_>)) -> PageViewOutput {
        self.run(ui, rect, false, body)
    }

    fn run(self, ui: &mut Ui, rect: Rect, take_room: bool, mut body: impl FnMut(&mut Ui, &PageBody<'_>)) -> PageViewOutput {
        let PageView { label, source, state, options, navigation, bodies, describer, paper } = self;
        let spacing = PageMetrics::new().show_at(ui, Rect::from_min_size(rect.min, Vec2::ZERO)).spacing;
        let count = source.page_count();
        let sizes: Vec<Size> = (0..count).map(|page| source.page_size(page)).collect();
        let viewport = Size::new(f64::from(rect.width()), f64::from(rect.height()));
        let start = state.clone();
        let build = Geometry { sizes: &sizes, viewport, options, spacing, bodies, cache: RefCell::new(None) };
        let geo = build.of(&start);
        let frame = Frame::of(&geo);
        let given = frame.view(rect, start.offset, geo.scale / SCREEN_POINTS_PER_POINT);
        let mut navigation = navigation;
        navigation.overscroll = !start.continuous;
        let mut canvas = Canvas::new(label.clone()).view(given).navigation(navigation);
        if start.continuous {
            canvas = canvas.document(frame.document(&geo));
        }
        let mut held = Held::new(Bind::bound(state));
        let mut seen: Option<Seen> = None;
        let mut canvas_body = |ui: &mut Ui, cx: &mut BodyCx<'_>| {
            let next = run_pass(ui, cx, &mut held, &Inputs { source, describer, build: &build, rect, given, paper })?;
            body(ui, &PageBody { rect: cx.rect, pages: &next.pages, view: next.view });
            let decided = next.next.clone();
            seen = Some(next);
            decided
        };
        let mut shown = if take_room { canvas.show_at_taking_room(ui, rect, &mut canvas_body) } else { canvas.show_at(ui, rect, &mut canvas_body) };
        // Once per pass, after the real run: the memory and the state the pass decided are stored.
        shown.settle(|cx, decided| {
            if let Some(Next { started, state, mem, edit }) = decided.take() {
                *cx.memory::<Memory>() = mem;
                if state != started {
                    held.write(cx, &started, state, &edit);
                }
            }
        });
        let output = shown.output;
        let seen = seen.unwrap_or_else(|| Seen::empty(given, frame));
        let toggle_white_space = match output.press.phase {
            PressPhase::Release { start, pos, click: true, .. } => {
                start.clicks == 2 && seen.geo.layout.on_page_gap(seen.frame.layout_point(seen.view.to_doc(pos)))
            }
            _ => false,
        };
        PageViewOutput {
            canvas: output,
            pages: seen.pages,
            layout: seen.geo.layout.clone(),
            visible: seen.visible,
            current_changed: seen.current_changed,
            toggle_white_space,
            pasteboard: seen.pasteboard,
            frame: seen.frame,
        }
    }
}

/// What a pass reads besides the canvas body's context.
struct Inputs<'a> {
    source: &'a dyn PageSource,
    describer: Option<&'a dyn AccessDescriber>,
    build: &'a Geometry<'a>,
    rect: Rect,
    given: CanvasView,
    paper: DataColor,
}

/// What a pass found out, for the output.
struct Seen {
    geo: Arc<Placed>,
    frame: Frame,
    view: CanvasView,
    pages: Vec<PlacedPage>,
    visible: Range<usize>,
    current_changed: bool,
    pasteboard: Option<PasteboardOutput>,
    /// What the caller stores after the real run.
    next: Option<Next>,
}

/// The state and memory a pass decided, stored by `run` once per pass after the real run.
#[derive(Clone)]
struct Next {
    started: PageViewState,
    state: PageViewState,
    mem: Memory,
    edit: String,
}

impl Seen {
    /// A pass whose canvas body did not run (no area).
    fn empty(view: CanvasView, frame: Frame) -> Seen {
        Seen {
            geo: Arc::new(Placed { layout: PagedLayout::default(), scale: SCREEN_POINTS_PER_POINT, row: 0..0, doc: DocRect::ZERO }),
            frame,
            view,
            pages: Vec::new(),
            visible: 0..0,
            current_changed: false,
            pasteboard: None,
            next: None,
        }
    }
}

/// What a pass hands to the next, kept in the body's memory: the last layout, the offset the pass ended with (a
/// different one now means a tool, a scroll bar or the app moved the view) and the wheel gathered past the edge of a
/// row that pans.
#[derive(Clone, Debug, Default)]
struct Memory {
    layout: Option<(LayoutKey, Arc<Placed>)>,
    offset: Option<(f64, f64)>,
    overshoot: f64,
}

/// The mapping between the layout's screen points and the canvas view's document units: the view shows the shown
/// row (or the whole layout) as a document at the origin, one unit a screen point at 100 %.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    /// The layout point that is the document's origin.
    origin: Point,
    /// The zoom the layout was made at (screen points per document unit).
    zoom: f64,
}

impl Frame {
    fn of(geo: &Placed) -> Frame {
        Frame { origin: Point::new(geo.doc.x0, geo.doc.y0), zoom: geo.scale / SCREEN_POINTS_PER_POINT }
    }

    /// The document the view shows.
    fn document(&self, geo: &Placed) -> Size {
        Size::new(geo.doc.width() / self.zoom, geo.doc.height() / self.zoom)
    }

    /// The canvas view of `offset` (screen points) at `zoom`.
    fn view(&self, rect: Rect, offset: (f64, f64), zoom: f64) -> CanvasView {
        let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
        CanvasView::new(rect, Point::new((offset.0 + w / 2.0 - self.origin.x) / self.zoom, (offset.1 + h / 2.0 - self.origin.y) / self.zoom), zoom)
    }

    /// The scroll offset of a view centred at `center` (document units) at `zoom`, for a view `w` by `h` points.
    fn offset_of(&self, center: Point, w: f64, h: f64) -> (f64, f64) {
        (center.x * self.zoom + self.origin.x - w / 2.0, center.y * self.zoom + self.origin.y - h / 2.0)
    }

    /// A layout rect as document units.
    fn document_rect(&self, r: &DocRect) -> DocRect {
        DocRect::new(
            (r.x0 - self.origin.x) / self.zoom,
            (r.y0 - self.origin.y) / self.zoom,
            (r.x1 - self.origin.x) / self.zoom,
            (r.y1 - self.origin.y) / self.zoom,
        )
    }

    /// A document point as a layout point.
    fn layout_point(&self, p: Point) -> Point {
        Point::new(p.x * self.zoom + self.origin.x, p.y * self.zoom + self.origin.y)
    }
}

/// One pass of the canvas body: layout, scrolling, zoom and paging folded into the state; the pasteboard, the
/// pictures and the scroll bars drawn.
fn run_pass(ui: &mut Ui, cx: &mut BodyCx<'_>, held: &mut Held<'_, PageViewState>, inputs: &Inputs<'_>) -> Option<Seen> {
    let Inputs { source, describer, build, rect, given, paper } = *inputs;
    let mut state = held.read(cx)?;
    let started = state.clone();
    let shown = canvas_space(&cx.env)?.view;
    let env = Env::of(ui);
    let words = env.words::<PageViewWords>();
    let mut mem = cx.memory::<Memory>().clone();
    let (vw, vh) = (build.viewport.width, build.viewport.height);
    let count = build.sizes.len();
    let spacing = build.spacing;

    // The layout the canvas was given its view for: what this pass draws, whatever the pass makes of the state.
    let drawn = build.of(&state);
    let drawn_frame = Frame::of(&drawn);
    let mut geo = Arc::clone(&drawn);
    let started_on = state.current;

    // A scale the fit (a new viewport or pages) or the app changed keeps the page at the top of the view where it
    // was, unless the app moved the view in the same breath.
    let previous = mem.layout.take().map(|(_, placed)| placed);
    let unmoved = mem.offset == Some(state.offset);
    if let Some(previous) = previous.filter(|p| !scale_settled(p.scale, geo.scale)).filter(|_| unmoved && state.go_to.is_none())
        && let Some(offset) = anchored_offset(&previous, &geo, state.offset, vw)
    {
        state.offset = offset;
    }
    state.zoom = geo.scale / SCREEN_POINTS_PER_POINT;

    // The canvas folded the wheel, a pinch and the view commands into the view it shows: a different zoom is the
    // user's, anchored at the point that stayed still; a different centre is a scroll.
    let zooming = (shown.zoom - given.zoom).abs() > given.zoom * 1e-9;
    let mut want = state.offset;
    let mut wheel_left = 0.0;
    if zooming {
        let fixed = (shown.center - given.center) * (given.zoom * shown.zoom / (shown.zoom - given.zoom));
        let pointer = rect.center() + Vec2::new(fixed.x as f32, fixed.y as f32);
        geo = zoom_about(build, &mut state, &geo, rect, pointer, shown.zoom / given.zoom);
        want = state.offset;
    } else {
        // The canvas was given the view the pass started with, so its change of it is the pan; the state's offset
        // may have moved on since (a scale change anchors the page at the top), and the pan adds to that.
        let panned = drawn_frame.offset_of(shown.center, vw, vh);
        let given_at = drawn_frame.offset_of(given.center, vw, vh);
        let (along_x, along_y) = (panned.0 - given_at.0, panned.1 - given_at.1);
        if along_x.abs() > OFFSET_EPSILON || along_y.abs() > OFFSET_EPSILON {
            wheel_left = along_y;
            want = (state.offset.0 + along_x, state.offset.1 + along_y);
        }
    }
    if wheel_left == 0.0 {
        mem.overshoot = 0.0;
    }

    // A page that fits turns with the wheel; one panned to its edge turns with the wheel past it.
    if !state.continuous
        && wheel_left != 0.0
        && let Some(forward) = mem.turn(&geo, want.1, vh, notch())
    {
        let next = if forward {
            (geo.row.end < count).then_some(geo.row.end)
        } else {
            geo.row.start.checked_sub(1).map(|before| shown_row(count, before, &build.layout_options(&state)).start)
        };
        if let Some(next) = next {
            state.current = next;
            geo = build.of(&state);
            state.zoom = geo.scale / SCREEN_POINTS_PER_POINT;
            let (lo, hi) = offset_range(geo.doc.y0, geo.doc.y1, vh);
            want.1 = if forward { lo } else { hi };
        } else {
            want.1 = state.offset.1;
        }
    }
    let went = state.go_to.is_some();
    if let Some((page, fraction)) = state.go_to.take() {
        state.current = page.min(count.saturating_sub(1));
        geo = build.of(&state);
        state.zoom = geo.scale / SCREEN_POINTS_PER_POINT;
        want.1 = match geo.layout.rects.get(state.current) {
            Some(r) if state.continuous => {
                let (top, bottom) = geo.layout.bands[state.current];
                r.y0 + top + fraction * (bottom - top) - spacing.margin
            }
            _ => geo.doc.y0,
        };
    }

    // The pasteboard, the pictures and the scroll bars are drawn through the view the canvas shows; a zoom still
    // under way draws the layout the view was made for, scaled.
    let ratio = shown.zoom / drawn_frame.zoom;
    let seen = shown.visible();
    let in_view = DocRect::new(
        seen.x0 * drawn_frame.zoom + drawn_frame.origin.x,
        seen.y0 * drawn_frame.zoom + drawn_frame.origin.y,
        seen.x1 * drawn_frame.zoom + drawn_frame.origin.x,
        seen.y1 * drawn_frame.zoom + drawn_frame.origin.y,
    );
    let rows: Vec<usize> = drawn.row.clone().filter(|&p| overlaps(band_rect(&drawn.layout, p), in_view)).collect();
    let visible = match (rows.first(), rows.last()) {
        (Some(&first), Some(&last)) => first..last + 1,
        _ => 0..0,
    };
    let to_screen = |r: &DocRect| {
        let r = drawn_frame.document_rect(r);
        Rect::from_min_max(shown.to_screen(Point::new(r.x0, r.y0)), shown.to_screen(Point::new(r.x1, r.y1)))
    };
    let pages: Vec<PlacedPage> = rows
        .iter()
        .map(|&page| PlacedPage {
            page,
            rect: to_screen(&drawn.layout.rects[page]),
            scale: drawn.scale * ratio,
            band: (drawn.layout.bands[page].0 as f32 * ratio as f32, drawn.layout.bands[page].1 as f32 * ratio as f32),
        })
        .collect();
    let frames: Vec<PageFrame> = rows.iter().map(|&p| PageFrame::new(drawn_frame.document_rect(&band_rect(&drawn.layout, p))).fill(paper)).collect();
    let pasteboard = Pasteboard::new(words.pasteboard.clone(), &frames).show_at(ui, cx.rect);
    let near: Vec<(usize, Vec2)> = near_pages(&drawn, state.continuous, &visible, count, &build.layout_options(&state))
        .into_iter()
        .map(|page| {
            let size = drawn.layout.rects[page].size();
            (page, Vec2::new(size.width as f32, size.height as f32))
        })
        .collect();
    let pictures = PagePictures::new(source, &pages, &near, paper).label(words.text.clone()).hold(zooming || ratio != 1.0);
    match describer {
        Some(describer) => pictures.describer(describer).show_at(ui, cx.rect),
        None => pictures.show_at(ui, cx.rect),
    };

    // The scroll bars come before the clamp, so a drag of this pass lands in this pass.
    let document = drawn_frame.document(&drawn);
    let bars =
        CanvasScrollbars::new(words.scroll_bars.clone(), Bind::given(shown), DocRect::from_origin_size(Point::ZERO, document)).show_at(ui, cx.rect);
    if let Some(moved) = bars.view.value() {
        want = drawn_frame.offset_of(moved.center, vw, vh);
    }
    let (xs, ys) = (offset_range(geo.doc.x0, geo.doc.x1, vw), offset_range(geo.doc.y0, geo.doc.y1, vh));
    let offset = (want.0.clamp(xs.0, xs.1), want.1.clamp(ys.0, ys.1));
    state.offset = offset;
    if state.continuous && count > 0 && !went && mem.offset.is_some_and(|last| last != offset) {
        state.current = page_on_line(&geo.layout, offset.1 + vh / 2.0);
    }
    mem.offset = Some(offset);
    mem.layout = build.cache.borrow().clone();
    let current_changed = state.current != started_on;
    Some(Seen {
        geo: drawn,
        frame: drawn_frame,
        view: shown,
        pages,
        visible,
        current_changed,
        pasteboard: Some(pasteboard),
        next: Some(Next { started, state, mem, edit: words.edit.as_str().to_owned() }),
    })
}

/// A scroll smaller than this many screen points is the arithmetic of the view's round trip, not a scroll.
const OFFSET_EPSILON: f64 = 1e-6;

craft_ui::words! {
    /// The words of the page view.
    pub struct PageViewWords {
        pasteboard: "pageView.pasteboard" = "Pasteboard";
        scroll_bars: "pageView.scrollBars" = "Scroll bars";
        edit: "pageView.edit" = "Change view";
        text: "pageView.text" = "Page text";
    }
}

/// The wheel's notch in points: what gathered past a row's edge turns the page.
fn notch() -> f64 {
    match WheelNotch::DEFAULT {
        WheelNotch::Points(points) => f64::from(points),
        _ => 1.0,
    }
}

impl Memory {
    /// Gather `wanted_y` past the edge of the shown row (over passes) and say whether a page turns: forward past
    /// the bottom, back past the top.
    fn turn(&mut self, geo: &Placed, wanted_y: f64, view_height: f64, notch: f64) -> Option<bool> {
        let (lo, hi) = offset_range(geo.doc.y0, geo.doc.y1, view_height);
        let beyond = if wanted_y > hi {
            wanted_y - hi
        } else if wanted_y < lo {
            wanted_y - lo
        } else {
            0.0
        };
        let gathered = if beyond == 0.0 || self.overshoot * beyond < 0.0 { beyond } else { self.overshoot + beyond };
        if gathered.abs() >= notch {
            self.overshoot = 0.0;
            Some(gathered > 0.0)
        } else {
            self.overshoot = gathered;
            None
        }
    }
}

/// Two scales a pass apart that are the same scale (round trips through the zoom differ in the last bits).
fn scale_settled(before: f64, after: f64) -> bool {
    (before - after).abs() <= before.abs().max(after.abs()) * 1e-9
}

/// The layout of this pass: what [`place_pages`] and the state's fit give, the row shown and the scrollable document.
#[derive(Debug)]
struct Placed {
    layout: PagedLayout,
    /// Screen points per page point.
    scale: f64,
    /// The pages that show: all when continuous, else the current page or spread.
    row: Range<usize>,
    /// The scroll area in content coordinates: the whole layout, or the shown row with its margins.
    doc: DocRect,
}

/// What a layout is made from, so a pass with the same inputs reuses the last layout.
#[derive(Clone, Debug, PartialEq)]
struct LayoutKey {
    sizes: Vec<Size>,
    viewport: Size,
    options: LayoutOptions,
    spacing: PageSpacing,
    fit: Fit,
    /// The fixed zoom; a fit ignores it.
    zoom: Option<f64>,
    /// The current page, when one row shows (it picks the row and a single page's fit).
    current: Option<usize>,
    /// Each body, when white space is hidden.
    bodies: Option<Vec<(f64, f64)>>,
}

/// What a layout is made from.
struct Geometry<'a> {
    sizes: &'a [Size],
    viewport: Size,
    options: LayoutOptions,
    spacing: PageSpacing,
    bodies: Option<&'a [(f64, f64)]>,
    /// The last layout and its inputs.
    cache: RefCell<Option<(LayoutKey, Arc<Placed>)>>,
}

impl Geometry<'_> {
    fn layout_options(&self, state: &PageViewState) -> LayoutOptions {
        LayoutOptions { display: state.display, cover: state.cover, continuous: state.continuous, page_gaps: state.page_gaps, ..self.options }
    }

    fn key(&self, state: &PageViewState) -> LayoutKey {
        let options = self.layout_options(state);
        LayoutKey {
            sizes: self.sizes.to_vec(),
            viewport: self.viewport,
            options,
            spacing: self.spacing,
            fit: state.fit,
            zoom: (state.fit == Fit::None).then_some(state.zoom),
            current: (!options.continuous).then_some(state.current),
            bodies: self.bodies.filter(|_| state.hide_white_space).map(<[_]>::to_vec),
        }
    }

    /// The layout for `state`: the last one when nothing it is made from changed.
    fn of(&self, state: &PageViewState) -> Arc<Placed> {
        let key = self.key(state);
        if let Some((last, placed)) = &*self.cache.borrow()
            && *last == key
        {
            return Arc::clone(placed);
        }
        let placed = Arc::new(self.build(state));
        *self.cache.borrow_mut() = Some((key, Arc::clone(&placed)));
        placed
    }

    fn build(&self, state: &PageViewState) -> Placed {
        let options = self.layout_options(state);
        let scale = fit_scale(self.sizes, self.viewport, state.fit, &options, &self.spacing, Some(state.current))
            .unwrap_or(state.zoom * SCREEN_POINTS_PER_POINT)
            .clamp(ZOOM_RANGE.start() * SCREEN_POINTS_PER_POINT, ZOOM_RANGE.end() * SCREEN_POINTS_PER_POINT);
        let mut layout = place_pages(self.sizes, scale, self.viewport, &options, &self.spacing);
        if let Some(bodies) = self.bodies.filter(|_| state.hide_white_space) {
            layout.hide_white_space(bodies, scale, self.viewport.height, &self.spacing);
        }
        let count = self.sizes.len();
        let row = if state.continuous { 0..count } else { shown_row(count, state.current, &options) };
        let whole = DocRect::new(0.0, 0.0, layout.size.width, layout.size.height);
        let doc = if state.continuous || row.is_empty() {
            whole
        } else {
            let shown = row.clone().map(|p| band_rect(&layout, p)).reduce(|a, b| a.union(b)).unwrap_or(whole);
            DocRect::new(0.0, shown.y0 - self.spacing.margin, layout.size.width, shown.y1 + self.spacing.margin)
        };
        Placed { layout, scale, row, doc }
    }
}

/// The part of page `page` that shows, content coordinates.
fn band_rect(layout: &PagedLayout, page: usize) -> DocRect {
    let r = layout.rects[page];
    let (top, bottom) = layout.bands[page];
    DocRect::new(r.x0, r.y0 + top, r.x1, r.y0 + bottom)
}

fn overlaps(a: DocRect, b: DocRect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

/// The offsets a view `len` long can take over `lo..hi`; a document shorter than the view sits centred in it.
fn offset_range(lo: f64, hi: f64, len: f64) -> (f64, f64) {
    let spare = hi - lo - len;
    if spare <= 0.0 {
        let middle = lo + spare / 2.0;
        (middle, middle)
    } else {
        (lo, lo + spare)
    }
}

/// The offset that keeps the page at the top of the view where it was when the scale goes from `old`'s to `new`'s:
/// the page there keeps the same fraction of its height at the top and its middle in the middle of the view; in a
/// gap, the next page keeps its distance below the top. `None` when `new` has no such page.
fn anchored_offset(old: &Placed, new: &Placed, offset: (f64, f64), view_width: f64) -> Option<(f64, f64)> {
    let top = offset.1;
    let held = |page: usize| {
        let r = old.layout.rects[page];
        let band = band_rect(&old.layout, page);
        (band, r)
    };
    let pages = 0..old.layout.rects.len();
    let (page, fraction, below) = pages
        .clone()
        .find(|&p| held(p).0.y0 <= top && held(p).0.y1 > top)
        .map(|p| {
            let r = held(p).1;
            (p, if r.height() > 0.0 { ((top - r.y0) / r.height()).clamp(0.0, 1.0) } else { 0.0 }, 0.0)
        })
        .or_else(|| pages.clone().find(|&p| held(p).0.y0 >= top).map(|p| (p, 0.0, (held(p).1.y0 - top).max(0.0))))
        .or_else(|| pages.last().map(|p| (p, 1.0, (held(p).1.y1 - top).max(0.0))))?;
    let r = *new.layout.rects.get(page)?;
    Some((r.x0 + r.width() / 2.0 - view_width / 2.0, r.y0 + fraction * r.height() - below))
}

/// The page that owns the horizontal line `y` (content coordinates); a tie at a shared edge goes half a point to the
/// earlier page.
fn page_on_line(layout: &PagedLayout, y: f64) -> usize {
    let last = layout.rects.len().saturating_sub(1);
    layout.rects.iter().zip(&layout.bands).position(|(r, &(_, bottom))| r.y0 + bottom + 0.5 > y).unwrap_or(last)
}

/// Zoom by `delta` keeping the page point under `pointer` where it is.
fn zoom_about(build: &Geometry, state: &mut PageViewState, old: &Placed, rect: Rect, pointer: egui::Pos2, delta: f64) -> Arc<Placed> {
    let inside = (f64::from(pointer.x - rect.min.x), f64::from(pointer.y - rect.min.y));
    let at = Point::new(state.offset.0 + inside.0, state.offset.1 + inside.1);
    state.fit = Fit::None;
    state.zoom = (state.zoom * delta).clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
    let new = build.of(state);
    let (x, y) = match crate::layout::page_at(&old.layout, at) {
        Some(page) => {
            let (before, after) = (old.layout.rects[page], new.layout.rects[page]);
            (after.x0 + (at.x - before.x0) / before.width() * after.width(), after.y0 + (at.y - before.y0) / before.height() * after.height())
        }
        None => {
            let ratio = new.scale / old.scale;
            (at.x * ratio, at.y * ratio)
        }
    };
    state.offset = (x - inside.0, y - inside.1);
    new
}

/// The pages one row either side of what shows: the neighbouring rows, or the neighbouring row of a single shown
/// row.
fn near_pages(placed: &Placed, continuous: bool, visible: &Range<usize>, count: usize, options: &LayoutOptions) -> Vec<usize> {
    if continuous {
        if visible.is_empty() {
            return Vec::new();
        }
        let columns = placed.layout.columns.max(1);
        let before = visible.start.saturating_sub(columns)..visible.start;
        let after = visible.end..(visible.end + columns).min(count);
        before.chain(after).collect()
    } else {
        let before = placed.row.start.checked_sub(1).map(|p| shown_row(count, p, options));
        let after = (placed.row.end < count).then(|| shown_row(count, placed.row.end, options));
        before.into_iter().chain(after).flatten().collect()
    }
}
