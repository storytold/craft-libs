//! `PageView`: a scrolling view of a document's pages, laid out by [`crate::layout`], drawn on a craft-ui [`Canvas`]
//! through its [`Pasteboard`] page frames, with the page pictures the app renders through craft-ui's texture
//! contract ([`ThumbSource`]).
//!
//! The pre-draw callback registers canvas controls and runs the app's tool host. Its returned view is folded in
//! before the page pictures and the overlay callback are drawn.
//!
//! ```ignore
//! PageView::new("Pages", &doc_pages, &mut view_state, paper).show_with(ui, &mut tool,
//!     |ui, body, tool| body.pass.and_then(|pass| ToolHost::new().run(ui, pass, tool).view),
//!     |ui, body, _| { TextMarks::new(body.pages, &marks).show_at(ui, body.rect); });
//! ```

mod pager;
pub(crate) mod pictures;
#[cfg(feature = "suite")]
pub(crate) mod stories;

use self::pager::wheel_turn;
use self::pictures::{PageMetrics, PagePictures, page_stores, picture_options};
use super::access::AccessDescriber;
use crate::layout::{
    Fit, LayoutOptions, PageDisplay, PageSpacing, PagedLayout, SCREEN_POINTS_PER_POINT, ViewDefaults, ZOOM_RANGE, fit_scale, place_pages, shown_row,
};
use craft_behavior::geometry::{SnapOptions, SnapTarget, SnapTargets};
use craft_behavior::kurbo::{Point, Rect as DocRect, Size};
use craft_behavior::{Axis, CanvasView, NavMove, PressPhase, ZoomScheme};
use craft_keymap::Mods;
use craft_ui::canvas::{
    BodyKeys, Canvas, CanvasOutput, CanvasScrollbars, CanvasSpace, NavigationOptions, PageBorder, PageFrame, Pasteboard, PasteboardColor,
    PasteboardOutput, Rulers, Snap, ToolPass, ToolRegion, canvas_space, pixel_aligned, tool_pass, with_canvas_space,
};
use craft_ui::contract::{Bind, BodyCx, Held, Props as _, Show as _, ShowContainer as _};
use craft_ui::primitives::textures::{Priority, TextureState, TextureStatistics, ThumbRequest, ThumbSource};
use craft_ui::primitives::{EditorPurpose, PaintSlot};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::focus::{EditSet, NudgeMods};
use craft_ui::runtime::memory::Remember as _;
use craft_ui::style::DataColor;
use egui::{Rect, Ui, Vec2, pos2};
use std::cell::RefCell;
use std::ops::Range;
use std::sync::Arc;

/// The document and page revisions represented by a page picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PageRevision {
    /// The revision of the document containing the page.
    pub document: u64,
    /// The revision of the individual page.
    pub page: u64,
}

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

    /// The document and page revision of `page`, when the source tracks them. The source's texture revision must
    /// change whenever this revision changes, so a ready picture corresponds to this stamp.
    fn page_revision(&self, _page: usize) -> Option<PageRevision> {
        None
    }

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
    /// agree on it. Each pass turns any other value into one of the four (`450` is `90`), so a caller may add `90`
    /// to the value it reads. A pass that finds the rotation, `display`, `continuous`, `cover` or `fit` other than
    /// the layout it placed last keeps the current page in view, as a go-to to its top.
    pub rotation: u16,
    /// Where the next pass takes the view. A pass with no pointer zoom takes it and clears it; while a pointer zoom
    /// runs (a wheel or pinch zoom, or its ease still arriving on screen) each pass places it again at the new scale
    /// and keeps it, so the zoom stays anchored on it, and the first pass without one clears it.
    #[serde(skip)]
    pub go_to: Option<GoTo>,
    /// A page or spread the next pass turns to: forward when `true`; that pass clears it.
    #[serde(skip)]
    pub turn: Option<bool>,
}

/// Where the creative set's page view ([`ComponentSet::Creative`](craft_theme::ComponentSet::Creative)) takes its
/// spacing from. An app sets it once at its root (`RootEnv::values`); the productivity set's spacing is its own either
/// way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, craft_ui::runtime::env::EnvValue, craft_ui::contract::MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum PageInsets {
    /// The section gap between pages, the roomy pad above and below, the rail's width at the sides.
    #[default]
    Standard,
    /// The theme's page insets (`space.inset.page_gap`, `page_margin` and `page_side`): PdfCraft's page view.
    Theme,
}

/// Where Go To ([`GoTo::Top`]) puts a page's top in the view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum GoToPlace {
    /// The view's top where what shows of the row above ends (the document's start for the first row), so the space
    /// the layout keeps above the page shows and nothing of the page before it; a fraction of the page goes to the
    /// view's top.
    #[default]
    AfterRowAbove,
    /// PdfCraft's, as production places it: the page gap above the page's top, or above the fraction of it, never
    /// above the document's start.
    GapAbove,
}

/// Where the next pass takes the view.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum GoTo {
    /// `page` at the top of the view, with `fraction` of its height above the top.
    Top {
        /// The page.
        page: usize,
        /// The fraction of the page's height to put at the top.
        fraction: f64,
    },
    /// The point `at` on `page`, placed at `view` times the viewport; a NaN axis keeps that axis's offset.
    Point {
        /// The page.
        page: usize,
        /// The point on the page, in page points.
        at: Point,
        /// Where in the viewport the point goes, as fractions of its width and height.
        view: (f64, f64),
    },
    /// `rect` of `page` fitted into the viewport with `margin` around it and centred: the pass sets the zoom from its own
    /// viewport and `PageSpacing.side`, clamped to `ZOOM_RANGE`.
    Rect {
        /// The page.
        page: usize,
        /// The rect on the page, in page points.
        rect: DocRect,
        /// The margin around the rect, in screen points.
        margin: f64,
    },
    /// The point `at` on `page`, placed at the viewport's start corner with the side gap.
    Corner {
        /// The page.
        page: usize,
        /// The point on the page, in page points.
        at: Point,
    },
}

impl GoTo {
    /// The page it goes to.
    pub const fn page(&self) -> usize {
        match *self {
            GoTo::Top { page, .. } | GoTo::Point { page, .. } | GoTo::Rect { page, .. } | GoTo::Corner { page, .. } => page,
        }
    }
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
            turn: None,
        }
    }

    /// Ask the next pass to bring `page` to the top of the view (page box, outline, thumbnails, find), and make it
    /// current now.
    pub fn go_to(&mut self, page: usize) {
        self.go_to = Some(GoTo::Top { page, fraction: 0.0 });
        self.current = page;
    }

    /// Ask the next pass to place the point `at` on `page` at `view` times the viewport; a NaN axis keeps that axis's
    /// offset.
    pub fn go_to_point(&mut self, page: usize, at: Point, view: (f64, f64)) {
        self.go_to = Some(GoTo::Point { page, at, view });
        self.current = page;
    }

    /// Ask the next pass to bring `fraction` of `page`'s height to the top of the view.
    pub fn go_to_fraction(&mut self, page: usize, fraction: f64) {
        self.go_to = Some(GoTo::Top { page, fraction });
        self.current = page;
    }

    /// Turns one page in a single-page view and one spread in Two-up, forward or back; past either end it does
    /// nothing.
    ///
    /// The next pass turns: it knows the page count and the spreads. Forward from the cover goes to page 1,
    /// otherwise to the first page after the current spread; back goes to the first page of the spread before.
    pub fn step(&mut self, forward: bool) {
        self.turn = Some(forward);
    }

    /// Zoom In / Zoom Out: the next rung of `scheme` above (or below) the shown zoom, within [`ZOOM_RANGE`]; a fit
    /// ends. The zoom a fit showed is the start.
    pub fn zoom_step(&mut self, larger: bool, scheme: &ZoomScheme) {
        self.fit = Fit::None;
        self.zoom = scheme.step_in_range(self.zoom, if larger { 1 } else { -1 }, ZOOM_RANGE);
    }

    /// The state as a pass reads it: the rotation one of the four quarter turns (rounded down), the zoom within
    /// [`ZOOM_RANGE`] (a zoom that is no number is 100 %), so the layout never hides a field out of range.
    fn normalise(&mut self) {
        self.rotation = self.rotation % 360 / 90 * 90;
        self.zoom = if self.zoom.is_finite() { self.zoom.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end()) } else { 1.0 };
    }
}

/// One page on screen this pass, for overlays: its index, screen rectangle and scale.
#[derive(Clone, Debug, PartialEq)]
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
    /// What the page's picture shows this pass; `Missing` during the geometry-only pre-draw callback.
    pub picture: TextureState,
    /// The source's document and page revision when `picture` is ready; `None` for a stand-in or an unknown stamp.
    pub picture_revision: Option<PageRevision>,
}

impl PlacedPage {
    /// Page `page` at `rect` on screen, `scale` screen points to a page point, with `band` the visible part of its height in
    /// screen points; its picture `Missing` until [`showing`](Self::showing) says what it shows.
    pub const fn new(page: usize, rect: Rect, scale: f64, band: (f32, f32)) -> Self {
        PlacedPage { page, rect, scale, band, picture: TextureState::Missing, picture_revision: None }
    }

    /// The page with `picture` as what its picture shows this pass.
    #[must_use]
    pub fn showing(mut self, picture: TextureState) -> Self {
        self.picture = picture;
        self
    }

    /// Page point (origin top-left, y down) to screen.
    pub fn to_screen(&self, p: Point) -> egui::Pos2 {
        pos2(self.rect.min.x + (p.x * self.scale) as f32, self.rect.min.y + (p.y * self.scale) as f32)
    }

    /// Screen to page point.
    pub fn to_page(&self, p: egui::Pos2) -> Point {
        Point::new(f64::from(p.x - self.rect.min.x) / self.scale, f64::from(p.y - self.rect.min.y) / self.scale)
    }

