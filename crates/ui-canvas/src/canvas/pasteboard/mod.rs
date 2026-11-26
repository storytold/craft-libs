//! `Pasteboard`: the area around a document's pages or artboards, and the frames of the pages on it, a canvas
//! control (20.6). The pasteboard colour (set on the canvas) and the page border come from the application's
//! preferences; the default colour is the canvas surround. Artboards show their names above them, and a click on a
//! name selects the artboard.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::space::canvas_space;
use crate::contract::overlay::{Dismiss, LayerRole, Outside};
use crate::contract::{
    Access, Bind, Common, Component, Cx, MeasureCx, Name, NoValue, Outcome, Overlay, OverlayUi, Overlays as _, PaintUi, PartsCx, Pointer as _,
    Props as _, Rename, Role, Show as _,
};
use crate::primitives::{Checker, CheckerArea, PaintShape, PaintSlot, Rule, StrokeColor, StrokeWidth, TextRun, paint_surround};
use crate::runtime::env::Label;
use crate::runtime::focus::{FocusPolicy, Initial};
use crate::runtime::memory::Remember as _;
use crate::style::{
    ComponentStyle, DataColor, RuleLook, RuleWidth, StyleColor, StyleCx, SurfaceEdge, SurfaceKind, SurfaceStyle, SurroundLook, TypeRole,
};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::{CancelReason, CanvasView, EditEvent, GestureId, HitLevel, HitPriority, HitShape, Metrics, Mixed, PartKey, Sense};
use craft_keymap::Mods;
use craft_theme::Rgba;
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Vec2};

/// A page name: `NAME.at(i)` for page `i`, in the order of the frames given.
const NAME: PartKey = PartKey::new("name");
/// A visible piece of the pasteboard: `QUAD.at(i)` for the `i`th of [`Pasteboard::quads`] that shows. Inert: it takes
/// no press, and marks where a press keeps a typed draft unapplied.
const QUAD: PartKey = PartKey::new("quad");

/// The pasteboard's colour.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum PasteboardColor {
    /// The canvas surround: the theme's canvas surface and its pattern.
    #[default]
    Default,
    /// Black.
    Black,
    /// Grey 40.
    DarkGray,
    /// Grey 83.
    MediumGray,
    /// Grey 184.
    LightGray,
    /// A colour the user picked.
    Custom(DataColor),
}

impl PasteboardColor {
    /// The fill, or `None` for the theme's.
    pub fn fill(self) -> Option<DataColor> {
        match self {
            PasteboardColor::Default => None,
            PasteboardColor::Black => Some(DataColor::new(Color32::from_gray(0))),
            PasteboardColor::DarkGray => Some(DataColor::new(Color32::from_gray(40))),
            PasteboardColor::MediumGray => Some(DataColor::new(Color32::from_gray(83))),
            PasteboardColor::LightGray => Some(DataColor::new(Color32::from_gray(184))),
            PasteboardColor::Custom(color) => Some(color),
        }
    }
}

/// The border around each page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PageBorder {
    /// The floating surface's soft shadow, offset down; a plain offset copy of the page while the view is turned.
    #[default]
    DropShadow,
    /// A thin line round the page.
    Line,
    /// No border.
    None,
}

/// How the edges of artboards (pages marked with [`PageFrame::active()`]) show which one is active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ArtboardEdge {
    /// The active artboard's edge in `accent.solid`, the others in `line.border`, all at `stroke.thin`.
    #[default]
    Accent,
    /// Colours that read against the paper rather than the chrome: the active artboard's edge black at `stroke.thin`,
    /// the others mid grey at a hairline.
    Plain,
}

/// The edge of the active artboard under [`ArtboardEdge::Plain`]: black.
const PLAIN_ACTIVE: DataColor = DataColor::new(Color32::from_gray(0));
/// The edge of another artboard under [`ArtboardEdge::Plain`]: grey 120.
const PLAIN_OTHER: DataColor = DataColor::new(Color32::from_gray(120));

/// What fills a page under the document.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum PageFill {
    /// A plain colour: an artboard's background.
    Color(DataColor),
    /// A transparency grid laid from the page's top-left.
    Checker(Checker),
}

