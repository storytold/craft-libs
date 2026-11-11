//! The read record: which components read which binding in the running pass, in which run kind, and what their
//! measure took. An edit reruns the pass only when this record shows an earlier reader its change would make stale.

use crate::runtime::egui::pass;
use crate::runtime::frame::{PerViewport, Stamped};
use crate::runtime::layout_memory::{self, Contribution};
use craft_behavior::Proposal;
use egui::{Context, Id, Vec2};
use std::any::TypeId;
use std::sync::Arc;

/// The identity of a binding's value across the components of one pass: the bound place, or where a kept value
/// lives. A given value has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BindKey {
    /// A write-through place: its address and the type stored there.
    Place(usize, TypeId),
    /// Session memory under this component id.
    Kept(Id),
    /// The saved tier under the key with this hash.
    Saved(Id),
}

/// One measure a reader made from the value it read: what it was offered and the size it took.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sample {
    pub proposal: Proposal,
    pub room: Option<f32>,
    pub took: Vec2,
}

#[derive(Debug)]
struct Read {
    key: BindKey,
    reader: Id,
    measuring: bool,
    /// The measuring runs the read happened in, outermost first: the measured components' ids.
    runs: Vec<Id>,
    samples: Vec<Sample>,
}

#[derive(Default)]
struct Reads {
    reads: Vec<Read>,
}

/// The measuring runs in progress, outermost first: each opened and closed by [`measuring`].
#[derive(Default)]
struct Runs(Vec<Id>);

fn with<R>(ctx: &Context, f: impl FnOnce(&mut Reads) -> R) -> R {
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    PerViewport::<Stamped<Reads>>::with_pass(ctx, viewport, pass_nr, |reads, _| *reads = Reads::default(), f)
}

fn with_runs<R>(ctx: &Context, f: impl FnOnce(&mut Vec<Id>) -> R) -> R {
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    PerViewport::<Stamped<Runs>>::with_pass(ctx, viewport, pass_nr, |runs, _| *runs = Runs::default(), |runs| f(&mut runs.0))
}

/// Runs `f` as the measuring run of the component `id`: the reads inside it are attributed to it.
pub(crate) fn measuring<R>(ctx: &Context, id: Id, f: impl FnOnce() -> R) -> R {
    let depth = with_runs(ctx, |runs| {
        runs.push(id);
        runs.len() - 1
    });
    let out = f();
    with_runs(ctx, |runs| runs.truncate(depth));
    out
}

/// Notes that `reader` read the binding `key`, in a measuring run when `measuring`, and the measures it made from it.
/// A read in a measuring run is handed forward through layout memory, so a pass that takes the body's kept
/// transcript instead of running it holds the read as well.
pub(crate) fn note(ctx: &Context, key: BindKey, reader: Id, measuring: bool, samples: &[Sample]) {
    let read = MeasuredRead { key, reader, runs: with_runs(ctx, |runs| runs.clone()), samples: samples.to_vec() };
    if measuring && crate::runtime::measure::active(ctx) {
        layout_memory::contribute(ctx, Arc::new(read));
    } else {
        let MeasuredRead { key, reader, runs, samples } = read;
        with(ctx, |reads| reads.reads.push(Read { key, reader, measuring, runs, samples }));
    }
}

/// A read a measuring run made, as its body's transcript keeps it.
struct MeasuredRead {
    key: BindKey,
    reader: Id,
    /// The measuring runs open at the read, outermost first.
    runs: Vec<Id>,
    samples: Vec<Sample>,
}

impl Contribution for MeasuredRead {
    fn apply(&self, ctx: &Context) {
        // Made again from a kept transcript, inside the measuring run of the body that keeps it: the runs open now,
        // then those the read was inside within that body.
        let runs: Vec<Id> = with_runs(ctx, |open| {
            let inner = open.last().and_then(|body| self.runs.iter().rposition(|id| id == body)).map_or(0, |at| at + 1);
            open.iter().chain(&self.runs[inner..]).copied().collect()
        });
        with(ctx, |reads| {
            reads.reads.push(Read { key: self.key, reader: self.reader, measuring: true, runs, samples: self.samples.clone() });
        });
    }

    fn report(&self) -> Option<(Id, f32)> {
        None
    }
}

/// The reads of a binding made so far in this pass, as an edit of it by one component sees them.
#[derive(Default)]
pub(crate) struct EarlierReads {
    /// Another component read it, in a real or a measuring run.
    pub other: bool,
    /// The editor read it in a measuring run.
    pub measured: bool,
    /// The measures the editor made from the value it read, each once.
    pub samples: Vec<Sample>,
}

/// The reads of `key` made so far in this pass, as `editor`'s edit of it sees them.
pub(crate) fn earlier(ctx: &Context, key: BindKey, editor: Id) -> EarlierReads {
    with(ctx, |reads| {
        let mut earlier = EarlierReads::default();
        for read in reads.reads.iter().filter(|read| read.key == key) {
            if read.reader != editor {
                earlier.other = true;
                continue;
            }
            earlier.measured |= read.measuring;
            for sample in &read.samples {
                if !earlier.samples.contains(sample) {
                    earlier.samples.push(*sample);
                }
            }
        }
        earlier
    })
}

/// The bindings read inside the measuring runs of `body` in this pass, nested measuring runs included.
#[expect(dead_code, reason = "the measure reuse of a body reads it")]
pub(crate) fn measured(ctx: &Context, body: Id) -> Vec<BindKey> {
    with(ctx, |reads| {
        let mut keys = Vec::new();
        for read in reads.reads.iter().filter(|read| read.measuring && read.runs.contains(&body)) {
            if !keys.contains(&read.key) {
                keys.push(read.key);
            }
        }
        keys
    })
}
