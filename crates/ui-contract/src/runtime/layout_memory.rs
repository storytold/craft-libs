//! Layout memory: sizes one pass measures and the next pass places by, the one discard pass a change asks for, what
//! this pass's measuring runs found, and the measurements bodies keep across passes.
//!
//! A body's measuring run leaves a transcript: every value it hands forward (a record by id, a contribution to an
//! aggregator open around it, the transcripts of the bodies nested in it, in order) and every measure call each
//! component in it got. A later pass whose inputs are the body's key takes the transcript instead of running the
//! content: its records serve this pass and its contributions are made again. The real run then asks each component
//! the calls the transcript holds, at the measuring run's own proposal and room, and compares; a component added,
//! removed or answering differently drops the kept transcripts of the bodies the real run is inside (up to a boundary)
//! and asks for the frame's discard pass, so a change shows in its frame.

use crate::runtime::egui::{pass, sizing};
use crate::runtime::frame::{self, PerViewport};
use craft_behavior::layout_memory::{SizeTween, measure_tolerance, measured_change_asks_for_discard};
use craft_behavior::{Axis, Proposal};
use egui::{Context, Id, Vec2};
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// What an entry holds.
#[derive(Clone, Copy, Debug)]
enum Value {
    Size(Vec2),
    #[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "size tweens are used by the body step"))]
    Tween(SizeTween),
}

