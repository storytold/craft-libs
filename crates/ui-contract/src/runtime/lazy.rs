//! Out-of-view content: inside a scroll area, a child that is out of view does not run; it keeps what it last
//! showed: the extent it had in the last pass that showed it, at whatever room the scroll area has now, and the
//! stops it registered there, in their place in Tab order. A child no pass showed yet takes the estimate its user
//! gives (a `Lazy`'s argument, a collection's item layout). The kept extents are the one previous-pass state of the
//! run model, kept for every user in one store: `contract::Lazy` children and the rows of a collection. A scroll area
//! that does not run keeps its children's entries; one that runs drops the entries it did not reach.
//!
//! The store also hands a scroll area, for the run that lays its content out, where its children start: the child
//! its scroll position names (`Anchor`) and the first child at or after the view's start, so the area keeps the
//! content in view in place when extents before it change.
//!
//! A memoized view (`contract::MemoView`) also keeps its last real run (`runtime::replay`): while it is settled, it
//! adds that run to the pass again in place of running, in view or not, inside a scroll area or not, in a later frame
//! or a later pass of the frame it ran in. It is settled when its props key is the one of that run and nothing else the
//! framework tracks has changed for it since (`settled`). Its entry outlives a few frames of absence.

use crate::runtime::egui::areas::Layer;
use crate::runtime::egui::{input as input_state, pass};
use crate::runtime::env::Flow;
use crate::runtime::focus::{self, Transcript};
use crate::runtime::frame::{self, PerViewport};
use crate::runtime::inputs::{self, Input};
use crate::runtime::layout_memory;
use crate::runtime::reads::BindKey;
use crate::runtime::replay::{self, Record};
use crate::style::EnvKey;
use egui::emath::GuiRounding as _;
use egui::{Context, Id, IdMap, IdSet, Layout, RecordShift, Rect, Vec2};
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// What a memoized view's run returned, kept for its replays.
pub(crate) type Output = Arc<dyn Any + Send + Sync>;

/// What a child's runs are keyed by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    /// Nothing: a `Lazy` child or a collection row, which runs whenever it is in view.
    None,
    /// A memoized view's props key (`contract::MemoView`), taken again after each run.
    Memo(u128),
}

/// The view of the innermost scroll area, as the run that lays its content out sees it: the rect the pass shows,
/// placed where this run puts the content (a measuring run lays it out from offset zero).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct View {
    /// The scroll area; children's entries are kept under it.
    pub(crate) owner: Id,
    pub(crate) rect: Rect,
    /// The content's rect in this run: offset zero is its start edge (the right edge in a right-to-left flow).
    pub(crate) content: Rect,
    /// The axes the area scrolls along (x, y).
    pub(crate) axes: (bool, bool),
    pub(crate) rtl: bool,
}

impl View {
    /// How far `rect` starts from the content's start edge, along each axis.
    pub(crate) fn start_of(&self, rect: Rect) -> Vec2 {
        let x = if self.rtl { self.content.max.x - rect.max.x } else { rect.min.x - self.content.min.x };
        Vec2::new(x, rect.min.y - self.content.min.y)
    }

    /// The extent a child no pass showed yet takes when it is `along` long on the axes the area scrolls along. Across
    /// them it takes nothing: its content gives its width there once it ran, so a guess never widens the content.
    pub(crate) fn estimate(&self, along: f32) -> Vec2 {
        Vec2::new(if self.axes.0 { along } else { 0.0 }, if self.axes.1 { along } else { 0.0 })
    }

    /// The view moved by `offset`, with its area.
    fn moved(self, offset: Vec2) -> View {
        View { rect: self.rect.translate(offset), content: self.content.translate(offset), ..self }
    }
}

/// A scroll position named by content: the child `child` starts `at` from the view's start edge. The scroll area
/// keeps it with its offset (same memory, same lifetime) and derives the offset from it while the child is laid out
/// where the view can show it, so extents that change before the child do not move what is in view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Anchor {
    pub(crate) child: Id,
    pub(crate) at: Vec2,
}

/// Where the children of a scroll area started in the real run that laid its content out, from the view's start.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Placed {
    /// The child the area's scroll position names, the child it glides to (`Lazy::reveal`) and the axes it scrolls
    /// along, as the area had them when the run began. They are the area's own state, not inputs of its children, so
    /// a replayed child reports against the area's current ones.
    names: (Option<Id>, Option<Id>, (bool, bool)),
    /// The view the run lays the content out for (`begin`), which a child's rect is placed from.
    view: Option<View>,
    /// The place and extent of the child the area's anchor names, when the run reached it.
    pub(crate) anchor: Option<(Vec2, Vec2)>,
    /// The first child, in flow order, that starts at or after the view's start, and its place.
    pub(crate) first: Option<(Id, Vec2)>,
    /// The place of the child the area glides to, when the run reached it.
    pub(crate) reveal: Option<Vec2>,
}

impl Placed {
    /// No places yet, for an area that names `anchor` as its scroll position, glides to `reveal` and scrolls along
    /// `axes`.
    pub(crate) fn named(anchor: Option<Id>, reveal: Option<Id>, axes: (bool, bool)) -> Placed {
        Placed { names: (anchor, reveal, axes), ..Placed::default() }
    }

