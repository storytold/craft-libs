//! `Divider`: a separator line one physical pixel wide, horizontal or vertical.
use crate::contract::{Common, Component, Cx, InBar, InMenu, MeasureCx, PaintUi, Role};
use crate::primitives::RuleKind;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{Metrics, Sense};
use egui::{Response, Vec2};

#[cfg(feature = "testing")]
mod stories;

/// The style of a divider.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct DividerStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The line's width.
    width: f32,
    /// The width of the box a vertical line sits in inside a bar, the line centred: `space.gap.item`.
    slot: f32,
    /// The least length of a line, vertical or horizontal: one control height.
    length: f32,
    /// Above and below a horizontal line in a menu: `space.gap.inline`.
    separator_pad: f32,
}

/// Which way the line runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

/// A separator line in the divider colour, snapped to the pixel grid. A horizontal line takes the width it is
/// offered (at least one control height; in a content-sized column it never widens the column) and a vertical one the height of its row, at least a control's height. Decoration: it is not in the
/// AccessKit tree.
#[derive(crate::contract::Component)]
pub struct Divider {
    #[common]
    common: Common,
    axis: Axis,
}

impl Divider {
    /// A line across the width offered.
    pub fn horizontal() -> Self {
        Divider { common: Common::new(), axis: Axis::Horizontal }
    }

    /// A line down the full height of its row.
    pub fn vertical() -> Self {
        Divider { common: Common::new(), axis: Axis::Vertical }
    }
}

impl Component for Divider {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = DividerStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: DividerStyleKey) -> DividerStyle {
        DividerStyle {
            metrics: Metrics::new(),
            width: cx.rule(RuleKind::Divider).width,
            slot: cx.tokens.space.gap.item,
            length: cx.control().height,
            separator_pad: cx.tokens.space.gap.inline,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        match self.axis {
            Axis::Horizontal if cx.env.get::<InMenu>().0 => Vec2::new(cx.style.length, cx.style.width + 2.0 * cx.style.separator_pad),
            Axis::Horizontal => Vec2::new(cx.style.length, cx.style.width),
            Axis::Vertical if cx.env.get::<InBar>().0 => Vec2::new(cx.style.slot.max(cx.style.width), cx.style.length),
            Axis::Vertical => Vec2::new(cx.style.width, cx.style.length),
        }
    }

    fn fills(&self) -> Option<craft_behavior::Axis> {
        Some(match self.axis {
            Axis::Horizontal => craft_behavior::Axis::X,
            Axis::Vertical => craft_behavior::Axis::Y,
        })
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rect = cx.rect;
        let (from, to) = match self.axis {
            Axis::Horizontal => (rect.left_center(), rect.right_center()),
            Axis::Vertical => (rect.center_top(), rect.center_bottom()),
        };
        ui.draw().rule(RuleKind::Divider, from, to);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