impl Value {
    fn at(self, now: f64) -> Vec2 {
        match self {
            Value::Size(size) => size,
            Value::Tween(tween) => tween.at(now),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Entry {
    value: Option<Value>,
    /// The pass that read the entry for a placement, and what it read.
    used: Option<(u64, Option<Vec2>)>,
    /// The frame the size last changed in.
    changed: Option<u64>,
    /// The last frame that read or wrote the entry.
    touched: u64,
}

/// What a measuring run found for a container (under its id) or one of its regions (under `sub(id, "header")`,
/// `"footer"`, `"end"` and `"controls"`): the room the container was offered and what the region took. Valid only in the egui pass
/// that recorded it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Measured {
    pub room: Vec2,
    pub took: Vec2,
    /// The other inputs the measure read, as a fingerprint: the shared column widths its rows were given (0 for none).
    /// [`emit_record`] sets it.
    pub inputs: u64,
}

impl Measured {
    pub(crate) fn new(room: Vec2, took: Vec2) -> Measured {
        Measured { room, took, inputs: 0 }
    }

    /// Whether this record serves a run offered `now` along one axis, where it was offered `was` and what it measures
    /// took `took`: the rooms are equal (within `tolerance`), or the axis is not `given`, the old room did not bound
    /// what it took and it fits the room now. The one reuse rule for every record of this pass.
    pub(crate) fn serves(now: f32, was: f32, given: bool, took: f32, tolerance: f32) -> bool {
        now == was || (now - was).abs() <= tolerance || (!given && took < was - tolerance && took <= now + tolerance)
    }
}

/// A value a measuring run hands to an aggregator open around it (a column group, a label scope). `apply` finds its
/// target among the aggregators open now and does nothing when that one is not open: an aggregator inside the body
/// closed inside the run, and the body's transcript holds its result as a record.
pub(crate) trait Contribution: Send + Sync {
    fn apply(&self, ctx: &Context);

    /// What the contribution reports: its target and kind by id, and its value. A real run's contributions are
    /// compared with the transcript's by these. `None` for a report only a measuring run makes.
    fn report(&self) -> Option<(Id, f32)>;
}

/// One thing a measuring run handed forward, in the order it did.
enum Item {
    Record(Id, Measured),
    Contribute(Arc<dyn Contribution>),
    Body(Id, Arc<Transcript>),
}

/// The inputs outside a measure that it read (`read_input`), by key: the widest value of each.
pub(crate) type Inputs = HashMap<Id, f32>;

/// One measure call a component got in a measuring run, the inputs besides its room it read, and its answer.
#[derive(Clone)]
struct Call {
    proposal: Proposal,
    room: Option<f32>,
    inputs: Inputs,
    took: Vec2,
}

/// What the components of a body's content got from its measuring run: each measure call, and whether it filled the
/// body's main axis (`Some(axis)`).
#[derive(Clone, Default)]
struct Leaf {
    fill: Option<Axis>,
    calls: Vec<Call>,
}

/// What a body's measuring run found.
struct Transcript {
    /// The inputs the run measured under.
    key: Box<dyn Any + Send + Sync>,
    items: Vec<Item>,
    leaves: HashMap<Id, Leaf>,
    /// What the real run of the pass that measured found for the body's regions and its rows that run lazily: the
    /// backstop for content that allocates without a component.
    real: Mutex<Option<HashMap<Id, Vec2>>>,
}

/// A record of this pass's measures.
#[derive(Clone, Copy)]
struct Found {
    /// The cumulative pass number that recorded it.
    pass: u64,
    measured: Measured,
    /// The body whose kept transcript gave it, rather than a run of this pass.
    kept: Option<Id>,
}

/// What a body's last measuring run found, kept for later passes.
struct Kept {
    transcript: Arc<Transcript>,
    /// Other inputs the transcript's records served in the pass that measured (`kept_also_under`).
    aliases: Vec<Box<dyn Any + Send + Sync>>,
    /// False once a real run found the content changed.
    valid: bool,
    /// The last frame a real run found the content changed under the same inputs: content that changes measures
    /// again in that frame and the next.
    moved: Option<u64>,
    touched: u64,
}

impl Scope {
    fn boundary() -> Scope {
        Scope { body: None, boundary: true, served: None, fresh: None, visited: HashSet::new(), fills: HashSet::new(), reported: Vec::new() }
    }
}

/// What a measuring run has open: a body's transcript, content whose emits no enclosing transcript takes, or a
/// component's measure call.
enum Open {
    /// `inputs`: what the body's content read outside the body.
    Body {
        id: Id,
        items: Vec<Item>,
        leaves: HashMap<Id, Leaf>,
        inputs: Inputs,
    },
    Boundary,
    /// `inputs`: what the measure read outside its own room.
    Call {
        inputs: Inputs,
    },
}

/// A content scope of the real run.
struct Scope {
    body: Option<Id>,
    /// Whether a change found inside stops here: a collection's rows, a layer's content.
    boundary: bool,
    /// The transcript the body's records came from in this pass (checked against), or the one its measuring run of
    /// this pass left (whose `real` this run fills).
    served: Option<Arc<Transcript>>,
    fresh: Option<Arc<Transcript>>,
    visited: HashSet<Id>,
    fills: HashSet<Id>,
    /// What the real run contributed in this scope.
    reported: Vec<(Id, f32)>,
}

#[derive(Default)]
struct Entries {
    entries: HashMap<Id, Entry>,
    measured: HashMap<Id, Found>,
    kept: HashMap<Id, Kept>,
    /// The bodies whose records a kept transcript gave in this pass, with that transcript.
    served: HashMap<Id, Arc<Transcript>>,
    measuring: Vec<Open>,
    scopes: Vec<Scope>,
    /// The inputs of the kept calls a real run asks again, innermost last: `read_input` gives what they read.
    replaying: Vec<Inputs>,
    ask: bool,
    asked: Option<u64>,
}

impl Entries {
    /// A real run found the content changed: drops the kept transcripts of `body` and of every body the real run is
    /// inside, up to the first boundary, and asks for the discard pass.
    fn changed(&mut self, frame: u64, body: Option<Id>) {
        let invalidate = |kept: &mut HashMap<Id, Kept>, body: Id| {
            if let Some(kept) = kept.get_mut(&body) {
                kept.valid = false;
                kept.moved = Some(frame);
            }
        };
        if let Some(body) = body {
            invalidate(&mut self.kept, body);
        }
        for scope in self.scopes.iter().rev() {
            if let Some(body) = scope.body {
                invalidate(&mut self.kept, body);
            }
            if scope.boundary {
                break;
            }
        }
        self.ask = true;
    }

    /// The innermost open transcript: a body's or a boundary, past the measure calls open inside it.
    fn transcript(&mut self) -> Option<&mut Open> {
        self.measuring.iter_mut().rev().find(|open| !matches!(open, Open::Call { .. }))
    }

    fn push(&mut self, item: Item) {
        if let Some(Open::Body { items, .. }) = self.transcript() {
            items.push(item);
        }
    }

    fn leaf(&mut self) -> Option<&mut HashMap<Id, Leaf>> {
        match self.transcript() {
            Some(Open::Body { leaves, .. }) => Some(leaves),
            _ => None,
        }
    }

    /// The innermost real-run scope when it checks a kept transcript.
    fn checking(&mut self) -> Option<&mut Scope> {
        self.scopes.last_mut().filter(|scope| scope.served.is_some())
    }
}

/// The pass, frame and clock of the running viewport, read before the service lock is taken.
struct Moment {
    viewport: egui::ViewportId,
    pass: u64,
    frame: u64,
    now: f64,
}

impl Moment {
    fn of(ctx: &Context) -> Moment {
        let viewport = pass::current_viewport(ctx);
        Moment { viewport, pass: pass::pass_nr(ctx, viewport), frame: pass::frame_nr(ctx, viewport), now: frame::clock(ctx) }
    }
}

fn with<R>(ctx: &Context, moment: &Moment, f: impl FnOnce(&mut Entries) -> R) -> R {
    PerViewport::<Entries>::with(ctx, moment.viewport, f)
}

/// The size entry under `id` as this pass places by it. A placement that read the entry (`None` on a block's
/// first appearance) compares it with the real size at `record`.
pub(crate) fn read(ctx: &Context, id: Id) -> Option<Vec2> {
    if crate::runtime::measure::active(ctx) {
        return peeking(ctx, |read| read(id));
    }
    let moment = Moment::of(ctx);
    with(ctx, &moment, |memory| {
        let entry = memory.entries.entry(id).or_default();
        entry.touched = moment.frame;
        let value = entry.value.map(|value| value.at(moment.now));
        if entry.used.is_none_or(|(pass, _)| pass != moment.pass) {
            entry.used = Some((moment.pass, value));
        }
        value
    })
}

/// The size entry under `id` without a placement using it.
#[cfg(feature = "testing")]
pub(crate) fn peek(ctx: &Context, id: Id) -> Option<Vec2> {
    peeking(ctx, |read| read(id))
}

/// Runs `f` with a reader of the size entries that takes no placement and fixes the pass and clock once, for a
/// caller that reads many entries.
pub(crate) fn peeking<R>(ctx: &Context, f: impl FnOnce(&dyn Fn(Id) -> Option<Vec2>) -> R) -> R {
    let moment = Moment::of(ctx);
    with(ctx, &moment, |memory| f(&|id| memory.entries.get(&id).and_then(|entry| entry.value).map(|value| value.at(moment.now))))
}

/// Records the size a block measured this pass. A change asks for a discard pass only under the tolerance
/// rule: this pass placed with the old value, and the size did not also change in the previous frame.
pub(crate) fn record(ctx: &Context, id: Id, real: Vec2) {
    let moment = Moment::of(ctx);
    let tolerance = measure_tolerance(crate::runtime::egui::scale::pixels_per_point(ctx));
    with(ctx, &moment, |memory| {
        let entry = memory.entries.entry(id).or_default();
        entry.touched = moment.frame;
        let placed = entry.used.take().filter(|(pass, _)| *pass == moment.pass).map(|(_, value)| value);
        let changed_last_frame = entry.changed.is_some_and(|frame| frame + 1 == moment.frame);
        let previous = entry.value.map(|value| value.at(moment.now));
        if let Some(recorded) = placed
            && measured_change_asks_for_discard(recorded, real, changed_last_frame, tolerance)
        {
            memory.ask = true;
        }
        let entry = memory.entries.entry(id).or_default();
        let differs = previous.is_none_or(|old| (old - real).abs().max_elem() > tolerance);
        if differs {
            entry.changed = Some(moment.frame);
        }
        entry.value = Some(Value::Size(if differs { real } else { previous.unwrap_or(real) }));
    });
}

/// Whether this pass is a real run's: no measuring run is open and egui does not run a sizing pass.
fn real(ctx: &Context) -> bool {
    !crate::runtime::measure::active(ctx)
}

/// Hands forward what a measuring run found under `id`: it serves the rest of this pass, and joins the transcript of
/// the innermost body whose measuring run is open. A run of this pass that finds, at the same room, something other
/// than what a kept transcript gave the pass shows that transcript changed.
pub(crate) fn emit_record(ctx: &Context, id: Id, mut measured: Measured) {
    let moment = Moment::of(ctx);
    let pass = ctx.cumulative_pass_nr();
    let tolerance = measure_tolerance(crate::runtime::egui::scale::pixels_per_point(ctx));
    with(ctx, &moment, |memory| {
        // A record keys on what its measure read: the room, and the inputs the innermost open body read so far.
        measured.inputs = memory
            .measuring
            .iter()
            .rev()
            .find_map(|open| match open {
                Open::Body { inputs, .. } => Some(fingerprint(inputs)),
                Open::Boundary | Open::Call { .. } => None,
            })
            .unwrap_or(0);
        let last = memory.measured.insert(id, Found { pass, measured, kept: None });
        if let Some(Found { pass: at, measured: was, kept: Some(from) }) = last
            && at == pass
            && was.inputs == measured.inputs
            && (was.room - measured.room).abs().max_elem() <= tolerance
            && (was.took - measured.took).abs().max_elem() > tolerance
        {
            #[cfg(feature = "testing")]
            if std::env::var_os("CRAFT_UI_TRACE_KEPT").is_some() {
                eprintln!("layout memory: a run of this pass found {id:?} at {measured:?}, its kept record said {was:?}");
            }
            memory.changed(moment.frame, Some(from));
        }
        memory.push(Item::Record(id, measured));
    });
}

/// How many bodies and measure calls a measuring run has open: an aggregator opened now hands [`read_input`] this
/// depth, so only the bodies and calls opened inside it count what they read from it as an input.
pub(crate) fn depth(ctx: &Context) -> usize {
    let moment = Moment::of(ctx);
    with(ctx, &moment, |memory| memory.measuring.len())
}

/// Where the measure calls a measuring run has open on top of its innermost transcript start: the column the
/// innermost call lays out in is outside each of them, since each call measures in the `Ui` of the one around it.
pub(crate) fn calls_from(ctx: &Context) -> usize {
    let moment = Moment::of(ctx);
    with(ctx, &moment, |memory| memory.measuring.iter().rposition(|open| !matches!(open, Open::Call { .. })).map_or(0, |at| at + 1))
}

/// The bodies and measure calls open from `from` on read `value` from the input `key` outside them (a shared column's
/// width, the column a component sizes by): the records a body emits key on it, and a call keeps it. Returns the value
/// to measure with: while a real run asks a kept call again, the value that call read.
pub(crate) fn read_input(ctx: &Context, from: usize, key: Id, value: f32) -> f32 {
    let moment = Moment::of(ctx);
    with(ctx, &moment, |memory| {
        if let Some(read) = memory.replaying.iter().rev().find_map(|inputs| inputs.get(&key)) {
            return *read;
        }
        for open in memory.measuring.iter_mut().skip(from) {
            if let Open::Body { inputs, .. } | Open::Call { inputs } = open {
                let read = inputs.entry(key).or_insert(value);
                *read = read.max(value);
            }
        }
        value
    })
}

/// Runs a component's measure `f` as a measure call: in a measuring run, returns with its answer the inputs it read
/// outside its room, which the call's transcript entry keeps.
pub(crate) fn measure_call<R>(ctx: &Context, f: impl FnOnce() -> R) -> (R, Inputs) {
    if !crate::runtime::measure::active(ctx) {
        return (f(), Inputs::new());
    }
    let moment = Moment::of(ctx);
    with(ctx, &moment, |memory| memory.measuring.push(Open::Call { inputs: Inputs::new() }));
    let out = f();
    let inputs = with(ctx, &moment, |memory| {
        let at = memory.measuring.iter().rposition(|open| matches!(open, Open::Call { .. }))?;
        match memory.measuring.remove(at) {
            Open::Call { inputs } => Some(inputs),
            _ => None,
        }
    });
    (out, inputs.unwrap_or_default())
}

fn fingerprint(inputs: &Inputs) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    if inputs.is_empty() {
        return 0;
    }
    let mut sorted: Vec<(u64, u32)> = inputs.iter().map(|(key, value)| (key.value(), value.to_bits())).collect();
    sorted.sort_unstable();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    sorted.hash(&mut hasher);
    hasher.finish().max(1)
}

/// Makes `contribution` now. A measuring run joins it to the transcript of the innermost body whose run is open; a
/// real run inside a body a kept transcript serves keeps it to compare with the transcript's.
pub(crate) fn contribute(ctx: &Context, contribution: Arc<dyn Contribution>) {
    guard::emitting(|| contribution.apply(ctx));
    let moment = Moment::of(ctx);
    let measuring = crate::runtime::measure::active(ctx);
    with(ctx, &moment, |memory| {
        if measuring {
            memory.push(Item::Contribute(contribution));
        } else if let (Some(scope), Some(report)) = (memory.checking(), contribution.report()) {
            scope.reported.push(report);
        }
    });
}

/// Runs `f` as content whose emits no enclosing transcript takes, and whose real run no enclosing body checks: a
/// collection's rows and its widest-row probe (measured every pass by the collection), a layer's content, a lazy
/// child. `stops_changes`: a change the real run finds inside drops no kept transcript outside. Content whose extent
/// in the body around it comes from its measure (a lazy child) lets the change through.
pub(crate) fn boundary<R>(ctx: &Context, stops_changes: bool, f: impl FnOnce() -> R) -> R {
    let moment = Moment::of(ctx);
    let measuring = crate::runtime::measure::active(ctx);
    with(ctx, &moment, |memory| {
        if measuring {
            memory.measuring.push(Open::Boundary);
        } else {
            memory.scopes.push(Scope { boundary: stops_changes, ..Scope::boundary() });
        }
    });
    let out = f();
    with(ctx, &moment, |memory| {
        if measuring {
            if let Some(at) = memory.measuring.iter().rposition(|open| matches!(open, Open::Boundary)) {
                memory.measuring.remove(at);
            }
        } else {
            memory.scopes.pop();
        }
    });
    out
}

/// A measure call component `id` got, with the `inputs` it read (`measure_call`) and its answer `took`. A measuring
/// run adds it to the innermost body's transcript. A real run inside a body whose records a kept transcript gave asks
/// the component again (`ask`) at each call the transcript holds for it, under the inputs that call read, once per
/// pass, and compares; `ask` runs outside the memory's lock.
pub(crate) fn leaf_call(
    ctx: &Context,
    id: Id,
    proposal: Proposal,
    room: Option<f32>,
    inputs: Inputs,
    took: Vec2,
    ask: impl FnMut(Proposal, Option<f32>) -> Vec2,
) {
    let moment = Moment::of(ctx);
    if !real(ctx) {
        with(ctx, &moment, |memory| {
            if let Some(leaves) = memory.leaf() {
                let calls = &mut leaves.entry(id).or_default().calls;
                if !calls.iter().any(|call| call.proposal == proposal && call.room == room) {
                    calls.push(Call { proposal, room, inputs, took });
                }
            }
        });
        return;
    }
    check(ctx, &moment, id, ask);
}

fn check(ctx: &Context, moment: &Moment, id: Id, mut ask: impl FnMut(Proposal, Option<f32>) -> Vec2) {
    let calls = with(ctx, moment, |memory| {
        let scope = memory.checking()?;
        if !scope.visited.insert(id) {
            return None;
        }
        let transcript = scope.served.clone()?;
        Some(transcript.leaves.get(&id).map(|leaf| leaf.calls.clone()))
    });
    let Some(calls) = calls else { return };
    let tolerance = measure_tolerance(crate::runtime::egui::scale::pixels_per_point(ctx));
    // A component the transcript does not hold was added; one that answers differently under the inputs it read then
    // changed. An input that changed since (a shared column a later row widened in the measuring run's sweep) is not a
    // change of this component: the input's own records and reports are checked where they are made.
    let same = calls.is_some_and(|calls| {
        calls.iter().all(|call| {
            with(ctx, moment, |memory| memory.replaying.push(call.inputs.clone()));
            let now = ask(call.proposal, call.room);
            with(ctx, moment, |memory| memory.replaying.pop());
            (now - call.took).abs().max_elem() <= tolerance
        })
    });
    if !same {
        mismatch(ctx, moment, id);
    }
}

fn mismatch(ctx: &Context, moment: &Moment, what: Id) {
    #[cfg(feature = "testing")]
    if std::env::var_os("CRAFT_UI_TRACE_KEPT").is_some() {
        eprintln!("layout memory: a kept transcript changed at {what:?}\n{}", std::backtrace::Backtrace::force_capture());
    }
    #[cfg(not(feature = "testing"))]
    let _ = what;
    with(ctx, moment, |memory| memory.changed(moment.frame, None));
}

/// Component `id` fills the main axis `axis` of the body `body` it sits in. A measuring run adds it to `body`'s
/// transcript; a real run checks that the transcript says so.
pub(crate) fn leaf_fill(ctx: &Context, id: Id, body: Id, axis: Axis) {
    let moment = Moment::of(ctx);
    let measuring = !real(ctx);
    let same = with(ctx, &moment, |memory| {
        if measuring {
            if let Some(Open::Body { id: open, leaves, .. }) = memory.transcript()
                && *open == body
            {
                leaves.entry(id).or_default().fill = Some(axis);
            }
            return true;
        }
        let Some(scope) = memory.checking().filter(|scope| scope.body == Some(body)) else { return true };
        scope.fills.insert(id);
        scope.served.as_ref().is_some_and(|transcript| transcript.leaves.get(&id).is_some_and(|leaf| leaf.fill == Some(axis)))
    });
    if !same {
        mismatch(ctx, &moment, id);
    }
}

/// What a real run found under `id` for the innermost body that does not come from a component: a region's size, a
/// lazy row's extent. The pass that measured the body keeps it; a pass its kept transcript serves compares.
pub(crate) fn backstop(ctx: &Context, id: Id, took: Vec2) {
    if !real(ctx) {
        return;
    }
    let moment = Moment::of(ctx);
    let tolerance = measure_tolerance(crate::runtime::egui::scale::pixels_per_point(ctx));
    let same = with(ctx, &moment, |memory| {
        let Some(scope) = memory.scopes.last() else { return true };
        if let Some(fresh) = &scope.fresh {
            fresh.real.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_or_insert_default().insert(id, took);
            return true;
        }
        let Some(served) = &scope.served else { return true };
        let real = served.real.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        real.as_ref().is_none_or(|real| real.get(&id).is_some_and(|was| (*was - took).abs().max_elem() <= tolerance))
    });
    if !same {
        mismatch(ctx, &moment, id);
    }
}

/// What a measuring run of this pass found under `id`, or a kept transcript gave this pass; `None` when neither.
pub(crate) fn measured(ctx: &Context, id: Id) -> Option<Measured> {
    let moment = Moment::of(ctx);
    let pass = ctx.cumulative_pass_nr();
    with(ctx, &moment, |memory| memory.measured.get(&id).filter(|found| found.pass == pass).map(|found| found.measured))
}

/// Whether a run of this pass, not a kept transcript, recorded what serves under `id`.
pub(crate) fn run_found(ctx: &Context, id: Id) -> bool {
    let moment = Moment::of(ctx);
    let pass = ctx.cumulative_pass_nr();
    with(ctx, &moment, |memory| memory.measured.get(&id).is_some_and(|found| found.pass == pass && found.kept.is_none()))
}

/// Opens the measuring run of `body`: what it hands forward until `end_measure` becomes its transcript.
pub(crate) fn begin_measure(ctx: &Context, body: Id) {
    let moment = Moment::of(ctx);
    with(ctx, &moment, |memory| memory.measuring.push(Open::Body { id: body, items: Vec::new(), leaves: HashMap::new(), inputs: Inputs::new() }));
}

/// Closes the measuring run of `body` and keeps its transcript under the inputs `key` it ran under.
pub(crate) fn end_measure<K: PartialEq + Send + Sync + 'static>(ctx: &Context, body: Id, key: K) {
    let moment = Moment::of(ctx);
    with(ctx, &moment, |memory| {
        let Some(at) = memory.measuring.iter().rposition(|open| matches!(open, Open::Body { id, .. } if *id == body)) else { return };
        let Open::Body { items, leaves, .. } = memory.measuring.remove(at) else { return };
        let transcript = Arc::new(Transcript { key: Box::new(key), items, leaves, real: Mutex::new(None) });
        memory.served.remove(&body);
        let moved = memory.kept.remove(&body).and_then(|last| last.moved);
        memory.kept.insert(body, Kept { transcript: transcript.clone(), aliases: Vec::new(), valid: true, moved, touched: moment.frame });
        memory.push(Item::Body(body, transcript));
    });
}

