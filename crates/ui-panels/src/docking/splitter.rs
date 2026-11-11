//! `Splitter`: the look of a split handle, a leaf the handles of a `SplitArea` wear as their face.

use crate::contract::{Common, Component, Cx, PaintUi, Role, Styled as _};
use crate::primitives::{DataWidth, PaintShape};
use crate::style::{ComponentStyle, Stateful, StyleColor, StyleCx};
use craft_behavior::{Axis, Metrics, Sense};
use craft_theme::States;
use egui::{Response, Vec2};

/// The style of a splitter: a hairline that becomes a thick accent line under the pointer.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SplitterStyle {
    #[style(metrics)]
    metrics: Metrics,
    line: Stateful<StyleColor>,
    /// The line of a column's inner edge: none at rest, as the column's frame already draws that edge, and the accent
    /// under the pointer.
    edge_line: Stateful<StyleColor>,
    /// The line's width: a hairline at rest, thick under the pointer.
    width: Stateful<f32>,
}

/// A line between two panes, as thick across as the pointer's grab distance.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct Splitter {
    #[common]
    common: Common,
    /// The axis the panes it divides follow: `X` for panes side by side, so the line is vertical.
    #[style_key]
    axis: Axis,
    /// The boundary's coordinate across the line, where the handle's rect does not centre on it: a column's inner
    /// edge, whose handle stays inside the column. The line lies on the boundary, over the column's frame.
    #[prop(into, quantity = "coordinate")]
    boundary: Option<f32>,
}

impl Splitter {
    /// A splitter between panes that follow `axis`.
    pub fn new(axis: Axis) -> Self {
        Splitter { common: Common::new(), axis, boundary: None }
    }
}

impl Component for Splitter {
    const ROLE: Role = Role::SPLITTER;
    const PARTS: bool = false;
    type Style = SplitterStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, key: SplitterStyleKey) -> SplitterStyle {
        let color = &cx.tokens.color;
        let (across, along) = (cx.tokens.size.hit.target, cx.control().height);
        let least = match key.axis {
            Axis::X => Vec2::new(across, along),
            _ => Vec2::new(along, across),
        };
        let rest = cx.color(color.line.border);
        let accent = cx.color(color.accent.solid);
        let clear = cx.color(craft_theme::Rgba::TRANSPARENT);
        SplitterStyle {
            metrics: Metrics::new().min_size(least),
            line: Stateful::new(rest).off(States::new(rest, accent, accent, rest)),
            edge_line: Stateful::new(clear).off(States::new(clear, accent, accent, clear)),
            width: Stateful::new(cx.tokens.stroke.thin).off(States::new(
                cx.tokens.stroke.thin,
                cx.tokens.stroke.thick,
                cx.tokens.stroke.thick,
                cx.tokens.stroke.thin,
            )),
        }
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rect = cx.rect;
        let width = cx.resolve(&cx.style.width);
        let ends = match self.axis {
            Axis::X => {
                let x = self.boundary.unwrap_or(rect.center().x);
                vec![egui::Pos2::new(x, rect.top()), egui::Pos2::new(x, rect.bottom())]
            }
            _ => {
                let y = self.boundary.unwrap_or(rect.center().y);
                vec![egui::Pos2::new(rect.left(), y), egui::Pos2::new(rect.right(), y)]
            }
        };
        let line = if self.boundary.is_some() { &cx.style.edge_line } else { &cx.style.line };
        ui.draw().stroke(&PaintShape::Path(ends), DataWidth::points(width), cx.resolve(line));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