/// One page or artboard on the pasteboard, in document units.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PageFrame {
    /// The page's rectangle in document space.
    pub rect: DocRect,
    /// Shown above the page's top-left corner (artboards, spreads); `None` shows nothing.
    pub name: Option<String>,
    /// The page's own fill under the document (an artboard's background), or `None` for none.
    pub fill: Option<PageFill>,
    /// An artboard's edge: `Some(true)` for the active board (its name in the normal text colour), `Some(false)` for
    /// another board (its name muted), each edged as the pasteboard's [`ArtboardEdge`] says; `None` for a plain page,
    /// which only the border shows.
    pub active: Option<bool>,
    /// Whether a double press on the page's name renames the page.
    pub renames: bool,
}

impl PageFrame {
    /// A page at `rect` with no name, fill or active mark.
    pub fn new(rect: DocRect) -> Self {
        Self { rect, name: None, fill: None, active: None, renames: false }
    }

    /// Marks the frame as an artboard, the active one when `active`.
    #[must_use]
    pub fn active(mut self, active: bool) -> Self {
        self.active = Some(active);
        self
    }

    /// Sets the name shown above the page's top-left corner.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the page's own fill under the document.
    #[must_use]
    pub fn fill(mut self, fill: DataColor) -> Self {
        self.fill = Some(PageFill::Color(fill));
        self
    }

    /// Fills the page with a transparency grid laid from its top-left, under the document.
    #[must_use]
    pub fn checker(mut self, checker: Checker) -> Self {
        self.fill = Some(PageFill::Checker(checker));
        self
    }

    /// Whether a double press on the page's name renames the page.
    #[must_use]
    pub fn renames(mut self, on: bool) -> Self {
        self.renames = on;
        self
    }
}

/// The style of the pasteboard.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PasteboardStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The pasteboard unless the canvas sets a colour, before the pattern preference is applied.
    surround: SurroundLook,
    /// A page's drop shadow: the floating surface's shadow alone, blurred under a page of an unturned view.
    shadow: SurfaceStyle,
    /// The shadow's colour and offset, for the plain offset copy under a page of a turned view.
    turned_shadow: StyleColor,
    shadow_offset: Vec2,
    /// A page's outline, and the edge of an artboard that is not the active one.
    outline: StyleColor,
    /// The edge of the active artboard.
    active_edge: StyleColor,
    /// The width of a page or artboard edge.
    edge: f32,
    /// A page name in the muted ink, and in the normal ink for the active artboard.
    name: StyleColor,
    name_active: StyleColor,
    /// Between a page's top edge and its name.
    name_gap: f32,
}

/// What the pasteboard reports: its response, the page name clicked and the pasteboard menu request.
#[derive(Debug)]
#[non_exhaustive]
pub struct PasteboardOutput {
    /// The pasteboard's response.
    pub response: Response,
    /// A page name was clicked: the index of its frame and the modifiers of the release. The app selects the artboard
    /// or page.
    pub name_clicked: Option<(usize, Mods)>,
    /// The rename of a page's name: the draft edited, committed or cancelled.
    pub rename: Option<EditEvent<Rename<usize>>>,
    /// The secondary button was released on the canvas background outside every page and name (a press no canvas
    /// control claimed), at this document point, or the Menu key or Shift+F10 reached the focused canvas (at the
    /// view's centre): the app opens its pasteboard menu there.
    pub context_at: Option<Point>,
}

impl Outcome for PasteboardOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The pasteboard and page frames under a document. Show it first in the canvas body, then the document, then the
/// overlays. The pages' shadows and fills paint in the canvas's picture slot, ahead of the pictures shown after it, so
/// a picture under the controls ([`RasterDisplay::picture`](super::RasterDisplay::picture)) lies over them. The
/// surround of plain pages paints there too; the surround between artboards (pages marked with [`PageFrame::active()`]) paints over the
/// pictures, hiding the document outside the boards except where [`uncovered`](Self::uncovered) leaves it open, unless
/// [`shows_outside`](Self::shows_outside) puts it under them. A double press on the name of a frame that
/// [`renames`](PageFrame::renames) opens a rename field over it, reported in [`PasteboardOutput::rename`]. The
/// page edges and names paint in place, over the pictures. The canvas must be given a view; without one nothing is
/// shown.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(canvas_control, content_painter, overlay)]
pub struct Pasteboard<'a> {
    #[common]
    common: Common,
    pages: &'a [PageFrame],
    /// The border round each page; a drop shadow by default.
    #[prop]
    border: PageBorder,
    /// How the artboards' edges show the active one; the accent by default.
    #[prop]
    artboard_edge: ArtboardEdge,
    /// Document rects the pasteboard does not cover, subtracted like the pages.
    #[prop]
    uncovered: &'a [DocRect],
    /// The surround paints under the pictures while the frames keep `active` edges and names.
    #[prop(flag)]
    shows_outside: bool,
    /// The size of each page's name, measured in `parts`; `None` for a page without one and for one whose page is
    /// narrower than the name's readable least.
    name_sizes: Vec<Option<Vec2>>,
    /// The page whose name is being renamed, read from the rename's memory in `update`.
    renaming_at: Option<usize>,
    name_clicked: Option<(usize, Mods)>,
    rename: Option<EditEvent<Rename<usize>>>,
    context_at: Option<Point>,
}