/// The records `body`'s measuring run of this pass left also serve the inputs `key` (a real run offers its body other
/// inputs than an enclosing measuring run did, and the records still serve them): a later pass offering `key` takes the
/// transcript too.
pub(crate) fn kept_also_under<K: PartialEq + Send + Sync + 'static>(ctx: &Context, body: Id, key: K) {
    let moment = Moment::of(ctx);
    let pass = ctx.cumulative_pass_nr();
    with(ctx, &moment, |memory| {
        let fresh = memory.measured.get(&body).is_some_and(|found| found.pass == pass && found.kept.is_none());
        let Some(kept) = memory.kept.get_mut(&body).filter(|_| fresh) else { return };
        let known = kept.transcript.key.downcast_ref::<K>() == Some(&key) || kept.aliases.iter().any(|alias| alias.downcast_ref::<K>() == Some(&key));
        if !known && kept.aliases.len() < 2 {
            kept.aliases.push(Box::new(key));
        }
    });
}

/// Whether the running pass may take kept transcripts: it is not a rerun that shows something the measuring pass did
/// not see (an edit, a held update, input, an app's own reason).
fn reuse_allowed(ctx: &Context) -> bool {
    sizing::rerun_reasons(ctx).is_none_or(|reasons| !reasons.is_empty() && reasons.iter().all(|reason| *reason == sizing::Rerun::Layout))
}

