//! `Divider`: a separator line as wide as a divider rule, horizontal or vertical.
use crate::contract::{Common, Component, Cx, InBar, InMenu, MeasureCx, PaintUi, Role};
use crate::primitives::RuleKind;
use crate::style::{ComponentStyle, RuleWidth, StyleCx};
use craft_behavior::{Metrics, Sense};
use egui::{Response, Vec2};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The style of a divider.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct DividerStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The line's width.
    width: RuleWidth,
    /// The width of the box a vertical line sits in inside a bar, the line centred: `space.gap.item`.
    slot: f32,
    /// The least length of a line, vertical or horizontal: one control height.
    length: f32,
    /// A horizontal line in a menu, with its room.
    separator: SeparatorRule,
}

/// A line between groups of items, in a menu (a horizontal [`Divider`]) or a list (`ListRow::separator`): the divider
/// rule with `space.gap.inline` of room above and below it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SeparatorRule {
    width: RuleWidth,
    pad: f32,
}

impl SeparatorRule {
    pub(crate) fn of(cx: &StyleCx<'_>) -> Self {
        SeparatorRule { width: cx.rule(RuleKind::Divider).width, pad: cx.tokens.space.gap.inline }
    }

    /// The height of the line and its room on a display of `pixels_per_point`.
    pub(crate) fn height(self, pixels_per_point: f32) -> f32 {
        self.width.points(pixels_per_point) + 2.0 * self.pad
    }
}

/// Which way the line runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

/// A separator line in the divider colour, snapped to the pixel grid. A horizontal line takes the width it is
/// offered (at least one control height; in a content-sized column it never widens the column) and a vertical one the height of its row, at least a control's height. In a bar it
/// shows only with the member after it. Decoration: it is not in the AccessKit tree.
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
    const RULE: bool = true;
    type Style = DividerStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: DividerStyleKey) -> DividerStyle {
        DividerStyle {
            metrics: Metrics::new(),
            width: cx.rule(RuleKind::Divider).width,
            slot: cx.tokens.space.gap.item,
            length: cx.control().height,
            separator: SeparatorRule::of(cx),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let width = cx.style.width.points(cx.ui.ctx().pixels_per_point());
        let length = cx.style.length;
        match self.axis {
            Axis::Horizontal if cx.env.get::<InMenu>().0 => Vec2::new(length, cx.style.separator.height(cx.ui.ctx().pixels_per_point())),
            Axis::Horizontal => Vec2::new(length, width),
            Axis::Vertical if cx.env.get::<InBar>().0 => Vec2::new(cx.style.slot.max(width), length),
            Axis::Vertical => Vec2::new(width, length),
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
