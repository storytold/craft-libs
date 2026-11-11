//! [`MeshLines`] and [`paint_mesh_lines`]: the one painter of a deformed mesh's lines.

use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, Metrics, Sense};
use craft_ui::canvas::canvas_space;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Role};
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth};
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{Response, Vec2};

/// The look of a deformed mesh's lines: `overlay.line` at `stroke.thin`.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct MeshLinesStyle {
    #[style(metrics)]
    metrics: Metrics,
    line: StyleColor,
}

/// The lines of a deformed mesh over a canvas, each edge once: Puppet Warp's pin mesh and Liquify's
/// mesh. A paint-only canvas control: it has no parts and no accessibility node. The edges are in
/// document units.
#[derive(craft_ui::contract::Component)]
#[component(canvas_control)]
pub(crate) struct MeshLines<'a> {
    #[common]
    common: Common,
    edges: &'a [[Point; 2]],
}

impl<'a> MeshLines<'a> {
    /// The lines of `edges`, document points in pairs.
    pub(crate) fn new(edges: &'a [[Point; 2]]) -> Self {
        MeshLines { common: Common::new(), edges }
    }
}

/// Draws `edges` (document points) under `view`, each once, in `color` at `stroke.thin`.
pub(crate) fn paint_mesh_lines(draw: &Draw<'_>, view: &CanvasView, edges: &[[Point; 2]], color: StyleColor) {
    for [a, b] in edges {
        draw.stroke(&PaintShape::Path(vec![view.to_screen(*a), view.to_screen(*b)]), StrokeWidth::Thin, color);
    }
}

impl Component for MeshLines<'_> {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = MeshLinesStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: MeshLinesStyleKey) -> MeshLinesStyle {
        MeshLinesStyle { metrics: Metrics::new(), line: cx.color(cx.tokens.color.overlay.line) }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        paint_mesh_lines(&ui.draw(), &space.view, self.edges, cx.style.line);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
