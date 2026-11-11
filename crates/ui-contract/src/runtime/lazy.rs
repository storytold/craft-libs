//! Out-of-view content: inside a scroll area, a child that is out of view does not run; it keeps what it last
//! showed: the extent it had in the last pass that showed it, at whatever room the scroll area has now, and the
//! stops it registered there, in their place in Tab order. This is the one previous-pass state of the run model,
//! kept for every user in one store: `contract::Lazy` children and the rows of a collection.

use crate::runtime::egui::pass;
use crate::runtime::focus::{self, Transcript};
use crate::runtime::frame::PerViewport;
use egui::{Context, Id, Rect, Vec2};
use std::collections::HashMap;
use std::sync::Arc;

/// The view of the innermost scroll area, as the run that lays its content out sees it: the rect the pass shows,
/// placed where this run puts the content (a measuring run lays it out from offset zero).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct View {
    /// The scroll area; children's entries are kept under it.
    pub(crate) owner: Id,
    pub(crate) rect: Rect,
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
    /// The last frame that read or wrote the entry.
    touched: u64,
}

#[derive(Default)]
struct Store {
    entries: HashMap<Id, Entry>,
    /// The frame the entries were last pruned in.
    pruned: u64,
}

fn with<R>(ctx: &Context, f: impl FnOnce(&mut Store, u64, u64) -> R) -> R {
    let viewport = pass::current_viewport(ctx);
    let (pass_nr, frame) = (pass::pass_nr(ctx, viewport), pass::frame_nr(ctx, viewport));
    PerViewport::<Store>::with(ctx, viewport, |store| {
        // A child no pass reached in the last frame is gone from its scroll area.
        if store.pruned != frame {
            store.pruned = frame;
            store.entries.retain(|_, entry| entry.touched + 1 >= frame);
        }
        f(store, pass_nr, frame)
    })
}

/// Decides whether the child `id`, whose content starts at `at` in this run, runs this pass. It runs when no run
/// showed it yet, when its extent placed at `at` meets the view, when one of its stops holds focus or is the target
/// of a focus request, or when `keep` asks for it. The first run of a pass decides; every later run of the pass gets the same answer, so a measuring
/// run and the real run lay out the same children.
pub(crate) fn decide(ctx: &Context, id: Id, view: &View, at: impl FnOnce(Vec2) -> Rect, sizing: bool, keep: bool) -> Decision {
    let targets = crate::runtime::focus::targets(ctx);
    with(ctx, |store, pass_nr, frame| {
        let entry = store.entries.entry(id).or_default();
        entry.touched = frame;
        let kind = usize::from(!sizing);
        // A child the other kind of run showed last takes that run's extent.
        let size = entry.sizes[kind].or(entry.sizes[1 - kind]);
        let skip = match entry.decided {
            Some((decided_in, skip)) if decided_in == pass_nr => skip,
            _ => {
                let focus = entry.focus.stop_ids().any(|stop| targets.contains(&Some(stop)));
                let skip = size.is_some_and(|size| !keep && !focus && !at(size).intersects(view.rect));
                entry.decided = Some((pass_nr, skip));
                skip
            }
        };
        match size {
            Some(size) if skip => Decision::Skip(size),
            _ => Decision::Run,
        }
    })
}

/// Records what a run that showed the child `id` found: its extent in a run of this kind and, from a real run, what
/// it registered with the focus service.
pub(crate) fn shown(ctx: &Context, id: Id, size: Vec2, sizing: bool, registered: Option<Transcript>) {
    with(ctx, |store, _, frame| {
        let entry = store.entries.entry(id).or_default();
        entry.touched = frame;
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

/// Records the extent of a collection row that was built this pass. The collection places the rows it does not
/// build by it (its item layout answers for rows it never built).
pub(crate) fn remember(ctx: &Context, id: Id, size: Vec2) {
    shown(ctx, id, size, false, None);
}

/// Runs `f` with a reader of the extents rows had in the last pass that built them, for a collection that reads
/// many entries at once.
pub(crate) fn reading<R>(ctx: &Context, f: impl FnOnce(&dyn Fn(Id) -> Option<Vec2>) -> R) -> R {
    with(ctx, |store, _, frame| {
        let store = std::cell::RefCell::new(store);
        f(&|id| {
            let mut store = store.borrow_mut();
            let entry = store.entries.get_mut(&id)?;
            entry.touched = frame;
            entry.sizes[1].or(entry.sizes[0])
        })
    })
}