    /// Notes that the child `id` took a place `at` from the view's start and is `extent` long.
    pub(crate) fn note(&mut self, id: Id, at: Vec2, extent: Vec2) {
        let (anchor, reveal, axes) = self.names;
        if anchor == Some(id) {
            self.anchor = Some((at, extent));
        }
        if self.first.is_none() && (!axes.0 || at.x >= 0.0) && (!axes.1 || at.y >= 0.0) {
            self.first = Some((id, at));
        }
        if reveal == Some(id) {
            self.reveal = Some(at);
        }
    }
}

/// What a child of a scroll area does this pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Decision {
    Run,
    /// Out of view: it takes this extent and does not run.
    Skip(Vec2),
}

#[derive(Clone, Default)]
struct Entry {
    /// The scroll area the child is kept under.
    owner: Option<Id>,
    /// What the last measuring run and the last real run that showed the child measured. A run that skips the child
    /// takes the extent of its own kind: a content-sized scroll area is offered another room in its measuring run than
    /// in its real run, so each kind places the child as it last laid it out. Out of view the child keeps it at any
    /// room; it is measured again when it meets the reach.
    sizes: [Option<Measured>; 2],
    /// What the child registered with the focus service in the last real run that showed it. Out of view the
    /// child registers it again in its place, so Tab and F6 reach its stops; it runs while one of them holds focus
    /// or a focus request asks to reveal one.
    focus: Arc<Transcript>,
    /// Whether this pass skips the child, under the pass number that decided it.
    decided: Option<(u64, bool)>,
    /// The last real run of a memoized view.
    settled: Option<Arc<Settled>>,
    /// The last measuring run of a measure-only child (`measured`).
    measured: Option<MeasuredRun>,
    /// Whether this pass replays the memoized view's last run, as the first real run of the pass decided, under the pass
    /// number.
    replays: Option<(u64, bool)>,
    /// The pass a collection row's extent was last measured in (`remember`).
    measured_in: Option<u64>,
    /// The last frame that read or wrote the entry.
    touched: u64,
}

/// What a run that showed a child measured: the extent it took, the key of the content it showed, the room it was
/// offered (`Rect::NOTHING` for a collection row, whose extent its collection reports), the flow of the body around
/// it (the default for a collection row) and the axis that body shares which a memoized view's content filled, which
/// makes the view one of the children that fill the body (`layout::fill_size`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Measured {
    pub(crate) size: Vec2,
    pub(crate) key: Key,
    pub(crate) room: Rect,
    pub(crate) flow: Flow,
    pub(crate) fills: Option<craft_behavior::Axis>,
}

/// Where a memoized view ran: it replays only at a place of the same size, layer, layout, opacity, flow and environment,
/// and its record moves with the room's position (`Place::shift_to`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Place {
    /// The region its parent offered it.
    pub(crate) room: Rect,
    pub(crate) clip: Rect,
    pub(crate) layer: Layer,
    pub(crate) layout: Layout,
    pub(crate) pixels_per_point: f32,
    /// The font atlas the run's text points into (`Fonts::atlas_generation`): egui clears the atlas when it fills up
    /// and rebuilds it for new text options, and a kept galley's glyphs then name other glyphs.
    pub(crate) atlas: u64,
    /// The opacity everything painted in the room is faded to (`Ui::opacity`, a layer fading in or out): the run's
    /// colours hold it.
    pub(crate) opacity: f32,
    /// The scope's environment (`Env::key`): its tokens (theme, density), scale, direction, language and whether the
    /// window is active.
    pub(crate) env: EnvKey,
    /// The flow of the body around the view (`Scoped::flow`).
    pub(crate) flow: Flow,
}

/// How a memoized view is shown: in a real run at its place, or in a measuring run laid out by the flow of the body
/// around it, offered `room` in a sizing pass (`None` outside one).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Call {
    Real(Place),
    Measuring { room: Option<Rect>, flow: Flow },
}

/// How a kept run replays at another place: the shift its record moves by, and whether the clip moved with the room.
#[derive(Clone, Copy, Debug)]
struct Moved {
    shift: RecordShift,
    with_clip: bool,
}

