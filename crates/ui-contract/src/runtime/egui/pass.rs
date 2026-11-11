//! Per-viewport frame and pass counters and the viewport list.

use crate::runtime::frame;
use egui::{Context, ViewportId};

/// The pass number of `viewport`; it counts every pass, discard passes included.
pub(crate) fn pass_nr(ctx: &Context, viewport: ViewportId) -> u64 {
    match frame::running(ctx) {
        Some(facts) if facts.viewport == viewport => facts.pass_nr,
        _ => ctx.cumulative_pass_nr_for(viewport),
    }
}

/// The frame number of `viewport`.
pub(crate) fn frame_nr(ctx: &Context, viewport: ViewportId) -> u64 {
    ctx.cumulative_frame_nr_for(viewport)
}

/// The root viewport's frame number: the key of every once-per-frame service, because child viewports'
/// passes run inside the root's frame.
pub(crate) fn root_frame_nr(ctx: &Context) -> u64 {
    ctx.cumulative_frame_nr_for(ViewportId::ROOT)
}

/// The viewport whose pass is running.
#[doc(hidden)]
pub fn current_viewport(ctx: &Context) -> ViewportId {
    match frame::running(ctx) {
        Some(facts) => facts.viewport,
        None => ctx.viewport_id(),
    }
}

/// The viewports egui reports for the running pass.
pub(crate) fn live_viewports(ctx: &Context) -> Vec<ViewportId> {
    ctx.input(|input| input.raw.viewports.keys().copied().collect())
}

/// The pass's time in seconds.
pub(crate) fn now(ctx: &Context) -> f64 {
    ctx.input(|input| input.time)
}
