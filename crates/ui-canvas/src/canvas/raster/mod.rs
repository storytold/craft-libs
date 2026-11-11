//! `RasterDisplay`: the CPU raster path of a canvas. The app gives the document's composite as RGBA for a region
//! ([`RasterSource`]); the display keeps it in `TextureStore` textures, draws it through the canvas's
//! [`CanvasSpace`](super::CanvasSpace) over a checkerboard, and re-reads only what the source reports damaged.
//!
//! A document larger than the device's texture limit (`texture_side`) is uploaded reduced; zoomed in, the part in
//! view is added at the screen's resolution as a detail texture ([`DetailPlan`]), so a large document shows its own
//! pixels at 100 percent and the cost follows the window, not the document. Pixels are drawn sharp (nearest) from
//! 100 percent up and filtered below.
//!
//! The display takes no pointer and no keys and declares no parts: a press on it reaches the canvas's tool.

mod detail;
#[cfg(feature = "testing")]
mod stories;

pub use detail::{DetailPlan, snap_to_texels};

use super::{PixelRect, RasterSource, canvas_space};
use crate::contract::{Access, Common, Component, Cx, NoValue, Outcome, PaintUi, Props as _, Role};
use crate::primitives::textures::UNTINTED;
use crate::primitives::{CachedTexture, Checker, Revision, TextureState, TextureStore, TextureStoreOptions, texture_factor, texture_side};
use crate::runtime::env::Label;
use crate::runtime::memory::Remember as _;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::geometry::quad::Homography;
use craft_behavior::kurbo::{Point as Point64, Rect as Rect64};
use craft_behavior::{CanvasView, Metrics, PixelSize};
use detail::{texel_extent, texel_origin};
use egui::epaint::{Mesh, Vertex};
use egui::{Color32, ColorImage, Context, Id, Pos2, Rect, Response, Shape, Stroke, TextureFilter, TextureOptions, pos2};
use std::collections::HashMap;

/// The style of a raster display: it has no look of its own.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct RasterStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// Which picture of a display a texture holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Level {
    /// The whole document at one texel per `texture_factor` pixels.
    Base,
    /// The part in view at the screen's resolution. A new plan is a new key, so a texture is never partly of
    /// two plans.
    Detail(DetailPlan),
}

/// What `update` leaves for `paint` and `finish`: the textures to draw and the view to draw them through.
struct Shown {
    view: CanvasView,
    size: PixelSize,
    base: (CachedTexture, u32),
    detail: Option<(CachedTexture, DetailPlan)>,
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

/// The CPU raster display of one canvas: the document drawn through the view of the [`Canvas`](super::Canvas) it
/// is shown in, at the canvas rect. Outside a canvas, or in one without a view, it draws nothing.
///
/// - **Reading.** The display asks the source for pixels only when it has none for the source's `revision`. A
///   changed revision re-reads what [`RasterSource::damage`] reports since the revision the display shows (snapped
///   outward to whole texels), or everything where the source cannot tell.
/// - **Transparency.** [`checkerboard`](Self::checkerboard) draws a transparency grid under the document, holding
///   still while the document scrolls; the app's [`Checker`] sets its cells, and an empty one is the theme's.
///   Without it the document draws straight on the canvas (an opaque document, a video frame).
/// - **Turned views.** A rotated or mirrored view draws each texture as one quad on the view's mapped corners.
/// - **Memory.** The textures live in two `TextureStore`s per display id and go with the store's budget; a
///   detail texture of a plan no longer in use is dropped in the pass the plan changes.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct RasterDisplay<'a> {
    #[common]
    common: Common,
    source: &'a dyn RasterSource,
    /// The transparency grid under the document; none draws the document straight on the canvas.
    #[prop]
    checkerboard: Option<Checker>,
    read: PixelRect,
    shown: Option<Shown>,
    mapping: Option<Mapping>,
}

/// A display drawn through a homography on a mesh of cells instead of as a flat quad.
#[derive(Clone, Copy, Debug)]
struct Mapping {
    frame: Homography,
    cells: u32,
    /// Where the image lies in the frame's unit square.
    placed: Rect64,
}