impl<'a> Pasteboard<'a> {
    /// A pasteboard named `label` around `pages`. A single-image document passes one page: its bounds.
    pub fn new(label: impl Into<Label>, pages: &'a [PageFrame]) -> Self {
        Pasteboard {
            common: Common::new(),
            pages,
            border: PageBorder::DropShadow,
            artboard_edge: ArtboardEdge::Accent,
            uncovered: &[],
            shows_outside: false,
            name_sizes: Vec::new(),
            renaming_at: None,
            name_clicked: None,
            rename: None,
            context_at: None,
        }
        .label(label)
    }

    /// The pasteboard as screen quads: the canvas minus the pages and the `uncovered` rects (document units), each
    /// piece as its four corners (a turned view needs quads, not rects). For an app that draws the pasteboard itself.
    pub fn quads(view: &CanvasView, pages: &[PageFrame], uncovered: &[DocRect]) -> Vec<[Pos2; 4]> {
        let canvas = view.visible();
        if !(canvas.x1 > canvas.x0 && canvas.y1 > canvas.y0 && canvas.area().is_finite()) {
            return Vec::new();
        }
        let boards: Vec<DocRect> =
            pages.iter().map(|page| page.rect).chain(uncovered.iter().copied()).filter_map(|rect| clip(rect.abs(), canvas)).collect();
        let mut xs = vec![canvas.x0, canvas.x1];
        for board in &boards {
            xs.extend([board.x0, board.x1]);
        }
        xs.sort_by(f64::total_cmp);
        xs.dedup();
        let mut out = Vec::new();
        for pair in xs.windows(2) {
            let (x0, x1) = (pair[0], pair[1]);
            let mut spans: Vec<(f64, f64)> =
                boards.iter().filter(|board| board.x0 <= x0 && board.x1 >= x1).map(|board| (board.y0, board.y1)).collect();
            spans.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut y = canvas.y0;
            for (top, bottom) in spans {
                if top > y {
                    out.push(view.quad(DocRect::new(x0, y, x1, top)));
                }
                y = y.max(bottom);
            }
            if y < canvas.y1 {
                out.push(view.quad(DocRect::new(x0, y, x1, canvas.y1)));
            }
        }
        out
    }

    /// The page under the document point `at`, if any; of overlapping pages the last one.
    pub fn page_at(pages: &[PageFrame], at: Point) -> Option<usize> {
        pages.iter().rposition(|page| page.rect.abs().contains(at))
    }
}

/// `rect` clipped to `canvas`, or `None` when they share no area.
fn clip(rect: DocRect, canvas: DocRect) -> Option<DocRect> {
    let clipped = DocRect::new(rect.x0.max(canvas.x0), rect.y0.max(canvas.y0), rect.x1.min(canvas.x1), rect.y1.min(canvas.y1));
    (clipped.x1 > clipped.x0 && clipped.y1 > clipped.y0).then_some(clipped)
}

fn bounds(quad: &[Pos2; 4]) -> Rect {
    Rect::from_points(quad)
}

/// The run a page name is shown and measured with.
fn name_run<'n>(name: &'n str, color: StyleColor) -> TextRun<'n> {
    TextRun::new(name).role(TypeRole::Caption).color(color)
}

fn name_color(style: &PasteboardStyle, active: Option<bool>) -> StyleColor {
    if active == Some(true) { style.name_active } else { style.name }
}

/// The page's top-left corner on screen: the origin of its checker and the anchor of its name, whatever the view's
/// turn or flip.
fn page_corner(view: &CanvasView, page: &PageFrame) -> Pos2 {
    view.to_screen(Point::new(page.rect.x0.min(page.rect.x1), page.rect.y0.min(page.rect.y1)))
}

