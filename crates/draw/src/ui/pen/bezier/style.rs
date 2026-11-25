//! The look of [`BezierOverlay`](super::BezierOverlay).

use craft_behavior::Metrics;
use craft_ui::style::{ComponentStyle, FocusLook, HandleLook, StyleColor, StyleCx};

/// The style of the path overlay: the line and handle colours of the theme, which a path's own colour or the Path
/// Options replace.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct BezierStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The outline of a path that has no colour of its own, and the fill of a selected anchor.
    pub(crate) line: StyleColor,
    /// The halo under a segment the hover mark shows.
    pub(crate) halo: StyleColor,
    /// An anchor: a hollow square.
    pub(crate) anchor: HandleLook,
    /// The end of a direction handle.
    pub(crate) control: HandleLook,
    /// The anchor or handle end under the pointer, which a click would take.
    pub(crate) close: HandleLook,
    pub(crate) focus: FocusLook,
    /// The width of an outline.
    pub(crate) line_width: f32,
    /// The width of the selected segment and of the hover mark's line.
    pub(crate) segment_width: f32,
    /// How far from a part's centre a press still takes it.
    pub(crate) reach: f32,
}

impl BezierStyle {
    pub(in crate::ui::pen) fn of(cx: &StyleCx<'_>) -> Self {
        let tokens = cx.tokens;
        let overlay = &tokens.color.overlay;
        let mut anchor = cx.handle();
        anchor.side = tokens.size.handle.sm;
        let mut control = anchor.clone();
        control.fill = cx.handle().fill;
        let close = cx.handle();
        BezierStyle {
            metrics: Metrics::new(),
            line: cx.color(overlay.line),
            halo: cx.color(overlay.halo),
            anchor,
            control,
            close,
            focus: cx.focus_look(),
            line_width: tokens.stroke.medium,
            segment_width: tokens.stroke.thick,
            reach: tokens.size.hit.target,
        }
    }
}