/// Whether `body`, offered the inputs `key`, takes its kept transcript in this pass instead of measuring: the inputs
/// are the ones it measured under, no real run since found its content changed, the content did not change in this
/// frame or the last, and the pass may take kept transcripts. When it does, the transcript's records serve this pass,
/// its contributions are made again, and the bodies nested in it take theirs as well.
///
/// Only the outermost measuring run takes a kept transcript (a body's own measuring run, from its real run). Inside an
/// enclosing measuring run every body runs its content, so a show call runs its closure and returns its result.
pub(crate) fn kept_serves<K: PartialEq + Send + Sync + 'static>(ctx: &Context, body: Id, key: &K) -> bool {
    if !reuse_allowed(ctx) || !crate::runtime::measure::outermost(ctx) {
        return false;
    }
    let moment = Moment::of(ctx);
    let pass = ctx.cumulative_pass_nr();
    let replay = with(ctx, &moment, |memory| {
        // An enclosing body's transcript gave this body's records already, when the inputs are the ones it ran under
        // there; else its own kept transcript (from a run at the inputs of its own real run) may serve.
        let under = |kept: &Kept| {
            kept.transcript.key.downcast_ref::<K>() == Some(key) || kept.aliases.iter().any(|alias| alias.downcast_ref::<K>() == Some(key))
        };
        if let Some(transcript) = memory.served.get(&body).cloned()
            && (transcript.key.downcast_ref::<K>() == Some(key)
                || memory.kept.get(&body).is_some_and(|kept| Arc::ptr_eq(&kept.transcript, &transcript) && under(kept)))
        {
            memory.push(Item::Body(body, transcript));
            return Some(Vec::new());
        }
        let kept = memory.kept.get_mut(&body)?;
        let settled = kept.moved.is_none_or(|frame| frame + 1 < moment.frame);
        if !settled || !kept.valid || !under(kept) {
            return None;
        }
        kept.touched = moment.frame;
        let transcript = kept.transcript.clone();
        let mut contributions = Vec::new();
        let mut open = vec![(body, transcript.clone())];
        while let Some((id, transcript)) = open.pop() {
            memory.served.insert(id, transcript.clone());
            if let Some(kept) = memory.kept.get_mut(&id) {
                kept.touched = moment.frame;
            }
            // Nested transcripts are walked after the items before them, in order: first-wins aggregators see the
            // order a run makes.
            let mut nested = Vec::new();
            for item in &transcript.items {
                match item {
                    Item::Record(at, measured) => {
                        memory.measured.insert(*at, Found { pass, measured: *measured, kept: Some(body) });
                    }
                    Item::Contribute(contribution) => contributions.push(contribution.clone()),
                    Item::Body(at, inner) => nested.push((*at, inner.clone())),
                }
            }
            open.extend(nested.into_iter().rev());
        }
        memory.push(Item::Body(body, transcript));
        Some(contributions)
    });
    let Some(contributions) = replay else { return false };
    guard::emitting(|| {
        for contribution in contributions {
            contribution.apply(ctx);
        }
    });
    true
}