impl<'a> RasterDisplay<'a> {
    /// A display named `label` of the document `source` gives. An empty label makes a display that only paints, with
    /// no AccessKit node: a further image over a named one.
    pub fn new(label: impl Into<Label>, source: &'a dyn RasterSource) -> Self {
        RasterDisplay { common: Common::new(), source, checkerboard: None, read: PixelRect::default(), shown: None, mapping: None }.label(label)
    }

    /// Draws the image through `frame` (the unit square onto the document) on a `cells` by `cells` mesh, so a
    /// perspective frame shows the image foreshortened. The image fills the unit square unless
    /// [`placed`](Self::placed) says otherwise; a cell whose centre lies outside the unit square is left out. A
    /// mapped display draws its whole-document texture alone: the detail texture of a flat display has no place on
    /// a mesh.
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

    fn read_region(&self, region: PixelRect, factor: u32) -> ColorImage {
        let mut image = ColorImage::filled(texel_extent(region, factor), Color32::TRANSPARENT);
        self.source.read(region, factor, &mut image);
        image
    }

    /// The texture of `level`, which holds `region` of the document at one texel per `factor` pixels, brought up to
    /// the source's revision, and the pixels read to do it.
    /// - A texture with the revision is kept.
    /// - A texture of an older revision is updated with what the source reports damaged since then: that part is
    ///   read again alone, and damage elsewhere moves the texture to the revision without a read.
    /// - Anything else (no texture, another size, damage the source cannot tell) is read whole.
    fn bring_up_to_date(&self, ctx: &Context, id: Id, held: &mut Held, level: Level, region: PixelRect, factor: u32) -> (CachedTexture, PixelRect) {
        let sharp = factor == 1;
        // A key never lives in both samplings: the other store's copy is from before the factor changed.
        Self::store(ctx, id, !sharp).forget(ctx, &level);
        let store = Self::store(ctx, id, sharp);
        let revision = self.source.revision();
        let origin = [region.x0, region.y0];
        let expected = texel_extent(region, factor);
        let state = store.state(ctx, &level, revision);
        let current = state.shown().filter(|texture| [texture.size.width as usize, texture.size.height as usize] == expected);
        if let Some(texture) = current {
            if matches!(state, TextureState::Ready(_)) {
                held.0.insert(level, revision);
                return (texture, PixelRect::default());
            }
            if let Some(damage) = held.0.get(&level).and_then(|since| self.source.damage(*since)) {
                let dirty = snap_to_texels(damage.intersect(&region), origin, factor).intersect(&region);
                let updated = if dirty.is_empty() {
                    // The pixels changed elsewhere: no texel of this texture is new, and the entry takes the revision.
                    store.put_region(ctx, &level, revision, [0, 0], ColorImage::new([0, 0], Vec::new())).map(|texture| (texture, dirty))
                } else {
                    let image = self.read_region(dirty, factor);
                    store.put_region(ctx, &level, revision, texel_origin(dirty, origin, factor), image).map(|texture| (texture, dirty))
                };
                if let Some(updated) = updated {
                    held.0.insert(level, revision);
                    return updated;
                }
            }
        }
        held.0.insert(level, revision);
        (store.put(ctx, level, revision, self.read_region(region, factor)), region)
    }

    /// Drops every texture of this display.
    fn release(ctx: &Context, id: Id) {
        for sharp in [true, false] {
            Self::store(ctx, id, sharp).clear(ctx);
        }
    }

