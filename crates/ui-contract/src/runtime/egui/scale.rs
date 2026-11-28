//! Pixels per point, read into `Scale`.

use egui::Context;

/// The pixels per point of the running pass: the native density times the interface scale.
pub(crate) fn pixels_per_point(ctx: &Context) -> f32 {
    ctx.pixels_per_point()
}

/// Sets the interface scale over the native pixels per point. egui applies it from the next pass, and its
/// zoom factor is context-wide.
pub(crate) fn set_interface_scale(ctx: &Context, scale: f32) {
    ctx.set_zoom_factor(scale);
}