/// Opens a content scope of the real run: `body`'s content, or (`None`) content the enclosing body's measure does not
/// count component by component. `boundary`: a change found inside does not reach the bodies outside.
pub(crate) fn open_content(ctx: &Context, body: Option<Id>, boundary: bool) {
    let moment = Moment::of(ctx);
    let pass = ctx.cumulative_pass_nr();
    with(ctx, &moment, |memory| {
        let served = body.and_then(|body| memory.served.get(&body).cloned());
        let fresh = served.is_none().then(|| body.and_then(|body| memory.kept.get(&body))).flatten().and_then(|kept| {
            let fresh = memory.measured.get(&body?).is_some_and(|found| found.pass == pass && found.kept.is_none());
            fresh.then(|| kept.transcript.clone())
        });
        memory.scopes.push(Scope { body, boundary: boundary || body.is_none(), served, fresh, ..Scope::boundary() });
    });
}

/// Closes the innermost content scope, after `regions` (what the real run found for the body's regions) joined its
/// backstop. A component the kept transcript holds that the real run did not ask was removed.
pub(crate) fn close_content(ctx: &Context, regions: &[(Id, Vec2)]) {
    for (id, took) in regions {
        backstop(ctx, *id, *took);
    }
    let moment = Moment::of(ctx);
    let tolerance = measure_tolerance(crate::runtime::egui::scale::pixels_per_point(ctx));
    with(ctx, &moment, |memory| {
        let Some(mut scope) = memory.scopes.pop() else { return };
        let Some(served) = &scope.served else { return };
        let mut made: Vec<(Id, f32)> = served
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Contribute(contribution) => contribution.report(),
                _ => None,
            })
            .collect();
        let order = |a: &(Id, f32), b: &(Id, f32)| a.0.value().cmp(&b.0.value()).then(a.1.total_cmp(&b.1));
        // A measuring run may make one report twice (a label decided again after its control reported its minimum).
        let same = |a: &mut (Id, f32), b: &mut (Id, f32)| a.0 == b.0 && (a.1 - b.1).abs() <= tolerance;
        made.sort_by(order);
        made.dedup_by(same);
        scope.reported.sort_by(order);
        scope.reported.dedup_by(same);
        // What the real run reported to the aggregators around its rows is what the measuring run did.
        let reports_differ = made.len() != scope.reported.len()
            || made.iter().zip(&scope.reported).any(|(was, now)| was.0 != now.0 || (was.1 - now.1).abs() > tolerance);
        let removed = reports_differ
            || served
                .leaves
                .iter()
                .any(|(id, leaf)| (!leaf.calls.is_empty() && !scope.visited.contains(id)) || (leaf.fill.is_some() && !scope.fills.contains(id)));
        if removed {
            #[cfg(feature = "testing")]
            if std::env::var_os("CRAFT_UI_TRACE_KEPT").is_some() {
                eprintln!("layout memory: a component of {:?} did not run", scope.body);
            }
            memory.changed(moment.frame, scope.body);
        }
    });
}

