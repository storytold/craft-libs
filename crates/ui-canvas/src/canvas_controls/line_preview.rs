//! `StraightLinePreview`: the line a Shift-click will paint, a canvas control (20.6).
//!
//! While Shift is held with a painting tool and nothing is being painted, a thin line runs from the end of the last
//! stroke on the document to the pointer (Photoshop's rubber band). Which tools connect and where the last stroke
//! ended are the app's; this draws the line. The Ruler tool's line and protractor arm share its look.

use crate::canvas::canvas_space;
use crate::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleColor, StyleCx};
use craft_behavior::kurbo::Point;
use craft_behavior::{Input, Metrics};
use egui::Response;

/// The style of the straight-line preview: a halo under a thin line.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct LineGuideStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The line.
    line: StyleColor,
    /// The halo under the line, so it reads on any image.
    halo: StyleColor,
}

/// The Shift-click line from `from` (the end of the last stroke) to `to` (the pointer), both in document units. A
/// non-finite end draws nothing. The brush tip at the pointer is the cursor's, not this line's.
#[derive(crate::contract::Component)]
#[component(canvas_control)]
pub struct StraightLinePreview {
    #[common]
    common: Common,
    from: Point,
    to: Point,
}

impl StraightLinePreview {
    /// A line named `label` from `from` to `to` (document units).
    pub fn new(label: impl Into<Label>, from: Point, to: Point) -> Self {
        StraightLinePreview { common: Common::new(), from, to }.label(label)
    }
}

impl Component for StraightLinePreview {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = LineGuideStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: StraightLinePreviewStyleKey) -> LineGuideStyle {
        let overlay = &cx.tokens.color.overlay;
        LineGuideStyle { metrics: Metrics::new(), line: cx.color(overlay.stroke), halo: cx.color(overlay.halo) }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let finite = [self.from, self.to].iter().all(|point| point.x.is_finite() && point.y.is_finite());
        let Some(space) = canvas_space(&cx.env).filter(|_| finite) else { return };
        let ends = PaintShape::Path(vec![space.view.to_screen(self.from), space.view.to_screen(self.to)]);
        let draw = ui.draw();
        draw.stroke(&ends, StrokeWidth::Thick, cx.style.halo);
        draw.stroke(&ends, StrokeWidth::Thin, cx.style.line);
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
