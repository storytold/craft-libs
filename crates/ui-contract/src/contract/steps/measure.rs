//! Step 5, measure, and the size guard.

use super::ShapeKind;
use crate::contract::cx::MeasureCx;
use crate::contract::{Component, Resolved, ValueOf};
use crate::runtime::env::Env;
use crate::runtime::layout_memory::{self, Inputs};
use crate::style::ComponentStyle;
use craft_behavior::Proposal;
use egui::{Ui, Vec2};

/// What step 5 reads besides the component.
pub(crate) struct MeasureIn<'a> {
    pub ui: &'a Ui,
    pub env: &'a Env,
    pub proposal: Proposal,
    pub room: Option<f32>,
}

/// Step 5, measure: content size plus padding, then `max(min_size)`, and for any proposal but `Min` at least the
/// ideal width, up to the room. `extent` is what the shape adds to the content: a body's remembered size or a
/// collection's content size. In a measuring run it returns the inputs outside its room the measure read
/// (`layout_memory::measure_call`) with the size.
pub(crate) fn measure<C: Component>(
    comp: &mut C,
    input: &MeasureIn<'_>,
    style: &C::Style,
    resolved: &Resolved<'_>,
    value: Option<&ValueOf<C>>,
    extent: Vec2,
) -> (Vec2, Inputs) {
    let metrics = style.metrics();
    let cx = measure_cx::<C>(input, style, resolved, value);
    let (content, inputs) = layout_memory::measure_call(input.ui.ctx(), || comp.measure(&cx));
    let content = content + extent;
    let padding = Vec2::new(metrics.padding.start + metrics.padding.end, metrics.padding.top + metrics.padding.bottom);
    let mut size = (content + padding).max(metrics.min_size);
    if input.proposal != Proposal::Min {
        size.x = size.x.max(input.room.map_or(metrics.ideal_width, |room| metrics.ideal_width.min(room)));
    }
    (size, inputs)
}

/// The read view `measure` and `parts` get. Its room is the content's: the parent's room less the padding
/// `measure` adds.
pub(crate) fn measure_cx<'a, C: Component>(
    input: &MeasureIn<'a>,
    style: &'a C::Style,
    resolved: &'a Resolved<'a>,
    value: Option<&'a ValueOf<C>>,
) -> MeasureCx<'a, C> {
    let padding = style.metrics().padding;
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
