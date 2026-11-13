//! `RasterDisplay`: the document display of a canvas. It draws the document through the canvas's
//! [`CanvasSpace`](super::CanvasSpace) over a checkerboard, with the edge rule around it. The picture is either the
//! document's composite as RGBA for a region ([`RasterSource`]), which the display keeps in `TextureStore` textures and
//! re-reads only where the source reports damage, or the app's own paint ([`AppPaint`]), such as a GPU paint callback
//! over the app's compositor.
//!
//! A document larger than the device's texture limit (`texture_side`) is uploaded reduced; zoomed in, the part in
//! view is added at the screen's resolution as a detail texture ([`DetailPlan`]), so a large document shows its own
//! pixels at 100 percent and the cost follows the window, not the document. Pixels are drawn sharp (nearest) from
//! 100 percent up and filtered below.
//!
//! Past the document's edge the display shows nothing, the source's pixels out to a larger region (a crop preview of
//! layers past the canvas), or copies of the document around it (a pattern preview); see [`Extent`].
//!
//! The display takes no pointer and no keys and declares no parts: a press on it reaches the canvas's tool.

mod detail;
#[cfg(feature = "testing")]
mod stories;

pub use detail::{DetailPlan, snap_to_texels};

use super::{AppPaint, PixelRect, RasterSource, canvas_space};
use crate::contract::{Access, Common, Component, Cx, NoValue, Outcome, PaintUi, Props as _, Role};
use crate::primitives::textures::UNTINTED;
use crate::primitives::{
    CachedTexture, Checker, CheckerArea, Draw, Revision, RuleKind, TextureState, TextureStore, TextureStoreOptions, texture_factor, texture_side,
};
use crate::runtime::env::Label;
use crate::runtime::memory::Remember as _;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::geometry::quad::Homography;
use craft_behavior::kurbo::{Point as Point64, Rect as Rect64};
use craft_behavior::{CanvasView, Metrics, PixelSize};
use detail::{texel_extent, texel_origin};
use egui::epaint::{Mesh, Vertex};
use egui::{Color32, ColorImage, Context, Id, Pos2, Response, Shape, TextureFilter, TextureOptions, Vec2, pos2};
use std::collections::HashMap;
use std::ops::RangeInclusive;

/// The style of a raster display: it has no look of its own.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct RasterStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// How far a display's picture reaches past the document's edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Extent {
    /// The document alone.
    #[default]
    Document,
    /// The document and the pixels around it out to this region (Crop's preview of layers past the canvas). A source
    /// picture reads the region from the source; a painted picture is expected to paint it. The checkerboard covers
    /// the region; the edge stays at the document's edge.
    Region(PixelRect),
    /// Copies of the document a whole document size apart, filling the canvas around it (Pattern Preview), at most
    /// 31 along each axis, centred on the view. A [mapped](RasterDisplay::mapped) picture repeats its area instead, a
    /// whole area apart. A source picture draws its textures again for each copy; a painted picture is
    /// called once per copy with the view shifted by the copy's offset. The checkerboard covers the copies; the edge
    /// stays at the document's edge.
    Tiled,
}

/// Most copies a tiled display draws along each axis, the document included. A tiny document zoomed far out would
/// otherwise need thousands of copies per pass; past this the pattern stops, and the run is centred on the view.
const MOST_COPIES: i32 = 31;

/// The copies a tiled display draws, as columns and rows of whole picture areas from the picture: copy `(i, j)` is
/// the picture moved by `i` widths and `j` heights of its area. The picture itself is drawn whether or not it is in
/// the run.
#[derive(Clone, Debug, PartialEq)]
struct Tiles {
    columns: RangeInclusive<i32>,
    rows: RangeInclusive<i32>,
    /// The picture's area, in document pixels: the document, or a mapped picture's area. Copies lie a whole area apart.
    cell: Rect64,
}

impl Tiles {
    /// The picture alone.
    fn single() -> Self {
        Tiles { columns: 0..=0, rows: 0..=0, cell: Rect64::ZERO }
    }