impl Place {
    /// How a run at this place that took `rect` replays at `now`, or `None` when it cannot. The room may have moved;
    /// its size, the layer, layout, scale, font atlas, opacity, flow and environment are the ones the run had. A record
    /// that shows an overlay replays only at the room and clip of its run (`Record::pinned`). Any other record moves by
    /// the room's move when that keeps where a run places content (`keeps_placement`, so a moved record is what a run
    /// there gives) and, for a move, the record holds nothing a move cannot place (`Record::movable`: no paint
    /// callback) on an untransformed layer (`transformed`: egui's AccessKit bounds are layer space only there). Its
    /// clip rects follow the clip when either:
    /// - the clip moved with the room (`with_clip`): every rect and clip moves exactly, whatever the clip cut;
    /// - the run lay wholly inside its clip: nothing of it was culled (widgets skip paint outside the clip, and every
    ///   nested `Lazy` child in the scroll view around it ran), and every clip set inside it lies within the run's clip,
    ///   so the run's clip becomes the clip now and inner clips move (`RecordShift::clip`).
    ///
    /// Otherwise it runs: a run cut by its clip lacks what was culled.
    fn shift_to(&self, rect: Rect, now: Place, record: &Record, transformed: bool) -> Option<Moved> {
        let offset = now.room.min - self.room.min;
        let same = now.room.size() == self.room.size()
            && now.layer == self.layer
            && now.layout == self.layout
            && now.pixels_per_point == self.pixels_per_point
            && now.atlas == self.atlas
            && now.opacity == self.opacity
            && now.env == self.env
            && now.flow == self.flow;
        let placeable = if record.pinned() {
            offset == Vec2::ZERO && now.clip == self.clip
        } else {
            offset == Vec2::ZERO || (keeps_placement(rect.union(self.room), offset, now.pixels_per_point) && record.movable() && !transformed)
        };
        if !same || !placeable {
            return None;
        }
        let shift = RecordShift { offset, ran_clip: self.clip, clip: now.clip };
        let with_clip = now.clip == self.clip.translate(offset);
        (with_clip || self.clip.contains_rect(rect)).then_some(Moved { shift, with_clip })
    }
}

/// Whether content in `area` moved by `offset` lies where a run at the new place puts it: the move is a whole step of
/// egui's UI rounding, which every widget rect is rounded to, and a whole physical pixel, which paint snaps to
/// (`Surface`, text). A move between them leaves a rounded rect off the grid a run there rounds it to. Both roundings
/// take a half step away from zero (`f32::round`), so a coordinate on a half step rounds to the same place after the
/// move only on the same side of zero: the area keeps its side of zero along each axis.
pub(crate) fn keeps_placement(area: Rect, offset: Vec2, pixels_per_point: f32) -> bool {
    let moved = area.translate(offset);
    let one_side = |(min, max): (f32, f32), (to_min, to_max): (f32, f32)| (min >= 0.0 && to_min >= 0.0) || (max <= 0.0 && to_max <= 0.0);
    offset == offset.round_ui()
        && offset == offset.round_to_pixels(pixels_per_point)
        && one_side((area.min.x, area.max.x), (moved.min.x, moved.max.x))
        && one_side((area.min.y, area.max.y), (moved.min.y, moved.max.y))
}

/// The last measuring run of a measure-only child: content a pass measures and never shows, as a page `Pages` holds
/// out of view.
#[derive(Clone)]
struct MeasuredRun {
    /// The values it read.
    reads: Vec<BindKey>,
    /// The pass it ran in.
    pass: u64,
    size: Vec2,
}

/// The last real run of a memoized view.
struct Settled {
    record: Record,
    /// The frame it ran in.
    frame: u64,
    place: Place,
    /// The rect its content took.
    rect: Rect,
    /// The pointer was over it, or one of its stops held focus: it runs again to show that they left.
    engaged: bool,
    /// What it ran for: its props key after the run.
    key: Key,
    /// The pass it ran in.
    pass: u64,
    /// The edit serial when it started (`note_edit`): a later pass of its frame replays it only if no value it read was
    /// edited after that.
    edits: u64,
    /// What a memoized view's run returned, which its replays return.
    output: Option<Output>,
    /// The generation of ended shown lives after the run (`memory::ended_generation`).
    lives: u64,
    /// The generation of each framework input when the run started (`runtime::inputs`): a memoized view whose run read
    /// one replays only while it is the same.
    seen: inputs::Seen,
    /// The view of the scroll area around it when it ran: a run that decided a nested `Lazy` child by it replays only
    /// while it is the same.
    view: Option<View>,
}

/// The bound values edited in the frame, which the memoized views that read them follow.
#[derive(Default)]
struct Changes {
    /// The values changed in a frame, each with the serial of its last edit, and the frame.
    edited: (u64, HashMap<BindKey, u64>),
    /// The serial of the last edit (`note_edit`).
    serial: u64,
}

impl Changes {
    /// The values edited in `frame`, each with the serial of its last edit; `None` when there were none.
    fn edited_in(&self, frame: u64) -> Option<HashMap<BindKey, u64>> {
        (self.edited.0 == frame && !self.edited.1.is_empty()).then(|| self.edited.1.clone())
    }
}

/// Of the values `edited` in a frame, the ones whose last edit came after the edit serial `since`.
fn edited_since(edited: Option<&HashMap<BindKey, u64>>, since: u64) -> Option<HashSet<BindKey>> {
    edited.map(|edited| edited.iter().filter(|(_, serial)| **serial > since).map(|(key, _)| *key).collect())
}

#[derive(Default)]
pub(crate) struct Store {
    entries: IdMap<Entry>,
    changes: Changes,
    /// The last frame each scroll area reached one of its children in.
    reached: IdMap<u64>,
    /// What the real run that lays a scroll area's content out found, by area, from `begin` to `placed`.
    placed: IdMap<Placed>,
    /// The frame the entries were last pruned in.
    pruned: u64,
}

impl Entry {
    /// Whether the entry is shown: touched in this frame or the one before, or kept by a scroll area that did not run
    /// (a tab switched away), whose kept offset its extent places.
    fn live(&self, frame: u64, reached: &IdMap<u64>) -> bool {
        self.touched + 1 >= frame || self.owner.is_some_and(|owner| reached.get(&owner).is_none_or(|ran| ran + 1 < frame))
    }

