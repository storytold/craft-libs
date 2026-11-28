//! The tip's falloff, shown as the resize preview.

use crate::canvas::canvas_space;
use crate::contract::{Access, Common, Component, Cx, PaintUi, Role};
use crate::primitives::PaintShape;
use crate::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use craft_behavior::kurbo::Point;
use craft_behavior::{Input, Metrics};
use egui::{Color32, Mesh, Pos2, Rect, Response, Shape, vec2};

/// The share of the bad tone's solid colour the preview is tinted with when the app gives no colour.
const TINT_SHARE: f32 = 0.43;
/// The ring segments per point of the falloff's screen radius, and the least and most segments.
const SEGMENTS_PER_POINT: f32 = 0.5;
const LEAST_SEGMENTS: usize = 24;
const MOST_SEGMENTS: usize = 160;

/// How a dab of the tip covers from its centre out: solid to `core`, then falling to nothing at `reach` (document
/// units), sampled at [`RINGS`](Self::RINGS) + 1 rings. The engine supplies the numbers
/// ([`sampled`](Self::sampled)); an app without an engine uses [`from_hardness`](Self::from_hardness).
#[derive(Clone, Debug, PartialEq)]
pub struct TipFalloff {
    core: f64,
    reach: f64,
    /// The coverage at ring `k`, at `core + (reach - core) * k / RINGS`; the last is always 0.
    coverage: [f32; TipFalloff::RINGS + 1],
}

impl TipFalloff {
    /// Rings across the falloff: the shoulder is smooth, so straight steps between them are close.
    pub const RINGS: usize = 16;

    /// The falloff the engine computes: solid to `core`, `coverage(d)` (0 to 1) at each ring's distance `d` out to
    /// `reach`, and 0 at `reach` whatever `coverage` says there. A `reach` below `core` counts as `core`; non-finite
    /// or negative values give an empty falloff (nothing drawn).
    pub fn sampled(core: f64, reach: f64, coverage: impl Fn(f64) -> f32) -> TipFalloff {
        if !(core.is_finite() && reach.is_finite() && core >= 0.0 && reach >= 0.0) {
            return TipFalloff { core: 0.0, reach: 0.0, coverage: [0.0; Self::RINGS + 1] };
        }
        let reach = reach.max(core);
        let mut rings = [0.0f32; Self::RINGS + 1];
        for (k, ring) in rings.iter_mut().enumerate().take(Self::RINGS) {
            let c = if k == 0 { 1.0 } else { coverage(core + (reach - core) * k as f64 / Self::RINGS as f64) };
            *ring = if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 };
        }
        TipFalloff { core, reach, coverage: rings }
    }

    /// A hard tip of `radius`: solid to the edge.
    pub fn hard(radius: f64) -> TipFalloff {
        TipFalloff::sampled(radius, radius, |_| 1.0)
    }

    /// The library's own falloff for a round tip of `radius` and `hardness` (0 to 1), for apps without a brush
    /// engine and for stories: solid to `radius * hardness`, then a smoothstep from 1 down to 0 at `radius`.
    pub fn from_hardness(radius: f64, hardness: f64) -> TipFalloff {
        if !(radius.is_finite() && radius > 0.0) {
            return TipFalloff::sampled(f64::NAN, f64::NAN, |_| 0.0);
        }
        let hardness = if hardness.is_finite() { hardness.clamp(0.0, 1.0) } else { 1.0 };
        let core = radius * hardness;
        TipFalloff::sampled(core, radius, |d| {
            let t = ((d - core) / (radius - core)).clamp(0.0, 1.0);
            (1.0 - t * t * (3.0 - 2.0 * t)) as f32
        })
    }

    /// The core radius (document units).
    pub fn core(&self) -> f64 {
        self.core
    }

    /// The outer radius where the coverage reaches 0 (document units).
    pub fn reach(&self) -> f64 {
        self.reach
    }

    /// The falloff as one mesh centred at `centre` (screen) at `zoom` screen points per document unit in `tint`: a
    /// centre vertex and the core ring in `tint`, each ring out to the reach in `tint` scaled by its coverage, the
    /// reach transparent. `None` when the disc lies outside `clip`, or for a non-finite or non-positive size or zoom.
    fn mesh(&self, centre: Pos2, zoom: f32, tint: Color32, clip: Rect) -> Option<Mesh> {
        let finite = centre.x.is_finite() && centre.y.is_finite() && zoom.is_finite() && zoom > 0.0;
        if !finite || self.reach <= 0.0 {
            return None;
        }
        let outer = self.reach as f32 * zoom;
        if !outer.is_finite() || !clip.expand(outer).contains(centre) {
            return None;
        }
        let n = ((outer * SEGMENTS_PER_POINT) as usize).clamp(LEAST_SEGMENTS, MOST_SEGMENTS);
        let mut mesh = Mesh::default();
        mesh.colored_vertex(centre, tint);
        for (k, &coverage) in self.coverage.iter().enumerate() {
            let d = (self.core + (self.reach - self.core) * k as f64 / Self::RINGS as f64) as f32 * zoom;
            let colour = if k == Self::RINGS { Color32::TRANSPARENT } else { tint.gamma_multiply(coverage) };
            for j in 0..n {
                let a = j as f32 / n as f32 * std::f32::consts::TAU;
                mesh.colored_vertex(centre + vec2(a.cos(), a.sin()) * d, colour);
            }
        }
        let n = n as u32;
        let at = |ring: u32, j: u32| 1 + ring * n + j % n;
        for j in 0..n {
            mesh.add_triangle(0, at(0, j), at(0, j + 1));
            for ring in 0..Self::RINGS as u32 {
                mesh.add_triangle(at(ring, j), at(ring + 1, j), at(ring + 1, j + 1));
                mesh.add_triangle(at(ring, j), at(ring + 1, j + 1), at(ring, j + 1));
            }
        }
        Some(mesh)
    }
}