/// Starts a size tween under `id` that readers evaluate at their clock. It asks for no discard pass: the
/// size changes by design while it runs.
#[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "size tweens are used by the body step"))]
pub(crate) fn tween(ctx: &Context, id: Id, from: Vec2, to: Vec2, duration: f64) {
    let moment = Moment::of(ctx);
    with(ctx, &moment, |memory| {
        let entry = memory.entries.entry(id).or_default();
        entry.touched = moment.frame;
        entry.value = Some(Value::Tween(SizeTween::new(from, to, moment.now, duration)));
    });
}

/// Whether a tween under `id` has not reached its end.
#[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "size tweens are used by the body step"))]
pub(crate) fn is_animating(ctx: &Context, id: Id) -> bool {
    let moment = Moment::of(ctx);
    with(ctx, &moment, |memory| {
        memory.entries.get(&id).is_some_and(|entry| matches!(entry.value, Some(Value::Tween(tween)) if !tween.is_done(moment.now)))
    })
}

/// Drops every kept measurement, so the next pass measures each body as a first pass would.
#[cfg(feature = "testing")]
pub fn forget_kept(ctx: &Context) {
    let moment = Moment::of(ctx);
    with(ctx, &moment, |memory| {
        memory.kept.clear();
        memory.served.clear();
    });
}