    /// Whether the entry keeps a memoized view's run.
    fn memo(&self) -> bool {
        self.settled.as_ref().is_some_and(|settled| matches!(settled.key, Key::Memo(_)))
    }
}

impl Store {
    /// The entry of the child `id` of the scroll area `owner`, touched in `frame`.
    fn touch(&mut self, owner: Option<Id>, id: Id, frame: u64) -> &mut Entry {
        if let Some(owner) = owner {
            self.reached.insert(owner, frame);
        }
        let entry = self.entries.entry(id).or_default();
        entry.owner = owner;
        entry.touched = frame;
        entry
    }
}

/// The memo rule for absent entries, shared by every memo store (memoized views, `runtime::derived`): an entry
/// outlives its absence while the absent ones are at most twice as many as the shown ones, the longest absent dropped
/// first. Takes the absent entries as (frame last touched, id) and returns the ones to drop.
pub(crate) fn absent_past_room(shown: usize, absent: impl IntoIterator<Item = (u64, Id)>) -> Vec<Id> {
    let mut absent: Vec<(u64, Id)> = absent.into_iter().collect();
    let room = 2 * shown;
    if absent.len() <= room {
        return Vec::new();
    }
    absent.sort_unstable_by_key(|(touched, _)| *touched);
    absent.truncate(absent.len() - room);
    absent.into_iter().map(|(_, id)| id).collect()
}

fn with<R>(ctx: &Context, f: impl FnOnce(&mut Store, u64, u64) -> R) -> R {
    let viewport = pass::current_viewport(ctx);
    let (pass_nr, frame) = (pass::pass_nr(ctx, viewport), pass::frame_nr(ctx, viewport));
    PerViewport::<Store>::with(ctx, viewport, |store| {
        // A child its scroll area did not reach in a frame the area ran is gone from it. An area that did not run
        // (a tab switched away) keeps its children: their extents place the content its kept offset shows.
        if store.pruned != frame {
            store.pruned = frame;
            let reached = &store.reached;
            store.entries.retain(|_, entry| entry.memo() || entry.live(frame, reached));
            // A memoized view's entry outlives its absence by the memo rule: a view shown again with the same props
            // replays.
            let shown = store.entries.values().filter(|entry| entry.memo() && entry.live(frame, reached)).count();
            let absent = store.entries.iter().filter(|(_, entry)| !entry.live(frame, reached)).map(|(id, entry)| (entry.touched, *id));
            for id in absent_past_room(shown, absent) {
                store.entries.remove(&id);
            }
            let owners: IdSet = store.entries.values().filter_map(|entry| entry.owner).collect();
            store.reached.retain(|owner, _| owners.contains(owner));
        }
        f(store, pass_nr, frame)
    })
}

/// Runs `f` on the store as `with` does, for a write to the entry of the child `id`: inside a draft
/// (`layout_memory::draft`) the entry as it was goes to the draft, so a measuring run taken back leaves the entry, its
/// extents and this pass's decisions included, as it found it.
fn write<R>(ctx: &Context, id: Id, f: impl FnOnce(&mut Store, u64, u64) -> R) -> R {
    let drafting = layout_memory::drafting(ctx);
    let (out, before) = with(ctx, |store, pass_nr, frame| {
        let before = drafting.then(|| store.entries.get(&id).cloned());
        (f(store, pass_nr, frame), before)
    });
    if let Some(before) = before {
        let viewport = pass::current_viewport(ctx);
        layout_memory::note_undo(
            ctx,
            Box::new(move |ctx: &Context| {
                PerViewport::<Store>::with(ctx, viewport, |store| match before {
                    Some(entry) => drop(store.entries.insert(id, entry)),
                    None => drop(store.entries.remove(&id)),
                });
            }),
        );
    }
    out
}

/// Decides whether the child `id`, whose content starts at `at` in this run, runs this pass. It runs when its extent
/// placed at `at` meets the view, or when one of its stops holds focus or is the target of a focus request. Its
/// extent is the one a run showed it at, else `estimate`. The first run of a pass decides; every later run of the
/// pass gets the same answer, so a measuring run and the real run lay out the same children.
pub(crate) fn decide(ctx: &Context, id: Id, view: &View, at: impl FnOnce(Vec2) -> Rect, sizing: bool, estimate: Vec2) -> Decision {
    let targets = crate::runtime::focus::targets(ctx);
    write(ctx, id, |store, pass_nr, frame| {
        let entry = store.touch(Some(view.owner), id, frame);
        let kind = usize::from(!sizing);
        // A child the other kind of run showed last takes that run's extent.
        let size = entry.sizes[kind].or(entry.sizes[1 - kind]).map_or(estimate, |measured| measured.size);
        let skip = match entry.decided {
            Some((decided_in, skip)) if decided_in == pass_nr => skip,
            _ => {
                let focus = entry.focus.stop_ids().any(|stop| targets.contains(&Some(stop)));
                let skip = !focus && !at(size).intersects(view.rect);
                entry.decided = Some((pass_nr, skip));
                skip
            }
        };
        if skip { Decision::Skip(size) } else { Decision::Run }
    })
}