    /// Page point `p` to document coordinates of `view`.
    pub fn to_doc(&self, view: &CanvasView, p: Point) -> Point {
        view.to_doc(self.to_screen(p))
    }

    /// Document point `d` of `view` to page coordinates.
    pub fn from_doc(&self, view: &CanvasView, d: Point) -> Point {
        self.to_page(view.to_screen(d))
    }
}

/// What the page view's canvas body hands the app's canvas controls (margins, zones, marks, grids, cell overlay,
/// text marks).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct PageBody<'b> {
    /// The canvas body's rect; with [`PageView::rulers`] it also covers the strips along the top and left.
    pub rect: Rect,
    /// The pages on screen this pass.
    pub pages: &'b [PlacedPage],
    /// The view the canvas shows this pass.
    pub view: CanvasView,
    /// The canvas's tool input of this pass; `None` in a measuring run. Run the tool host only in the pre-draw callback.
    pub pass: Option<&'b ToolPass>,
}

impl PageBody<'_> {
    /// The page under document point `d` and the point in its coordinates; the nearest page when `d` is outside all.
    pub fn page_at_doc(&self, d: Point) -> Option<(&PlacedPage, Point)> {
        let screen = self.view.to_screen(d);
        // The squared distance from the point to a page's rectangle; zero inside it. A strictly smaller distance wins,
        // so a tie goes to the earlier page.
        let distance = |placed: &PlacedPage| {
            let dx = (placed.rect.min.x - screen.x).max(0.0).max(screen.x - placed.rect.max.x);
            let dy = (placed.rect.min.y - screen.y).max(0.0).max(screen.y - placed.rect.max.y);
            f64::from(dx) * f64::from(dx) + f64::from(dy) * f64::from(dy)
        };
        let mut nearest: Option<(&PlacedPage, f64)> = None;
        for placed in self.pages {
            let away = distance(placed);
            if nearest.is_none_or(|(_, best)| away < best) {
                nearest = Some((placed, away));
            }
        }
        nearest.map(|(placed, _)| (placed, placed.from_doc(&self.view, d)))
    }
}

/// The snapping of the coordinates the pointer sets on the view ([`PageView::snap`]), over targets on the pages in
/// each page's own points: the view places them where their page shows in each pass.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PageSnap {
    options: SnapOptions,
    pages: Vec<(usize, Vec<SnapTarget>)>,
}

impl PageSnap {
    /// Snapping with `options` and no targets yet.
    pub fn new(options: SnapOptions) -> Self {
        PageSnap { options, pages: Vec::new() }
    }

    /// Adds `targets` on page `page`, in the page's own points from its top left corner, their spans included.
    #[must_use]
    pub fn page(mut self, page: usize, targets: Vec<SnapTarget>) -> Self {
        self.pages.push((page, targets));
        self
    }
}

/// An area of a page where the active tool takes a press over the canvas controls ([`PageView::tool_regions`]), in the
/// page's own points: the view places it where its page shows in each pass.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PageRegion {
    page: usize,
    points: Vec<Point>,
}

impl PageRegion {
    /// The polygon with the corners `points` on page `page`, in the page's own points from its top left corner.
    pub fn polygon(page: usize, points: Vec<Point>) -> Self {
        PageRegion { page, points }
    }
}

/// What the view did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct PageViewOutput {
    /// The canvas this pass, for the app's `Rulers` and key handling.
    pub canvas: CanvasOutput,
    /// The pages drawn this pass, in order: the page rectangles overlays and the access page window read.
    pub pages: Vec<PlacedPage>,
    /// The whole layout, in screen points from the content's top-left.
    pub layout: PagedLayout,
    /// The pages at least partly in view.
    pub visible: Range<usize>,
    /// The current page changed (scrolling, the wheel turning a page, a tool's view).
    pub current_changed: bool,
    /// The reader scrolled the view this pass: the wheel (its scroll, or the page it turned), a drag or a press on a
    /// scroll bar, or the view's page and arrow keys. A zoom, a go-to and a view the app or a tool set are not the
    /// reader's scroll, so an app that follows something with the view (search results) stops following on this.
    pub scrolled: bool,
    /// A double-click on the gap between pages: the app toggles Hide White Space (Word).
    pub toggle_white_space: bool,
    /// The pasteboard outside the pages (`None` when the pass drew nothing): a secondary press or the Menu key there asks the app for Photoshop's
    /// pasteboard colour menu at [`PasteboardOutput::context_at`]; a page name click is not used here.
    pub pasteboard: Option<PasteboardOutput>,
    /// What the page and detail picture stores hold and did this pass, summed field by field.
    pub textures: TextureStatistics,
    /// The spacing the pass laid the pages out with.
    pub spacing: PageSpacing,
    resident: usize,
}

impl PageViewOutput {
    /// How many page pictures the view holds after this pass: its pages' own pictures and the pictures of the part
    /// in view of pages larger than the texture side, shown or kept for later, as their stores count them. A page
    /// scrolled out of [`PageView::picture_reach`] gives its pictures back, and the stores evict the pictures drawn
    /// longest ago past their budget, so the count stays bounded by the pages near the view however many pages the
    /// view scrolls through. 0 for a pass that drew nothing.
    pub fn resident_pictures(&self) -> usize {
        self.resident
    }
}

/// The page view, built per pass over the app's [`PageSource`] and [`PageViewState`].
///
/// - Layout: [`place_pages`] at the state's zoom, or the fit's ([`fit_scale`]), with the [`PageSpacing`] of the
///   view's style, recomputed when the viewport or the pages change; the zoom is written back so it equals what
///   shows. Pages draw as [`PageFrame`]s on a [`Pasteboard`], the page picture over each through `Thumbnails` at
///   `Priority::VISIBLE`, one row either side at `Priority::NEAR`; a page larger than the texture side adds a
///   second picture of the part in view. Sharp renders wait while a zoom gesture or ease runs, and those of the part
///   in view while the view pans too. While the view scrolls, the two rows ahead are asked for instead. A page
///   scrolled further than [`picture_reach`](Self::picture_reach) from the view gives its picture back.
/// - Layout: a scale the fit or the app changes keeps the document point at the centre of the view in place; a new
///   room keeps the current page's top where it was. Pages (a page or spread, or a scrolling column) shorter than
///   the view without their margins sit in its middle. A zoom about the pointer keeps the point under it in place
///   relative to the page there (or, in a gap, the nearest page), and one that does not ease draws its new layout,
///   reflowed, in the pass it happens, so the next pass moves nothing.
/// - Keys: the arrows go to the app's pre-draw tool host first ([`PageBody::pass`]); the view moves with those it leaves, in
///   the same pass.
/// - Scrolling: the canvas folds in the wheel (the wheel scrolls, Shift or Ctrl/Cmd + wheel scrolls sideways, Alt +
///   wheel and a pinch zoom about the pointer, as [`NavigationOptions`] say) and the view commands; the view
///   reads the result back into its state. [`CanvasScrollbars`] land in their own pass. Not continuous, a page
///   that fits turns with the wheel instead, and one zoomed in far enough to pan pans and turns the page at its
///   edge. Space-drag and the Hand and Zoom tools run in the app's pre-draw `ToolHost` ([`PageBody::pass`]).
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
    border: PageBorder,
    pasteboard: PasteboardColor,
    keep: &'a [usize],
    editor: Option<EditorPurpose>,
    inactive_edits: EditSet,
    snap: Option<PageSnap>,
    tool_regions: Vec<PageRegion>,
    rulers: bool,
    reach: f32,
    go_to_place: GoToPlace,
    fill_screen: bool,
}