/// The rect of a page's name: above its top-left corner on screen, `size` big.
fn name_rect(view: &CanvasView, page: &PageFrame, gap: f32, size: Vec2) -> Rect {
    let corner = page_corner(view, page);
    Rect::from_min_size(Pos2::new(corner.x, corner.y - gap - size.y), size)
}

/// The rename of a page's name: the page, its gesture, its name when it began and the text being typed. Kept in the
/// session so the draft lasts while the field shows.
#[derive(Clone, Default)]
struct Renaming {
    open: Option<RenameOpen>,
    draft: String,
    /// Where the field showed last: a field that closes fades out there, with no name left to place it by.
    shown: Option<Rect>,
}

/// The fixed part of an open rename.
#[derive(Clone)]
struct RenameOpen {
    at: usize,
    gesture: GestureId,
    name: String,
}

impl RenameOpen {
    fn before(&self) -> Mixed<Rename<usize>> {
        Mixed::Same(Rename::new(self.at, self.name.clone()))
    }

    /// The end of the rename by `typed`: a commit of a changed, non-empty name, else a cancel for `reason`.
    fn end(&self, typed: String, reason: CancelReason) -> EditEvent<Rename<usize>> {
        if typed != self.name && !typed.trim().is_empty() {
            EditEvent::commit(self.gesture, self.before(), Rename::new(self.at, typed))
        } else {
            EditEvent::cancel(self.gesture, reason, Some(self.before()))
        }
    }
}

