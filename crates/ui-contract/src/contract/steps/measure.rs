//! Step 5, measure, and the size guard.

use super::ShapeKind;
use crate::contract::cx::MeasureCx;
use crate::contract::{Component, Resolved, Sizing, ValueOf};
use crate::runtime::env::Env;
use crate::runtime::layout_memory::{self, Inputs};
use crate::style::{ComponentStyle, StyleView};
use craft_behavior::{Insets, Metrics, Proposal};
use egui::{Ui, Vec2};

/// What step 5 reads besides the component.
pub(crate) struct MeasureIn<'a> {
    pub ui: &'a Ui,
    pub env: &'a Env,
    pub proposal: Proposal,
    pub room: Option<f32>,
}

/// The content size `C::measure` gives, through its read view, and in a measuring run the inputs outside its room it
/// read (`layout_memory::measure_call`).
pub(crate) fn measure_content<C: Component>(
    comp: &mut C,
    input: &MeasureIn<'_>,
    style: &C::Style,
    resolved: &Resolved<'_>,
    value: Option<&ValueOf<C>>,
) -> (Vec2, Inputs) {
    let cx = measure_cx::<C>(input, style, resolved, value);
    layout_memory::measure_call(input.ui.ctx(), || comp.measure(&cx))
}

/// Step 5, measure: the `content` size plus padding, then `max(min_size)`, and for any proposal but `Min` at least the
/// ideal width, up to the room, unless the component is sized to its content (`sizing`, a leaf's `Sizing::Content`).
/// At `Min` a least width the style sets takes the place of `min_size`'s width. `extent` is what the shape adds to the
/// content: a body's remembered size or a collection's content size. The padding is the style's and the props' (`padding`).
pub(crate) fn measure(style: &dyn StyleView, input: &MeasureIn<'_>, content: Vec2, extent: Vec2, resolved: &Resolved<'_>) -> Vec2 {
    let (metrics, sizing) = (style.metrics(), resolved.sizing);
    let content = content + extent;
    let padding = padding(&metrics, resolved.pad);
    let padding = Vec2::new(padding.start + padding.end, padding.top + padding.bottom);
    let floor = match (input.proposal, metrics.least_width) {
        (Proposal::Min, Some(least)) => Vec2::new(least, metrics.min_size.y),
        _ => metrics.min_size,
    };
    let mut size = (content + padding).max(floor);
    if input.proposal != Proposal::Min && sizing != Some(Sizing::Content) {
        size.x = size.x.max(input.room.map_or(metrics.ideal_width, |room| metrics.ideal_width.min(room)));
    }
    size
}

/// The padding between the component's edge and its content: the style's, and `pad` on every side that the shape's
/// props add (`Resolved::pad`).
pub(crate) fn padding(metrics: &Metrics, pad: f32) -> Insets {
    let padding = metrics.padding;
    Insets::new(padding.start + pad, padding.end + pad, padding.top + pad, padding.bottom + pad)
}

/// The read view `measure` and `parts` get. Its room is the content's: the parent's room less the padding
/// `measure` adds.
pub(crate) fn measure_cx<'a, C: Component>(
    input: &MeasureIn<'a>,
    style: &'a C::Style,
    resolved: &'a Resolved<'a>,
    value: Option<&'a ValueOf<C>>,
) -> MeasureCx<'a, C> {
    let padding = padding(&ComponentStyle::metrics(style), resolved.pad);
    MeasureCx {
        env: input.env.view(),
        style,
        resolved,
        proposal: input.proposal,
        room: input.room.map(|room| (room - padding.start - padding.end).max(0.0)),
        value,
        ui: input.ui,
        tokens: &input.env.tokens,
        ink: input.env.ink,
        id: resolved.id,
        enabled_size: (resolved.enabled, resolved.size),
    }
}

/// The size guard: finite and at most `MAX_EXTENT` on both sides. A leaf or collection also needs both sides
/// above zero; a zero size from a leaf means nothing to show.
pub(crate) fn guard(size: Vec2, shape: ShapeKind) -> bool {
    let sane = size.x.is_finite()
        && size.y.is_finite()
        && size.x >= 0.0
        && size.y >= 0.0
        && size.x <= crate::contract::MAX_EXTENT
        && size.y <= crate::contract::MAX_EXTENT;
    sane && (shape == ShapeKind::Body || size.x > 0.0 && size.y > 0.0)
}
