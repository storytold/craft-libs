//! The style of the title bar.
//!
//! The bar paints a chrome surface and a divider rule along its bottom edge; its height is its slot's
//! ([`SlotSize::TitleBar`](crate::shell::SlotSize), `size.bar.title`), the caption buttons carry their own
//! family metrics and colours (`caption.*`), and the title is a `Text`. Nothing else of the legacy bar style has a
//! token: the corner size belongs to the window's grips, the gaps to the body's row gap.

use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::Metrics;

/// The style of a title bar.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TitleBarStyle {
    #[style(metrics)]
    metrics: Metrics,
}

impl TitleBarStyle {
    /// The style at `cx`.
    pub(super) fn of(_: &StyleCx<'_>) -> TitleBarStyle {
        TitleBarStyle { metrics: Metrics::new() }
    }
}