    /// Drops the detail textures of other plans than `plan`.
    fn retain_plan(ctx: &Context, id: Id, held: &mut Held, plan: Option<DetailPlan>) {
        let kept = |level: &Level| match level {
            Level::Base => true,
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

    /// Reads what changed from the source and uploads it.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.read = PixelRect::default();
        self.shown = None;
        let Some(space) = canvas_space(&cx.env) else { return };
        let (ctx, id) = (cx.ctx.clone(), cx.id);
        let size = self.source.size();
        let bounds = PixelRect::of_size(size);
        let mut held = std::mem::take(cx.keep::<Held>());
        if bounds.is_empty() {
            Self::release(&ctx, id);
            return;
        }
        let side = texture_side(&ctx);
        let factor = texture_factor(size, side);
        let (base, base_read) = self.bring_up_to_date(&ctx, id, &mut held, Level::Base, bounds, factor);
        let plan = DetailPlan::in_view(&space.view, size, side, space.pixels_per_point).filter(|_| self.mapping.is_none());
        let detail = plan.map(|plan| {
            let (texture, read) = self.bring_up_to_date(&ctx, id, &mut held, Level::Detail(plan), plan.region, plan.factor);
            self.read = self.read.union(&read);
            (texture, plan)
        });
        Self::retain_plan(&ctx, id, &mut held, plan);
        *cx.keep::<Held>() = held;
        self.read = self.read.union(&base_read);
        self.shown = Some(Shown { view: space.view, size, base: (base, factor), detail });
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(shown) = &self.shown else { return };
        let painter = ui.content_painter(cx);
        let corners = shown.view.quad(Rect64::new(0.0, 0.0, f64::from(shown.size.width), f64::from(shown.size.height)));
        if let Some(checker) = &self.checkerboard {
            let around = Rect::from_points(&corners);
            for shape in checker.shapes(&ui.env.tokens, shown.view.rect, around) {
                if shown.view.axis_aligned() {
                    painter.painter.add(shape);
                } else if let Some(cell) = clipped_cell(&shape, &corners) {
                    painter.painter.add(cell);
                }
            }
        }
        let (texture, factor) = shown.base;
        if let Some(mapping) = &self.mapping {
            painter.painter.add(mapped_mesh(&shown.view, texture, factor, shown.size, mapping));
            return;
        }
        let whole = PixelRect::of_size(shown.size);
        painter.painter.add(textured_quad(&shown.view, texture, factor, whole));
        if let Some((texture, plan)) = shown.detail {
            painter.painter.add(textured_quad(&shown.view, texture, plan.factor, plan.region));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> RasterDisplayOutput {
        let detail = self.shown.as_ref().and_then(|shown| shown.detail.as_ref().map(|(_, plan)| *plan));
        RasterDisplayOutput { response: cx.response.clone(), read: (!self.read.is_empty()).then_some(self.read), detail }
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

/// The filled rect `shape` (a transparency grid cell) cut to the convex quad `corners`, or `None` when it is not a
/// filled rect or lies outside the quad.
fn clipped_cell(shape: &Shape, corners: &[Pos2; 4]) -> Option<Shape> {
    let Shape::Rect(cell) = shape else { return None };
    let rect = cell.rect;
    let polygon = clip_to_convex(&[rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()], corners);
    (polygon.len() >= 3).then(|| Shape::convex_polygon(polygon, cell.fill, Stroke::NONE))
}

/// `subject` cut to the convex polygon `clip` (Sutherland-Hodgman), either winding.
fn clip_to_convex(subject: &[Pos2], clip: &[Pos2; 4]) -> Vec<Pos2> {
    let cross = |a: Pos2, b: Pos2, p: Pos2| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let winding = cross(clip[0], clip[1], clip[2]).signum();
    let mut polygon = subject.to_vec();
    for edge in 0..clip.len() {
        let (a, b) = (clip[edge], clip[(edge + 1) % clip.len()]);
        let inside = |p: Pos2| cross(a, b, p) * winding >= 0.0;
        let input = std::mem::take(&mut polygon);
        for (at, current) in input.iter().enumerate() {
            let previous = input[(at + input.len() - 1) % input.len()];
            if inside(*current) != inside(previous) {
                let (from, to) = (cross(a, b, previous), cross(a, b, *current));
                let t = from / (from - to);
                polygon.push(previous + (*current - previous) * t);
            }
            if inside(*current) {
                polygon.push(*current);
            }
        }
        if polygon.is_empty() {
            break;
        }
    }
    polygon
}
