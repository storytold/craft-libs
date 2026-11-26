//! Run records: what a real run of a memoized view (`contract::MemoView`) added to its pass, so a later pass adds it
//! again in place of running the view. A measuring run of a measure-only child (a page `Pages` holds out of view) opens
//! a record too; only the values it read are kept, so a later run in the pass takes its size and notes them. A record
//! holds the view's paint, egui widget rects and AccessKit nodes (egui's pass records), what it registered with the
//! focus service (`focus::Transcript`), and a journal of every other registration the services below take per pass in
//! the real run (`journaling`):
//! each service that keeps one notes it here while a record is open, as the call that makes it again. Records nest: a
//! view's record holds the records of the views inside it that replayed.
//!
//! The record also holds the facts the settled test reads (`runtime::lazy`): the values the run read, whether
//! something in it still moves (an animation that is not at its end, a repaint request) and the earliest time it asked
//! to be shown again, the overlay layers it shows and where the focus was in it, and whether it did something a replay
//! cannot repeat (paint on a layer that is neither its own nor an overlay it shows, such as a raw egui popup or a
//! transformed body; a window of another viewport; a label scope open around it). Such a record is never replayed:
//! the view runs. An overlay it shows is journaled as every other registration is (its `Area`, its layer stack
//! entry), so its record replays only where it ran (`Record::pinned`).

use crate::runtime::egui::areas::Layer;
use crate::runtime::focus::{self, Transcript};
use crate::runtime::frame;
use crate::runtime::inputs::{Input, Inputs};
use crate::runtime::pass_state;
use crate::runtime::reads::BindKey;
use crate::runtime::shift::Shift;
use egui::{Context, IdSet, RecordShift, Rect};
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
    /// The entry a replay at the place `shift` names calls (`call`).
    fn shifted(&self, shift: &RecordShift) -> Entry;
    fn as_any(&self) -> &dyn Any;
    fn same(&self, other: &dyn Noted) -> bool;
    /// `same`, with each argument's coordinates and floats compared within `within` points (`Shift::snapped`).
    #[cfg(feature = "testing")]
    fn same_within(&self, other: &dyn Noted, within: f32) -> bool;
    fn describe(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}

struct Note<A, F> {
    args: A,
    again: F,
}