    /// The copies of the picture covering `cell` that cover `visible`, the document area a view shows.
    fn covering(visible: Rect64, cell: Rect64) -> Self {
        let columns = tile_run(visible.x0 - cell.x0, visible.x1 - cell.x0, cell.width());
        let rows = tile_run(visible.y0 - cell.y0, visible.y1 - cell.y0, cell.height());
        match (columns, rows) {
            (Some(columns), Some(rows)) => Tiles { columns, rows, cell },
            _ => Self::single(),
        }
    }

    /// Whether only the picture itself is drawn.
    fn is_single(&self) -> bool {
        self.columns == (0..=0) && self.rows == (0..=0)
    }

    /// The offset in document pixels of each copy to draw, the picture's own first.
    fn offsets(&self) -> impl Iterator<Item = Vec2> + '_ {
        let (width, height) = (self.cell.width() as f32, self.cell.height() as f32);
        let copies = self.rows.clone().flat_map(move |j| self.columns.clone().map(move |i| (i, j))).filter(|&copy| copy != (0, 0));
        std::iter::once((0, 0)).chain(copies).map(move |(i, j)| Vec2::new(i as f32 * width, j as f32 * height))
    }

    /// The area the run of copies covers, in document pixels. A capped run may leave out the picture.
    fn run(&self) -> Rect64 {
        let (width, height) = (self.cell.width(), self.cell.height());
        Rect64::new(
            self.cell.x0 + f64::from(*self.columns.start()) * width,
            self.cell.y0 + f64::from(*self.rows.start()) * height,
            self.cell.x0 + f64::from(*self.columns.end() + 1) * width,
            self.cell.y0 + f64::from(*self.rows.end() + 1) * height,
        )
    }
}

/// The checkerboard's areas in document pixels: what the picture covers (`bounds`) and the run of copies (`run`), as one
/// rect when one holds the other and as two otherwise, so no checkerboard shows where nothing is drawn over it.
fn checker_areas(bounds: Rect64, run: Rect64) -> impl Iterator<Item = Rect64> {
    let hull = bounds.union(run);
    let areas = if hull == bounds || hull == run { [Some(hull), None] } else { [Some(run), Some(bounds)] };
    areas.into_iter().flatten()
}

/// The tiles `len` pixels long along one axis that overlap `lo..hi`, at most [`MOST_COPIES`] of them, centred on the
/// middle of `lo..hi` when more would fit; `None` for an empty or non-finite span.
fn tile_run(lo: f64, hi: f64, len: f64) -> Option<RangeInclusive<i32>> {
    if !(len > 0.0 && len.is_finite()) || !lo.is_finite() || !hi.is_finite() || hi <= lo {
        return None;
    }
    let n = len;
    // Far past any real view; keeps the ± MOST_COPIES arithmetic inside i32.
    let index = |v: f64| v.clamp(-1e9, 1e9) as i32;
    let (first, last) = (index((lo / n).floor()), index((hi / n).ceil()) - 1);
    if last.saturating_sub(first) < MOST_COPIES {
        return Some(first..=last);
    }
    let middle = index(((lo + hi) / 2.0 / n).floor());
    let half = MOST_COPIES / 2;
    Some(middle - half..=middle + half)
}

/// `view` looking `offset` document pixels back, so the document drawn through it lands `offset` along on screen.
fn shifted(mut view: CanvasView, offset: Vec2) -> CanvasView {
    view.center -= craft_behavior::kurbo::Vec2::new(f64::from(offset.x), f64::from(offset.y));
    view
}

/// Which picture of a display a texture holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Level {
    /// What the picture covers (the document, or the region of [`Extent::Region`]) at one texel per `texture_factor`
    /// pixels. A new region is a new key, so a texture never shows one region at another's place.
    Base(PixelRect),
    /// The part in view at the screen's resolution. A new plan is a new key, so a texture is never partly of
    /// two plans.
    Detail(DetailPlan),
}

/// What a display shows.
enum Picture<'a> {
    /// The source's pixels, kept in the display's textures.
    Source(&'a dyn RasterSource),
    /// The app paints the document of `size` pixels itself, through the view it is given.
    Painted { size: PixelSize, paint: AppPaint<'a, CanvasView> },
}

impl Picture<'_> {
    fn size(&self) -> PixelSize {
        match self {
            Picture::Source(source) => source.size(),
            Picture::Painted { size, .. } => *size,
        }
    }
}

