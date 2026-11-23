//! The step of one pass of canvas motion: a Hand glide, a held Zoom press and an autoscroll integrate over it.

/// The longest step motion integrates, in seconds, so a stalled pass never jumps the view.
pub const FRAME_STEP_MAX: f64 = 0.1;

/// The step of a pass that came `dt` seconds after the previous one, read from the runtime's pass clock
/// ([`ToolContext::time`](super::ToolContext::time) is the running pass's reading of `runtime::frame::clock`): `dt`
/// within `0..=FRAME_STEP_MAX`, and 0 where it is not a number.
pub fn frame_step(dt: f64) -> f64 {
    if dt.is_finite() { dt.clamp(0.0, FRAME_STEP_MAX) } else { 0.0 }
}
