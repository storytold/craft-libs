//! `StrokeTrail`: the footprint of the stroke being painted, over a coverage raster with partial uploads.

use crate::canvas::canvas_space;
use crate::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role};
use crate::primitives::{Revision, Serials, TextureState, TextureStore, TextureStoreOptions};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleColor, StyleCx};
use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{Input, Metrics};
use egui::epaint::{Mesh, Vertex};
use egui::{Color32, ColorImage, Response, Shape, pos2};

/// The share of the tone's colour a full-coverage texel shows; the texture holds coverage only.
const LIGHT_SHARE: f32 = 0.25;
const SHADE_SHARE: f32 = 0.5;
const DANGER_SHARE: f32 = 0.45;

/// The texture store's salt: one store for every trail of the context, keyed by the trail's component id and the
/// mask's serial (serials are unique per owner, not per context).
const STORE_SALT: &str = "canvas_controls.stroke_trail";

/// A coverage raster over a document rect: one byte per texel, `scale` document units wide, stamped by the tool as the
/// stroke grows and uploaded by [`StrokeTrail`] by its changed rows.
#[derive(Debug)]
pub struct CoverageMask {
    /// The document rect the mask covers, normalised and finite.
    bounds: Rect,
    scale: f64,
    size: (u32, u32),
    texels: Vec<u8>,
    /// The coverage when the stroke began; `restamp` never writes less.
    base: Vec<u8>,
    /// The texels written since the last upload, as `[x0, y0, x1, y1]`.
    dirty: Option<[u32; 4]>,
    /// Bumped by every write; the texture's revision.
    revision: u64,
    /// The texture's key.
    id: u64,
}

/// The texel indices that `lo..hi` (document units, along one axis from `origin`) touch, clamped to `0..extent`.
fn texel_span(lo: f64, hi: f64, origin: f64, scale: f64, extent: u32) -> Option<(u32, u32)> {
    if lo.is_nan() || hi.is_nan() {
        return None;
    }
    let top = f64::from(extent);
    let first = ((lo - origin) / scale).floor().clamp(0.0, top);
    let last = ((hi - origin) / scale).ceil().clamp(0.0, top);
    (first < last).then_some((first as u32, last as u32))
}

/// A coverage in 0..=1 as a byte; a NaN is none.
fn byte(coverage: f32) -> u8 {
    (coverage.clamp(0.0, 1.0) * 255.0).round() as u8
}

