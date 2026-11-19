//! Run records: what a real run of a memoized view (`contract::MemoView`) added to its pass, so a later pass adds it
//! again in place of running the view. A measuring run of a measure-only child (a page `Pages` holds out of view) opens
//! a record too; only the values it read are kept, so a later run in the pass takes its size and notes them. A record
//! holds the view's paint, egui widget rects and AccessKit nodes (egui's pass records), what it registered with the
//! focus service (`focus::Transcript`), and a journal of every other registration the services below take per pass:
//! each service that keeps one notes it here while a record is open, as the call that makes it again. Records nest: a
//! view's record holds the records of the views inside it that replayed.
//!
//! The record also holds the facts the settled test reads (`runtime::lazy`): the values the run read, whether
//! something in it still moves (an animation that is not at its end, a repaint request) and the earliest time it asked
//! to be shown again, and whether it did something a replay cannot repeat (an overlay it shows, paint on another
//! layer, a label scope open around it). Such a record is never replayed: the view runs.

use crate::runtime::egui::areas::Layer;
use crate::runtime::focus::{self, Transcript};
use crate::runtime::frame;
use crate::runtime::inputs::{Input, Inputs};
use crate::runtime::pass_state;
use crate::runtime::reads::BindKey;
use crate::runtime::shift::Shift;
use egui::{Context, IdSet, RecordShift};
use std::any::Any;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A registration of the recorded run: the call that makes it again and what it is called with. Two entries are equal
/// when they come from the same call (the same `again` of [`journal`]) with equal arguments.
#[derive(Clone)]
pub(crate) struct Entry(Arc<dyn Noted>);

trait Noted: Send + Sync {
    fn call(&self, ctx: &Context, shift: &RecordShift);
    fn as_any(&self) -> &dyn Any;
    fn same(&self, other: &dyn Noted) -> bool;
    fn describe(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}

struct Note<A, F> {
    args: A,
    again: F,
}

impl<A, F> Noted for Note<A, F>
where
    A: PartialEq + fmt::Debug + Shift + Send + Sync + 'static,
    F: Fn(&Context, &A) + Send + Sync + 'static,
{
    fn call(&self, ctx: &Context, shift: &RecordShift) {
        if shift.is_none() {
            (self.again)(ctx, &self.args);
        } else {
            (self.again)(ctx, &self.args.shifted(shift));
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn same(&self, other: &dyn Noted) -> bool {
        // `F` is capture-free, so its type names the call: the same type is the same call.
        other.as_any().downcast_ref::<Self>().is_some_and(|other| other.args == self.args)
    }

    fn describe(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({:?})", std::any::type_name::<F>(), self.args)
    }
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.0.same(&*other.0)
    }
}

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.describe(f)
    }
}

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
    /// How many discard requests the pass's output held when the record opened.
    discards: usize,
    focus: focus::Mark,
    egui: egui::PassMark,
    facts: Facts,
}

impl Open {
    /// A record that starts at what the pass holds now.
    fn empty(ctx: &Context) -> Open {
        Open {
            from: 0,
            discards: discard_requests(ctx),
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
    /// The run asked for a discard pass: what it added is what the pass is thrown away for, so it is never replayed.
    pub(crate) discards: bool,
    /// The components whose memory owners registered in the run (`memory::register_owner`): a replay needs their
    /// shown-tier state still kept (`memory::ended_since`).
    pub(crate) owners: Vec<egui::Id>,
    /// The framework inputs the run read outside its props (`runtime::inputs`).
    pub(crate) inputs: Inputs,
    /// The command scope instances that registered in the run (`commands::register_instance`): a command queued for one
    /// is taken only by a run, so a replay waits until the queue holds none.
    pub(crate) instances: Vec<egui::Id>,
}

/// What one real run of a region added to its pass.
pub(crate) struct Record {
    pub(crate) focus: Transcript,
    pub(crate) journal: Vec<Entry>,
    egui: egui::PassRecords,
    /// The egui widgets of the run.
    pub(crate) widgets: IdSet,
    pub(crate) facts: Facts,
}

impl Record {
    /// Whether a replay can add the record at another place: egui's records hold no paint callback
    /// (`PassRecords::paints_callback`).
    pub(crate) fn movable(&self) -> bool {
        !self.egui.paints_callback()
    }
}

/// Whether a record is open around the running code.
pub(crate) fn recording(ctx: &Context) -> bool {
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
pub(crate) fn close(ctx: &Context, layer: Layer) -> Record {
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
            outer.facts.discards |= open.facts.discards;
            outer.facts.owners.extend(open.facts.owners.iter().copied());
            outer.facts.wake = min_wake(outer.facts.wake, open.facts.wake);
            outer.facts.reads.extend(open.facts.reads.iter().copied());
            outer.facts.inputs.extend(open.facts.inputs);
            outer.facts.instances.extend(open.facts.instances.iter().copied());
        } else {
            recorder.journal.clear();
        }
        (open, journal)
    });
    let egui = ctx.pass_records_since(&open.egui);
    let mut facts = open.facts;
    facts.blocked |= egui.layers().any(|on| on != layer);
    // Every discard request reaches egui's output (`sizing::request_discard` and egui's own); layout memory's waits
    // for the end of the pass and notes itself (`note_discard`).
    facts.discards |= discard_requests(ctx) > open.discards;
    Record { focus: focus::transcript(ctx, open.focus), journal, widgets: egui.widgets().iter().map(|widget| widget.id).collect(), egui, facts }
}