/// Asks the end hook for the frame's discard pass, as a change of measured size does.
pub(crate) fn ask_discard(ctx: &Context) {
    let moment = Moment::of(ctx);
    with(ctx, &moment, |memory| memory.ask = true);
}

/// The end hook: asks egui for at most one discard pass per frame, and for a repaint when a change cannot get one, so
/// the next frame shows it; then drops the entries no pass touched in the last frame.
pub(crate) fn end_hook(ctx: &Context) {
    let moment = Moment::of(ctx);
    let (wanted, ask) = with(ctx, &moment, |memory| {
        let wanted = std::mem::take(&mut memory.ask);
        let ask = wanted && memory.asked != Some(moment.frame);
        if ask {
            memory.asked = Some(moment.frame);
        }
        memory.entries.retain(|_, entry| entry.touched + 1 >= moment.frame);
        memory.kept.retain(|_, kept| kept.touched + 1 >= moment.frame);
        memory.measured.clear();
        memory.served.clear();
        memory.measuring.clear();
        memory.scopes.clear();
        memory.replaying.clear();
        (wanted, ask)
    });
    if ask {
        sizing::request_discard(ctx, sizing::Rerun::Layout);
    }
    if wanted && !(ask && sizing::discardable(ctx)) {
        ctx.request_repaint();
    }
}

/// The testing-build guard of measuring runs. A measuring run hands values forward only through `emit_record` and
/// `contribute`: in a frame a test fixture drives, a service it touches otherwise panics with the service's type, unless
/// the type is classified below.
/// Services give `&mut` for reads and writes alike, so the guard classifies by type, not by access. `Arc` handles kept
/// in component memory and raw `ctx.data_mut` writes are outside it.
pub(crate) mod guard {
    #[cfg(feature = "testing")]
    thread_local! {
        static MEASURING: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        static EMITTING: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        /// Whether each service type name touched so far is in `CLASSIFIED`, by the address and length of the name.
        static SEEN: std::cell::RefCell<std::collections::HashMap<(usize, usize), bool>> = std::cell::RefCell::default();
    }

