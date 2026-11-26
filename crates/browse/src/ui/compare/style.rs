//! `CompareTileStyle`.

use craft_ui::style::{ComponentStyle, Metrics, StyleColor, StyleCx};

use super::CompareTile;

/// The tile's own marks: the rejected veil and the active ring with its halo. The picture is a `PreviewPane` and the
/// caption its own components.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CompareTileStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The veil over a rejected picture (`surface.scrim`).
    pub(super) veil: StyleColor,
    /// The active ring (`accent.solid`).
    pub(super) ring: StyleColor,
    /// The ring's halo (`overlay.halo`).
    pub(super) halo: StyleColor,
}

impl CompareTile<'_> {
    pub(super) fn tile_style(cx: &StyleCx<'_>) -> CompareTileStyle {
        let colors = &cx.tokens.color;
        CompareTileStyle {
            metrics: Metrics::new(),
            veil: cx.color(colors.surface.scrim),
            ring: cx.color(colors.accent.solid),
            halo: cx.color(colors.overlay.halo),
        }
    }
}