/// The textures of a source picture: the whole document at one texel per `factor` pixels, and the part in view at the
/// screen's resolution while zoomed in.
struct Textures {
    base: (CachedTexture, u32),
    detail: Option<(CachedTexture, DetailPlan)>,
}

/// What `update` leaves for `paint` and `finish`: the view to draw through, what the picture covers and where its
/// copies go, and, for a source picture, its textures.
struct Shown {
    view: CanvasView,
    size: PixelSize,
    /// The document, or the region of [`Extent::Region`] around it.
    bounds: PixelRect,
    tiles: Tiles,
    textures: Option<Textures>,
}

/// The revision each texture of a display shows, which is what the source is asked about: what changed since then.
/// It lives as long as the display does, so a pass the display skipped loses no damage.
#[derive(Clone, Default)]
struct Held(HashMap<Level, Revision>);

/// What a display did in a pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct RasterDisplayOutput {
    /// The display's response.
    pub response: Response,
    /// The pixels read from the source this pass (damage, a new detail region, the first pass).
    pub read: Option<PixelRect>,
    /// The detail texture in use this pass, if any.
    pub detail: Option<DetailPlan>,
}

impl Outcome for RasterDisplayOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The document display of one canvas: the document drawn through the view of the [`Canvas`](super::Canvas) it is
/// shown in, at the canvas rect. Outside a canvas, or in one without a view, it draws nothing.
///
/// - **Picture.** [`new`](Self::new) draws a [`RasterSource`]'s pixels from textures the display keeps;
///   [`painted`](Self::painted) calls the app's paint once per pass, in the display's place between the checkerboard
///   and the edge, with the view on screen. Both pictures get the same checkerboard and edge; the shadow is the
///   [`Pasteboard`](super::Pasteboard)'s and the pixel grid the `PixelGrid` control's, shown around and over either.
/// - **Reading.** The display asks the source for pixels only when it has none for the source's `revision`. A
///   changed revision re-reads what [`RasterSource::damage`] reports since the revision the display shows (snapped
///   outward to whole texels), or everything where the source cannot tell.
/// - **Transparency.** [`checkerboard`](Self::checkerboard) draws a transparency grid under the document, holding
///   still while the document scrolls; the app's [`Checker`] sets its cells, and an empty one is the theme's.
///   Without it the document draws straight on the canvas (an opaque document, a video frame).
/// - **Extent.** [`extent`](Self::extent) lets the picture reach past the document's edge: out to a larger region,
///   or as copies around it ([`Extent`]).
/// - **Area.** The picture covers the document, 0..size, unless [`mapped`](Self::mapped) places it elsewhere in
///   document space (a size-changing preview centred off the document's origin, a plane in perspective): the
///   checkerboard and the edge then lie on the mapped area, for a source and a painted picture alike.
/// - **Edge.** [`outlined`](Self::outlined) draws the divider rule just outside the picture's area, so an image
///   reads apart from the surface around it.
/// - **Turned views.** A rotated or mirrored view draws each texture as one quad on the view's mapped corners.
/// - **Memory.** The textures live in two `TextureStore`s per display id and go with the store's budget; a
///   detail texture of a plan no longer in use is dropped in the pass the plan changes.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct RasterDisplay<'a> {
    #[common]
    common: Common,
    picture: Picture<'a>,
    /// The transparency grid under the document; none draws the document straight on the canvas.
    #[prop]
    checkerboard: Option<Checker>,
    /// The divider rule just outside the document's edge.
    #[prop(flag)]
    outlined: bool,
    /// How far the picture reaches past the document's edge.
    #[prop]
    extent: Extent,
    read: PixelRect,
    shown: Option<Shown>,
    mapping: Option<Mapping>,
}

/// Where a mapped display's picture lies in document space; a source picture is drawn through the homography on a
/// mesh of cells instead of as a flat quad.
#[derive(Clone, Copy, Debug)]
struct Mapping {
    frame: Homography,
    cells: u32,
    /// Where the image lies in the frame's unit square.
    placed: Rect64,
}