impl Component for Pasteboard<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = true;
    type Style = PasteboardStyle;
    type Output = PasteboardOutput;

    fn style(cx: &StyleCx<'_>, _: PasteboardStyleKey) -> PasteboardStyle {
        let tokens = cx.tokens;
        let color = &tokens.color;
        let cast = cx.surface(SurfaceKind::Floating).shadow;
        let (ink, offset) =
            cast.map_or((Color32::TRANSPARENT, Vec2::ZERO), |cast| (cast.color, Vec2::new(f32::from(cast.offset[0]), f32::from(cast.offset[1]))));
        PasteboardStyle {
            metrics: Metrics::new(),
            surround: SurroundLook::of(tokens),
            shadow: SurfaceStyle { fill: Color32::TRANSPARENT, radius: CornerRadius::ZERO, edge: SurfaceEdge::None, shadow: cast },
            turned_shadow: cx.color(Rgba(ink.to_srgba_unmultiplied())),
            shadow_offset: offset,
            outline: cx.color(color.line.border),
            active_edge: cx.color(color.accent.solid),
            edge: tokens.stroke.thin,
            name: cx.color(color.text.muted),
            name_active: cx.color(color.text.normal),
            name_gap: tokens.space.gap.inline,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    /// The page names, each cut to its page's on-screen width and left out when the page is narrower than the name's
    /// readable least: a click away from the tool. They have no keyboard path of their own; the app's artboard list
    /// selects the same page. Then one inert part per visible pasteboard piece, so a press on the pasteboard keeps a
    /// typed draft (a zoom being typed) unapplied, and a press on a page, which the pieces leave out, applies it. The
    /// press itself still reaches the canvas's background part.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        self.name_sizes = self
            .pages
            .iter()
            .map(|page| {
                let run = name_run(page.name.as_ref()?, name_color(cx.style, page.active)).elide();
                let width = bounds(&space.view.quad(page.rect)).width();
                (width >= cx.least(&run).width).then(|| cx.layout_at(&run, width).size())
            })
            .collect();
        for (at, page) in self.pages.iter().enumerate() {
            let Some(size) = self.name_sizes[at] else { continue };
            cx.part(NAME.at(at as u64), name_rect(&space.view, page, cx.style.name_gap, size))
                .sense(Sense::CLICK)
                .priority(HitPriority::new(HitLevel::Body, 0))
                .pointer_only("the application's artboard list selects the same page");
        }
        let visible = Self::quads(&space.view, self.pages, self.uncovered).into_iter().filter(|quad| bounds(quad).intersects(space.view.rect));
        for (at, quad) in visible.enumerate() {
            cx.part(QUAD.at(at as u64), HitShape::Polygon(quad.to_vec()))
                .sense(Sense::empty())
                .holds_drafts()
                .pointer_only("an inert area that takes no input; it only marks where a press keeps a typed draft");
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let popup = cx.popup("rename");
        for (at, page) in self.pages.iter().enumerate() {
            let (Some(name), Some(_)) = (&page.name, self.name_sizes.get(at).copied().flatten()) else { continue };
            let part = NAME.at(at as u64);
            if cx.part_activated(part).is_some() {
                self.name_clicked = Some((at, crate::runtime::pointer::frame(&cx.core.ctx).pick_mods));
            }
            // Only the pass that receives the second press begins; a rerun pass finds the rename open.
            if page.renames && cx.keep::<Renaming>().open.is_none() && cx.part_if_declared(part).is_some_and(|result| result.press.double_pressed()) {
                let gesture = crate::runtime::edit::next_gesture_id(&cx.core.ctx);
                let open = RenameOpen { at, gesture, name: name.clone() };
                self.rename = Some(EditEvent::begin(gesture, open.before(), cx.core.label_arc(), None));
                *cx.keep::<Renaming>() = Renaming { draft: name.clone(), open: Some(open), shown: None };
                popup.open(cx);
            }
        }
        self.renaming_at = cx.keep::<Renaming>().open.as_ref().map(|open| open.at);
        let background = space.background();
        if let Some(pos) = background.context {
            let doc = space.view.to_doc(pos);
            if Self::page_at(self.pages, doc).is_none() {
                self.context_at = Some(doc);
            }
        } else if background.context_key {
            self.context_at = Some(space.view.center);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let (view, style) = (space.view, cx.style);
        let draw = ui.draw();
        let edges = ui.content_painter(cx);
        // What lies under the pages (their shadows and fills) marks where it starts; it moves into the canvas's picture
        // slot, in show order before the pictures, so every picture lies over it. The surround of plain pages goes
        // with it, so a picture reaching past the page (copies around it, pixels past the canvas) shows over it; the
        // surround between artboards paints over the pictures and hides the document there, except where `uncovered`
        // leaves it open. The page edges and names, which mark the pages, paint in place over the pictures. Outside a
        // canvas, or in a space with no slot, everything paints in place.
        let under = space.picture_slot().map(|_| PaintSlot::reserve(&edges.painter));
        let artboards = self.pages.iter().any(|page| page.active.is_some()) && !self.shows_outside;
        let polygon = |quad: &[Pos2; 4]| PaintShape::Polygon(quad.to_vec());
        // A colour the canvas sets is a plain data fill, with no pattern, as in production.
        let fill = space.pasteboard().fill();
        let look = style.surround.shown(cx.env.canvas_pattern());
        let surround = || {
            for quad in Self::quads(&view, self.pages, self.uncovered).iter().filter(|quad| bounds(quad).intersects(view.rect)) {
                match fill {
                    Some(color) => draw.data_fill(&polygon(quad), color),
                    None => paint_surround(&edges.painter, CheckerArea::Quad(*quad), &look),
                }
            }
        };
        if !artboards {
            surround();
        }
        let aligned = view.axis_aligned();
        // An edge in `color`, at `stroke.thin` or a hairline: `rule` is the same colour as the snapped rule paints it.
        let edge = |quad: &[Pos2; 4], rule: Color32, color: StrokeColor, hairline: bool| {
            if aligned {
                let rect = bounds(quad);
                let look = RuleLook { color: rule, width: if hairline { RuleWidth::Hairline } else { RuleWidth::Points(style.edge) } };
                let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
                for (from, to) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)) {
                    edges.painter.extend(Rule::shapes(&look, from, to, edges.pixels_per_point()));
                }
            } else {
                draw.stroke(&polygon(quad), if hairline { StrokeWidth::Hairline } else { StrokeWidth::Thin }, color);
            }
        };
        let shown = |page: &PageFrame| page.name.is_some() || bounds(&view.quad(page.rect)).intersects(view.rect);
        for page in self.pages.iter().filter(|page| shown(page)) {
            let quad = view.quad(page.rect);
            if self.border == PageBorder::DropShadow {
                if aligned {
                    draw.surface_styled(bounds(&quad), &style.shadow);
                } else {
                    draw.fill(&polygon(&quad.map(|corner| corner + style.shadow_offset)), style.turned_shadow);
                }
            }
            match page.fill {
                Some(PageFill::Color(color)) => draw.data_fill(&polygon(&quad), color),
                Some(PageFill::Checker(checker)) => {
                    let grid = checker.shape(edges.painter.ctx(), &ui.env.tokens, CheckerArea::Quad(quad), page_corner(&view, page));
                    edges.painter.add(grid);
                }
                None => {}
            }
        }
        if let Some(under) = under {
            let ctx = edges.painter.ctx().clone();
            let shapes = under.take_after(&ctx);
            space.paint_picture(&ctx, shapes);
        }
        if artboards {
            surround();
        }
        for (at, page) in self.pages.iter().enumerate().filter(|(_, page)| shown(page)) {
            let quad = view.quad(page.rect);
            if self.border == PageBorder::Line {
                edge(&quad, style.outline.egui(), style.outline.into(), false);
            }
            if let Some(active) = page.active {
                match self.artboard_edge {
                    ArtboardEdge::Accent => {
                        let color = if active { style.active_edge } else { style.outline };
                        edge(&quad, color.egui(), color.into(), false);
                    }
                    ArtboardEdge::Plain => {
                        let color = if active { PLAIN_ACTIVE } else { PLAIN_OTHER };
                        edge(&quad, color.0, color.into(), !active);
                    }
                }
            }
            if let (Some(name), Some(size)) = (&page.name, self.name_sizes.get(at).copied().flatten()) {
                let rect = name_rect(&view, page, style.name_gap, size);
                // The field of an open rename stands in the name's place.
                if rect.intersects(view.rect) && self.renaming_at != Some(at) {
                    draw.text(rect, &name_run(name, name_color(style, page.active)).elide());
                }
            }
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        for (at, page) in self.pages.iter().enumerate() {
            if let (Some(name), Some(_)) = (&page.name, self.name_sizes.get(at).copied().flatten()) {
                access.node(NAME.at(at as u64), Role::BUTTON).name = Name::Text(name.clone());
            }
        }
    }

    /// The rename field over a page's name while a rename is open. The field ends the rename: Enter or a blur commits a
    /// changed, non-empty name, Escape or an unchanged name cancels. A page that went away or whose name is no longer
    /// shown cancels it.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("rename");
        let state = cx.keep::<Renaming>().clone();
        let Some(space) = canvas_space(&cx.env) else { return };
        let field = state.open.as_ref().and_then(|open| {
            let page = self.pages.get(open.at)?;
            let size = self.name_sizes.get(open.at).copied().flatten()?;
            let rect = name_rect(&space.view, page, cx.style.name_gap, size);
            // Typing needs room beyond a name cut to its page: the field is as wide as the page.
            Some(Rect::from_min_size(rect.min, Vec2::new(rect.width().max(bounds(&space.view.quad(page.rect)).width()), rect.height())))
        });
        let layer = Overlay::popover()
            .dismiss(Dismiss::new().outside(Outside::Keep))
            .focus(FocusPolicy::POPUP.initial(Initial::First))
            .role(LayerRole::InPlaceEditor);
        let layer = match field.or(state.shown) {
            Some(rect) => layer.placed(rect),
            None => layer,
        };
        let mut draft = state.draft;
        let editor = ui.show(popup, layer, |ui, _| {
            let (open, field) = (state.open.as_ref()?, field?);
            let size = field.size();
            let at = Rect::from_min_size(ui.max_rect().min, size);
            Some(crate::text::rename_field(Bind::bound(&mut draft), &open.name).show_at(ui, at))
        });
        let Some(open) = state.open else { return };
        // `None`: the popup is not shown yet (it opens at the end of the pass that began the rename).
        let Some(editor) = editor else { return };
        let cancel = |reason| EditEvent::cancel(open.gesture, reason, Some(open.before()));
        let end = match editor {
            Some(editor) => editor
                .events
                .iter()
                .rev()
                .find_map(|event| match event {
                    EditEvent::Commit { after, .. } => Some(open.end(after.clone(), CancelReason::Unchanged)),
                    EditEvent::Cancel { reason, .. } => Some(cancel(*reason)),
                    _ => None,
                })
                .or_else(|| editor.escaped.then(|| cancel(CancelReason::Escape)))
                .or_else(|| editor.submitted.then(|| open.end(draft.clone(), CancelReason::Unchanged)))
                .or_else(|| ui.closing(popup).map(|_| cancel(CancelReason::Dismissed))),
            // The page or its name is gone: nothing to edit.
            None => Some(cancel(CancelReason::Removed)),
        };
        match end {
            Some(end) => {
                self.rename = Some(end);
                *cx.keep::<Renaming>() = Renaming { shown: field.or(state.shown), ..Renaming::default() };
                popup.close(cx);
            }
            None => {
                let kept = cx.keep::<Renaming>();
                kept.draft = draft;
                kept.shown = field;
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> PasteboardOutput {
        PasteboardOutput { response: cx.response.clone(), name_clicked: self.name_clicked, rename: self.rename, context_at: self.context_at }
    }
}