/// How far from the view, in view heights above and below it (and one view width either side), the page view keeps a
/// page's picture once it has scrolled out: three, as WordCraft kept its page textures (#811).
pub const PICTURE_REACH: f32 = 3.0;

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
            border: PageBorder::DropShadow,
            pasteboard: PasteboardColor::Default,
            keep: &[],
            editor: None,
            inactive_edits: EditSet::NONE,
            snap: None,
            tool_regions: Vec::new(),
            rulers: false,
            reach: PICTURE_REACH,
            go_to_place: GoToPlace::AfterRowAbove,
            fill_screen: false,
        }
    }

    /// Where Go To puts a page's top in the view: after the row above by default, the page gap above it with
    /// [`GoToPlace::GapAbove`] (PdfCraft's).
    #[must_use]
    pub fn go_to_place(mut self, place: GoToPlace) -> Self {
        self.go_to_place = place;
        self
    }

    /// Full screen's edge-to-edge view: no margin above the first page or below the last, no side gutter, no page
    /// border or shadow, and in Two-up that is not continuous the shown spread fitted by its own pages (two pages'
    /// widths side by side, a page alone keeping a spread's room) and centred. Off by default.
    #[must_use]
    pub fn fill_screen(mut self, on: bool) -> Self {
        self.fill_screen = on;
        self
    }

    /// The colour round the pages: the theme's by default; production's full screen view is black.
    #[must_use]
    pub fn pasteboard(mut self, colour: PasteboardColor) -> Self {
        self.pasteboard = colour;
        self
    }

    /// The border round each page; a drop shadow by default, and none with the page gaps off.
    #[must_use]
    pub fn border(mut self, border: PageBorder) -> Self {
        self.border = border;
        self
    }

    /// The pages whose pictures the view keeps loaded whether or not they are in view.
    #[must_use]
    pub fn keep(mut self, pages: &'a [usize]) -> Self {
        self.keep = pages;
        self
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

    /// Registers the canvas's editor part of `purpose`; the view's keys give it none.
    #[must_use]
    pub fn editor(mut self, purpose: EditorPurpose) -> Self {
        self.editor = Some(purpose);
        self
    }

    /// Edit commands the view's editor part cannot act on right now, forwarded to [`Canvas::inactive_edits`].
    #[must_use]
    pub fn inactive_edits(mut self, inactive: EditSet) -> Self {
        self.inactive_edits = inactive;
        self
    }

    /// The snapping of the coordinates the pointer sets on the view, with its targets in page points. The view places
    /// them on the pages that show this pass, drops those of pages outside the shown row and passes the result to the
    /// inner canvas.
    #[must_use]
    pub fn snap(mut self, snap: Option<PageSnap>) -> Self {
        self.snap = snap;
        self
    }

    /// The areas where the active tool takes a press over the canvas controls, in page points. The view places them
    /// as [`snap`](Self::snap)'s targets and passes them to the inner canvas.
    #[must_use]
    pub fn tool_regions(mut self, regions: Vec<PageRegion>) -> Self {
        self.tool_regions = regions;
        self
    }

    /// Leaves room for rulers along the top and left edges, which insets the view: the canvas shows inside the strips
    /// and the app shows its rulers (`PageRulers`) over [`PageBody::rect`], which still covers the strips.
    pub fn rulers(mut self, on: bool) -> Self {
        self.rulers = on;
        self
    }

    /// How far from the view a page's picture is kept once it has scrolled out, in view heights above and below the
    /// view; sideways the view keeps one view width either side. A page further away loses its picture (and the
    /// picture of its part in view) in the pass it leaves that reach and is rendered again when it comes back, so the
    /// pictures held are bounded by the pages near the view, not only by the stores' budget. The pages drawn and the
    /// pages asked for ahead (one row either side) are always kept. [`PICTURE_REACH`] by default; a negative or
    /// non-finite reach is taken as 0.
    #[must_use]
    pub fn picture_reach(mut self, screens: f32) -> Self {
        self.reach = if screens.is_finite() { screens.max(0.0) } else { 0.0 };
        self
    }

    /// Shows the view in the room left in `ui`, and runs `body` for the app's overlays.
    /// `body` runs on every run of the canvas body (the measuring run and the real one), so it builds what it shows
    /// each time and consumes nothing.
    pub fn show(self, ui: &mut Ui, mut body: impl FnMut(&mut Ui, &PageBody<'_>)) -> PageViewOutput {
        let rect = ui.available_rect_before_wrap();
        self.run(ui, rect, true, &mut (), |_, _, _| None, |ui, page, _| body(ui, page))
    }

    /// Shows the view at `rect` without allocating (a pane of split views).
    pub fn show_at(self, ui: &mut Ui, rect: Rect, mut body: impl FnMut(&mut Ui, &PageBody<'_>)) -> PageViewOutput {
        self.run(ui, rect, false, &mut (), |_, _, _| None, |ui, page, _| body(ui, page))
    }

    /// Shows the view at `rect`, registering controls and running tools in `before` before drawing.
    /// Return the view reported by the tool host; `body` receives the resulting placement for overlays. The canvas's
    /// own view, which the host returns when no tool changed it, is applied only once.
    /// Both callbacks borrow `session` in sequence and run on measuring and real passes.
    pub fn show_at_with<T>(
        self,
        ui: &mut Ui,
        rect: Rect,
        session: &mut T,
        before: impl FnMut(&mut Ui, &PageBody<'_>, &mut T) -> Option<CanvasView>,
        body: impl FnMut(&mut Ui, &PageBody<'_>, &mut T),
    ) -> PageViewOutput {
        self.run(ui, rect, false, session, before, body)
    }

    /// Shows the view in the available room with a pre-draw tool callback and an overlay callback.
    pub fn show_with<T>(
        self,
        ui: &mut Ui,
        session: &mut T,
        before: impl FnMut(&mut Ui, &PageBody<'_>, &mut T) -> Option<CanvasView>,
        body: impl FnMut(&mut Ui, &PageBody<'_>, &mut T),
    ) -> PageViewOutput {
        let rect = ui.available_rect_before_wrap();
        self.run(ui, rect, true, session, before, body)
    }

    fn run<T>(
        self,
        ui: &mut Ui,
        rect: Rect,
        take_room: bool,
        session: &mut T,
        mut before: impl FnMut(&mut Ui, &PageBody<'_>, &mut T) -> Option<CanvasView>,
        mut body: impl FnMut(&mut Ui, &PageBody<'_>, &mut T),
    ) -> PageViewOutput {
        let PageView {
            label,
            source,
            state,
            options,
            navigation,
            bodies,
            describer,
            paper,
            border,
            pasteboard,
            keep,
            editor,
            inactive_edits,
            snap,
            tool_regions,
            rulers,
            reach,
            go_to_place,
            fill_screen,
        } = self;
        let mut spacing = PageMetrics::new().show_at(ui, Rect::from_min_size(rect.min, Vec2::ZERO)).spacing;
        // Filling the screen, the pages keep no margin above or below and no gutter beside them, and draw no border.
        let border = if fill_screen {
            spacing.margin = 0.0;
            spacing.side = 0.0;
            PageBorder::None
        } else {
            border
        };
        let count = source.page_count();
        let sizes: Vec<Size> = (0..count).map(|page| source.page_size(page)).collect();
        // With rulers the canvas shows inside the strips, so its view lies inside them. Its body is clipped to its view
        // (`ContainerProps::scroll_owner`), so the app's body runs with the clip opened over the strips again, where
        // its `PageRulers` show. The canvas's own `with_rulers` would show the general rulers there instead.
        let (canvas_rect, strips_clip) = if rulers {
            let (top, left, _) = Rulers::strips(ui, rect);
            let inner = Rect::from_min_max(pos2(left.max.x.min(rect.max.x), top.max.y.min(rect.max.y)), rect.max);
            (inner, Some(ui.clip_rect().intersect(rect)))
        } else {
            (rect, None)
        };
        // One rect for the canvas view and the page offset: the layout, the given view and every clamp use the inset one.
        let full = rect;
        let rect = canvas_rect;
        let viewport = Size::new(f64::from(rect.width()), f64::from(rect.height()));
        // The canvas is given the view of the state as the pass will read it.
        let mut start = state.clone();
        start.normalise();
        let build = Geometry { sizes: &sizes, viewport, options, spacing, bodies, go_to_place, fill_screen, cache: RefCell::new(None) };
        let geo = build.of(&start);
        let frame = Frame::of(&geo);
        let given = frame.view(rect, start.offset, geo.scale / SCREEN_POINTS_PER_POINT);
        let mut navigation = navigation;
        // The canvas keeps the view inside the shown row, as the pass does, so a wheel share that cannot pan a row at its
        // edge leaves the gesture unmoved and a page turn can follow it.
        navigation.overscroll = false;
        // The app's page geometry, in page points, placed where its page shows in this pass's layout.
        let regions: Vec<ToolRegion> = tool_regions.iter().filter_map(|region| placed_region(region, &geo, &frame)).collect();
        let mut canvas = Canvas::new(label.clone())
            .view(given)
            .navigation(navigation)
            .keys_to_body(true)
            .body_nudges(KeyMoves::NUDGES)
            .pasteboard(pasteboard)
            .tool_regions(regions)
            .document(frame.document(&geo));
        if let Some(purpose) = editor {
            canvas = canvas.editor(purpose).inactive_edits(inactive_edits);
        }
        if let Some(snap) = &snap {
            canvas = canvas.snap(placed_snap(snap, &geo, &frame));
        }
        let mut held = Held::new(Bind::bound(state));
        let mut seen: Option<Seen> = None;
        let mut canvas_body = |ui: &mut Ui, cx: &mut BodyCx<'_>| {
            let inputs =
                Inputs { source, describer, build: &build, rect, given, paper, navigation, border, keep, reach, strips: strips_clip.map(|_| full) };
            let (mut next, pending) = run_pass(ui, cx, &mut held, &inputs, |ui, page| before(ui, page, session))?;
            let pass = tool_pass(&cx.env);
            if let Some(clip) = strips_clip {
                ui.set_clip_rect(clip);
            }
            let body_rect = inputs.body_rect(cx.rect);
            with_canvas_space(ui, pending.space, |ui| {
                body(ui, &PageBody { rect: body_rect, pages: &next.pages, view: next.view, pass: pass.as_ref() }, session)
            });
            // The body's tool host took the arrows a tool claims; the view moves with the keys it left.
            pending.finish(ui.ctx(), &mut next, &inputs);
            let decided = next.next.clone();
            seen = Some(next);
            decided
        };
        let mut shown =
            if take_room { canvas.show_at_taking_room(ui, canvas_rect, &mut canvas_body) } else { canvas.show_at(ui, canvas_rect, &mut canvas_body) };
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
            scrolled: seen.scrolled,
            toggle_white_space,
            pasteboard: seen.pasteboard,
            textures: seen.textures,
            spacing,
            resident: seen.resident,
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
    navigation: NavigationOptions,
    border: PageBorder,
    keep: &'a [usize],
    /// `PageView::picture_reach`.
    reach: f32,
    /// With `PageView::rulers`, the view's whole rect, strips included; the canvas shows inside the strips.
    strips: Option<Rect>,
}

impl Inputs<'_> {
    /// The rect the app's callbacks get as [`PageBody::rect`]: the canvas body's, and with rulers the strips too.
    fn body_rect(&self, canvas: Rect) -> Rect {
        self.strips.map_or(canvas, |full| Rect::from_min_max(full.min, canvas.max))
    }
}

/// What a pass found out, for the output.
struct Seen {
    geo: Arc<Placed>,
    frame: Frame,
    view: CanvasView,
    pages: Vec<PlacedPage>,
    visible: Range<usize>,
    current_changed: bool,
    scrolled: bool,
    pasteboard: Option<PasteboardOutput>,
    textures: TextureStatistics,
    /// The page pictures held.
    resident: usize,
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
            geo: Arc::new(Placed {
                layout: PagedLayout::default(),
                scale: SCREEN_POINTS_PER_POINT,
                row: 0..0,
                doc: DocRect::ZERO,
                pages: DocRect::ZERO,
            }),
            frame,
            view,
            pages: Vec::new(),
            visible: 0..0,
            current_changed: false,
            scrolled: false,
            pasteboard: None,
            textures: TextureStatistics::default(),
            resident: 0,
            next: None,
        }
    }
}

/// What a pass hands to the next, kept in the body's memory: the last layout and what it was made from (a state
/// that differs from it now means the app changed the display), and the offset the pass ended with (a different one
/// now means a tool, a scroll bar or the app moved the view).
#[derive(Clone, Debug, Default)]
struct Memory {
    layout: Option<(LayoutKey, Arc<Placed>)>,
    offset: Option<(f64, f64)>,
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

    /// A layout point as a document point.
    fn document_point(&self, l: Point) -> Point {
        Point::new((l.x - self.origin.x) / self.zoom, (l.y - self.origin.y) / self.zoom)
    }
}

/// The document point of `p`, a point of page `page` in the page's own points; `None` for a page outside the row
/// `geo` shows.
fn page_to_document(geo: &Placed, frame: &Frame, page: usize, p: Point) -> Option<Point> {
    if !geo.row.contains(&page) {
        return None;
    }
    let origin = geo.layout.rects.get(page)?.origin();
    Some(frame.document_point(Point::new(origin.x + p.x * geo.scale, origin.y + p.y * geo.scale)))
}

/// The snapping over `snap`'s targets on the pages `geo` shows, in `frame`'s document units.
fn placed_snap(snap: &PageSnap, geo: &Placed, frame: &Frame) -> Snap {
    let mut targets = SnapTargets::new();
    for (page, list) in &snap.pages {
        let to_doc = |p: Point| page_to_document(geo, frame, *page, p);
        for target in list {
            // A line along Y snaps x and spans y; one along X snaps y and spans x. The mapping is separate per axis.
            let vertical = target.along == Axis::Y;
            let across = |v: f64| to_doc(if vertical { Point::new(v, 0.0) } else { Point::new(0.0, v) }).map(|d| if vertical { d.x } else { d.y });
            let along = |v: f64| to_doc(if vertical { Point::new(0.0, v) } else { Point::new(v, 0.0) }).map(|d| if vertical { d.y } else { d.x });
            let Some(position) = across(target.position) else { break };
            let mut placed = *target;
            placed.position = position;
            placed.span = target.span.and_then(|(a, b)| Some((along(a)?, along(b)?)));
            targets.add(placed);
        }
    }
    Snap::new(Arc::new(targets), snap.options)
}

/// `region` on the page it names, in `frame`'s document units; `None` for a page outside the row `geo` shows.
fn placed_region(region: &PageRegion, geo: &Placed, frame: &Frame) -> Option<ToolRegion> {
    let points = region.points.iter().map(|p| page_to_document(geo, frame, region.page, *p)).collect::<Option<Vec<Point>>>()?;
    Some(ToolRegion::polygon(points))
}

/// One pass of the canvas body up to the app's body: layout, the wheel, a pinch and a pan folded into the state; the
/// pasteboard, the pictures and the scroll bars drawn. The keys, the turns and the go-to wait for [`Pending::finish`],
/// after the app's body.
fn run_pass(
    ui: &mut Ui,
    cx: &mut BodyCx<'_>,
    held: &mut Held<'_, PageViewState>,
    inputs: &Inputs<'_>,
    mut before: impl FnMut(&mut Ui, &PageBody<'_>) -> Option<CanvasView>,
) -> Option<(Seen, Pending)> {
    let Inputs { source, describer, build, rect, given, paper, navigation, border, keep, reach, strips: _ } = *inputs;
    let mut state = held.read(cx)?;
    let started = state.clone();
    state.normalise();
    let mut space = canvas_space(&cx.env)?;
    // The canvas's settled view decides the zoom and the pan; the view on screen, part way there while a zoom eases,
    // draws.
    let (target, shown) = (space.target(), space.view);
    let wheel = space.wheel();
    let env = Env::of(ui);
    let words = env.words::<PageViewWords>();
    let mut mem = cx.memory::<Memory>().clone();
    let (vw, vh) = (build.viewport.width, build.viewport.height);
    let count = build.sizes.len();

    // A display, continuous, cover, fit or rotation other than the layout placed last keeps the current page in
    // view. Writing a field to the value it has changes nothing, and the first pass has no layout to differ from.
    let previous = mem.layout.take();
    if state.go_to.is_none() && previous.as_ref().is_some_and(|(key, _)| key.display_differs(&state)) {
        state.go_to = Some(GoTo::Top { page: state.current, fraction: 0.0 });
    }
    // The layout the canvas was given its view for: what this pass draws, whatever the pass makes of the state.
    let drawn = build.of(&state);
    let drawn_frame = Frame::of(&drawn);
    let mut geo = Arc::clone(&drawn);
    let started_on = state.current;

    // A scale the fit (a new viewport or pages) or the app changed keeps the document point at the centre of the
    // view where it was, unless the app moved the view or asked for a go-to in the same breath.
    // A new room (a resized window, a panel opened beside the view) keeps the current page's top where it was instead.
    let unmoved = mem.offset == Some(state.offset);
    if let Some((key, previous)) = previous.filter(|(_, p)| !scale_settled(p.scale, geo.scale)).filter(|_| unmoved && state.go_to.is_none()) {
        let room = (key.viewport.width, key.viewport.height);
        let centred = centred_offset(&previous, &geo, state.offset, room, (vw, vh));
        let top_kept = |(x, _): (f64, f64)| {
            let (before, after) = (previous.layout.rects.get(state.current)?, geo.layout.rects.get(state.current)?);
            Some((x, after.y0 - (before.y0 - state.offset.1)))
        };
        if let Some(offset) = if room == (vw, vh) { centred } else { centred.and_then(top_kept) } {
            state.offset = offset;
        }
    }
    state.zoom = geo.scale / SCREEN_POINTS_PER_POINT;

    // The canvas folded the wheel, a pinch and the view commands into the view it settled on: a different zoom is the
    // user's, anchored at the point that stayed still; a different centre is a scroll.
    let zooming = (target.zoom - given.zoom).abs() > given.zoom * 1e-9;
    let mut want = state.offset;
    if zooming {
        // The point that stayed still, in the given view's frame: the canvas places the given view in its own rect (inside
        // the rulers) about the same centre. The wheel's and a pinch's point is the canvas's own: its clamp of the
        // zoomed view (a document no wider than the view stays centred) moves the centre, so the change of centre
        // would put the point elsewhere, across a gutter on another page that reflows into another row. A view
        // command's zoom keeps the centre.
        let pointer = match space.zoom_anchor() {
            Some(at) => at + (rect.center() - target.rect.center()),
            None => {
                let fixed = (target.center - given.center) * (given.zoom * target.zoom / (target.zoom - given.zoom));
                rect.center() + Vec2::new(fixed.x as f32, fixed.y as f32)
            }
        };
        geo = zoom_about(build, &mut state, &geo, rect, pointer, target.zoom / given.zoom);
        let (xs, ys) = (offset_range(geo.doc.x0, geo.doc.x1, vw), geo.y_range(vh));
        state.offset = (state.offset.0.clamp(xs.0, xs.1), state.offset.1.clamp(ys.0, ys.1));
        want = state.offset;
    } else {
        // The canvas was given the view the pass started with, so its change of it is the pan; the state's offset
        // may have moved on since (a scale change anchors the point at the centre), and the pan adds to that.
        let panned = drawn_frame.offset_of(target.center, vw, vh);
        // A new layout can put the old offset outside its document. The canvas clamps that view before
        // reporting it; that correction is not a pan to add to the anchor computed above.
        let mut baseline = given;
        CanvasScrollbars::clamp(&mut baseline, DocRect::from_origin_size(Point::ZERO, drawn_frame.document(&drawn)));
        let given_at = drawn_frame.offset_of(baseline.center, vw, vh);
        let (along_x, along_y) = (panned.0 - given_at.0, panned.1 - given_at.1);
        if along_x.abs() > OFFSET_EPSILON || along_y.abs() > OFFSET_EPSILON {
            want = (state.offset.0 + along_x, state.offset.1 + along_y);
        }
    }
    let mut scrolled = wheel.is_some_and(|wheel| !zooming && wheel.scroll != Vec2::ZERO);
    // A zoom that does not ease draws the layout it made in its own pass, through the view of the anchored offset: the
    // pages reflow and the point under the pointer stays in the pass the zoom happens, and the next pass, given that
    // view, moves nothing. (Drawing the layout the pass started with, scaled, put the reflow and the spacing that does
    // not scale one pass late.) An ease draws the layout it started from, scaled, until it lands.
    let (drawn, drawn_frame, mut shown) = if zooming && !space.easing() {
        let frame = Frame::of(&geo);
        let mut view = frame.view(rect, state.offset, geo.scale / SCREEN_POINTS_PER_POINT);
        // As the canvas places a given view: in its own rect (inside the rulers), about the same centre.
        view.rect = target.rect;
        let view = if shown == target { view } else { pixel_aligned(view, ui.ctx().pixels_per_point()) };
        (Arc::clone(&geo), frame, view)
    } else {
        (drawn, drawn_frame, shown)
    };

    // Not continuous, the wheel turns the page at the row's edge. The edge is read at the offset the pass started
    // with, not at `want.1`: the share that pans a long page to its edge has not been marked moved yet, and must not
    // also turn.
    let wheel_turned = wheel
        .filter(|_| !state.continuous)
        .and_then(|wheel| wheel_turn(&wheel, edge_ahead(&geo, state.offset.1, vh, wheel.scroll.y < 0.0), f64::from(navigation.wheel_line)));

    let preliminary_view = shown.visible();
    let preliminary_view = DocRect::new(
        preliminary_view.x0 * drawn_frame.zoom + drawn_frame.origin.x,
        preliminary_view.y0 * drawn_frame.zoom + drawn_frame.origin.y,
        preliminary_view.x1 * drawn_frame.zoom + drawn_frame.origin.x,
        preliminary_view.y1 * drawn_frame.zoom + drawn_frame.origin.y,
    );
    let preliminary: Vec<PlacedPage> = drawn
        .row
        .clone()
        .filter_map(|page| {
            if !overlaps(band_rect(&drawn.layout, page), preliminary_view) {
                return None;
            }
            let r = drawn_frame.document_rect(&drawn.layout.rects[page]);
            let rect = Rect::from_min_max(shown.to_screen(Point::new(r.x0, r.y0)), shown.to_screen(Point::new(r.x1, r.y1)));
            let ratio = shown.zoom / drawn_frame.zoom;
            let band = (drawn.layout.bands[page].0 as f32 * ratio as f32, drawn.layout.bands[page].1 as f32 * ratio as f32);
            Some(PlacedPage::new(page, rect, drawn.scale * ratio, band))
        })
        .collect();
    let slot = PaintSlot::reserve(&ui.painter_at(cx.rect));
    let pass = tool_pass(&cx.env);
    // A tool host returns the canvas's own changed view when no tool moved it. That view was already
    // applied above, including its page anchor; applying it again here would undo that anchoring.
    let tool_view = before(ui, &PageBody { rect: inputs.body_rect(cx.rect), pages: &preliminary, view: shown, pass: pass.as_ref() })
        .filter(|view| Some(*view) != pass.as_ref().and_then(|p| p.view));
    let pre_shapes = slot.take_after(ui.ctx());
    if let Some(view) = tool_view {
        space.set_view(view);
        shown = space.view;
        if (view.zoom - drawn_frame.zoom).abs() > drawn_frame.zoom * 1e-9 {
            state.fit = Fit::None;
            state.zoom = view.zoom.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
            geo = build.of(&state);
        }
        want = Frame::of(&geo).offset_of(view.center, vw, vh);
    }

    with_canvas_space(ui, space, |ui| {
        // The pasteboard and the pictures are drawn through the view the canvas shows; a zoom still
        // under way draws the layout the view was made for, scaled. A pointer zoom runs while the canvas zooms this pass
        // or its eased zoom is still arriving on screen.
        let ratio = shown.zoom / drawn_frame.zoom;
        let pointer_zoom = zooming || ratio != 1.0;
        let travel = travel_of(mem.offset, want, wheel.as_ref());

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
        // Each page's picture state is asked of the page store before the pages are listed, once per page per pass,
        // then the pages near what shows (ahead of the travel) and the pages the app keeps, at their side in this
        // layout. Sharp renders of the page store wait while a pointer zoom runs.
        let ctx = ui.ctx().clone();
        let (thumbs, details) = page_stores(&ctx, cx.id, picture_options(&ctx));
        thumbs.hold(pointer_zoom);
        let ppp = ctx.pixels_per_point();
        let ask = |page: usize, side: f32, priority: Priority| thumbs.state(&ctx, source, &page, &ThumbRequest::for_cell(side, ppp), priority);
        let pages: Vec<PlacedPage> = rows
            .iter()
            .map(|&page| {
                let rect = to_screen(&drawn.layout.rects[page]);
                let band = (drawn.layout.bands[page].0 as f32 * ratio as f32, drawn.layout.bands[page].1 as f32 * ratio as f32);
                let picture = ask(page, rect.width().max(rect.height()), Priority::VISIBLE);
                let picture_revision = matches!(picture, TextureState::Ready(_)).then(|| source.page_revision(page)).flatten();
                let mut placed = PlacedPage::new(page, rect, drawn.scale * ratio, band).showing(picture);
                placed.picture_revision = picture_revision;
                placed
            })
            .collect();
        let near = near_pages(&drawn, state.continuous, &visible, count, &build.layout_options(&state), travel);
        let kept = keep.iter().copied().filter(|page| *page < count && !rows.contains(page) && !near.contains(page));
        for page in near.iter().copied().chain(kept) {
            let size = drawn.layout.rects[page].size();
            ask(page, size.width.max(size.height) as f32, Priority::NEAR);
        }

        let frames: Vec<PageFrame> =
            rows.iter().map(|&p| PageFrame::new(drawn_frame.document_rect(&band_rect(&drawn.layout, p))).fill(paper)).collect();
        let border = if state.page_gaps { border } else { PageBorder::None };
        let pasteboard = Pasteboard::new(words.pasteboard.clone(), &frames).border(border).show_at(ui, cx.rect);
        // The scroll bars read their drag before the pictures show, so a drag holds the part in view in its own pass; they
        // paint where they always did, over the pictures: their shapes are lifted out of the paint list here and put back
        // in a slot reserved after the pictures.
        let slot = PaintSlot::reserve(&ui.painter_at(cx.rect));
        // The bars edit the view the state keeps, the canvas's settled one: the view on screen is pixel aligned, and a
        // drag handed it back would differ from the drag's own value and cancel it.
        let document = drawn_frame.document(&drawn);
        let bars = CanvasScrollbars::new(words.scroll_bars.clone(), Bind::given(target), DocRect::from_origin_size(Point::ZERO, document));
        // The tip names the page at the top of the view the thumb moved to: the bars ask for it once they know the move.
        let format = env.format();
        let (top_lo, top_hi) = geo.y_range(vh);
        let tip = |moved: &CanvasView| {
            let top = drawn_frame.offset_of(moved.center, vw, vh).1.clamp(top_lo, top_hi);
            let page = if state.continuous { page_on_line(&geo.layout, top) } else { state.current };
            let (label, total) = (source.page_label(page), format.integer(count as u64));
            Label::from(words.page_of.fill(&[("page", label.as_str()), ("count", total.as_str())]))
        };
        let bars = if count > 0 { bars.tip(&tip) } else { bars };
        // Pages that fit the view's width can't scroll sideways, so there is no horizontal bar. The layout drawn
        // decides, as the bars measure it.
        let (x_lo, x_hi) = offset_range(drawn.doc.x0, drawn.doc.x1, vw);
        let bars = if x_lo >= x_hi { bars.vertical_only() } else { bars };
        // Full screen's edge-to-edge view (`PageView::fill_screen`) has no scroll bars: nothing is drawn and nothing takes a press
        // along the window's edges.
        let bar_moved = if build.fill_screen {
            drop(bars);
            None
        } else {
            bars.show_at(ui, cx.rect).view.value().cloned()
        };
        let bar_drag = bar_moved.is_some();
        scrolled |= bar_drag;
        let bar_want = bar_moved.map(|moved| drawn_frame.offset_of(moved.center, vw, vh));
        let bar_shapes = slot.take_after(&ctx);

        // The part in view waits while the view zooms or moves.
        let pictures =
            PagePictures::new(source, &pages, cx.id, paper).label(words.text.clone()).hold_detail(pointer_zoom || travel != 0.0 || bar_drag);
        match describer {
            Some(describer) => pictures.describer(describer).show_at(ui, cx.rect),
            None => pictures.show_at(ui, cx.rect),
        };
        // Pages scrolled out of reach give their pictures back (WordCraft #811: a long scroll held every page it had
        // shown). Those drawn, those asked for ahead, the app's and those of the shown row within the reach stay; a
        // part-in-view picture is only of use while its page shows.
        let reach = f64::from(reach);
        let around = DocRect::new(
            in_view.x0 - in_view.width(),
            in_view.y0 - reach * in_view.height(),
            in_view.x1 + in_view.width(),
            in_view.y1 + reach * in_view.height(),
        );
        let mut held_pages: Vec<usize> =
            drawn.row.clone().filter(|&p| p < drawn.layout.rects.len() && overlaps(band_rect(&drawn.layout, p), around)).collect();
        held_pages.extend(rows.iter().chain(&near).chain(keep).copied());
        held_pages.sort_unstable();
        held_pages.dedup();
        thumbs.retain(&ctx, |page| held_pages.binary_search(page).is_ok());
        details.retain(&ctx, |page| rows.contains(page));
        // PLAN: assumed impl Add for TextureStatistics (pdf-canvas-w7)
        let textures = thumbs.cache().statistics(&ctx) + details.cache().statistics(&ctx);
        PaintSlot::reserve(&ui.painter_at(cx.rect)).extend(&ctx, bar_shapes);

        let seen = Seen {
            geo: drawn,
            frame: drawn_frame,
            view: shown,
            pages,
            visible,
            current_changed: false,
            scrolled,
            pasteboard: Some(pasteboard),
            textures,
            resident: textures.entries,
            next: None,
        };
        let pending =
            Pending { started, state, mem, geo, want, bar_want, wheel_turned, pointer_zoom, started_on, space, edit: words.edit.as_str().to_owned() };
        PaintSlot::reserve(&ui.painter_at(cx.rect)).extend(&ctx, pre_shapes);
        Some((seen, pending))
    })
}

/// What a pass decides after the app's body ran in it: the keys the body's tool host left, the turns, the go-to and
/// the clamp of the offset.
struct Pending {
    started: PageViewState,
    state: PageViewState,
    mem: Memory,
    geo: Arc<Placed>,
    /// The offset the wheel, a pinch and a pan ask for.
    want: (f64, f64),
    /// The offset a scroll bar's drag or press asks for, which overrides every other move.
    bar_want: Option<(f64, f64)>,
    wheel_turned: Option<bool>,
    pointer_zoom: bool,
    started_on: usize,
    space: CanvasSpace,
    edit: String,
}

impl Pending {
    /// Folds the page keys and the arrow nothing in the body took into the state, with the app's step, the wheel's
    /// turn and the go-to, clamps the offset, and hands `seen` what the caller stores.
    fn finish(self, ctx: &egui::Context, seen: &mut Seen, inputs: &Inputs<'_>) {
        let Pending { started, mut state, mut mem, mut geo, mut want, bar_want, wheel_turned, pointer_zoom, started_on, space, edit } = self;
        let build = inputs.build;
        let (vw, vh) = (build.viewport.width, build.viewport.height);
        let count = build.sizes.len();
        let options = build.layout_options(&state);
        let keys = KeyMoves::of(space.keys(), space.take_nudge(ctx), &state, &geo, &options, inputs.rect, f64::from(inputs.navigation.key_line));
        seen.scrolled |= keys.moves();
        want.0 += keys.pan.0;
        want.1 = if keys.to_end { f64::INFINITY } else { want.1 + keys.pan.1 };

        // A key or the app's step turns the page; else, not continuous, the wheel turns it at the row's edge.
        let step = state.turn.take();
        let mut go_to = keys.go_to;
        let before_turn = state.current;
        if let Some(forward) = keys.turn.or(step).or(wheel_turned) {
            let row = if state.continuous { shown_row(count, state.current, &options) } else { geo.row.clone() };
            match turned_row(count, &row, &options, forward) {
                // A continuous view scrolls to the row, as a go-to of its first page.
                Some(next) if state.continuous => go_to = go_to.or(Some(GoTo::Top { page: next, fraction: 0.0 })),
                Some(next) => {
                    state.current = next;
                    geo = build.of(&state);
                    state.zoom = geo.scale / SCREEN_POINTS_PER_POINT;
                    let (lo, hi) = geo.y_range(vh);
                    want.1 = if forward { lo } else { hi };
                }
                // Past either end there is nowhere to go: the view stays where it was.
                None => want.1 = state.offset.1,
            }
        }

        // A key's or the step's go-to replaces a pending one. While a pointer zoom runs, the go-to is placed at the
        // zoom's new scale and kept, so the zoom anchors on it; the first pass without one lets it go.
        let go_to = go_to.or(state.go_to);
        state.go_to = go_to.filter(|_| pointer_zoom);
        let went = go_to.is_some();
        if let Some(go_to) = go_to {
            geo = place_go_to(build, &mut state, go_to, (vw, vh));
            want = go_to_offset(go_to, &geo, state.continuous, want, (vw, vh), &build.spacing, build.go_to_place);
        }
        // Turned by Left or Right at an edge, a row wider than the view starts at the edge the reader comes from, so
        // the next key scrolls across it rather than turning again.
        if let Some(forward) = keys.across.filter(|_| state.current != before_turn)
            && let Some(row) = row_rect(&geo, state.current, &options)
            && row.width() + 2.0 * build.spacing.side > vw
        {
            want.0 = if forward { row.x0 - build.spacing.side } else { row.x1 + build.spacing.side - vw };
        }
        if let Some(bar) = bar_want {
            want = bar;
        }

        let (xs, ys) = (offset_range(geo.doc.x0, geo.doc.x1, vw), geo.y_range(vh));
        let offset = (want.0.clamp(xs.0, xs.1), want.1.clamp(ys.0, ys.1));
        state.offset = offset;
        if state.continuous && count > 0 && !went && mem.offset.is_some_and(|last| last != offset) {
            state.current = shown_most(&geo.layout, state.current, offset.1, vh);
        }
        mem.offset = Some(offset);
        mem.layout = build.cache.borrow().clone();
        seen.current_changed = state.current != started_on;
        seen.next = Some(Next { started, state, mem, edit });
    }
}

/// The page or spread `current` shows in, content coordinates; `None` with no pages.
fn row_rect(geo: &Placed, current: usize, options: &LayoutOptions) -> Option<DocRect> {
    shown_row(geo.layout.rects.len(), current, options).filter_map(|page| geo.layout.rects.get(page).copied()).reduce(|a, b| a.union(b))
}

/// Which way the view travels this pass along the page column: `1.0` down the document, `-1.0` up, `0.0` still. The
/// sign of the offset change against the offset the last pass ended with, or with no change and a wheel gesture
/// running, of the gesture's gathered scroll (a negative scroll moves down). Nothing of it is kept between passes.
fn travel_of(last: Option<(f64, f64)>, want: (f64, f64), wheel: Option<&craft_behavior::Wheel>) -> f64 {
    match last {
        Some(last) if (want.1 - last.1).abs() > OFFSET_EPSILON => (want.1 - last.1).signum(),
        _ => match wheel.map(|wheel| wheel.gathered.y) {
            Some(gathered) if gathered < 0.0 => 1.0,
            Some(gathered) if gathered > 0.0 => -1.0,
            _ => 0.0,
        },
    }
}

/// The edge of the shown row the view sits at in the direction it would move: `Some(true)` the end, going forward,
/// `Some(false)` the start, going back, `None` when the row still has room that way. A row that fits sits at both.
fn edge_ahead(geo: &Placed, y: f64, view_height: f64, forward: bool) -> Option<bool> {
    let (lo, hi) = geo.y_range(view_height);
    if forward { (y >= hi - EDGE).then_some(true) } else { (y <= lo + EDGE).then_some(false) }
}

/// How near the edge of a row, in screen points, the view counts as at it: the offset a pass ends with is clamped
/// exactly, and a scale anchor puts it within rounding of it.
const EDGE: f64 = 0.5;

/// The first page of the row after (`forward`) or before `row`, `None` past either end. With a cover, forward from
/// the cover goes to page 1 and back from the first spread to the cover, as `shown_row` groups the pages.
fn turned_row(count: usize, row: &Range<usize>, options: &LayoutOptions, forward: bool) -> Option<usize> {
    if forward { (row.end < count).then_some(row.end) } else { row.start.checked_sub(1).map(|before| shown_row(count, before, options).start) }
}

/// Makes `go_to`'s page current and, for a [`GoTo::Rect`], sets the zoom that fits its rect and margin into the
/// viewport `(vw, vh)` beside the side columns; returns the layout of the state.
fn place_go_to(build: &Geometry, state: &mut PageViewState, go_to: GoTo, (vw, vh): (f64, f64)) -> Arc<Placed> {
    state.current = go_to.page().min(build.sizes.len().saturating_sub(1));
    if let GoTo::Rect { rect, margin, .. } = go_to {
        let rect = rect.abs();
        let fits = ((vw - 2.0 * (margin + build.spacing.side)) / rect.width()).min((vh - 2.0 * margin) / rect.height());
        if fits.is_finite() && fits > 0.0 {
            state.fit = Fit::None;
            state.zoom = (fits / SCREEN_POINTS_PER_POINT).clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
        }
    }
    let geo = build.of(state);
    state.zoom = geo.scale / SCREEN_POINTS_PER_POINT;
    geo
}

/// The offset that shows `go_to` in `geo`, a view `(vw, vh)` points: `want` on an axis the go-to leaves alone or
/// gives as NaN. A page's own top puts the view's top where what shows of the row above ends (the document's start
/// for the first row), so the space the layout keeps above the page shows and nothing of the page before it; a point
/// inside the page goes to the view's top. Out of a continuous view, a page's own top is the top of its row.
fn go_to_offset(
    go_to: GoTo,
    geo: &Placed,
    continuous: bool,
    want: (f64, f64),
    (vw, vh): (f64, f64),
    spacing: &PageSpacing,
    place: GoToPlace,
) -> (f64, f64) {
    let page = go_to.page().min(geo.layout.rects.len().saturating_sub(1));
    let Some(&r) = geo.layout.rects.get(page) else { return (want.0, geo.doc.y0) };
    let scale = geo.scale;
    let on_page = |p: Point| Point::new(r.x0 + p.x * scale, r.y0 + p.y * scale);
    let or_want = |x: f64, y: f64| (if x.is_nan() { want.0 } else { x }, if y.is_nan() { want.1 } else { y });
    match go_to {
        GoTo::Top { fraction, .. } => {
            let (top, bottom) = geo.layout.bands[page];
            let start = r.y0 + top;
            // Production's place: the page gap above the page's top, or above the fraction of it, never above the
            // document's start; out of a continuous view a page's own top is the top of its row.
            let gap_above = match place {
                GoToPlace::GapAbove if continuous || fraction > 0.0 => Some((start + fraction * (bottom - top) - spacing.gap).max(geo.doc.y0)),
                GoToPlace::GapAbove => Some(geo.doc.y0),
                _ => None,
            };
            let y = if let Some(y) = gap_above {
                y
            } else if fraction > 0.0 {
                start + fraction * (bottom - top)
            } else if continuous {
                geo.layout.rects.iter().zip(&geo.layout.bands).map(|(r, band)| r.y0 + band.1).filter(|&end| end <= start).fold(geo.doc.y0, f64::max)
            } else {
                geo.doc.y0
            };
            let x = if r.width() + 2.0 * spacing.side <= vw {
                clamp_between(want.0, r.x1 - vw + spacing.side, r.x0 - spacing.side)
            } else if r.x1 <= want.0 {
                r.x1 - vw + spacing.side
            } else if r.x0 >= want.0 + vw {
                r.x0 - spacing.side
            } else {
                want.0
            };
            or_want(x, y)
        }
        GoTo::Point { at, view, .. } => {
            let p = on_page(at);
            or_want(p.x - view.0 * vw, p.y - view.1 * vh)
        }
        GoTo::Corner { at, .. } => {
            let p = on_page(at);
            or_want(p.x - spacing.side, p.y)
        }
        GoTo::Rect { rect, .. } => {
            let c = on_page(rect.center());
            or_want(c.x - vw / 2.0, c.y - vh / 2.0)
        }
    }
}

/// The scale that fits the spread `spread` (its one or two pages' sizes) into `viewport` by `fit`, as full screen fits
/// the spread it shows: two pages side by side by their own widths, a page alone keeping a spread's room (a cover keeps
/// its blank slot); `None` for no fit or no pages.
fn spread_fit_scale(spread: &[Size], viewport: Size, fit: Fit, options: &LayoutOptions, spacing: &PageSpacing) -> Option<f64> {
    let dims = |s: &Size| {
        (if s.width.is_finite() && s.width > 0.0 { s.width } else { 1.0 }, if s.height.is_finite() && s.height > 0.0 { s.height } else { 1.0 })
    };
    let width = match spread {
        [a, b] => dims(a).0 + dims(b).0,
        _ => spread.iter().map(|s| dims(s).0).fold(0.0, f64::max) * 2.0,
    };
    let height = spread.iter().map(|s| dims(s).1).fold(0.0, f64::max);
    if fit == Fit::None || spread.is_empty() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let extent = |v: f64| if v.is_finite() { v.max(0.0) } else { 0.0 };
    let gap = if options.page_gaps { spacing.gap } else { 0.0 };
    let by_width = (extent(viewport.width) - 2.0 * spacing.side - gap).max(1.0) / (width + 2.0 * options.markup.max(0.0));
    let by_height = (extent(viewport.height) - 2.0 * spacing.margin).max(1.0) / height;
    let scale = match fit {
        Fit::Width => by_width,
        Fit::Height => by_height,
        _ => by_width.min(by_height),
    };
    scale.is_finite().then(|| scale.clamp(ZOOM_RANGE.start() * SCREEN_POINTS_PER_POINT, ZOOM_RANGE.end() * SCREEN_POINTS_PER_POINT))
}

/// Centres the pages of `row` across a view `width` wide and makes the layout that wide when the row fits it: the
/// content is as wide as the spread full screen shows, not as the document's widest spread, which would centre a
/// narrower spread past the screen's edge.
fn centre_row(layout: &mut PagedLayout, row: Range<usize>, width: f64) {
    let Some(shown) = layout.rects.get(row.clone()).and_then(|rects| rects.iter().copied().reduce(|a, b| a.union(b))) else { return };
    if !width.is_finite() || shown.width() > width {
        return;
    }
    let dx = (width - shown.width()) / 2.0 - shown.x0;
    for rect in layout.rects.get_mut(row).into_iter().flatten() {
        *rect = *rect + craft_behavior::kurbo::Vec2::new(dx, 0.0);
    }
    layout.size.width = width;
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
        /// The tip beside the vertical scroll bar's thumb while it is dragged.
        page_of: template "pageView.pageOf" = "Page {page} of {count}";
    }
}

/// What this pass's keys ask of the view.
#[derive(Default)]
struct KeyMoves {
    /// Screen points to move the offset.
    pan: (f64, f64),
    /// End in a continuous view: the bottom.
    to_end: bool,
    /// Turn to the next (`true`) or previous row or spread.
    turn: Option<bool>,
    /// The turn is Right's (`true`) or Left's at the row's edge: a wide row lands at the edge the reader comes from.
    across: Option<bool>,
    /// Where to take the view.
    go_to: Option<GoTo>,
}

impl KeyMoves {
    /// The arrow modifiers `of` reads besides the plain arrows: Cmd or Ctrl, alone or with Shift. Alt is the app's.
    const NUDGES: NudgeMods =
        NudgeMods::NONE.with(Mods::CMD).with(Mods::CTRL).with(Mods { shift: true, ..Mods::CMD }).with(Mods { shift: true, ..Mods::CTRL });

    /// Page Up and Down a screen less a key line (a row when not continuous, and with Cmd), Home and End the first
    /// and last page, Up and Down a key line (not continuous, a turn once the row is at its edge that way), Left and
    /// Right a key line sideways while the current page or spread has more to show that way, else a turn of a page or
    /// spread; Cmd with Left or Right turns at once. Home and End with Shift, or with Cmd off macOS (which gives Cmd
    /// with them no move), and Page Up, Page Down, Up and
    /// Down with Shift and Cmd together, go to the first or last page; Alt, and Shift alone with the other keys,
    /// leave them to nothing.
    fn of(
        keys: BodyKeys,
        nudge: Option<(Vec2, Mods)>,
        state: &PageViewState,
        geo: &Placed,
        options: &LayoutOptions,
        rect: Rect,
        line: f64,
    ) -> KeyMoves {
        let last = geo.layout.rects.len().saturating_sub(1);
        let top = |page: usize| Some(GoTo::Top { page, fraction: 0.0 });
        let mut moves = KeyMoves::default();
        let end = |forward: bool| if forward && state.continuous { (true, None) } else { (false, top(if forward { last } else { 0 })) };
        let to_end = match keys.page {
            Some((step @ (NavMove::Start | NavMove::End), mods)) if !mods.fine => Some(step == NavMove::End),
            Some((step @ (NavMove::PageUp | NavMove::PageDown), mods)) if mods.extend && mods.lead && !mods.fine => Some(step == NavMove::PageDown),
            _ => None,
        };
        if let Some(forward) = to_end {
            (moves.to_end, moves.go_to) = end(forward);
        }
        if let Some((step @ (NavMove::PageUp | NavMove::PageDown), mods)) = keys.page.filter(|(_, mods)| !mods.extend && !mods.fine) {
            let forward = step == NavMove::PageDown;
            if state.continuous && !mods.lead {
                // A key line of the last screen stays in view, so the reader keeps their place.
                let screen = (f64::from(rect.height()) - line).max(line);
                moves.pan.1 += if forward { screen } else { -screen };
            } else {
                moves.turn = Some(forward);
            }
        }
        if let Some((direction, mods)) = nudge
            && direction.y != 0.0
            && mods.shift
            && (mods.cmd || mods.ctrl)
            && !mods.alt
        {
            (moves.to_end, moves.go_to) = end(direction.y > 0.0);
            return moves;
        }
        let Some((direction, mods)) = nudge.filter(|(_, mods)| !mods.shift && !mods.alt) else { return moves };
        let lead = mods.cmd || mods.ctrl;
        if direction.y != 0.0 && !lead {
            moves.line_down(direction.y > 0.0, state, geo, rect, line);
        } else if direction.x != 0.0 {
            let forward = direction.x > 0.0;
            if lead {
                moves.turn = Some(forward);
            } else if side_edge(geo, state, options, rect, forward) {
                moves.turn = Some(forward);
                moves.across = Some(forward);
            } else {
                moves.pan.0 += if forward { line } else { -line };
            }
        }
        moves
    }

    /// Whether the keys move the view.
    fn moves(&self) -> bool {
        self.pan != (0.0, 0.0) || self.to_end || self.turn.is_some() || self.go_to.is_some()
    }

    /// Up or Down (`forward`): a key line, or, not continuous with the row at its edge that way, a turn.
    fn line_down(&mut self, forward: bool, state: &PageViewState, geo: &Placed, rect: Rect, line: f64) {
        if !state.continuous && edge_ahead(geo, state.offset.1, f64::from(rect.height()), forward).is_some() {
            self.turn = Some(forward);
        } else {
            self.pan.1 += if forward { line } else { -line };
        }
    }
}

/// Whether the view shows the edge of the current page or spread to the right (`forward`) or left: the row is shown
/// whole that way, or the content can scroll no farther. The current row decides, not the widest page, so a narrow
/// page beside wide ones turns at once.
fn side_edge(geo: &Placed, state: &PageViewState, options: &LayoutOptions, rect: Rect, forward: bool) -> bool {
    let width = f64::from(rect.width());
    let (lo, hi) = offset_range(geo.doc.x0, geo.doc.x1, width);
    let x = state.offset.0;
    let row = row_rect(geo, state.current, options);
    if forward { x >= hi - EDGE || row.is_some_and(|r| r.x1 <= x + width + EDGE) } else { x <= lo + EDGE || row.is_some_and(|r| r.x0 >= x - EDGE) }
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
    /// The scroll area in content coordinates: the pages that show with the margins above and below them.
    doc: DocRect,
    /// The part of the pages that show, without the margins.
    pages: DocRect,
}

impl Placed {
    /// The offsets a view `view_height` tall can take down the shown pages: the pages in the view's middle when they
    /// fit it without their margins, else anywhere over the scroll area.
    fn y_range(&self, view_height: f64) -> (f64, f64) {
        if self.pages.height() <= view_height {
            let middle = self.pages.y0 - (view_height - self.pages.height()) / 2.0;
            (middle, middle)
        } else {
            offset_range(self.doc.y0, self.doc.y1, view_height)
        }
    }
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
    /// The view's rotation: the sizes follow it, but a square page's would not tell a turn.
    rotation: u16,
}

impl LayoutKey {
    /// Whether `state` shows the pages otherwise than this layout: its display, continuous, cover, fit or rotation
    /// differ. The zoom and the page gaps are not among them: a new zoom keeps the point at the centre instead.
    fn display_differs(&self, state: &PageViewState) -> bool {
        self.options.display != state.display
            || self.options.continuous != state.continuous
            || self.options.cover != state.cover
            || self.fit != state.fit
            || self.rotation != state.rotation
    }
}

/// What a layout is made from.
struct Geometry<'a> {
    sizes: &'a [Size],
    viewport: Size,
    options: LayoutOptions,
    spacing: PageSpacing,
    bodies: Option<&'a [(f64, f64)]>,
    /// Where Go To puts a page's top (`PageView::go_to_place`).
    go_to_place: GoToPlace,
    /// `PageView::fill_screen`: a spread that is not continuous fits by its own pages and stands centred.
    fill_screen: bool,
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
            rotation: state.rotation,
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
        let spread = self.fill_screen && options.display == PageDisplay::TwoUp && !options.continuous;
        let row = shown_row(self.sizes.len(), state.current, &options);
        let fitted = if spread {
            spread_fit_scale(self.sizes.get(row.clone()).unwrap_or(&[]), self.viewport, state.fit, &options, &self.spacing)
        } else {
            fit_scale(self.sizes, self.viewport, state.fit, &options, &self.spacing, Some(state.current))
        };
        let scale = fitted
            .unwrap_or(state.zoom * SCREEN_POINTS_PER_POINT)
            .clamp(ZOOM_RANGE.start() * SCREEN_POINTS_PER_POINT, ZOOM_RANGE.end() * SCREEN_POINTS_PER_POINT);
        let mut layout = place_pages(self.sizes, scale, self.viewport, &options, &self.spacing);
        if spread {
            centre_row(&mut layout, row, self.viewport.width);
        }
        if let Some(bodies) = self.bodies.filter(|_| state.hide_white_space) {
            layout.hide_white_space(bodies, scale, self.viewport.height, &self.spacing);
        }
        let count = self.sizes.len();
        let row = if state.continuous { 0..count } else { shown_row(count, state.current, &options) };
        // The layout is at least the viewport's height; the scroll area is the pages' own, so pages shorter than the
        // view sit in its middle.
        let whole = DocRect::new(0.0, 0.0, layout.size.width, layout.size.height);
        let pages = row.clone().map(|p| band_rect(&layout, p)).reduce(|a, b| a.union(b));
        let doc = pages.map_or(whole, |shown| DocRect::new(0.0, shown.y0 - self.spacing.margin, layout.size.width, shown.y1 + self.spacing.margin));
        Placed { layout, scale, row, doc, pages: pages.unwrap_or(whole) }
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
/// `value` kept between `a` and `b`, in either order. A page that just fits the view gives bounds that can cross by a
/// rounding error, where `f64::clamp` would panic.
fn clamp_between(value: f64, a: f64, b: f64) -> f64 {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    if value.is_nan() { lo } else { value.max(lo).min(hi) }
}

fn offset_range(lo: f64, hi: f64, len: f64) -> (f64, f64) {
    let spare = hi - lo - len;
    if spare <= 0.0 {
        let middle = lo + spare / 2.0;
        (middle, middle)
    } else {
        (lo, lo + spare)
    }
}

/// The offset that keeps the document point at the centre of a view `(vw, vh)` points where it was when the scale
/// goes from `old`'s to `new`'s: the page under the centre keeps the same fractions of its width and height there;
/// with the centre in a gap, the next page keeps its distance below the centre. `offset` is of a view of `old_view`
/// points, the result of one of `(vw, vh)` points (a new room changes both). `None` when `new` has no such page.
fn centred_offset(old: &Placed, new: &Placed, offset: (f64, f64), old_view: (f64, f64), (vw, vh): (f64, f64)) -> Option<(f64, f64)> {
    let centre = Point::new(offset.0 + old_view.0 / 2.0, offset.1 + old_view.1 / 2.0);
    let pages = 0..old.layout.rects.len();
    let band = |page: usize| band_rect(&old.layout, page);
    let fraction = |at: f64, from: f64, len: f64| if len > 0.0 { (at - from) / len } else { 0.0 };
    // The page under the centre, or the one whose band holds the centre's line, or the next one below it.
    let (page, fy, below) = crate::layout::page_at(&old.layout, centre)
        .or_else(|| pages.clone().find(|&p| band(p).y0 <= centre.y && band(p).y1 > centre.y))
        .map(|p| {
            let r = old.layout.rects[p];
            (p, fraction(centre.y, r.y0, r.height()).clamp(0.0, 1.0), 0.0)
        })
        .or_else(|| pages.clone().find(|&p| band(p).y0 >= centre.y).map(|p| (p, 0.0, (old.layout.rects[p].y0 - centre.y).max(0.0))))
        .or_else(|| pages.last().map(|p| (p, 1.0, (old.layout.rects[p].y1 - centre.y).max(0.0))))?;
    let before = old.layout.rects[page];
    let fx = fraction(centre.x, before.x0, before.width());
    let r = *new.layout.rects.get(page)?;
    Some((r.x0 + fx * r.width() - vw / 2.0, r.y0 + fy * r.height() - below - vh / 2.0))
}

/// The current page of a continuous view whose top is at `top` (content coordinates) and `height` tall: the page that
/// shows the most of its height, the topmost of those within [`PAGE_TIE`]. The page `current` was stays current
/// while it shows as much as that page, or shows whole: a short page gone to keeps the mark from a taller one below.
// pdfcraft: crates/ui-egui/src/canvas.rs:1763 `draw_page` (best overlap), :2065 (the current page stays).
fn shown_most(layout: &PagedLayout, current: usize, top: f64, height: f64) -> usize {
    let shown = |page: usize| {
        let band = band_rect(layout, page);
        ((band.y1.min(top + height) - band.y0.max(top)).max(0.0), band.height())
    };
    let mut best = (current, -1.0);
    for page in 0..layout.rects.len() {
        let (seen, _) = shown(page);
        if seen > best.1 + PAGE_TIE {
            best = (page, seen);
        }
    }
    match (current < layout.rects.len()).then(|| shown(current)) {
        Some((seen, whole)) if seen >= best.1 - PAGE_TIE || seen >= whole - PAGE_TIE => current,
        _ => best.0,
    }
}

/// The page whose row the horizontal line at `y` (content coordinates) crosses or lies above: the first page whose
/// shown band ends below it, the last page past the end.
fn page_on_line(layout: &PagedLayout, y: f64) -> usize {
    let last = layout.rects.len().saturating_sub(1);
    layout.rects.iter().zip(&layout.bands).position(|(r, &(_, bottom))| r.y0 + bottom + 0.5 > y).unwrap_or(last)
}

/// Shown heights (screen points) closer than this are equal: one page size can meet the view a fraction of a point
/// differently once the offset is applied.
const PAGE_TIE: f64 = 0.5;

/// Zoom by `delta` keeping the page point under `pointer` where it is.
fn zoom_about(build: &Geometry, state: &mut PageViewState, old: &Placed, rect: Rect, pointer: egui::Pos2, delta: f64) -> Arc<Placed> {
    let inside = (f64::from(pointer.x - rect.min.x), f64::from(pointer.y - rect.min.y));
    let at = Point::new(state.offset.0 + inside.0, state.offset.1 + inside.1);
    state.fit = Fit::None;
    state.zoom = (state.zoom * delta).clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
    let new = build.of(state);
    // The page under the pointer, or in a gap (between columns, rows or pages) the nearest: the point keeps its place
    // relative to that page, outside it too, since the gaps keep their size and the pages may reflow.
    let (x, y) = match crate::layout::page_at(&old.layout, at).or_else(|| nearest_page(&old.layout, at)) {
        Some(page) if page < new.layout.rects.len() => {
            let (before, after) = (old.layout.rects[page], new.layout.rects[page]);
            (after.x0 + (at.x - before.x0) / before.width() * after.width(), after.y0 + (at.y - before.y0) / before.height() * after.height())
        }
        _ => {
            let ratio = new.scale / old.scale;
            (at.x * ratio, at.y * ratio)
        }
    };
    state.offset = (x - inside.0, y - inside.1);
    new
}

/// The page whose shown band lies nearest `p` (content coordinates); the earlier page on a tie, `None` with no pages.
fn nearest_page(layout: &PagedLayout, p: Point) -> Option<usize> {
    let distance = |page: usize| {
        let band = band_rect(layout, page);
        let dx = (band.x0 - p.x).max(p.x - band.x1).max(0.0);
        let dy = (band.y0 - p.y).max(p.y - band.y1).max(0.0);
        dx.hypot(dy)
    };
    (0..layout.rects.len().min(layout.bands.len())).min_by(|&a, &b| distance(a).total_cmp(&distance(b)))
}

/// The pages near what shows: while a continuous view travels (`travel` down the document when positive, up when
/// negative), the two rows ahead and none behind; otherwise one row either side, or the neighbouring rows of a
/// single shown row.
fn near_pages(placed: &Placed, continuous: bool, visible: &Range<usize>, count: usize, options: &LayoutOptions, travel: f64) -> Vec<usize> {
    if continuous {
        if visible.is_empty() {
            return Vec::new();
        }
        let columns = placed.layout.columns.max(1);
        let rows = if travel == 0.0 { columns } else { 2 * columns };
        let before = visible.start.saturating_sub(rows)..visible.start;
        let after = visible.end..(visible.end + rows).min(count);
        match travel {
            t if t > 0.0 => after.collect(),
            t if t < 0.0 => before.rev().collect(),
            _ => before.chain(after).collect(),
        }
    } else {
        let before = placed.row.start.checked_sub(1).map(|p| shown_row(count, p, options));
        let after = (placed.row.end < count).then(|| shown_row(count, placed.row.end, options));
        before.into_iter().chain(after).flatten().collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::clamp_between;
    use craft_ui::testing::trial;

    #[trial]
    fn bounds_crossed_by_a_rounding_error_keep_the_value_between_them() {
        // A page exactly as wide as the view: the bounds cross by 1e-13 (seen at fit-width).
        assert_eq!(clamp_between(5.0, 1.136_868_377_216_160_3e-13, 0.0), 1.136_868_377_216_160_3e-13);
        assert_eq!(clamp_between(-5.0, 1.136_868_377_216_160_3e-13, 0.0), 0.0);
        assert_eq!(clamp_between(3.0, 0.0, 10.0), 3.0);
        assert_eq!(clamp_between(f64::NAN, 0.0, 10.0), 0.0);
    }
}