impl CoverageMask {
    /// A mask over `bounds` with no side longer than `max_side` texels. Its texture's key comes from `serials`, which
    /// the masks of one owner share.
    pub fn new(serials: &mut Serials, bounds: Rect, max_side: u32) -> Self {
        let bounds = bounds.abs();
        let finite = [bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().all(|v| v.is_finite());
        let bounds = if finite { bounds } else { Rect::ZERO };
        let long = bounds.width().max(bounds.height());
        let scale = (long / f64::from(max_side.max(1))).ceil().max(1.0);
        let extent = |side: f64| (side / scale).ceil().max(1.0) as u32;
        let size = (extent(bounds.width()), extent(bounds.height()));
        let len = size.0 as usize * size.1 as usize;
        CoverageMask { bounds, scale, size, texels: vec![0; len], base: vec![0; len], dirty: None, revision: 0, id: serials.mint() }
    }

    /// Stamps the capsule from `from` to `to` (document units) of radius `radius`, keeping the larger coverage.
    pub fn stamp_capsule(&mut self, from: Point, to: Point, radius: f64) {
        let finite = [from.x, from.y, to.x, to.y, radius].iter().all(|v| v.is_finite());
        if !finite || radius < 0.0 {
            return;
        }
        // Texel units from here on: texel (i, j) has its centre at (i + 0.5, j + 0.5).
        let to_texels = |p: Point| ((p.x - self.bounds.x0) / self.scale, (p.y - self.bounds.y0) / self.scale);
        let (a, b) = (to_texels(from), to_texels(to));
        let r = radius / self.scale;
        let reach = r + 1.0;
        let Some((x0, x1)) = texel_span(a.0.min(b.0) - reach, a.0.max(b.0) + reach, 0.0, 1.0, self.size.0) else { return };
        let Some((y0, y1)) = texel_span(a.1.min(b.1) - reach, a.1.max(b.1) + reach, 0.0, 1.0, self.size.1) else { return };
        let along = (b.0 - a.0, b.1 - a.1);
        let length_squared = along.0 * along.0 + along.1 * along.1;
        for y in y0..y1 {
            for x in x0..x1 {
                let (px, py) = (f64::from(x) + 0.5 - a.0, f64::from(y) + 0.5 - a.1);
                let t = if length_squared > 0.0 { ((px * along.0 + py * along.1) / length_squared).clamp(0.0, 1.0) } else { 0.0 };
                let distance = (px - along.0 * t).hypot(py - along.1 * t);
                // One texel of smooth edge, centred on the radius.
                let value = byte((r + 0.5 - distance) as f32);
                let texel = &mut self.texels[y as usize * self.size.0 as usize + x as usize];
                *texel = (*texel).max(value);
            }
        }
        self.touch([x0, y0, x1, y1]);
    }

    /// Writes `max(coverage(point), base)` into the texels of `rect`; `coverage` is asked at document points.
    pub fn restamp(&mut self, rect: Rect, coverage: impl Fn(Point) -> f32) {
        let rect = rect.abs();
        let Some((x0, x1)) = texel_span(rect.x0, rect.x1, self.bounds.x0, self.scale, self.size.0) else { return };
        let Some((y0, y1)) = texel_span(rect.y0, rect.y1, self.bounds.y0, self.scale, self.size.1) else { return };
        for y in y0..y1 {
            for x in x0..x1 {
                let (left, top) = (self.bounds.x0 + f64::from(x) * self.scale, self.bounds.y0 + f64::from(y) * self.scale);
                let mut sample = coverage(Point::new(left + self.scale / 2.0, top + self.scale / 2.0)).max(0.0);
                // A texel wider than a document unit shows what any of its corners covers, so a thin stroke stays visible.
                if self.scale > 1.0 {
                    for corner in [(left, top), (left + self.scale, top), (left, top + self.scale), (left + self.scale, top + self.scale)] {
                        sample = sample.max(coverage(Point::new(corner.0, corner.1)));
                    }
                }
                let at = y as usize * self.size.0 as usize + x as usize;
                self.texels[at] = byte(sample).max(self.base[at]);
            }
        }
        self.touch([x0, y0, x1, y1]);
    }

    /// Starts a stroke: the coverage so far becomes the base `restamp` keeps.
    pub fn begin_stroke(&mut self) {
        self.base.copy_from_slice(&self.texels);
    }

    /// The coverage at the document point `at`, 0 outside the mask.
    pub fn coverage_at(&self, at: Point) -> u8 {
        let index = |value: f64, origin: f64, extent: u32| {
            let texel = ((value - origin) / self.scale).floor();
            (texel >= 0.0 && texel < f64::from(extent)).then_some(texel as usize)
        };
        match (index(at.x, self.bounds.x0, self.size.0), index(at.y, self.bounds.y0, self.size.1)) {
            (Some(x), Some(y)) => self.texels[y * self.size.0 as usize + x],
            _ => 0,
        }
    }

    /// Document units per texel.
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// The mask's size in texels, width then height.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Records a write to the texels `area` (`[x0, y0, x1, y1]`, end exclusive).
    fn touch(&mut self, area: [u32; 4]) {
        self.revision += 1;
        self.dirty = Some(match self.dirty {
            Some(held) => [held[0].min(area[0]), held[1].min(area[1]), held[2].max(area[2]), held[3].max(area[3])],
            None => area,
        });
    }

    /// The texels of `area` as a coverage image: white with alpha as coverage, so the quad's vertex colour tints it.
    fn image(&self, area: [u32; 4]) -> ColorImage {
        let width = self.size.0 as usize;
        let [x0, y0, x1, y1] = area.map(|v| v as usize);
        let mut pixels = Vec::with_capacity((x1 - x0) * (y1 - y0));
        for y in y0..y1 {
            pixels.extend(self.texels[y * width + x0..y * width + x1].iter().map(|&coverage| Color32::from_white_alpha(coverage)));
        }
        ColorImage::new([x1 - x0, y1 - y0], pixels)
    }

    /// The whole mask as an image.
    fn full_image(&self) -> ColorImage {
        self.image([0, 0, self.size.0, self.size.1])
    }

    /// The written rows' origin and image, clearing the dirty rect.
    fn take_dirty(&mut self) -> Option<([u32; 2], ColorImage)> {
        let area = self.dirty.take()?;
        Some(([area[0], area[1]], self.image(area)))
    }
}

/// What the trail shows its footprint as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrailTone {
    /// The marquee's colour.
    #[default]
    Light,
    /// The halo's colour.
    Shade,
    /// The bad status colour.
    Danger,
}