/// Records what a run that showed the child `id` found: what it measured in a run of this kind and, from a real run,
/// what it registered with the focus service.
pub(crate) fn shown(ctx: &Context, owner: Option<Id>, id: Id, sizing: bool, measured: Measured, registered: Option<Transcript>) {
    write(ctx, id, |store, _, frame| {
        let entry = store.touch(owner, id, frame);
        entry.sizes[usize::from(!sizing)] = Some(measured);
        if let Some(registered) = registered {
            entry.focus = Arc::new(registered);
        }
    });
}

/// The axis the content of the memoized view `id` filled in its last run of this kind (`sizing`), else of the other
/// kind: the run of this pass once one ran, else the kept one. `None` before any run showed it.
pub(crate) fn fills(ctx: &Context, id: Id, sizing: bool) -> Option<craft_behavior::Axis> {
    let (own, other) = (usize::from(!sizing), usize::from(sizing));
    with(ctx, |store, _, _| store.entries.get(&id).and_then(|entry| entry.sizes[own].or(entry.sizes[other])).and_then(|measured| measured.fills))
}

/// The extent the child `id` took in the last measuring or real run that showed it (this pass's measuring run once it
/// ran), at whatever room it had; `None` before any run showed it.
pub(crate) fn last_extent(ctx: &Context, id: Id) -> Option<Vec2> {
    with(ctx, |store, _, _| store.entries.get(&id).and_then(|entry| entry.sizes[1].map(|measured| measured.size)))
}

/// Registers what the skipped child `id` registered with the focus service in the last real run that showed it, in
/// the real run that skips it.
pub(crate) fn replay(ctx: &Context, id: Id) {
    let registered = with(ctx, |store, _, _| store.entries.get(&id).map(|entry| Arc::clone(&entry.focus)));
    if let Some(registered) = registered {
        focus::replay(ctx, &registered);
    }
}

