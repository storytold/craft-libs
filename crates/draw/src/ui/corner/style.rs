//! [`CornerStyle`]: the corner widgets' look, from tokens. Every size is the same at every Size: the widgets sit on
//! the document.

use craft_behavior::Metrics;
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx};

/// The shortest side, in screen points, whose corners show widgets.
const MIN_SIDE: f32 = 30.0;

/// What [`CornerWidgets`](super::CornerWidgets) draws and how far it reaches.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CornerStyle {
    #[style(metrics)]
    pub(super) metrics: Metrics,
    /// The widget's diameter.
    pub(super) widget: f32,
    /// The inner dot's diameter.
    pub(super) dot: f32,
    /// The pointer's reach.
    pub(super) reach: f32,
    /// The least distance inside a corner.
    pub(super) inset: f32,
    /// The shortest side, on the screen, whose corners show widgets.
    pub(super) min_side: f32,
    /// The fill: `overlay.handle_fill`.
    pub(super) fill: StyleColor,
    /// The outline and the dot: `overlay.line`.
    pub(super) color: StyleColor,
}

impl CornerStyle {
    /// The corner widgets' look in `cx`.
    pub(super) fn from_cx(cx: &StyleCx<'_>) -> CornerStyle {
        let tokens = cx.tokens;
        CornerStyle {
            metrics: Metrics::new(),
            widget: tokens.size.handle.md,
            dot: tokens.size.handle.sm,
            reach: tokens.size.hit.target,
            inset: tokens.space.gap.stack,
            min_side: MIN_SIDE,
            fill: cx.color(tokens.color.overlay.handle_fill),
            color: cx.color(tokens.color.overlay.line),
        }
    }
}
