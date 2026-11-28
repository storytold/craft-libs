//! `SourceMarker`: the clone source's crosshair.

use crate::canvas::canvas_space;
use crate::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleColor, StyleCx};
use craft_behavior::kurbo::Point;
use craft_behavior::{Input, Metrics};
use egui::{Pos2, Response, Vec2};

/// The style of the source marker: the arms in a thin colour over a halo.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SourceMarkerStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The arms.
    line: StyleColor,
    /// The halo under the arms, so they read on any image.
    halo: StyleColor,
    /// The length of one arm from the crosshair's centre.
    arm: f32,
}

/// The clone source's crosshair at document point `at`: two arms through the view, a thin line over a thick halo.
/// Display only.
#[derive(crate::contract::Component)]
#[component(canvas_control, content_painter)]
pub struct SourceMarker {
    #[common]
    common: Common,
    at: Point,
}

impl SourceMarker {
    /// A marker named `label` at the document point `at`.
    pub fn new(label: impl Into<Label>, at: Point) -> Self {
        SourceMarker { common: Common::new(), at }.label(label)
    }
}

impl Component for SourceMarker {
    const ROLE: Role = Role::GROUP;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = SourceMarkerStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: SourceMarkerStyleKey) -> SourceMarkerStyle {
        let overlay = &cx.tokens.color.overlay;
        SourceMarkerStyle {
            metrics: Metrics::new(),
            line: cx.color(overlay.handle_fill),
            halo: cx.color(overlay.halo),
            arm: cx.tokens.size.icon.sm / 2.0,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if !(self.at.x.is_finite() && self.at.y.is_finite()) {
            return;
        }
        let Some(space) = canvas_space(&cx.env) else { return };
        let centre = space.view.to_screen(self.at);
        let arm = cx.style.arm;
        let draw = ui.draw();
        for ends in [
            [Pos2::new(centre.x - arm, centre.y), Pos2::new(centre.x + arm, centre.y)],
            [Pos2::new(centre.x, centre.y - arm), Pos2::new(centre.x, centre.y + arm)],
        ] {
            let path = PaintShape::Path(ends.to_vec());
            draw.stroke(&path, StrokeWidth::Thick, cx.style.halo);
            draw.stroke(&path, StrokeWidth::Thin, cx.style.line);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
