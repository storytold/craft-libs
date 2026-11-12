//! Run records: what a real run of a settling region (`Lazy::settles`) added to its pass, so a later pass adds it again
//! in place of running the region. A record holds the region's paint, egui widget rects and AccessKit nodes (egui's
//! pass records), what it registered with the focus service (`focus::Transcript`), and a journal of every other
//! registration the services below take per pass: each service that keeps one notes it here while a record is open,
//! as the call that makes it again. Records nest: a region's record holds the records its settled regions replayed.
//!
//! The record also holds the facts the settled test reads (`runtime::lazy`): the values the run read, whether
//! something in it still moves (an animation that is not at its end, a repaint request) and the earliest time it asked
//! to be shown again, and whether it did something a replay cannot repeat (an overlay it shows, paint on another
//! layer, a label scope open around it). Such a record is never replayed: the region runs.

use crate::runtime::focus::{self, Transcript};
use crate::runtime::frame;
use crate::runtime::pass_state;
use crate::runtime::reads::BindKey;
use egui::{Context, Id, LayerId};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A registration of the recorded run, made again by calling it.
type Entry = Arc<dyn Fn(&Context) + Send + Sync>;

/// Records open across all contexts, so a registration made while none is open takes no lock.
static OPEN: AtomicUsize = AtomicUsize::new(0);

/// The journal of the running pass while a record is open, and the records open around the running code.
#[derive(Default)]
pub(crate) struct Recorder {
    journal: Vec<Entry>,
    open: Vec<Open>,
}

struct Open {
    from: usize,
    focus: focus::Mark,
    egui: egui::PassMark,
    facts: Facts,
}

impl Open {
    /// A record that starts at what the pass holds now.
    fn empty(ctx: &Context) -> Open {
        Open {
            from: 0,
            focus: focus::mark(ctx),
            egui: ctx.pass_mark(),
            facts: Facts { blocked: crate::runtime::access::scope::depth(ctx) > 0, ..Facts::default() },
        }
    }
}

/// What the settled test reads from a record.
#[derive(Clone, Default)]
pub(crate) struct Facts {
    /// The values the run read: bound values, inspection records and command states.
    pub(crate) reads: HashSet<BindKey>,
    /// Something in the run still moves: it runs again next pass.
    pub(crate) unsettled: bool,
    /// The earliest pass-clock time the run asked to be shown again.
    pub(crate) wake: Option<f64>,
    /// The run did something a replay cannot repeat.
    pub(crate) blocked: bool,
}

/// What one real run of a region added to its pass.
pub(crate) struct Record {
    pub(crate) focus: Transcript,
    journal: Vec<Entry>,
    egui: egui::PassRecords,
    /// The egui widgets of the run.
    pub(crate) widgets: HashSet<Id>,
    pub(crate) facts: Facts,
}

fn recording(ctx: &Context) -> bool {
    OPEN.load(Ordering::Acquire) > 0 && pass_state::with::<Recorder, _>(ctx, |recorder| !recorder.open.is_empty())
}

/// Opens a record around the real run of a region, which `close` ends. A run inside an open label scope cannot be
/// replayed: the scope's patches reach nodes the record would not hold.
pub(crate) fn open(ctx: &Context) {
    let open = Open::empty(ctx);
    pass_state::with::<Recorder, _>(ctx, |recorder| recorder.open.push(Open { from: recorder.journal.len(), ..open }));
    OPEN.fetch_add(1, Ordering::AcqRel);
}

/// Ends the innermost record, the one around a run on `layer`: paint or widgets on another layer block it.
pub(crate) fn close(ctx: &Context, layer: LayerId) -> Record {
    OPEN.fetch_sub(1, Ordering::AcqRel);
    let (open, journal) = pass_state::with::<Recorder, _>(ctx, |recorder| {
        // A close with no open record ends one that holds nothing and is never replayed.
        let open = recorder.open.pop().unwrap_or_else(|| {
            let mut open = Open::empty(ctx);
            open.from = recorder.journal.len();
            open.facts.blocked = true;
            open
        });
        let journal = recorder.journal.get(open.from..).unwrap_or_default().to_vec();
        if let Some(outer) = recorder.open.last_mut() {
            outer.facts.unsettled |= open.facts.unsettled;
            outer.facts.blocked |= open.facts.blocked;
            outer.facts.wake = min_wake(outer.facts.wake, open.facts.wake);
            outer.facts.reads.extend(open.facts.reads.iter().copied());
        } else {
            recorder.journal.clear();
        }
        (open, journal)
    });
    let egui = ctx.pass_records_since(&open.egui);
    let mut facts = open.facts;
    facts.blocked |= egui.layers().any(|on| on != layer);
    Record { focus: focus::transcript(ctx, open.focus), journal, widgets: egui.widgets().iter().map(|widget| widget.id).collect(), egui, facts }
}

fn min_wake(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Adds `record` to the running pass in place of the run that made it.
pub(crate) fn replay(ctx: &Context, record: &Record) {
    ctx.replay_pass_records(&record.egui);
    focus::replay(ctx, &record.focus);
    for entry in &record.journal {
        entry(ctx);
    }
    if let Some(at) = record.facts.wake {
        crate::runtime::egui::repaint::request_after(ctx, crate::runtime::egui::pass::current_viewport(ctx), at - frame::clock(ctx));
    }
    with_open(ctx, |facts| {
        facts.reads.extend(record.facts.reads.iter().copied());
        facts.wake = min_wake(facts.wake, record.facts.wake);
    });
}

fn with_open(ctx: &Context, f: impl FnOnce(&mut Facts)) {
    if OPEN.load(Ordering::Acquire) > 0 {
        pass_state::with::<Recorder, _>(ctx, |recorder| {
            if let Some(open) = recorder.open.last_mut() {
                f(&mut open.facts);
            }
        });
    }
}

/// Notes, in the records open around the running code, the registration `make` builds: called again, it registers
/// what the running code registered.
pub(crate) fn journal<F: Fn(&Context) + Send + Sync + 'static>(ctx: &Context, make: impl FnOnce() -> F) {
    if recording(ctx) {
        let entry: Entry = Arc::new(make());
        pass_state::with::<Recorder, _>(ctx, |recorder| recorder.journal.push(entry));
    }
}

/// Notes that the running code read the value `key`.
pub(crate) fn note_read(ctx: &Context, key: BindKey) {
    with_open(ctx, |facts| {
        facts.reads.insert(key);
    });
}

/// Notes that something in the running code still moves.
pub(crate) fn note_unsettled(ctx: &Context) {
    with_open(ctx, |facts| facts.unsettled = true);
}

/// Notes that the running code asks to be shown again at `at` on the pass clock.
pub(crate) fn note_wake(ctx: &Context, at: f64) {
    with_open(ctx, |facts| facts.wake = min_wake(facts.wake, Some(at)));
}

/// Notes that the running code did something a replay cannot repeat.
pub(crate) fn block(ctx: &Context) {
    with_open(ctx, |facts| facts.blocked = true);
}
