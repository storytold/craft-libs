//! The replay audit (`CRAFT_UI_AUDIT_REPLAY=1`, test builds only): in a real pass, a memoized view that would replay
//! its kept run runs for real, and that run is compared with what the replay would have added. A difference panics,
//! naming the view and the first difference of each kind: a replay that differs from a real run is a view whose props
//! miss an input, or a registration the journal misses.
//!
//! A pass egui will discard paints nothing and keeps no record (`lazy::ran`): the audit skips it, and a view there
//! replays.
//!
//! The audit run keeps the kept run as it was, so every later pass compares with what a replay adds, and every decision
//! is the one the pass makes with the audit off. Only the outermost view that would replay runs: a view that would
//! replay inside an audit run replays, and the outer comparison covers that replay. The count trials' counters count
//! nothing inside an audit run (`runtime::measure`).

use crate::runtime::egui::memory as store;
use crate::runtime::lazy::Output;
use crate::runtime::replay::{self, Record};
use crate::runtime::shift::Shift;
use egui::{Context, Id, RecordShift, Rect};
use std::collections::HashSet;

/// How far apart, in points, the audit lets a coordinate of a replay and of the real run lie
/// (`Shift::snapped`, `PassRecords::first_difference`): a record moved by a whole pixel from positions off the float
/// grid differs from the run at the new place by rounding only, far less than this; ids, counts, colours, text and
/// every other float compare exactly.
pub(crate) const WITHIN: f32 = 1.0 / 256.0;

/// Whether two outputs of a memoized view are equal: both of its output type, and `==` within `WITHIN`.
pub(crate) type SameOutput = fn(&Output, &Output, &RecordShift) -> bool;

/// The audit's switch for one context and the views it named.
#[derive(Default)]
struct Audit {
    /// `None` until read: then `CRAFT_UI_AUDIT_REPLAY=1`, unless a test set it (`testing::audit_replays`).
    on: Option<bool>,
    /// The views named for painting a paint callback.
    named: HashSet<&'static str>,
}

/// Whether the audit runs in `ctx`.
pub(crate) fn on(ctx: &Context) -> bool {
    store::with::<Audit, _>(ctx, |audit| *audit.on.get_or_insert_with(|| std::env::var("CRAFT_UI_AUDIT_REPLAY").is_ok_and(|value| value == "1")))
}

/// Turns the audit on or off in `ctx`, in place of `CRAFT_UI_AUDIT_REPLAY`.
pub fn set(ctx: &Context, on: bool) {
    store::with::<Audit, _>(ctx, |audit| audit.on = Some(on));
}

/// Whether the code running in `ctx` is a run of the audit: a test that counts its own runs of a view counts none in
/// it, as the framework's counters do.
pub fn in_run(ctx: &Context) -> bool {
    replay::in_audit_run(ctx)
}

/// Whether a view that would replay here runs for the audit: the audit is on, and no audit run is around it.
pub(crate) fn runs(ctx: &Context) -> bool {
    on(ctx) && !replay::in_audit_run(ctx)
}

/// Whether the kept output `kept`, as a replay at the place `shift` names returns it, equals `run`'s, as `R`.
pub(crate) fn same<R: PartialEq + Shift + 'static>(kept: &Output, run: &Output, shift: &RecordShift) -> bool {
    match (kept.downcast_ref::<R>(), run.downcast_ref::<R>()) {
        (Some(kept), Some(run)) => kept.shifted(shift).snapped(run, WITHIN) == *run,
        _ => false,
    }
}

/// What a view's kept run, or its audit run, added and returned.
pub(crate) struct Shown<'a> {
    pub(crate) record: &'a Record,
    pub(crate) rect: Rect,
    pub(crate) output: Option<&'a Output>,
}

/// Compares the audit run `run` of the view `name` (`id`) with its kept run `kept` replayed at the place `shift` names,
/// and panics on a difference.
pub(crate) fn compare(name: &'static str, id: Id, (kept, shift): (Shown<'_>, &RecordShift), run: Shown<'_>, same: Option<SameOutput>) {
    let mut found = Vec::new();
    let kept_rect = shift.rect(kept.rect).snapped(&run.rect, WITHIN);
    if kept_rect != run.rect {
        found.push(format!("rect: left {:?} / right {:?}", kept_rect, run.rect));
    }
    found.extend(kept.record.differences(run.record, shift));
    let output = match (kept.output, run.output, same) {
        (Some(kept), Some(run), Some(same)) => same(kept, run, shift),
        (kept, run, _) => kept.is_none() == run.is_none(),
    };
    if !output {
        found.push("output: the run returned another output than the kept one".to_owned());
    }
    assert!(
        found.is_empty(),
        "replay audit: memoized view `{name}` ({id:?}) differs from its real run (left: the replay, right: the run):\n  {}",
        found.join("\n  ")
    );
}

/// Names the view `name` once in `ctx` when its real run `record` painted a paint callback: it runs every pass.
pub(crate) fn name_callback(ctx: &Context, name: &'static str, record: &Record) {
    if record.paints_callback() && store::with::<Audit, _>(ctx, |audit| audit.named.insert(name)) {
        eprintln!("replay audit: memoized view `{name}` paints a paint callback, so it runs every pass");
    }
}