/// The style of the falloff preview.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FalloffPreviewStyle {
    #[style(metrics)]
    metrics: Metrics,
    tint: StyleColor,
}

/// A tip's falloff centred at a document point, with the circle of the brush's size round it: the preview a brush
/// resize and a hover show. Shown at the canvas rect, in a canvas given a view.
#[derive(Clone, crate::contract::Component, crate::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct FalloffPreview<'a> {
    #[common]
    common: Common,
    falloff: &'a TipFalloff,
    centre: Point,
    ring: f64,
    /// The tint of the falloff: the user's Brush Preview colour. Without one the bad tone's solid colour at a share.
    #[prop]
    tint: Option<DataColor>,
}

impl<'a> FalloffPreview<'a> {
    /// The preview of `falloff` centred at `centre` (document units), with a circle of `ring` radius in document
    /// units; no circle for a radius of 0.
    pub fn new(falloff: &'a TipFalloff, centre: Point, ring: f64) -> Self {
        FalloffPreview { common: Common::new(), falloff, centre, ring, tint: None }
    }
}

impl Component for FalloffPreview<'_> {
    const ROLE: Role = Role::GROUP;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = FalloffPreviewStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: FalloffPreviewStyleKey) -> FalloffPreviewStyle {
        let solid = cx.tokens.color.status.bad.solid;
        FalloffPreviewStyle { metrics: Metrics::new(), tint: cx.color(solid.faded(TINT_SHARE)) }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let painter = ui.content_painter(cx);
        let centre = space.view.to_screen(self.centre);
        let zoom = space.view.zoom as f32;
        let tint = self.tint.map_or_else(|| cx.style.tint.egui(), |tint| tint.0);
        if let Some(mesh) = self.falloff.mesh(centre, zoom, tint, painter.painter.clip_rect()) {
            painter.painter.add(Shape::mesh(mesh));
        }
        let radius = self.ring as f32 * zoom;
        if radius.is_finite() && radius > 0.0 {
            painter.outline(&PaintShape::Circle(centre, radius));
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