impl Facts {
    /// Whether the run these facts are of still holds at `now`, after the values `edited` changed in the frame: nothing
    /// in it moves, the time it asked to be shown again has not come, and it read none of them.
    pub(crate) fn quiet(&self, now: f64, edited: Option<&HashSet<BindKey>>) -> bool {
        !self.unsettled && self.wake.is_none_or(|at| at > now) && edited.is_none_or(|edited| self.reads.is_disjoint(edited))
    }
}

fn min_wake(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Adds `record` to the running pass in place of the run that made it, at the place `shift` names: egui's records and
/// every journal entry move by the one shift (`Shift`). The focus transcript holds no place: a stop's place is its
/// root widget's rect, which egui's records move.
pub(crate) fn replay(ctx: &Context, record: &Record, shift: &RecordShift) {
    ctx.replay_pass_records(&record.egui, shift);
    focus::replay(ctx, &record.focus);
    for entry in &record.journal {
        entry.0.call(ctx, shift);
    }
    if let Some(at) = record.facts.wake {
        crate::runtime::egui::repaint::request_after(ctx, crate::runtime::egui::pass::current_viewport(ctx), at - frame::clock(ctx));
    }
    with_open(ctx, |facts| {
        facts.reads.extend(record.facts.reads.iter().copied());
        facts.inputs.extend(record.facts.inputs);
        facts.wake = min_wake(facts.wake, record.facts.wake);
    });
}

fn with_open(ctx: &Context, f: impl FnOnce(&mut Facts)) {
    // Records are open only around runs of a pass on its own thread. A job's thread (a finished job's wake, a progress
    // report) runs no pass of `ctx`: it has no record to note into and must not take the pass state that the pass's
    // thread holds.
    if OPEN.load(Ordering::Acquire) > 0 && frame::running(ctx).is_some() {
        pass_state::with::<Recorder, _>(ctx, |recorder| {
            if let Some(open) = recorder.open.last_mut() {
                f(&mut open.facts);
            }
        });
    }
}

/// Notes, in the records open around the running code, the registration the running code made: `again(ctx, &args)`
/// registers it again. `args` builds the arguments only while a record is open. `again` captures nothing (a function
/// or a closure without captures; a capture does not compile), so everything the registration depends on is in the
/// arguments, and two notes of the same `again` with equal arguments are the same registration. A replay at another
/// place calls `again` with the arguments moved there (`Shift`). A service outside this crate that keeps registrations
/// per pass (a dock's drop spots) notes each one here, so a settled view's replay
/// makes it again.
pub fn journal<A, F>(ctx: &Context, args: impl FnOnce() -> A, again: F)
where
    A: PartialEq + fmt::Debug + Shift + Send + Sync + 'static,
    F: Fn(&Context, &A) + Send + Sync + 'static,
{
    const { assert!(std::mem::size_of::<F>() == 0, "`journal`'s `again` captures nothing: pass its state in `args`") };
    if recording(ctx) {
        let entry = Entry(Arc::new(Note { args: args(), again }));
        pass_state::with::<Recorder, _>(ctx, |recorder| recorder.journal.push(entry));
    }
}

/// Notes that the running code read the value `key`.
pub(crate) fn note_read(ctx: &Context, key: BindKey) {
    with_open(ctx, |facts| {
        facts.reads.insert(key);
    });
}

/// Notes that the running code read the framework input `input`.
pub(crate) fn note_input(ctx: &Context, input: Input) {
    with_open(ctx, |facts| facts.inputs.insert(input));
}

/// Notes that the command scope instance `id` registered in the running code.
pub(crate) fn note_instance(ctx: &Context, id: egui::Id) {
    with_open(ctx, |facts| facts.instances.push(id));
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

/// Notes that the running code asked for a discard pass that reaches egui only at the end of the pass (layout memory's
/// measured changes): every other request is counted in egui's output (`close`).
pub(crate) fn note_discard(ctx: &Context) {
    with_open(ctx, |facts| facts.discards = true);
}

/// Notes that the memory owner `owner` registered in the running code.
pub(crate) fn note_owner(ctx: &Context, owner: egui::Id) {
    with_open(ctx, |facts| facts.owners.push(owner));
}

/// How many discard requests the running pass's output holds.
fn discard_requests(ctx: &Context) -> usize {
    ctx.output(|output| output.request_discard_reasons.len())
}