/// Whether the memoized view `id` adds its last real run to this pass in place of running, the extent that run took in
/// a run of this kind (`call`), the rect the kept real run takes now and the shift its record moves by. The first real
/// run of a pass decides from what changed since that run and from its key, and every later real run of the pass takes
/// its answer; a real run also needs a place the run can move to (`Place::shift_to`), which a measuring run cannot
/// tell. A measuring run asks the same each time, with its own key, and decides nothing for the pass: a call that shows
/// other content runs and leaves the decision to the calls after it. A sizing run takes the extent the last sizing run
/// measured of the content the kept run showed (its key) in the room it offers now and in the same flow; when that
/// sizing run showed other content or had another room or flow, or there was none, it runs once as a measuring run,
/// which keeps no record, so the real run still replays.
///
/// It replays when its last run was in an earlier pass, of this frame or an earlier one, and since then:
/// - nothing in it moves: no animation off its end value, no repaint asked from inside it, no texture it shows still
///   loading, no wake time reached;
/// - the pointer is not over it or an overlay layer it shows, no press started on either and none of its widgets is
///   dragged, and none of that held in its last run;
/// - the focus is where its run left it (`replay::Held`: the same stop and egui widget of it, or none), and while the
///   focus is in it no key, text, IME, clipboard or AccessKit event came (`Input::Sent`, which such a run reads);
/// - none of its stops is the target of a focus request, now or in its last run;
/// - no intent of this pass is routed to a component in it (an AccessKit request on one of its nodes), and no scroll
///   move waits for a component in it (`moving`, the targets the contract's scroll owners queued);
/// - no value it read (a bound value, an inspection record, a command's state) changed this frame (`note_edit`), or,
///   replaying a run of this frame, after that run started;
/// - the call's key is the one of its last run (its props key after that run), and its environment (theme, density,
///   scale, language, window active) and the flow of the body around it (`Flow`) the ones it ran in;
/// - the run did nothing a replay cannot repeat (`runtime::replay`) and asked for no discard pass: what it added is
///   what the pass was thrown away for;
/// - the shown life of no component it showed ended (`memory::ended_since`): a replay would show memory state that is
///   gone;
/// - no framework input its run read outside its props has moved (`runtime::inputs`), the scroll view a nested `Lazy`
///   child of it decided by is the same (moved with it, `moved`), no command waits for a command scope instance of
///   it, and no close waits for a layer it shows.
pub(crate) fn settled(
    ctx: &Context,
    owner: Option<Id>,
    id: Id,
    call: Call,
    view: Option<View>,
    key: Key,
    moving: &[Id],
) -> Option<(Vec2, Rect, RecordShift)> {
    let (place, sizing, flow) = match call {
        Call::Real(place) => (Some(place), None, place.flow),
        Call::Measuring { room, flow } => (None, room, flow),
    };
    let real = place.is_some();
    let revealed = focus::targets(ctx)[1];
    let mut routed = crate::runtime::keys::routed_to(ctx);
    routed.extend_from_slice(moving);
    let now = frame::clock(ctx);
    let dragged = input_state::dragged(ctx);
    let focused = replay::Held::focused(ctx);
    let transformed = place.is_some_and(|place| ctx.layer_transform_to_global(place.layer).is_some());
    let settled = write(ctx, id, |store, pass_nr, frame| {
        let changes = &store.changes;
        // Usually `None`: only the values written this frame.
        let edited = changes.edited_in(frame);
        let entry = store.touch(owner, id, frame);
        let replays = match entry.replays.filter(|_| real) {
            Some((decided_in, replays)) if decided_in == pass_nr => replays,
            _ => {
                let replays = entry.settled.as_ref().is_some_and(|settled| {
                    let facts = &settled.record.facts;
                    // A run of this frame read the values edited before it started.
                    let edited = edited_since(edited.as_ref(), if settled.frame < frame { 0 } else { settled.edits });
                    (settled.frame < frame || settled.pass < pass_nr)
                        && settled.key == key
                        && !settled.engaged
                        && !facts.blocked
                        && !facts.discards
                        && facts.quiet(now, edited.as_ref())
                        && !settled.record.focus.stop_ids().any(|stop| revealed == Some(stop) || routed.contains(&stop))
                        && !routed.iter().any(|id| settled.record.widgets.contains(id) || settled.record.facts.node_owners.contains(id))
                        && !dragged.is_some_and(|widget| settled.record.widgets.contains(&widget))
                        && focused.within(&settled.record.focus, &settled.record.widgets) == settled.record.held
                });
                if real {
                    entry.replays = Some((pass_nr, replays));
                }
                replays
            }
        };
        // A measuring run cannot tell the place: it replays where the run was.
        let moved = |settled: &Settled| match place {
            Some(place) => settled.place.shift_to(settled.rect, place, &settled.record, transformed),
            None => Some(Moved { shift: RecordShift::none(settled.place.clip), with_clip: true }),
        };
        let settled = entry.settled.clone().filter(|_| replays).and_then(|settled| Some((moved(&settled)?, settled)));
        if real && replays && settled.is_none() {
            entry.replays = Some((pass_nr, false));
        }
        settled.and_then(|(moved, settled)| {
            // An extent depends on the room's size, not on where the room is.
            let size = match sizing {
                Some(room) => {
                    entry.sizes[0]
                        .filter(|measured| measured.key == settled.key && measured.flow == flow && measured.room.size() == room.size())?
                        .size
                }
                None => match entry.sizes[1].filter(|measured| measured.key == settled.key && measured.flow == flow) {
                    Some(measured) => measured.size,
                    // A real run's place has the kept run's flow (`Place::shift_to`); a measuring run checks it here.
                    None => (settled.place.flow == flow).then(|| settled.rect.size())?,
                },
            };
            Some((size, moved, settled))
        })
    });
    // The pointer over it, a press that started on it, the memory of the components it showed, the framework inputs it
    // read and the commands queued for its instances are read outside the store.
    let (size, Moved { shift, with_clip }, settled) = settled?;
    let rect = shift.rect(settled.rect);
    // The scroll view a nested child decided by: the run's, moved with the record, while the clip moved with it; a run
    // that lay wholly in its clip ran every nested child, which holds at any view.
    let view = place.and_then(|_| with_clip.then(|| (settled.view.map(|ran| ran.moved(shift.offset)), view)));
    if engaged(ctx, settled.place.layer, rect.intersect(shift.clip), &settled.record.facts.overlays)
        || crate::runtime::memory::ended_since(ctx, &settled.record.facts.owners, settled.lives)
        || moved(ctx, &settled, view)
    {
        if real {
            write(ctx, id, |store, pass_nr, _| {
                if let Some(entry) = store.entries.get_mut(&id) {
                    entry.replays = Some((pass_nr, false));
                }
            });
        }
        return None;
    }
    Some((size, rect, shift))
}

/// Whether something a memoized view's run read outside its props moved since: a framework input it read has another
/// generation, the scroll view a nested `Lazy` child decided by is another than the one it needs (`view`: the view it
/// needs and the view now, in a real run whose record replays only in that view; a measuring run lays the content out
/// from offset zero, so its view is not the one the real run decided by), a command waits for one of its instances,
/// or a close waits for a layer it shows.
fn moved(ctx: &Context, settled: &Settled, view: Option<(Option<View>, Option<View>)>) -> bool {
    let facts = &settled.record.facts;
    inputs::moved(ctx, facts.inputs, &settled.seen)
        || (facts.inputs.contains(Input::Scroll) && view.is_some_and(|(needs, now)| needs != now))
        || crate::runtime::commands::queued_for(ctx, &facts.instances)
        || crate::runtime::overlay::close_queued(ctx, &facts.layers)
}

/// Whether the pointer is over `rect` (shown) on `layer` or over one of the overlay layers `overlays` shown, or a
/// press that is held started there. Both halves ask whether the layer is the top layer at the point, so a pointer or
/// a press on an overlay above the view leaves it alone.
fn engaged(ctx: &Context, layer: Layer, rect: Rect, overlays: &[replay::ShownLayer]) -> bool {
    let over = |layer: Layer, rect: Rect| {
        let screen = ctx.layer_transform_to_global(layer).map_or(rect, |transform| transform * rect);
        ctx.rect_contains_pointer(layer, rect)
            || input_state::held_press_origin(ctx).is_some_and(|origin| screen.contains(origin) && ctx.layer_id_at(origin) == Some(layer))
    };
    over(layer, rect) || overlays.iter().any(|shown| over(shown.layer, shown.rect))
}