impl<A, F> Noted for Note<A, F>
where
    A: PartialEq + fmt::Debug + Shift + Send + Sync + 'static,
    F: Fn(&Context, &A) + Copy + Send + Sync + 'static,
{
    fn shifted(&self, shift: &RecordShift) -> Entry {
        Entry(Arc::new(Note { args: self.args.shifted(shift), again: self.again }))
    }

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

    #[cfg(feature = "testing")]
    fn same_within(&self, other: &dyn Noted, within: f32) -> bool {
        other.as_any().downcast_ref::<Self>().is_some_and(|other| self.args.snapped(&other.args, within) == other.args)
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

impl Entry {
    /// The entry at the place `shift` names. A shift that leaves every record as it is (`RecordShift::is_none`, a
    /// pinned record's) leaves the entry too, as it leaves egui's records: a clip a layer the record shows sets lies
    /// outside the clip around the record, which `RecordShift::clip` would cut it to.
    fn at(&self, shift: &RecordShift) -> Entry {
        if shift.is_none() { self.clone() } else { self.0.shifted(shift) }
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
    /// A replay is calling its journal's entries (`replay`): what they register is noted already.
    replaying: bool,
}

struct Open {
    from: usize,
    /// How many discard requests the pass's output held when the record opened.
    discards: usize,
    focus: focus::Mark,
    egui: egui::PassMark,
    facts: Facts,
    /// The record is kept for a later pass (a memoized view's), not used only within its pass (a measure-only child's):
    /// it keeps no read of a place by address, which names the place in this pass only.
    kept: bool,
    /// The record is around a run of the replay audit (`runtime::audit`).
    #[cfg(feature = "testing")]
    audit: bool,
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
            kept: true,
            #[cfg(feature = "testing")]
            audit: false,
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
    /// The components that take the AccessKit requests on the nodes the run registered (`note_node_owner`): a request
    /// routed to one of them is routed into the run, though the component is no egui widget (a collection's row).
    pub(crate) node_owners: Vec<egui::Id>,
    /// The layout memory entries the run kept (`note_layout_keep`): a replay keeps them (`layout_memory::keep`).
    pub(crate) layout_keeps: Vec<egui::Id>,
    /// The derived values the run asked for (`note_derived_keep`): a replay keeps them, since it asks for none.
    pub(crate) derived_keeps: Vec<egui::Id>,
    /// The framework inputs the run read outside its props (`runtime::inputs`).
    pub(crate) inputs: Inputs,
    /// The command scope instances that registered in the run (`commands::register_instance`): a command queued for one
    /// is taken only by a run, so a replay waits until the queue holds none.
    pub(crate) instances: Vec<egui::Id>,
    /// The overlay layers the run showed (`note_overlay`): their paint and widgets are the record's, and the pointer over
    /// one of them engages the record.
    pub(crate) overlays: Vec<ShownLayer>,
    /// The layer stack entries the run recorded (`note_layer`): a close queued for one is taken only by a run.
    pub(crate) layers: Vec<egui::Id>,
}

/// An overlay layer a run showed: its egui layer and its rect in screen space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ShownLayer {
    pub(crate) layer: Layer,
    pub(crate) rect: Rect,
}

/// Where the focus was in a run: the component of the focused stop when the run registered it, and the egui widget
/// that held egui focus when the run made it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Held {
    pub(crate) stop: Option<egui::Id>,
    pub(crate) widget: Option<egui::Id>,
}

impl Held {
    /// Where the focus is now: the component of the focused stop and the egui widget that holds egui focus.
    pub(crate) fn focused(ctx: &Context) -> Held {
        Held { stop: focus::focused_stop(ctx).map(|key| key.0), widget: crate::runtime::egui::focus::focused(ctx) }
    }

    /// The part of the focus `self` that is among the stops of `focus` and the egui widgets `widgets`.
    pub(crate) fn within(self, focus: &Transcript, widgets: &IdSet) -> Held {
        Held { stop: self.stop.filter(|stop| focus.stop_ids().any(|id| id == *stop)), widget: self.widget.filter(|widget| widgets.contains(widget)) }
    }

    /// Whether the focus is in the run at all.
    pub(crate) fn any(self) -> bool {
        self.stop.is_some() || self.widget.is_some()
    }
}

/// What one real run of a region added to its pass.
pub(crate) struct Record {
    pub(crate) focus: Transcript,
    pub(crate) journal: Vec<Entry>,
    egui: egui::PassRecords,
    /// The egui widgets of the run.
    pub(crate) widgets: IdSet,
    /// Where the focus was in the run when it ended.
    pub(crate) held: Held,
    pub(crate) facts: Facts,
}

impl Record {
    /// Whether a replay can add the record at another place: egui's records hold no paint callback
    /// (`PassRecords::paints_callback`) and the record is not pinned.
    pub(crate) fn movable(&self) -> bool {
        !self.egui.paints_callback() && !self.pinned()
    }

    /// Whether the record replays only where it ran, at the offset and clip of its run: it shows an overlay, which a
    /// host places in screen space by its anchor, so a move of the record is no move of the layer.
    pub(crate) fn pinned(&self) -> bool {
        !self.facts.overlays.is_empty()
    }
}

/// Whether a record is open around the running code.
pub(crate) fn recording(ctx: &Context) -> bool {
    OPEN.load(Ordering::Acquire) > 0 && pass_state::with::<Recorder, _>(ctx, |recorder| !recorder.open.is_empty())
}

/// Opens a record around the real run of a region, which `close` ends. A run inside an open label scope cannot be
/// replayed: the scope's patches reach nodes the record would not hold.
pub(crate) fn open(ctx: &Context) {
    push(ctx, Open::empty(ctx));
}

/// Opens a record used only within its pass, around a measuring run of a measure-only child, which `close` ends: it
/// keeps the places the run read by address too.
pub(crate) fn open_in_pass(ctx: &Context) {
    push(ctx, Open { kept: false, ..Open::empty(ctx) });
}

fn push(ctx: &Context, open: Open) {
    pass_state::with::<Recorder, _>(ctx, |recorder| recorder.open.push(Open { from: recorder.journal.len(), ..open }));
    OPEN.fetch_add(1, Ordering::AcqRel);
}

/// Opens a record around a run of the replay audit, which `close` ends as any other.
#[cfg(feature = "testing")]
pub(crate) fn open_audit(ctx: &Context) {
    push(ctx, Open { audit: true, ..Open::empty(ctx) });
}

/// Whether the running code is inside a run of the replay audit.
#[cfg(feature = "testing")]
pub(crate) fn in_audit_run(ctx: &Context) -> bool {
    OPEN.load(Ordering::Acquire) > 0 && pass_state::with::<Recorder, _>(ctx, |recorder| recorder.open.iter().any(|open| open.audit))
}

/// Ends the innermost record, the one around a run on `layer`: paint or widgets on a layer that is neither `layer` nor
/// an overlay the run showed block it.
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
            outer.facts.node_owners.extend(open.facts.node_owners.iter().copied());
            outer.facts.layout_keeps.extend(open.facts.layout_keeps.iter().copied());
            outer.facts.derived_keeps.extend(open.facts.derived_keeps.iter().copied());
            outer.facts.wake = min_wake(outer.facts.wake, open.facts.wake);
            let kept = outer.kept;
            outer.facts.reads.extend(open.facts.reads.iter().copied().filter(|key| !kept || !matches!(key, BindKey::Place(..))));
            outer.facts.inputs.extend(open.facts.inputs);
            outer.facts.instances.extend(open.facts.instances.iter().copied());
            outer.facts.overlays.extend(open.facts.overlays.iter().copied());
            outer.facts.layers.extend(open.facts.layers.iter().copied());
        } else {
            recorder.journal.clear();
        }
        (open, journal)
    });
    let egui = ctx.pass_records_since(&open.egui);
    let mut facts = open.facts;
    // Every measure of an entry keeps it again: the record keeps each one once.
    facts.layout_keeps.sort_unstable_by_key(|id| id.value());
    facts.layout_keeps.dedup();
    facts.derived_keeps.sort_unstable_by_key(|id| id.value());
    facts.derived_keeps.dedup();
    // Paint on a layer that is neither the run's own nor an overlay it showed (a raw egui popup, a transformed body)
    // comes from code that registers nothing to replay it with. A paint callback's output depends on what its closure
    // holds, which neither a view's key nor a comparison sees. Either run is never replayed.
    facts.blocked |= egui.layers().any(|on| on != layer && !facts.overlays.iter().any(|shown| shown.layer == on)) || egui.paints_callback();
    // Every discard request reaches egui's output (`sizing::request_discard` and egui's own); layout memory's waits
    // for the end of the pass and notes itself (`note_discard`).
    facts.discards |= discard_requests(ctx) > open.discards;
    let focus = focus::transcript(ctx, open.focus);
    let widgets = egui.widgets().iter().map(|widget| widget.id).collect();
    // A run that ends holding focus reads where the focus is and what is sent to it: a focus move, and the next key,
    // text, IME, clipboard or AccessKit event wherever the framework routes it, run it.
    let held = Held::focused(ctx).within(&focus, &widgets);
    if held.any() {
        facts.inputs.insert(Input::Focus);
        facts.inputs.insert(Input::Sent);
    }
    Record { focus, journal, widgets, held, egui, facts }
}

