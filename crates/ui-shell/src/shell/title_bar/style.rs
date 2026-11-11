//! The style of the title bar.
//!
//! The bar paints a chrome surface and a hairline along its bottom edge; its height is its slot's
//! ([`SlotSize::TitleBar`](crate::shell::SlotSize), `size.bar.title`), the caption buttons carry their own
//! family metrics and colours (`caption.*`), and the title is a `Text`. Nothing else of the legacy bar style has a
//! token: the corner size belongs to the window's grips, the gaps to the body's row gap.

use crate::primitives::RuleKind;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::Metrics;

/// The style of a title bar.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TitleBarStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The width of the rule along the bottom edge: one device pixel.
    pub(super) rule: f32,
}

impl TitleBarStyle {
    /// The style at `cx`.
    pub(super) fn of(cx: &StyleCx<'_>) -> TitleBarStyle {
        TitleBarStyle { metrics: Metrics::new(), rule: cx.rule(RuleKind::Divider).width }
    }
}