/// Adds the last real run of the memoized view `id` to the running pass, in place of running it, at the place `shift`
/// names (`settled`), and returns what its run returned, as the run left it: the caller moves it.
pub(crate) fn replay_settled(ctx: &Context, id: Id, shift: &RecordShift) -> Option<Output> {
    let settled = with(ctx, |store, _, _| store.entries.get(&id).and_then(|entry| entry.settled.clone()))?;
    replay::replay(ctx, &settled.record, shift);
    settled.output.clone()
}

/// Compares the audit run of the memoized view `name` (`id`), which added `record`, took `rect` and returned `output`,
/// with its kept run replayed at the place `shift` names (`runtime::audit`).
#[cfg(feature = "testing")]
pub(crate) fn audit(
    ctx: &Context,
    name: &'static str,
    id: Id,
    run: crate::runtime::audit::Shown<'_>,
    same: Option<crate::runtime::audit::SameOutput>,
    shift: &RecordShift,
) {
    // A pass that turned to discard during the audit run paints nothing: its run is no shown run to compare with.
    if ctx.will_discard() {
        return;
    }
    let Some(kept) = with(ctx, |store, _, _| store.entries.get(&id).and_then(|entry| entry.settled.clone())) else { return };
    let kept_run = crate::runtime::audit::Shown { record: &kept.record, rect: kept.rect, output: kept.output.as_ref() };
    crate::runtime::audit::compare(name, id, (kept_run, shift), run, same);
}

/// What the last real run of the memoized view `id` returned, for a measuring run that takes its kept extent.
pub(crate) fn kept_output(ctx: &Context, id: Id) -> Option<Output> {
    with(ctx, |store, _, _| store.entries.get(&id).and_then(|entry| entry.settled.as_ref()?.output.clone()))
}

/// The serial of the last edit (`note_edit`): a run notes it when it starts.
pub(crate) fn edits(ctx: &Context) -> u64 {
    with(ctx, |store, _, _| store.changes.serial)
}

/// Keeps the entry of the child `id` this frame: a replayed run does not reach the children inside it.
pub(crate) fn keep(ctx: &Context, owner: Option<Id>, id: Id) {
    with(ctx, |store, _, frame| {
        store.touch(owner, id, frame);
    });
}

/// What a memoized view's real run found besides its record: what it ran for, the edit serial and the framework
/// inputs' generations when it started (`edits`, `inputs::seen`), the scroll view around it and what its
/// run returned.
pub(crate) struct Ran {
    pub(crate) key: Key,
    pub(crate) edits: u64,
    pub(crate) seen: inputs::Seen,
    pub(crate) view: Option<View>,
    pub(crate) output: Option<Output>,
}

/// Keeps the real run `record` of the memoized view `id`, which ran at `place` and took `rect`. A run in a pass egui
/// will discard keeps none: egui paints nothing of that pass (`Painter::is_visible` is false in it), so the run registers
/// invisible widgets and no nodes, which a shown pass's replay must not add. The kept record stays the last shown pass's.
pub(crate) fn ran(ctx: &Context, id: Id, record: Record, place: Place, rect: Rect, Ran { key, edits, seen, view, output }: Ran) {
    if ctx.will_discard() {
        return;
    }
    let revealed = focus::targets(ctx)[1];
    let engaged_now = record.focus.stop_ids().any(|stop| revealed == Some(stop));
    let lives = crate::runtime::memory::ended_generation(ctx);
    let mut settled = Settled { record, frame: 0, place, rect, engaged: engaged_now, key, pass: 0, edits, output, lives, seen, view };
    settled.engaged |= engaged(ctx, place.layer, rect.intersect(place.clip), &settled.record.facts.overlays);
    with(ctx, |store, pass_nr, frame| {
        settled.frame = frame;
        settled.pass = pass_nr;
        if let Some(entry) = store.entries.get_mut(&id) {
            entry.settled = Some(Arc::new(settled));
        }
    });
}

/// The size this pass's measuring run of the measure-only child `id` found, and the values it read: a later run in the
/// pass takes that size, whatever room it has, so the pass places by one size. The caller notes the values as read
/// again, so an edit of one later in the pass reruns it. A run of an earlier pass is not reused: the content's inputs
/// are not tracked, so it is measured again in each pass.
pub(crate) fn measured(ctx: &Context, id: Id) -> Option<(Vec2, Vec<BindKey>)> {
    with(ctx, |store, pass_nr, frame| {
        let run = store.touch(None, id, frame).measured.as_ref()?;
        (run.pass == pass_nr).then(|| (run.size, run.reads.clone()))
    })
}

/// Keeps the measuring run of the measure-only child `id` in this pass: the values it read and the size it took.
pub(crate) fn measured_run(ctx: &Context, id: Id, reads: Vec<BindKey>, size: Vec2) {
    write(ctx, id, |store, pass_nr, frame| {
        store.touch(None, id, frame).measured = Some(MeasuredRun { reads, pass: pass_nr, size });
    });
}