#[cfg(feature = "testing")]
impl Record {
    /// Whether the run painted a paint callback, which keeps it from being replayed.
    pub(crate) fn paints_callback(&self) -> bool {
        self.egui.paints_callback()
    }

    /// How a replay of `self` at the place `shift` names differs from the real run `run` there: one line per kind that
    /// differs (paint, widget rects and AccessKit nodes; focus registrations; the journal; the values and framework
    /// inputs read), each with its first difference.
    pub(crate) fn differences(&self, run: &Record, shift: &RecordShift) -> Vec<String> {
        let within = crate::runtime::audit::WITHIN;
        let mut found = Vec::new();
        match self.egui.shifted(shift).first_difference(&run.egui, within) {
            // The display names a widget by its rect only; the difference may be in its sense, state or interact rect.
            Some(egui::PassRecordsDiff::Widget { index, left, right }) => found.push(format!("widget rect {index}: left {left:?} / right {right:?}")),
            // The display names a node by its bounds only; the difference may be in any other property.
            // The display names a shape by its rects only; the difference may be in its colour, stroke, texture or text.
            Some(egui::PassRecordsDiff::Shape { layer, index, left, right }) => {
                found.push(format!("paint: shape {index} on {layer:?}: left {left:?} / right {right:?}"))
            }
            Some(egui::PassRecordsDiff::Node { index, left, right }) => {
                found.push(format!("AccessKit node {index}: left {left:?} / right {right:?}"))
            }
            Some(diff) => found.push(format!("paint, widgets or AccessKit: {diff}")),
            None => {}
        }
        if let Some(diff) = self.focus.first_difference(&run.focus) {
            found.push(format!("focus: {diff}"));
        }
        let shifted: Vec<Entry> = self.journal.iter().map(|entry| entry.at(shift)).collect();
        let (left, right) = (&shifted, &run.journal);
        if let Some(index) = left
            .iter()
            .zip(right)
            .position(|(a, b)| !a.0.same_within(&*b.0, within))
            .or_else(|| (left.len() != right.len()).then(|| left.len().min(right.len())))
        {
            found.push(format!("journal entry {index}: left {:?} / right {:?}", left.get(index), right.get(index)));
        }
        // A replay notes the kept run's reads and inputs in place of the run's: one the kept run missed is one a later
        // edit doesn't rerun the view for.
        if self.facts.reads != run.facts.reads {
            let only = |a: &HashSet<BindKey>, b: &HashSet<BindKey>| {
                let mut only: Vec<String> = a.difference(b).map(|key| format!("{key:?}")).collect();
                only.sort_unstable();
                only
            };
            found.push(format!(
                "reads: left only {:?} / right only {:?}",
                only(&self.facts.reads, &run.facts.reads),
                only(&run.facts.reads, &self.facts.reads)
            ));
        }
        if self.facts.inputs != run.facts.inputs {
            let list = |inputs: Inputs| Input::ALL.into_iter().filter(|input| inputs.contains(*input)).collect::<Vec<_>>();
            found.push(format!("inputs: left {:?} / right {:?}", list(self.facts.inputs), list(run.facts.inputs)));
        }
        found
    }
}