impl Mapping {
    /// The picture's area on screen through `view`, clockwise from the top left: the frame's image of `placed`
    /// clipped to the unit square, where the checkerboard and the edge go. `None` when nothing of `placed` lies in
    /// the unit square.
    fn corners(&self, view: &CanvasView) -> Option<[Pos2; 4]> {
        Some(self.doc_corners()?.map(|corner| view.to_screen(corner)))
    }

    /// The picture's area in document space, clockwise from the top left; `None` as for [`corners`](Self::corners).
    fn doc_corners(&self) -> Option<[Point64; 4]> {
        let area = self.placed.intersect(Rect64::new(0.0, 0.0, 1.0, 1.0));
        if area.width() <= 0.0 || area.height() <= 0.0 {
            return None;
        }
        let corners = [(area.x0, area.y0), (area.x1, area.y0), (area.x1, area.y1), (area.x0, area.y1)];
        Some(corners.map(|(x, y)| self.frame.apply(Point64::new(x, y))))
    }

    /// The bounds of the picture's area in document space: the cell its Pattern Preview copies repeat.
    fn area(&self) -> Option<Rect64> {
        let corners = self.doc_corners()?;
        let area = corners[1..].iter().fold(Rect64::from_points(corners[0], corners[0]), |area, corner| area.union_pt(*corner));
        (area.width() > 0.0 && area.height() > 0.0 && area.area().is_finite()).then_some(area)
    }
}

impl<'a> RasterDisplay<'a> {
    /// A display named `label` of the document `source` gives. An empty label makes a display that only paints, with
    /// no AccessKit node: a further image over a named one.
    pub fn new(label: impl Into<Label>, source: &'a dyn RasterSource) -> Self {
        Self::of(label, Picture::Source(source))
    }

    /// A display named `label` of a document of `size` pixels that the app paints itself with `paint`, given the view
    /// on screen (its `rect` is the canvas rect). The app draws the document's pixels only: the display draws the
    /// checkerboard under them and the edge around them, as for a source. A picture that covers another area than
    /// 0..size says where with [`mapped`](Self::mapped).
    pub fn painted(label: impl Into<Label>, size: PixelSize, paint: AppPaint<'a, CanvasView>) -> Self {
        Self::of(label, Picture::Painted { size, paint })
    }

    fn of(label: impl Into<Label>, picture: Picture<'a>) -> Self {
        RasterDisplay {
            common: Common::new(),
            picture,
            checkerboard: None,
            outlined: false,
            extent: Extent::Document,
            read: PixelRect::default(),
            shown: None,
            mapping: None,
        }
        .label(label)
    }

    /// Places the picture at `frame` (the unit square onto document space): the image fills the unit square unless
    /// [`placed`](Self::placed) says otherwise, and the checkerboard and the edge lie on that area, clipped to the
    /// unit square. A source picture is drawn through the frame on a `cells` by `cells` mesh, so a perspective
    /// frame shows it foreshortened; a cell whose centre lies outside the unit square is left out. A painted
    /// picture is painted through the view as the app paints it, and `cells` does not apply. A mapped display draws
    /// its whole-document texture alone: the detail texture of a flat display has no place on a mesh, and neither has
    /// the region of [`Extent::Region`]. [`Extent::Tiled`] repeats the area.
    #[must_use]
    pub fn mapped(mut self, frame: Homography, cells: u32) -> Self {
        let placed = self.mapping.map_or(Rect64::new(0.0, 0.0, 1.0, 1.0), |mapping| mapping.placed);
        self.mapping = Some(Mapping { frame, cells: cells.max(1), placed });
        self
    }

    /// Where the image lies in the mapped frame's unit square (default: all of it). A mapped display only.
    #[must_use]
    pub fn placed(mut self, placed: Rect64) -> Self {
        if let Some(mapping) = self.mapping.as_mut() {
            mapping.placed = placed;
        }
        self
    }

    /// The store that holds this display's textures of one sampling. A texture is sharp (nearest) from 100 percent
    /// up, which is when one texel is one document pixel; a reduced texture holds averages and stays filtered.
    fn store(ctx: &Context, id: Id, sharp: bool) -> TextureStore<Level> {
        let magnification = if sharp { TextureFilter::Nearest } else { TextureFilter::Linear };
        let filter = TextureOptions { magnification, ..TextureOptions::LINEAR };
        TextureStore::in_context(ctx, (id, sharp), TextureStoreOptions::pages().filter(filter))
    }