/// Notes that the value `key` changed in this frame: a memoized view that read it runs.
pub(crate) fn note_edit(ctx: &Context, key: BindKey) {
    with(ctx, |store, _, frame| {
        let changes = &mut store.changes;
        if changes.edited.0 != frame {
            changes.edited = (frame, HashMap::new());
        }
        changes.serial += 1;
        changes.edited.1.insert(key, changes.serial);
    });
}

/// Records the extent of a row of the collection `owner` that was measured this pass. The collection places the rows
/// it does not build by it (its item layout answers for rows it never built), and every run of the pass that reaches
/// the row takes it without measuring the row again.
pub(crate) fn remember(ctx: &Context, owner: Id, id: Id, size: Vec2) {
    write(ctx, id, |store, pass_nr, frame| {
        let entry = store.touch(Some(owner), id, frame);
        entry.sizes[1] = Some(Measured { size, key: Key::None, room: Rect::NOTHING, flow: Flow::default(), fills: None });
        entry.measured_in = Some(pass_nr);
    });
}

/// Runs `f` with a reader of the extents the rows of the collection `owner` had in the last pass that measured them,
/// each with whether that pass is this one, for a collection that reads many entries at once.
pub(crate) fn reading<R>(ctx: &Context, owner: Id, f: impl FnOnce(&dyn Fn(Id) -> Option<(Vec2, bool)>) -> R) -> R {
    with(ctx, |store, pass_nr, frame| {
        store.reached.insert(owner, frame);
        let store = std::cell::RefCell::new(store);
        f(&|id| {
            let mut store = store.borrow_mut();
            let entry = store.entries.get_mut(&id)?;
            entry.touched = frame;
            let size = entry.sizes[1].or(entry.sizes[0]).map(|measured| measured.size)?;
            Some((size, entry.measured_in == Some(pass_nr)))
        })
    })
}

/// Starts the real run that lays out the content of the scroll area of `view`, which names `anchor` as its scroll
/// position and glides to `reveal`: the children it places report to it.
pub(crate) fn begin(ctx: &Context, view: View, anchor: Option<Id>, reveal: Option<Id>) {
    with(ctx, |store, _, _| {
        store.placed.insert(view.owner, Placed { view: Some(view), ..Placed::named(anchor, reveal, view.axes) });
    });
}

/// Reports, in the real run that lays out the content of `view`'s area, that the child `id` took `rect`, run or not.
pub(crate) fn place(ctx: &Context, view: &View, id: Id, rect: Rect) {
    note_place(ctx, view.owner, id, rect);
}

/// Reports that the child `id` of the area `owner` took `rect`. A settled run around the child that is replayed later
/// reports it again, moved with the run, and the area takes its place from the view it shows then; an area inside the
/// replayed run does not lay out in that pass, and its places were taken (`placed`).
fn note_place(ctx: &Context, owner: Id, id: Id, rect: Rect) {
    replay::journal(ctx, || (owner, id, rect), |ctx, &(owner, id, rect)| note_place(ctx, owner, id, rect));
    with(ctx, |store, _, _| {
        if let Some(placed) = store.placed.get_mut(&owner)
            && let Some(view) = placed.view
        {
            placed.note(id, view.start_of(rect) - view.start_of(view.rect), rect.size());
        }
    });
}

/// What the children of the area `owner` reported since `begin`.
pub(crate) fn placed(ctx: &Context, owner: Id) -> Placed {
    with(ctx, |store, _, _| store.placed.remove(&owner).unwrap_or_default())
}

/// How far the offset an anchor gives may differ from the offset shown before the anchor moves it: less than half a
/// pixel is the rounding of the starts it compares.
const ANCHOR_SLACK: f32 = 0.5;

/// The anchor rule, after the real run of a scroll area reported `placed`. `moved` is how far the offset moved since
/// the anchor was named (a move takes the anchor's place in the view with it), `view` the view's size, and `room` how
/// far the offset can go back and ahead. While the anchor's child shows in the view, the shift that puts it back at its
/// place is returned when there is one: the area moves by it, keeps the anchor and runs the pass again, so extents that
/// changed before the child do not move what is in view. Otherwise the anchor becomes the first child at or after the
/// view's start. Places are relative to the view, so the rule holds at any offset.
pub(crate) fn follow(
    anchor: &mut Option<Anchor>,
    placed: &Placed,
    moved: Vec2,
    view: Vec2,
    (back, ahead): (Vec2, Vec2),
    axes: (bool, bool),
) -> Option<Vec2> {
    if let Some(named) = anchor.as_mut() {
        named.at -= moved;
        let shows = |at: f32, extent: f32, view: f32| at < view && at + extent.max(0.0) >= 0.0;
        if let Some((at, extent)) = placed.anchor
            && (!axes.0 || shows(named.at.x, extent.x, view.x))
            && (!axes.1 || shows(named.at.y, extent.y, view.y))
        {
            let shift = (at - named.at).clamp(-back.max(Vec2::ZERO), ahead.max(Vec2::ZERO));
            let shift = Vec2::new(if axes.0 { shift.x } else { 0.0 }, if axes.1 { shift.y } else { 0.0 });
            if shift.abs().max_elem() > ANCHOR_SLACK {
                return Some(shift);
            }
        }
    }
    *anchor = placed.first.map(|(child, at)| Anchor { child, at });
    None
}