    /// The services a measuring run may touch, with why that hands nothing forward around layout memory.
    #[cfg(feature = "testing")]
    const CLASSIFIED: &[(&str, &str)] = &[
        ("layout_memory::Entries", "layout memory itself: emits, transcripts and reads of this pass's records"),
        // Stacks a run opens and closes itself; what reaches a frame open around the run goes through `contribute`.
        ("layout::column_group::Open", "column groups: frames the run opens and closes; reports go through contribute"),
        ("access::scope::Stack", "label and merge scopes: opened and closed by the run; control minimums go through contribute"),
        ("layout::body::FillCounts", "fill counts: opened and closed by the body's own measuring sweep"),
        ("layout::body::Cursors", "grid and spread cursors: opened and closed by the body's own run"),
        ("layout::body::ShrinkReports", "shrink reports: opened and closed by the row's own first sweep; the widths go through emit_record"),
        ("runtime::reads::Runs", "measuring runs of the read record: opened and closed by each run; reads go through contribute"),
        ("contract::steps::id::IdPass", "id registration: a measuring run keeps the ids it uses out of the pass (keeping_ids)"),
        // Read in measuring runs; written by real runs, the frame hooks or the host.
        ("runtime::env::ViewportEnvs", "env per viewport: installs the run restores, and reads"),
        ("runtime::frame::FrameState", "frame state: read"),
        ("runtime::frame::ViewportRecord", "viewport record: read"),
        ("runtime::windows::Shared", "window shells: read"),
        ("runtime::windows::Record", "window records: read"),
        ("runtime::windows::hit::Exclusive", "exclusive windows: read"),
        ("runtime::focus::State", "focus: read; stops register in real runs only"),
        ("runtime::keys::Stage", "key stage: read"),
        ("primitives::sensing::Arenas", "arenas: read; members join in real runs only"),
        ("runtime::pointer::", "pointer state: read; sensing is skipped in measuring runs"),
        ("runtime::wheel::State", "wheel: read"),
        ("runtime::drag::", "drag board and targets: read; targets register in real runs"),
        ("runtime::overlay::", "overlays: read; opens and closes are effects skipped in measuring runs"),
        ("contract::overlay::popups::Popups", "popups: read"),
        (
            "runtime::lazy::Store",
            "lazy extents: the kept extent of content out of view, its own mechanism; a pass a kept transcript serves decides in its real run, which compares each row by its backstop",
        ),
        ("contract::steps::scroll_owner::", "scroll reveals and moves: read; taken through take_queued"),
        ("contract::body::scroll::Jumps", "scroll jumps: taken through take_queued"),
        ("runtime::edit::CancelAll", "edit cancels: read; a measuring run ends nothing"),
        ("runtime::commands::board::Board", "commands: read; registration is an effect skipped in measuring runs"),
        (
            "shell::app_shell::dialogs::Hosts",
            "dialog hosts: one handle per window, made on first use; dialogs open on clicks, which only real runs sense",
        ),
        ("runtime::memory::Registry", "component memory: a measuring run writes only to its peek stage"),
        ("runtime::egui::access::Reported", "access reports: read; reports are written by real runs"),
        ("runtime::access::live::Announced", "live announcements: read"),
        ("runtime::toast::Queue", "toasts: read"),
        ("runtime::cursor::State", "cursor: read"),
        ("runtime::motion::Wanted", "motion: repaint wishes, read only to ask for a repaint; the real run adds the same"),
        ("runtime::ime::Engine", "IME: read"),
        ("runtime::sampler::", "samplers: read"),
        ("runtime::egui::input_filter::", "input filter: read"),
        ("runtime::egui::injected::Injected", "injected input: read"),
        ("runtime::egui::fixture::WindowFocus", "fixture window focus: read"),
        ("contract::tooltip::Grace", "tooltip grace: read"),
        ("runtime::egui::widgets::noted::Noted", "noted widgets: read"),
        ("runtime::egui::painted::Elisions", "elisions: written where text paints, which a measuring run does not"),
        ("Stamped<std::collections::hash::set::HashSet<egui::id::Id>>", "per-pass id sets: read"),
        // Caches: equal inputs give equal values, whichever run fills them.
        ("style::cache::StyleCache", "style cache"),
        ("style::replacements::Frozen", "frozen style replacements: cache"),
        ("runtime::env::tokens::RootTokenCache", "root tokens: cache"),
        ("runtime::env::host_words::HostWords", "host words: cache"),
        ("runtime::env::host_words::host_format::System", "host number format: cache"),
        ("runtime::env::host_words::host_dates::System", "host dates: cache"),
        ("fonts::stack::install::State", "installed fonts: cache"),
        ("primitives::textures::Loaded", "textures: cache"),
        (
            "primitives::textures::store::Stores",
            "texture stores: cache; a key's picture at a revision is the same whichever run asks, renders or uploads it",
        ),
        ("primitives::textures::loader::Pools", "texture loaders: cache; one per salt, made on first use, and the store's uploads"),
        ("primitives::textures::thumbnails::Hubs", "thumbnails: cache; one per salt, made on first use, and its per-frame render budget"),
        ("fonts::document::Registry", "document faces: cache; a face loads the same whichever run asks, and its repaint request is harmless"),
        ("runtime::egui::fonts::Steps", "fonts plugin begin-pass steps: cache; a step added twice runs once"),
        // Testing logs.
        ("runtime::measure::", "measuring-run counts and the skipped-effect log (testing)"),
        ("testing::style::StyleRecord", "style log (testing)"),
        ("runtime::inspect::Log", "inspection log"),
    ];

    /// Raises one of the depths for as long as it lives: a panic that unwinds out of the run lowers it too, so a later
    /// frame on the same thread (the next trial a test harness runs there) is not counted inside the run.
    #[cfg(feature = "testing")]
    struct Raised(&'static std::thread::LocalKey<std::cell::Cell<u32>>);

    #[cfg(feature = "testing")]
    impl Raised {
        fn new(depth: &'static std::thread::LocalKey<std::cell::Cell<u32>>) -> Self {
            depth.with(|depth| depth.set(depth.get() + 1));
            Raised(depth)
        }
    }

    #[cfg(feature = "testing")]
    impl Drop for Raised {
        fn drop(&mut self) {
            self.0.with(|depth| depth.set(depth.get() - 1));
        }
    }

    /// Runs `f` inside a measuring run (`measure::run`).
    pub(crate) fn measuring<R>(f: impl FnOnce() -> R) -> R {
        #[cfg(feature = "testing")]
        let _raised = Raised::new(&MEASURING);
        f()
    }

    /// Runs `f` as part of an emit or a replay: what it touches is handed forward through layout memory.
    pub(crate) fn emitting<R>(f: impl FnOnce() -> R) -> R {
        #[cfg(feature = "testing")]
        let _raised = Raised::new(&EMITTING);
        f()
    }

    /// A service of type `S` is touched.
    #[cfg(feature = "testing")]
    pub(crate) fn touch<S>() {
        if !crate::runtime::egui::fixture::driving() || MEASURING.with(std::cell::Cell::get) == 0 || EMITTING.with(std::cell::Cell::get) > 0 {
            return;
        }
        let name = std::any::type_name::<S>();
        // A measuring run touches its services many times; the substring search over the list runs once per name.
        let classified = SEEN.with(|seen| {
            *seen.borrow_mut().entry((name.as_ptr() as usize, name.len())).or_insert_with(|| CLASSIFIED.iter().any(|(ty, _)| name.contains(ty)))
        });
        if classified {
            return;
        }
        if std::env::var_os("CRAFT_UI_GUARD_LOG").is_some() {
            eprintln!("craft-ui guard: {name}");
            return;
        }
        panic!("a measuring run touched the service {name} outside layout_memory::emit_record and contribute");
    }
}
