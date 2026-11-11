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
//! A settling child (`Lazy::settles`) also keeps its last real run (`runtime::replay`): while it is settled, it adds
//! that run to the pass again in place of running, in view or not, inside a scroll area or not. It is settled when
//! nothing the framework tracks has changed for it since that run (`settled`).

use crate::runtime::egui::pass;
use crate::runtime::focus::{self, Transcript};
use crate::runtime::frame::{self, PerViewport};
use crate::runtime::reads::BindKey;
use crate::runtime::replay::{self, Record};
use egui::{Context, Id, LayerId, Layout, Rect, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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
    /// The child the area's scroll position names.
    pub(crate) anchor: Option<Id>,
    /// The child the area glides to (`Lazy::reveal`).
    pub(crate) reveal: Option<Id>,
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
    /// The place and extent of the child the area's anchor names, when the run reached it.
    pub(crate) anchor: Option<(Vec2, Vec2)>,
    /// The first child, in flow order, that starts at or after the view's start, and its place.
    pub(crate) first: Option<(Id, Vec2)>,
    /// The place of the child the area glides to, when the run reached it.
    pub(crate) reveal: Option<Vec2>,
}

impl Placed {
    /// Notes that the child `id` took a place `at` from the view's start and is `extent` long, in a view whose area
    /// scrolls along `axes` and names `anchor`.
    pub(crate) fn note(&mut self, (anchor, axes): (Option<Id>, (bool, bool)), id: Id, at: Vec2, extent: Vec2) {
        if anchor == Some(id) {
            self.anchor = Some((at, extent));
        }
        if self.first.is_none() && (!axes.0 || at.x >= 0.0) && (!axes.1 || at.y >= 0.0) {
            self.first = Some((id, at));
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
    /// The extent of the last measuring run and of the last real run that showed the child. A run that skips the
    /// child takes the extent of its own kind: a content-sized scroll area is offered another room in its measuring
    /// run than in its real run, so each kind places the child as it last laid it out. Out of view the child keeps
    /// it at any room; it is measured again when it meets the reach.
    sizes: [Option<Vec2>; 2],
    /// What the child registered with the focus service in the last real run that showed it. Out of view the
    /// child registers it again in its place, so Tab and F6 reach its stops; it runs while one of them holds focus
    /// or a focus request asks to reveal one.
    focus: Arc<Transcript>,
    /// Whether this pass skips the child, under the pass number that decided it.
    decided: Option<(u64, bool)>,
    /// The last real run of a settling child.
    settled: Option<Arc<Settled>>,
    /// Whether this pass replays the settling child's last run, under the pass number that decided it.
    replays: Option<(u64, bool)>,
    /// The last frame that read or wrote the entry.
    touched: u64,
}

/// Where a settling child ran: it is settled only while it runs at the same place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Place {
    /// The region its parent offered it.
    pub(crate) room: Rect,
    pub(crate) clip: Rect,
    pub(crate) layer: LayerId,
    pub(crate) layout: Layout,
    pub(crate) pixels_per_point: f32,
}

/// The last real run of a settling child.
struct Settled {
    record: Record,
    /// The frame it ran in.
    frame: u64,
    place: Place,
    /// The rect its content took.
    rect: Rect,
    /// The pointer was over it, or one of its stops held focus: it runs again to show that they left.
    engaged: bool,
}

/// What changed in the frame for every settling child.
#[derive(Default)]
struct Changes {
    /// Every settling child runs through this frame: a frame with input other than pointer motion, an app's repaint
    /// request or an edit of a given value unsettles every child for that frame and the next.
    unsettled_through: u64,
    /// The pass whose input was looked at.
    scanned: u64,
    /// The bound values written in a frame, and the frame.
    edited: (u64, HashSet<BindKey>),
}

#[derive(Default)]
pub(crate) struct Store {
    entries: HashMap<Id, Entry>,
    changes: Changes,
    /// The last frame each scroll area reached one of its children in.
    reached: HashMap<Id, u64>,
    /// What the real run that lays a scroll area's content out found, by area, from `begin` to `placed`.
    placed: HashMap<Id, Placed>,
    /// The frame the entries were last pruned in.
    pruned: u64,
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

fn with<R>(ctx: &Context, f: impl FnOnce(&mut Store, u64, u64) -> R) -> R {
    let viewport = pass::current_viewport(ctx);
    let (pass_nr, frame) = (pass::pass_nr(ctx, viewport), pass::frame_nr(ctx, viewport));
    PerViewport::<Store>::with(ctx, viewport, |store| {
        // A child its scroll area did not reach in a frame the area ran is gone from it. An area that did not run
        // (a tab switched away) keeps its children: their extents place the content its kept offset shows.
        if store.pruned != frame {
            store.pruned = frame;
            let reached = &store.reached;
            store.entries.retain(|_, entry| {
                entry.touched + 1 >= frame || entry.owner.is_some_and(|owner| reached.get(&owner).is_none_or(|ran| ran + 1 < frame))
            });
            let owners: HashSet<Id> = store.entries.values().filter_map(|entry| entry.owner).collect();
            store.reached.retain(|owner, _| owners.contains(owner));
        }
        f(store, pass_nr, frame)
    })
}

/// Decides whether the child `id`, whose content starts at `at` in this run, runs this pass. It runs when its extent
/// placed at `at` meets the view, or when one of its stops holds focus or is the target of a focus request. Its
/// extent is the one a run showed it at, else `estimate`. The first run of a pass decides; every later run of the
/// pass gets the same answer, so a measuring run and the real run lay out the same children.
pub(crate) fn decide(ctx: &Context, id: Id, view: &View, at: impl FnOnce(Vec2) -> Rect, sizing: bool, estimate: Vec2) -> Decision {
    let targets = crate::runtime::focus::targets(ctx);
    with(ctx, |store, pass_nr, frame| {
        let entry = store.touch(Some(view.owner), id, frame);
        let kind = usize::from(!sizing);
        // A child the other kind of run showed last takes that run's extent.
        let size = entry.sizes[kind].or(entry.sizes[1 - kind]).unwrap_or(estimate);
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

/// Records what a run that showed the child `id` found: its extent in a run of this kind and, from a real run, what
/// it registered with the focus service.
pub(crate) fn shown(ctx: &Context, owner: Option<Id>, id: Id, size: Vec2, sizing: bool, registered: Option<Transcript>) {
    with(ctx, |store, _, frame| {
        let entry = store.touch(owner, id, frame);
        entry.sizes[usize::from(!sizing)] = Some(size);
        if let Some(registered) = registered {
            entry.focus = Arc::new(registered);
        }
    });
}

/// Registers what the skipped child `id` registered with the focus service in the last real run that showed it, in
/// the real run that skips it.
pub(crate) fn replay(ctx: &Context, id: Id) {
    let registered = with(ctx, |store, _, _| store.entries.get(&id).map(|entry| Arc::clone(&entry.focus)));
    if let Some(registered) = registered {
        focus::replay(ctx, &registered);
    }
}

/// Whether the settling child `id` adds its last real run to this pass in place of running, and the extent that run
/// took in a run of this kind (`sizing`). The first run of a pass decides from what changed since that run; a real run
/// also needs the child at the `place` it ran at, which a measuring run cannot tell.
///
/// It replays when its last run was in an earlier frame and since then:
/// - nothing in it moves: no animation off its end value, no repaint asked from inside it, no wake time reached;
/// - the pointer is not over it, no press started on it and none of its widgets is dragged or holds egui focus, and
///   none of that held in its last run;
/// - none of its stops holds focus or is the target of a focus request, now or in its last run;
/// - no bound value it read was written this frame (`note_edit`);
/// - no frame unsettled every child (`unsettle`): input other than pointer motion this frame or the one before, an app's
///   repaint request or an edit of a given value;
/// - the run did nothing a replay cannot repeat (`runtime::replay`).
pub(crate) fn settled(ctx: &Context, owner: Option<Id>, id: Id, sizing: bool, place: Option<Place>) -> Option<Vec2> {
    let input = ctx.input(|input| {
        input.events.iter().any(|event| !matches!(event, egui::Event::PointerMoved(_) | egui::Event::MouseMoved(_) | egui::Event::PointerGone))
    });
    let targets = focus::targets(ctx);
    let now = frame::clock(ctx);
    let held = (ctx.dragged_id(), ctx.memory(|memory| memory.focused()));
    let settled = with(ctx, |store, pass_nr, frame| {
        if store.changes.scanned != pass_nr {
            store.changes.scanned = pass_nr;
            if input {
                store.changes.unsettled_through = store.changes.unsettled_through.max(frame + 1);
            }
        }
        let changes = &store.changes;
        // Usually empty: only the values written this frame.
        let edited = (changes.edited.0 == frame).then(|| changes.edited.1.clone());
        let unsettled = changes.unsettled_through >= frame;
        let entry = store.touch(owner, id, frame);
        let replays = match entry.replays {
            Some((decided_in, replays)) if decided_in == pass_nr => replays,
            _ => {
                let replays = !unsettled
                    && entry.settled.as_ref().is_some_and(|settled| {
                        let facts = &settled.record.facts;
                        settled.frame < frame
                            && !settled.engaged
                            && !facts.unsettled
                            && !facts.blocked
                            && facts.wake.is_none_or(|at| at > now)
                            && edited.as_ref().is_none_or(|edited| facts.reads.is_disjoint(edited))
                            && !settled.record.focus.stop_ids().any(|stop| targets.contains(&Some(stop)))
                            && ![held.0, held.1].iter().flatten().any(|widget| settled.record.widgets.contains(widget))
                    });
                entry.replays = Some((pass_nr, replays));
                replays
            }
        };
        let at = |settled: &Settled| place.is_none_or(|place| place == settled.place);
        let settled = entry.settled.clone().filter(|settled| replays && at(settled));
        if replays && settled.is_none() {
            entry.replays = Some((pass_nr, false));
        }
        let kind = usize::from(!sizing);
        settled.map(|settled| (entry.sizes[kind].or(entry.sizes[1 - kind]).unwrap_or(settled.rect.size()), settled))
    });
    // The pointer over it, or a press that started on it, is read outside the store.
    let (size, settled) = settled?;
    if engaged(ctx, &settled) {
        with(ctx, |store, pass_nr, _| {
            if let Some(entry) = store.entries.get_mut(&id) {
                entry.replays = Some((pass_nr, false));
            }
        });
        return None;
    }
    Some(size)
}

/// Whether the pointer is over the rect the settled run took on its layer, or a press that is held started there.
fn engaged(ctx: &Context, settled: &Settled) -> bool {
    let rect = settled.rect.intersect(settled.place.clip);
    let screen = ctx.layer_transform_to_global(settled.place.layer).map_or(rect, |transform| transform * rect);
    ctx.rect_contains_pointer(settled.place.layer, rect)
        || ctx.input(|input| input.pointer.any_down() && input.pointer.press_origin().is_some_and(|origin| screen.contains(origin)))
}

/// Adds the last real run of the settling child `id` to the running pass, in place of running it.
pub(crate) fn replay_settled(ctx: &Context, id: Id) {
    let settled = with(ctx, |store, _, _| store.entries.get(&id).and_then(|entry| entry.settled.clone()));
    if let Some(settled) = settled {
        replay::replay(ctx, &settled.record);
    }
}

/// Keeps the entry of the child `id` this frame: a replayed run does not reach the children inside it.
pub(crate) fn keep(ctx: &Context, owner: Option<Id>, id: Id) {
    with(ctx, |store, _, frame| {
        store.touch(owner, id, frame);
    });
}

/// Keeps the real run `record` of the settling child `id`, which ran at `place` and took `rect`.
pub(crate) fn ran(ctx: &Context, id: Id, record: Record, place: Place, rect: Rect) {
    let targets = focus::targets(ctx);
    let engaged_now = record.focus.stop_ids().any(|stop| targets.contains(&Some(stop)));
    let mut settled = Settled { record, frame: 0, place, rect, engaged: engaged_now };
    settled.engaged |= engaged(ctx, &settled);
    with(ctx, |store, _, frame| {
        settled.frame = frame;
        if let Some(entry) = store.entries.get_mut(&id) {
            entry.settled = Some(Arc::new(settled));
        }
    });
}

/// Notes that the bound value `key` was written in this frame: a settling child that read it runs.
pub(crate) fn note_edit(ctx: &Context, key: BindKey) {
    with(ctx, |store, _, frame| {
        let edited = &mut store.changes.edited;
        if edited.0 != frame {
            *edited = (frame, HashSet::new());
        }
        edited.1.insert(key);
    });
}

/// Unsettles every settling child for this frame and the next: a change the framework cannot attribute to a child.
pub(crate) fn unsettle(ctx: &Context) {
    with(ctx, |store, _, frame| store.changes.unsettled_through = store.changes.unsettled_through.max(frame + 1));
}

/// Records the extent of a row of the collection `owner` that was built this pass. The collection places the rows it
/// does not build by it (its item layout answers for rows it never built).
pub(crate) fn remember(ctx: &Context, owner: Id, id: Id, size: Vec2) {
    shown(ctx, Some(owner), id, size, false, None);
}

/// Runs `f` with a reader of the extents the rows of the collection `owner` had in the last pass that built them, for
/// a collection that reads many entries at once.
pub(crate) fn reading<R>(ctx: &Context, owner: Id, f: impl FnOnce(&dyn Fn(Id) -> Option<Vec2>) -> R) -> R {
    with(ctx, |store, _, frame| {
        store.reached.insert(owner, frame);
        let store = std::cell::RefCell::new(store);
        f(&|id| {
            let mut store = store.borrow_mut();
            let entry = store.entries.get_mut(&id)?;
            entry.touched = frame;
            entry.sizes[1].or(entry.sizes[0])
        })
    })
}

/// Starts the real run that lays out the content of the scroll area `owner`: the children it places report to it.
pub(crate) fn begin(ctx: &Context, owner: Id) {
    with(ctx, |store, _, _| {
        store.placed.insert(owner, Placed::default());
    });
}

/// Reports, in the real run that lays out the content of `view`'s area, that the child `id` took `rect`, run or not.
pub(crate) fn place(ctx: &Context, view: &View, id: Id, rect: Rect) {
    let at = view.start_of(rect) - view.start_of(view.rect);
    with(ctx, |store, _, _| {
        if let Some(placed) = store.placed.get_mut(&view.owner) {
            placed.note((view.anchor, view.axes), id, at, rect.size());
            if view.reveal == Some(id) {
                placed.reveal = Some(at);
            }
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
