//! The ctx-side view of a measuring run: the sizing pass the framework starts to measure a container's contents
//! before the real run. APIs that get only a `Context` ask here whether to skip an effect, so a measuring run
//! delivers nothing the real run then lacks.

use super::egui::memory as store;
use egui::{Context, Id};

fn depth_id() -> Id {
    store::unique("craft-ui measuring run depth")
}

/// The effects skipped in measuring runs, with the pass that logged them.
#[cfg(any(test, feature = "testing"))]
#[derive(Default)]
struct EffectLog {
    pass: u64,
    effects: Vec<&'static str>,
}

/// The bodies whose contents ran in a measuring run, with the pass that counted them.
#[cfg(feature = "testing")]
#[derive(Default)]
struct BodyRuns {
    pass: u64,
    runs: Vec<(Id, u32)>,
}

/// The collections whose widest row was built and measured, with the pass that counted them.
#[cfg(feature = "testing")]
#[derive(Default)]
struct WidestRuns {
    pass: u64,
    runs: Vec<(Id, u32)>,
}

/// Runs `f` as a measuring run. Nested calls are fine.
pub fn run<R>(ctx: &Context, f: impl FnOnce() -> R) -> R {
    /// Restores the depth when the run ends, by a panic too: a context used after a caught panic is not measuring.
    struct Open<'a>(&'a Context, u32);
    impl Drop for Open<'_> {
        fn drop(&mut self) {
            store::put(self.0, depth_id(), self.1);
        }
    }
    let before = depth(ctx);
    store::put(ctx, depth_id(), before + 1);
    let _open = Open(ctx, before);
    super::layout_memory::guard::measuring(f)
}

fn depth(ctx: &Context) -> u32 {
    store::peek_copy::<u32>(ctx, depth_id()).unwrap_or(0)
}

/// Whether a measuring run is in progress.
pub fn active(ctx: &Context) -> bool {
    depth(ctx) > 0
}

/// Whether the innermost measuring run is the outermost one: no other measuring run encloses it.
pub(crate) fn outermost(ctx: &Context) -> bool {
    depth(ctx) == 1
}

/// Whether the effect `what` may happen: `false` in a measuring run, and the caller skips it. Under test the
/// skipped effects are logged for `take_effects`.
pub fn effect(ctx: &Context, what: &'static str) -> bool {
    if !active(ctx) {
        return true;
    }
    #[cfg(any(test, feature = "testing"))]
    let frame = ctx.cumulative_pass_nr();
    #[cfg(any(test, feature = "testing"))]
    store::with::<EffectLog, _>(ctx, |log| {
        if log.pass != frame {
            *log = EffectLog { pass: frame, effects: Vec::new() };
        }
        log.effects.push(what);
    });
    #[cfg(not(any(test, feature = "testing")))]
    let _ = what;
    false
}

/// Takes the entries of the one-shot `queue` that `wanted` accepts. A measuring run takes none and leaves the queue
/// whole, so the real run finds them. Every take of a one-shot queue that a body or a component's
/// steps can reach goes through here.
pub fn take_queued<T>(ctx: &Context, queue: &mut Vec<T>, wanted: impl Fn(&T) -> bool) -> Vec<T> {
    if active(ctx) {
        return Vec::new();
    }
    let mut mine = Vec::new();
    let mut kept = Vec::new();
    for item in std::mem::take(queue) {
        if wanted(&item) { mine.push(item) } else { kept.push(item) }
    }
    *queue = kept;
    mine
}

/// The effects skipped in measuring runs this frame, in order; empties the log.
#[cfg(feature = "testing")]
pub fn take_effects(ctx: &Context) -> Vec<&'static str> {
    let frame = ctx.cumulative_pass_nr();
    store::with::<EffectLog, _>(ctx, |log| if log.pass == frame { std::mem::take(&mut log.effects) } else { Vec::new() })
}

/// Counts a measuring run of the contents of the body `id` in this pass.
#[cfg(feature = "testing")]
pub(crate) fn count_body(ctx: &Context, id: Id) {
    let pass = ctx.cumulative_pass_nr();
    store::with::<BodyRuns, _>(ctx, |log| {
        if log.pass != pass {
            *log = BodyRuns { pass, runs: Vec::new() };
        }
        match log.runs.iter_mut().find(|(body, _)| *body == id) {
            Some((_, count)) => *count += 1,
            None => log.runs.push((id, 1)),
        }
    });
}

/// The bodies whose contents ran in a measuring run in the last pass, and how many runs each; empty when that pass
/// ran none.
#[cfg(feature = "testing")]
pub fn last_pass_body_runs(ctx: &Context) -> Vec<(Id, u32)> {
    let last = ctx.cumulative_pass_nr().saturating_sub(1);
    store::with::<BodyRuns, _>(ctx, |log| if log.pass == last { log.runs.clone() } else { Vec::new() })
}

/// The bodies whose contents ran in a measuring run in the last pass that ran one, and how many runs each.
#[cfg(feature = "testing")]
pub fn body_runs(ctx: &Context) -> Vec<(Id, u32)> {
    store::with::<BodyRuns, _>(ctx, |log| log.runs.clone())
}

/// Counts a build and measure of the rows of the collection `id` for its widest row in this pass.
#[cfg(feature = "testing")]
pub(crate) fn count_widest(ctx: &Context, id: Id) {
    let pass = ctx.cumulative_pass_nr();
    store::with::<WidestRuns, _>(ctx, |log| {
        if log.pass != pass {
            *log = WidestRuns { pass, runs: Vec::new() };
        }
        match log.runs.iter_mut().find(|(collection, _)| *collection == id) {
            Some((_, count)) => *count += 1,
            None => log.runs.push((id, 1)),
        }
    });
}

/// The collections whose widest row was built and measured in the last pass that did so, and how many times each.
#[cfg(feature = "testing")]
pub fn widest_runs(ctx: &Context) -> Vec<(Id, u32)> {
    store::with::<WidestRuns, _>(ctx, |log| log.runs.clone())
}