    fn read_region(source: &dyn RasterSource, region: PixelRect, factor: u32) -> ColorImage {
        let mut image = ColorImage::filled(texel_extent(region, factor), Color32::TRANSPARENT);
        source.read(region, factor, &mut image);
        image
    }

    /// The texture of `level`, which holds `region` of the document at one texel per `factor` pixels, brought up to
    /// the source's revision, and the pixels read to do it.
    /// - A texture with the revision is kept.
    /// - A texture of an older revision is updated with what the source reports damaged since then: that part is
    ///   read again alone, and damage elsewhere moves the texture to the revision without a read.
    /// - Anything else (no texture, another size, damage the source cannot tell) is read whole.
    fn bring_up_to_date(
        source: &dyn RasterSource,
        ctx: &Context,
        id: Id,
        held: &mut Held,
        level: Level,
        region: PixelRect,
        factor: u32,
    ) -> (CachedTexture, PixelRect) {
        let sharp = factor == 1;
        // A key never lives in both samplings: the other store's copy is from before the factor changed.
        Self::store(ctx, id, !sharp).forget(ctx, &level);
        let store = Self::store(ctx, id, sharp);
        let revision = source.revision();
        let origin = [region.x0, region.y0];
        let expected = texel_extent(region, factor);
        let state = store.state(ctx, &level, revision);
        let current = state.shown().filter(|texture| [texture.size.width as usize, texture.size.height as usize] == expected);
        if let Some(texture) = current {
            if matches!(state, TextureState::Ready(_)) {
                held.0.insert(level, revision);
                return (texture, PixelRect::default());
            }
            if let Some(damage) = held.0.get(&level).and_then(|since| source.damage(*since)) {
                let dirty = snap_to_texels(damage.intersect(&region), origin, factor).intersect(&region);
                let updated = if dirty.is_empty() {
                    // The pixels changed elsewhere: no texel of this texture is new, and the entry takes the revision.
                    store.put_region(ctx, &level, revision, [0, 0], ColorImage::new([0, 0], Vec::new())).map(|texture| (texture, dirty))
                } else {
                    let image = Self::read_region(source, dirty, factor);
                    store.put_region(ctx, &level, revision, texel_origin(dirty, origin, factor), image).map(|texture| (texture, dirty))
                };
                if let Some(updated) = updated {
                    held.0.insert(level, revision);
                    return updated;
                }
            }
        }
        held.0.insert(level, revision);
        (store.put(ctx, level, revision, Self::read_region(source, region, factor)), region)
    }

    /// Drops every texture of this display.
    fn release(ctx: &Context, id: Id) {
        for sharp in [true, false] {
            Self::store(ctx, id, sharp).clear(ctx);
        }
    }

    /// Drops the textures of other regions than `bounds` and of other plans than `plan`.
    fn retain(ctx: &Context, id: Id, held: &mut Held, bounds: PixelRect, plan: Option<DetailPlan>) {
        let kept = |level: &Level| match level {
            Level::Base(kept) => *kept == bounds,
            Level::Detail(kept) => Some(*kept) == plan,
        };
        for sharp in [true, false] {
            Self::store(ctx, id, sharp).retain(ctx, kept);
        }
        held.0.retain(|level, _| kept(level));
    }
}

