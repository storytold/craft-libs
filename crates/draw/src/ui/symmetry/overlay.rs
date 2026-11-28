//! `SymmetryOverlay`: the symmetry axes drawn on the canvas, a paint-only canvas control.

use super::HANDLE_SHARE;
use super::model::Symmetry;
use craft_behavior::kurbo::Size;
use craft_behavior::{Input, Metrics};
use craft_ui::canvas::canvas_space;
use craft_ui::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Styled as _};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, HandleLook, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};

/// The style of the symmetry guides: the line colour of the overlay tokens and the look of the centre and rotation
/// handle marks.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SymmetryOverlayStyle {
    #[style(metrics)]
    metrics: Metrics,
    line: StyleColor,
    handle: HandleLook,
}

/// The guides of a painting symmetry: each axis clipped to the page, a bare line in the overlay line colour, drawn
/// while a painting tool is picked or the symmetry is being transformed. While [editing](Self::editing) it also draws
/// the centre mark, a line to the rotation handle and the handle's mark.
///
/// Drawing only: the pointer and keyboard parts are [`SymmetryHandles`](super::SymmetryHandles). Show the overlay
/// before the handles.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct SymmetryOverlay<'a> {
    #[common]
    common: Common,
    symmetry: &'a Symmetry,
    page: Size,
    /// Transform Symmetry is open: the centre mark and the rotation handle show.
    #[prop]
    editing: bool,
}

impl<'a> SymmetryOverlay<'a> {
    /// The guides of `symmetry` over a page of `page` document units.
    pub fn new(label: impl Into<Label>, symmetry: &'a Symmetry, page: Size) -> Self {
        SymmetryOverlay { common: Common::new(), symmetry, page, editing: false }.label(label)
    }
}

impl Component for SymmetryOverlay<'_> {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = SymmetryOverlayStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: SymmetryOverlayStyleKey) -> SymmetryOverlayStyle {
        SymmetryOverlayStyle { metrics: Metrics::new(), line: cx.color(cx.tokens.color.overlay.line), handle: cx.handle() }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let style = &cx.style;
        let draw = ui.draw();
        draw.clipped(cx.rect, |draw| {
            for line in self.symmetry.segments(self.page) {
                let ends: Vec<Pos2> = vec![space.view.to_screen(line.p0), space.view.to_screen(line.p1)];
                draw.stroke(&PaintShape::Path(ends), StrokeWidth::Thin, style.line);
            }
            if !self.editing {
                return;
            }
            let centre = space.view.to_screen(self.symmetry.center);
            let handle = space.view.to_screen(self.symmetry.rotation_handle(self.page, HANDLE_SHARE));
            draw.stroke(&PaintShape::Path(vec![centre, handle]), StrokeWidth::Thin, style.line);
            let look = &style.handle;
            let square = PaintShape::Rect(Rect::from_center_size(centre, Vec2::splat(look.side)), CornerRadius::ZERO);
            let round = PaintShape::Circle(handle, look.side / 2.0);
            for mark in [square, round] {
                draw.fill(&mark, cx.resolve(&look.fill));
                draw.stroke(&mark, StrokeWidth::Thin, look.stroke);
            }
        });
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
