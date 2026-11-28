//! [`VanishingPointStyle`]: Vanishing Point's marks, from tokens.

use craft_behavior::Metrics;
use craft_theme::variant::Tone;
use craft_ui::style::{ComponentStyle, HandleLook, StyleColor, StyleCx};

/// How much of the plane's colour the grid lines keep: they run across the picture, so they are fainter than the
/// outline.
const GRID_FADE: f32 = 0.6;

/// The planes' and the stamp's look: a plane's colour follows its check (accent when valid, the warning colour when
/// questionable, the error colour when invalid); nodes are handles with a white fill and the plane's colour outline;
/// lines lie over the overlay halo.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct VanishingPointStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The node squares and the pending corners.
    pub(super) handle: HandleLook,
    /// The side of the square that a press on a node reaches.
    pub(super) reach: f32,
    /// How far a press may move and still be a click, in points.
    pub(super) slop: f32,
    /// The length of the source cross's arms, from its centre.
    pub(super) arm: f32,
    pub(super) valid: StyleColor,
    pub(super) questionable: StyleColor,
    pub(super) invalid: StyleColor,
    pub(super) valid_grid: StyleColor,
    pub(super) questionable_grid: StyleColor,
    /// The fill of a node square.
    pub(super) node_fill: StyleColor,
    /// The outline drawn under lines so they read on any image.
    pub(super) halo: StyleColor,
    /// The ring's and the cross's own line.
    pub(super) ink: StyleColor,
}

impl VanishingPointStyle {
    fn of(cx: &StyleCx<'_>) -> VanishingPointStyle {
        let colors = &cx.tokens.color;
        let (hit, size) = (&cx.tokens.size.hit, &cx.tokens.size);
        VanishingPointStyle {
            metrics: Metrics::new(),
            handle: cx.handle(),
            reach: hit.target,
            slop: hit.slop,
            arm: size.handle.md,
            valid: cx.color(colors.accent.solid),
            questionable: cx.tone(Tone::Warn),
            invalid: cx.tone(Tone::Bad),
            valid_grid: cx.color(colors.accent.solid.faded(GRID_FADE)),
            questionable_grid: cx.color(colors.status.warn.fg.faded(GRID_FADE)),
            node_fill: cx.color(colors.overlay.handle_fill),
            halo: cx.color(colors.overlay.halo),
            ink: cx.color(colors.overlay.stroke),
        }
    }
}

/// The style the planes control reads.
pub(super) fn style(cx: &StyleCx<'_>) -> VanishingPointStyle {
    VanishingPointStyle::of(cx)
}