impl Component for RasterDisplay<'_> {
    const ROLE: Role = Role::IMAGE;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const OVERLAYS: bool = false;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = RasterStyle;
    type Output = RasterDisplayOutput;

    fn style(_: &StyleCx<'_>, _: RasterDisplayStyleKey) -> RasterStyle {
        RasterStyle::default()
    }

    /// A display without a label only paints: it has no node, so a further image over the document adds none.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if cx.resolved.label.is_empty() {
            access.role(Role::None);
        }
    }

    /// Reads what changed from a source and uploads it.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.read = PixelRect::default();
        self.shown = None;
        let Some(space) = canvas_space(&cx.env) else { return };
        let (ctx, id) = (cx.ctx.clone(), cx.id);
        let size = self.picture.size();
        let document = PixelRect::of_size(size);
        let whole = Rect64::new(0.0, 0.0, f64::from(size.width), f64::from(size.height));
        let (bounds, tiles) = match (self.mapping, self.extent) {
            (Some(mapping), Extent::Tiled) => {
                (document, mapping.area().map_or_else(Tiles::single, |area| Tiles::covering(space.view.visible(), area)))
            }
            (Some(_), _) | (None, Extent::Document) => (document, Tiles::single()),
            (None, Extent::Region(region)) => (region.union(&document), Tiles::single()),
            (None, Extent::Tiled) => (document, Tiles::covering(space.view.visible(), whole)),
        };
        let mut held = std::mem::take(cx.keep::<Held>());
        let source = match &self.picture {
            Picture::Source(source) if !document.is_empty() => *source,
            // An empty document, or a picture the app paints: the display holds no textures.
            _ => {
                Self::release(&ctx, id);
                if !document.is_empty() {
                    self.shown = Some(Shown { view: space.view, size, bounds, tiles, textures: None });
                }
                return;
            }
        };
        let side = texture_side(&ctx);
        let factor = texture_factor(bounds.size(), side);
        let (base, base_read) = Self::bring_up_to_date(source, &ctx, id, &mut held, Level::Base(bounds), bounds, factor);
        let plan = DetailPlan::in_view(&space.view, bounds, side, space.pixels_per_point).filter(|_| self.mapping.is_none());
        let detail = plan.map(|plan| {
            let (texture, read) = Self::bring_up_to_date(source, &ctx, id, &mut held, Level::Detail(plan), plan.region, plan.factor);
            self.read = self.read.union(&read);
            (texture, plan)
        });
        Self::retain(&ctx, id, &mut held, bounds, plan);
        *cx.keep::<Held>() = held;
        self.read = self.read.union(&base_read);
        self.shown = Some(Shown { view: space.view, size, bounds, tiles, textures: Some(Textures { base: (base, factor), detail }) });
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(shown) = &self.shown else { return };
        let painter = ui.content_painter(cx);
        // The picture's area on screen: the document, or the mapped area. A mapped area with nothing of the unit
        // square in it has no checkerboard and no edge.
        let corners = match &self.mapping {
            Some(mapping) => mapping.corners(&shown.view),
            None => Some(shown.view.quad(Rect64::new(0.0, 0.0, f64::from(shown.size.width), f64::from(shown.size.height)))),
        };
        if let Some(checker) = &self.checkerboard {
            let bounds = shown.bounds;
            let bounds = Rect64::new(f64::from(bounds.x0), f64::from(bounds.y0), f64::from(bounds.x1), f64::from(bounds.y1));
            // A mapped picture's copies are laid out from its area's bounds.
            let tiled = self.mapping.and_then(|mapping| mapping.area()).filter(|_| !shown.tiles.is_single());
            let areas = match (self.mapping, tiled) {
                (Some(_), None) => [corners, None],
                (_, area) => {
                    let mut areas = checker_areas(area.unwrap_or(bounds), shown.tiles.run()).map(|area| shown.view.quad(area));
                    [areas.next(), areas.next()]
                }
            };
            for area in areas.into_iter().flatten() {
                painter.painter.add(checker.shape(painter.painter.ctx(), &ui.env.tokens, CheckerArea::Quad(area), shown.view.rect.min));
            }
        }
        match (&mut self.picture, &shown.textures, &self.mapping) {
            (Picture::Source(_), Some(textures), Some(mapping)) => {
                let (texture, factor) = textures.base;
                for offset in shown.tiles.offsets() {
                    painter.painter.add(mapped_mesh(&shifted(shown.view, offset), texture, factor, shown.size, mapping));
                }
            }
            (picture, textures, _) => {
                for offset in shown.tiles.offsets() {
                    let view = shifted(shown.view, offset);
                    match (&mut *picture, textures) {
                        (Picture::Painted { paint, .. }, _) => paint.paint(&painter, view),
                        (Picture::Source(_), Some(textures)) => {
                            let (texture, factor) = textures.base;
                            painter.painter.add(textured_quad(&view, texture, factor, shown.bounds));
                            if let Some((texture, plan)) = textures.detail {
                                painter.painter.add(textured_quad(&view, texture, plan.factor, plan.region));
                            }
                        }
                        (Picture::Source(_), None) => {}
                    }
                }
            }
        }
        if self.outlined
            && let Some(corners) = corners
        {
            painter.with_draw(|draw| edge(draw, corners));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> RasterDisplayOutput {
        let detail = self.shown.as_ref().and_then(|shown| shown.textures.as_ref()?.detail.as_ref().map(|(_, plan)| *plan));
        RasterDisplayOutput { response: cx.response.clone(), read: (!self.read.is_empty()).then_some(self.read), detail }
    }
}

