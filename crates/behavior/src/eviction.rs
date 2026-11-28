//! When a component's remembered state is due for eviction.

/// Passes of the viewport that last showed an owner that must go by before its shown entries can be evicted.
const GRACE_PASSES: u64 = 3;

/// Seconds since an owner's last registration before its shown entries can be evicted.
const GRACE_SECS: f64 = 0.5;

/// Whether the owner last registered at pass `last_pass` and time `last_at` is due: both the pass count and the
/// time limit have run out. A viewport egui no longer reports counts as having run out the passes, so only the
/// time limit applies; `pass_now` is then unused.
pub fn owner_due(last_pass: u64, last_at: f64, pass_now: u64, now: f64, viewport_live: bool) -> bool {
    let passes_out = !viewport_live || pass_now >= last_pass + GRACE_PASSES;
    passes_out && now - last_at >= GRACE_SECS
}
