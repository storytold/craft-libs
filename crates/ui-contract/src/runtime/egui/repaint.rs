//! `request_repaint_after`, the one timed wake, and the immediate repaint request every other caller goes through.

use egui::Context;

/// Asks for one more pass after this one.
pub(crate) fn request(ctx: &Context) {
    ctx.request_repaint();
}

/// Asks for a pass of `viewport` at once.
pub(crate) fn request_of(ctx: &Context, viewport: egui::ViewportId) {
    ctx.request_repaint_of(viewport);
}

/// Asks for one pass of `viewport` after `seconds`; the earliest request of a frame wins.
pub(crate) fn request_after(ctx: &Context, viewport: egui::ViewportId, seconds: f64) {
    ctx.request_repaint_after_for(std::time::Duration::from_secs_f64(seconds.max(0.0)), viewport);
}

/// The animation time that makes egui's own animations jump; zero turns motion off.
pub(crate) fn animation_time(ctx: &Context) -> f32 {
    ctx.global_style().animation_time
}