impl Facts {
    /// Whether the run these facts are of still holds at `now`, after the values `edited` changed in the frame: nothing
    /// in it moves, the time it asked to be shown again has not come, and it read none of them, nor a value overlapping one.
    pub(crate) fn quiet(&self, now: f64, edited: Option<&HashSet<BindKey>>) -> bool {
        !self.unsettled && self.wake.is_none_or(|at| at > now) && edited.is_none_or(|edited| !edited.iter().any(|edit| self.read(*edit)))
    }

    /// Whether the run read a value the edit of `edit` changes (`BindKey::reaches`).
    fn read(&self, edit: BindKey) -> bool {
        match edit {
            BindKey::Tracked(..) => self.reads.iter().any(|read| edit.reaches(*read)),
            _ => self.reads.contains(&edit),
        }
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
    // The records open around the replay hold its journal at the place it replays at, as a run there would have noted
    // it: an entry's call notes nothing, whether it is a service's own registration call or not.
    let noting = recording(ctx);
    if noting {
        pass_state::with::<Recorder, _>(ctx, |recorder| {
            recorder.journal.extend(record.journal.iter().map(|entry| entry.at(shift)));
            recorder.replaying = true;
        });
    }
    for entry in &record.journal {
        entry.0.call(ctx, shift);
    }
    if noting {
        pass_state::with::<Recorder, _>(ctx, |recorder| recorder.replaying = false);
    }
    crate::runtime::layout_memory::keep(ctx, &record.facts.layout_keeps);
    crate::runtime::derived::keep(ctx, &record.facts.derived_keeps);
    if let Some(at) = record.facts.wake {
        crate::runtime::egui::repaint::request_after(ctx, crate::runtime::egui::pass::current_viewport(ctx), at - frame::clock(ctx));
    }
    with_open(ctx, |facts| {
        facts.layout_keeps.extend(record.facts.layout_keeps.iter().copied());
        facts.derived_keeps.extend(record.facts.derived_keeps.iter().copied());
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
/// or a closure without captures), so everything the registration depends on is in the arguments. A capture fails
/// the build, not `cargo check`: the size assert is a constant that is evaluated only when the code is generated.
/// Two notes of the same `again` with equal arguments are the same registration. A replay at another place calls
/// `again` with the arguments moved there (`Shift`). A service outside this crate that keeps registrations per pass
/// (a dock's drop spots) notes each one here, so a settled view's replay makes it again.
pub fn journal<A, F>(ctx: &Context, args: impl FnOnce() -> A, again: F)
where
    A: PartialEq + fmt::Debug + Shift + Send + Sync + 'static,
    F: Fn(&Context, &A) + Copy + Send + Sync + 'static,
{
    const { assert!(std::mem::size_of::<F>() == 0, "`journal`'s `again` captures nothing: pass its state in `args`") };
    if journaling(ctx) {
        let entry = Entry(Arc::new(Note { args: args(), again }));
        pass_state::with::<Recorder, _>(ctx, |recorder| recorder.journal.push(entry));
    }
}

/// Whether a registration the running code makes goes into the open records' journal: a record is open, no replay is
/// adding its journal, and no measuring run is in progress. A journal holds what the real run registered: a measuring
/// run inside it (a body measuring its content) runs only in a pass that measures, and what it hands forward lives in
/// that body's kept transcript, so a replay in a pass that measures nothing adds only what the real run added.
fn journaling(ctx: &Context) -> bool {
    recording(ctx) && !crate::runtime::measure::active(ctx) && !pass_state::with::<Recorder, _>(ctx, |recorder| recorder.replaying)
}

/// `journal` for a registration that a second equal one adds nothing to: the innermost open record keeps it once, so
/// its journal does not depend on how many times its run made it.
pub(crate) fn journal_distinct<A, F>(ctx: &Context, args: impl FnOnce() -> A, again: F)
where
    A: PartialEq + fmt::Debug + Shift + Send + Sync + 'static,
    F: Fn(&Context, &A) + Copy + Send + Sync + 'static,
{
    const { assert!(std::mem::size_of::<F>() == 0, "`journal_distinct`'s `again` captures nothing: pass its state in `args`") };
    if journaling(ctx) {
        let entry = Entry(Arc::new(Note { args: args(), again }));
        pass_state::with::<Recorder, _>(ctx, |recorder| {
            let from = recorder.open.last().map_or(0, |open| open.from);
            if !recorder.journal[from..].contains(&entry) {
                recorder.journal.push(entry);
            }
        });
    }
}

/// Notes that the running code read the value `key`.
pub(crate) fn note_read(ctx: &Context, key: BindKey) {
    if OPEN.load(Ordering::Acquire) > 0 && frame::running(ctx).is_some() {
        pass_state::with::<Recorder, _>(ctx, |recorder| {
            if let Some(open) = recorder.open.last_mut()
                && (!open.kept || !matches!(key, BindKey::Place(..)))
            {
                open.facts.reads.insert(key);
            }
        });
    }
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

/// Notes that the running code showed the overlay layer `layer` at the screen rect `rect`. Called by the journaled
/// registration of the layer's `Area`, so a record that replays it notes it again in the records open around it.
pub(crate) fn note_overlay(ctx: &Context, layer: Layer, rect: Rect) {
    with_open(ctx, |facts| facts.overlays.push(ShownLayer { layer, rect }));
}

/// Notes that the running code recorded the layer stack entry `id`, by a run or a replay.
pub(crate) fn note_layer(ctx: &Context, id: egui::Id) {
    with_open(ctx, |facts| facts.layers.push(id));
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

/// Notes that the running code kept the layout memory entry and kept transcript of `id`: a replay of the records open
/// around it keeps them too, as the replay reaches neither.
pub(crate) fn note_layout_keep(ctx: &Context, id: egui::Id) {
    with_open(ctx, |facts| facts.layout_keeps.push(id));
}

/// Notes that the running code asked for the derived value kept under `slot`: a replay of the records open around it
/// keeps it too, as the replay asks for nothing and an entry no pass asked for in a frame is dropped.
pub(crate) fn note_derived_keep(ctx: &Context, slot: egui::Id) {
    with_open(ctx, |facts| facts.derived_keeps.push(slot));
}

/// Notes that `owner` takes the AccessKit requests on a node the running code registered (`keys::register_node`). A
/// replay registers the node again from its journal, which notes it in the records open around the replay.
pub(crate) fn note_node_owner(ctx: &Context, owner: egui::Id) {
    with_open(ctx, |facts| facts.node_owners.push(owner));
}

/// Notes that the memory owner `owner` registered in the running code.
pub(crate) fn note_owner(ctx: &Context, owner: egui::Id) {
    with_open(ctx, |facts| facts.owners.push(owner));
}

/// How many discard requests the running pass's output holds.
fn discard_requests(ctx: &Context) -> usize {
    ctx.output(|output| output.request_discard_reasons.len())
}