/// The divider rule just outside `corners`, a convex quad on screen: each side moved out by half the rule's width and
/// run on by half a width at each end, so the corners close.
fn edge(draw: &Draw<'_>, corners: [Pos2; 4]) {
    if !corners.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
        return;
    }
    let half = draw.rule_width(RuleKind::Divider) / 2.0;
    let centre = (corners.iter().fold(Vec2::ZERO, |sum, p| sum + p.to_vec2()) / 4.0).to_pos2();
    for (at, &from) in corners.iter().enumerate() {
        let to = corners[(at + 1) % corners.len()];
        let along = (to - from).normalized();
        if along == Vec2::ZERO {
            continue;
        }
        let normal = along.rot90();
        let out = if normal.dot(from.lerp(to, 0.5) - centre) < 0.0 { -normal } else { normal };
        draw.rule(RuleKind::Divider, from + (out - along) * half, to + (out + along) * half);
    }
}

/// `texture`, which holds `region` of the document from its top left at one texel per `factor` pixels, as a quad on
/// the view's mapped corners of `region`.
fn textured_quad(view: &CanvasView, texture: CachedTexture, factor: u32, region: PixelRect) -> Shape {
    let (texels_x, texels_y) = (texture.size.width as f32 * factor as f32, texture.size.height as f32 * factor as f32);
    let (u, v) = (region.width() as f32 / texels_x, region.height() as f32 / texels_y);
    let corners = view.quad(Rect64::new(f64::from(region.x0), f64::from(region.y0), f64::from(region.x1), f64::from(region.y1)));
    let uv = [pos2(0.0, 0.0), pos2(u, 0.0), pos2(u, v), pos2(0.0, v)];
    let mut mesh = Mesh::with_texture(texture.id);
    mesh.vertices.extend(corners.iter().zip(uv).map(|(pos, uv)| Vertex { pos: *pos, uv, color: UNTINTED }));
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    Shape::mesh(mesh)
}

/// `texture`, which holds the whole document at one texel per `factor` pixels, as a mesh of `mapping.cells` squared
/// cells through the mapping's frame. A cell whose centre lies outside the frame's unit square is left out.
fn mapped_mesh(view: &CanvasView, texture: CachedTexture, factor: u32, size: PixelSize, mapping: &Mapping) -> Shape {
    let n = mapping.cells;
    let (texels_x, texels_y) = (texture.size.width as f32 * factor as f32, texture.size.height as f32 * factor as f32);
    let (u, v) = (size.width as f32 / texels_x, size.height as f32 / texels_y);
    let placed = mapping.placed;
    let at = |s: f64, t: f64| {
        let p = mapping.frame.apply(Point64::new(placed.x0 + s * placed.width(), placed.y0 + t * placed.height()));
        view.to_screen(p)
    };
    let mut mesh = Mesh::with_texture(texture.id);
    for j in 0..=n {
        for i in 0..=n {
            let (s, t) = (f64::from(i) / f64::from(n), f64::from(j) / f64::from(n));
            let uv = pos2(s as f32 * u, t as f32 * v);
            mesh.vertices.push(Vertex { pos: at(s, t), uv, color: UNTINTED });
        }
    }
    let row = n + 1;
    for j in 0..n {
        for i in 0..n {
            let x = placed.x0 + (f64::from(i) + 0.5) / f64::from(n) * placed.width();
            let y = placed.y0 + (f64::from(j) + 0.5) / f64::from(n) * placed.height();
            if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
                continue;
            }
            let a = j * row + i;
            mesh.add_triangle(a, a + 1, a + row + 1);
            mesh.add_triangle(a, a + row + 1, a + row);
        }
    }
    Shape::mesh(mesh)
}