/// The style of the stroke trail: the colour of each tone. The texture holds coverage only; the quad's vertex colour
/// carries the tone.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct StrokeTrailStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The colour of a [`TrailTone::Light`] footprint.
    light: StyleColor,
    /// The colour of a [`TrailTone::Shade`] footprint.
    shade: StyleColor,
    /// The colour of a [`TrailTone::Danger`] footprint.
    danger: StyleColor,
}

/// The footprint of the stroke being painted, drawn from a [`CoverageMask`] as one textured quad over the mask's
/// document rect. Display only.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct StrokeTrail<'a> {
    #[common]
    common: Common,
    mask: &'a mut CoverageMask,
    /// What the footprint shows as.
    #[prop]
    tone: TrailTone,
}

impl<'a> StrokeTrail<'a> {
    /// A trail named `label` over `mask`.
    pub fn new(label: impl Into<Label>, mask: &'a mut CoverageMask) -> Self {
        StrokeTrail { common: Common::new(), mask, tone: TrailTone::Light }.label(label)
    }
}

impl Component for StrokeTrail<'_> {
    const ROLE: Role = Role::GROUP;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = StrokeTrailStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: StrokeTrailStyleKey) -> StrokeTrailStyle {
        let color = &cx.tokens.color;
        StrokeTrailStyle {
            metrics: Metrics::new(),
            light: cx.color(color.overlay.marquee),
            shade: cx.color(color.overlay.halo),
            danger: cx.color(color.status.bad.solid),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let bounds = self.mask.bounds;
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return;
        }
        let painter = ui.content_painter(cx);
        let ctx = painter.painter.ctx();
        let store = TextureStore::<(egui::Id, u64)>::in_context(ctx, STORE_SALT, TextureStoreOptions::frames());
        let (key, revision) = ((cx.id, self.mask.id), Revision::new(self.mask.revision));
        // A second paint of the pass finds the entry at this revision and nothing dirty.
        let texture = match store.state(ctx, &key, revision) {
            TextureState::Ready(texture) => texture,
            state => {
                let partial = state
                    .shown()
                    .and_then(|_| self.mask.take_dirty())
                    .and_then(|(origin, image)| store.put_region(ctx, &key, revision, origin, image));
                match partial {
                    Some(texture) => texture,
                    None => {
                        self.mask.dirty = None;
                        store.put(ctx, key, revision, self.mask.full_image())
                    }
                }
            }
        };
        let (color, share) = match self.tone {
            TrailTone::Shade => (cx.style.shade, SHADE_SHARE),
            TrailTone::Danger => (cx.style.danger, DANGER_SHARE),
            _ => (cx.style.light, LIGHT_SHARE),
        };
        let tint = color.egui().gamma_multiply(share);
        // The mask's texels may reach past the bounds by less than one texel: the quad shows the used part.
        let (u, v) = (
            (bounds.width() / (f64::from(self.mask.size.0) * self.mask.scale)) as f32,
            (bounds.height() / (f64::from(self.mask.size.1) * self.mask.scale)) as f32,
        );
        let corners = space.view.quad(bounds);
        if !corners.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
            return;
        }
        let uv = [pos2(0.0, 0.0), pos2(u, 0.0), pos2(u, v), pos2(0.0, v)];
        let mut mesh = Mesh::with_texture(texture.id);
        mesh.vertices.extend(corners.iter().zip(uv).map(|(pos, uv)| Vertex { pos: *pos, uv, color: tint }));
        mesh.indices.extend([0, 1, 2, 0, 2, 3]);
        painter.painter.add(Shape::mesh(mesh));
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
