//! The Freeze Lasso line, a canvas control over document points.

use craft_behavior::kurbo::Point;
use craft_behavior::{Input, Metrics};
use craft_ui::canvas::canvas_space;
use craft_ui::contract::{Access, Common, Component, Cx, PaintUi, Role};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::Response;

/// Whether the Freeze Lasso freezes or thaws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LassoTone {
    /// The lasso freezes an area.
    #[default]
    Freeze,
    /// The lasso (with Alt) thaws an area.
    Thaw,
}

/// The style of the lasso line: its colour follows the tone.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct LassoLineStyle {
    #[style(metrics)]
    metrics: Metrics,
    line: StyleColor,
}

/// The Freeze Lasso's path while it is drawn, a canvas control over document points: the bad tone when it freezes,
/// the accent when it thaws.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control, sample = LassoLine::sample)]
pub struct LassoLine<'a> {
    #[common]
    common: Common,
    points: &'a [Point],
    /// Freezing or thawing.
    #[style_key]
    #[prop]
    tone: LassoTone,
}

impl<'a> LassoLine<'a> {
    /// The line through `points`, document pixels.
    pub fn new(points: &'a [Point]) -> Self {
        LassoLine { common: Common::new(), points, tone: LassoTone::Freeze }
    }
}

impl LassoLine<'static> {
    /// A line across the document, for the suite.
    fn sample() -> Self {
        const LINE: [Point; 3] = [Point::new(20.0, 20.0), Point::new(120.0, 40.0), Point::new(80.0, 120.0)];
        LassoLine::new(&LINE)
    }
}

impl Component for LassoLine<'_> {
    const ROLE: Role = Role::GROUP;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = LassoLineStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, key: LassoLineStyleKey) -> LassoLineStyle {
        let colors = &cx.tokens.color;
        let line = match key.tone {
            LassoTone::Freeze => colors.status.bad.solid,
            LassoTone::Thaw => colors.accent.solid,
        };
        LassoLineStyle { metrics: Metrics::new(), line: cx.color(line) }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        if self.points.len() < 2 {
            return;
        }
        let path = PaintShape::Path(self.points.iter().map(|point| space.view.to_screen(*point)).collect());
        ui.draw().stroke(&path, StrokeWidth::Medium, cx.style.line);
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
