//! The state of the lock keys, read from the platform once per pass.
//!
//! The key events carry presses, not whether a lock is on. `craft_window::caps_lock` is the platform query; this
//! service asks it once in a pass, so everything in the pass reads the same answer.

use crate::runtime::memory::{global, with_session};
use egui::Context;

#[derive(Clone, Default)]
struct Reading {
    pass: Option<u64>,
    caps: bool,
}

/// Whether Caps Lock is on in this pass.
#[doc(hidden)]
pub fn caps_lock(ctx: &Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    with_session(ctx, global("craft-ui lock keys", ()), |reading: &mut Reading| {
        if reading.pass != Some(pass) {
            *reading = Reading { pass: Some(pass), caps: craft_window::caps_lock() };
        }
        reading.caps
    })
}
