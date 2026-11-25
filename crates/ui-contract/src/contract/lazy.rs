//! `Lazy`: a child of a scroll area that runs only while it is in view, and the runner a memoized view shares with it
//! (`runtime::lazy`).

use crate::layout::{flow_region, take_cell, take_in_flow};
use crate::runtime::egui::alloc;
use crate::runtime::env::{Env, Shrink};
use crate::runtime::focus;
use crate::runtime::layout_memory;
use crate::runtime::lazy::{self, Call, Decision, Key, Place};
#[cfg(feature = "testing")]
use crate::runtime::measure::{MemoRun, count_memo_run};
use crate::runtime::memory::sub;
use crate::runtime::replay;
use craft_behavior::Axis;
use egui::{Pos2, Rect, Ui, UiBuilder, Vec2};
use std::hash::Hash;

/// A child of a scroll area that runs only while it is in view. Out of view its content does not run: it takes the
/// extent it had in the last pass that showed it, at any room, and its stops from that pass keep their place in Tab
/// order. A child no pass showed yet takes its estimate: that long along the axes the area scrolls and nothing across
/// them, as a collection places the rows it never built by its item layout's estimate. It runs when that extent
/// meets the view, and while one of the stops of its last run holds keyboard focus or is the target of a focus
/// request. Content that was never shown has no stops to find: Tab reaches it once it was shown, and an app brings
/// it into view with `reveal`. Outside a scroll area it always runs.
///
/// It is a child in flow of its body. In a `Grid` or spread `Row` body it holds one cell: its content is the one child
/// in flow that takes the cell, and out of view it takes the cell at its remembered extent, so the cell's row counts
/// it.
///
/// The key names the child among every `Lazy` of its scroll area, nested containers included. The idiom is
/// `Lazy::new(key, estimate).show(ui, |ui| ..)` around each large child (a card, a section of a panel) in the scroll
/// body.
///
/// A memoized view (`MemoView`) is a `Lazy` child keyed by its props: it also keeps what its last run painted and
/// registered, and while its key and the framework inputs it read are unchanged it adds that again in place of running,
/// in view or not.
#[must_use]
pub struct Lazy {
    key: u64,
    estimate: f32,
    reveal: bool,
}

impl Lazy {
    /// The child keyed `key` in its scroll area. `estimate` is how long it is along the axes the area scrolls, an
    /// app's guess from what it knows without running the child; the child's real extent replaces it once it ran.
    pub fn new(key: impl Hash, estimate: f32) -> Lazy {
        use std::hash::Hasher;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        Lazy { key: hasher.finish(), estimate, reveal: false }
    }

    /// When `reveal` holds, the scroll area glides until the child starts at the view's start (as far as the
    /// content lets it), whether or not the child ran yet: a request of the pass that sets it, as an app's jump to
    /// an item is. A wheel, drag or other move of the area ends the glide.
    pub fn reveal(mut self, reveal: bool) -> Lazy {
        self.reveal = reveal;
        self
    }

    /// Runs `add` in a child `Ui` placed in flow in `ui`, or takes the remembered extent there; `None` when it did not
    /// run.
    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
        match self.run_keyed(ui, "Lazy", Key::None, |ui, _| add(ui)) {
            Shown::Ran(out) => Some(out),
            Shown::Replayed(..) | Shown::OutOfView => None,
        }
    }

    /// The id of the child in `ui`: under its scroll area, or under `ui`'s scope outside every scroll area.
    pub(super) fn id(&self, ui: &Ui) -> egui::Id {
        match Env::of(ui).view().lazy_view() {
            Some(view) => sub(view.owner, ("lazy", self.key)),
            None => sub(ui.scope_id(), ("lazy", self.key)),
        }
    }

    /// The one runner of a `Lazy` child and a memoized view (`MemoView`), named `name` in the count trials: runs `add`,
    /// replays the kept run of a memoized view keyed `key`, or takes the extent of a child out of view.
    pub(super) fn run_keyed<R>(self, ui: &mut Ui, name: &'static str, key: Key, add: impl FnOnce(&mut Ui, &mut Run) -> R) -> Shown<R> {
        let ctx = ui.ctx().clone();
        let env = Env::of(ui);
        let view = env.view().lazy_view();
        let id = self.id(ui);
        let rtl = ui.layout().prefer_right_to_left();
        let memo = matches!(key, Key::Memo(_));
        let sizing = ui.is_sizing_pass();
        // The child is not part of its body's transcript: in a real run it counts there by its extent, whether it
        // runs or not, so what scrolls into reach does not change the body's content.
        let real = !sizing && !crate::runtime::measure::active(&ctx);
        // A memoized view whose content filled the axis the body around it shares is one of the children that fill
        // that body: a real run shows it in its share, as the body gives a child that fills it directly.
        let share = env
            .shares()
            .filter(|axis| memo && real && lazy::fills(&ctx, id, sizing) == Some(*axis))
            .and_then(|axis| crate::layout::fill_share(ui, axis).map(|share| (axis, share)));
        // A memoized view is one unit of the shrinking row around it. A row that narrows it gives it at most the width
        // its measuring run settled on, the room its place then holds, so a new width runs it. A view the row does not
        // narrow shows as in no room, in the row's flow region: its room does not follow its own content's width, so a
        // new content runs it once and the discard pass its new size asks for replays that run. A view that fills the
        // row takes no part in its shrink step, as a body that fills it does not.
        let shrink = env.shrink.filter(|_| memo);
        let cap = match shrink {
            Some(Shrink::Apply(row)) if share.is_none() => crate::layout::shrink_cap(&ctx, row, id).filter(|cap| cap.narrowed),
            _ => None,
        };
        let available = match share {
            Some((axis, share)) => share_of(flow_region(ui), rtl, axis, share),
            None => cap.map_or_else(|| flow_region(ui), |cap| at_most(flow_region(ui), rtl, cap.width)),
        };
        let available = if memo { on_row_line(ui, id, available) } else { available };
        let owner = view.map(|view| view.owner);
        // A run around this one that is replayed later keeps the child's entry.
        replay::journal(&ctx, || (owner, id), |ctx, &(owner, id)| lazy::keep(ctx, owner, id));
        if let Some(view) = view {
            // A run around this child decides it by the scroll view: it replays only while the view is the same.
            replay::note_input(&ctx, crate::runtime::inputs::Input::Scroll);
            let at = |size: Vec2| Rect::from_min_size(at_start(available, rtl, size), size);
            let estimate = view.estimate(self.estimate);
            if self.reveal {
                crate::contract::queue_child_reveal(&ctx, view.owner, id);
            }
            if let Decision::Skip(size) = lazy::decide(&ctx, id, &view, at, sizing, estimate) {
                let (took, _) = take_in_flow(ui, size);
                if real {
                    layout_memory::backstop(&ctx, id, size);
                    lazy::replay(&ctx, id);
                    lazy::place(&ctx, &view, id, took);
                }
                #[cfg(feature = "testing")]
                count_memo_run(&ctx, name, id, MemoRun::OutOfView, real);
                return Shown::OutOfView;
            }
        }
        let layout = *ui.layout();
        let place = Place {
            room: available,
            clip: ui.clip_rect(),
            layer: ui.layer_id(),
            layout,
            pixels_per_point: ctx.pixels_per_point(),
            atlas: ctx.fonts(|fonts| fonts.atlas_generation()),
            opacity: ui.opacity(),
            env: env.key,
            flow: env.flow(),
        };
        if real && memo {
            crate::runtime::memory::claim_id(&ctx, id, "two memoized views share an id in one pass");
        }
        let call = if real { Call::Real(place) } else { Call::Measuring { room: sizing.then_some(available), flow: place.flow } };
        // Scroll owners with queued moves wake the settled regions holding them, as routed intents do.
        let settled = if memo {
            let moving = crate::contract::steps::scroll_owner::move_targets(&ctx);
            lazy::settled(&ctx, owner, id, call, view, key, &moving)
        } else {
            None
        };
        // The replay audit runs a view that would replay for real in place of the replay, and compares the two.
        let audited = settled.as_ref().filter(|_| audits(&ctx, real)).map(|(_, _, shift)| *shift);
        if let Some((size, ran_at, shift)) = settled
            && audited.is_none()
        {
            // A real replay is where the run's content went, moved with the room.
            let rect = if real { ran_at } else { Rect::from_min_size(at_start(available, rtl, size), size) };
            let fills = if memo && !real { lazy::fills(&ctx, id, sizing) } else { None };
            let took = place_unit(ui, (shrink, fills), id, rect, rtl);
            let kept = if real {
                layout_memory::backstop(&ctx, id, size);
                let kept = lazy::replay_settled(&ctx, id, &shift);
                if let Some(view) = view {
                    lazy::place(&ctx, &view, id, took);
                }
                kept
            } else {
                // The kept real run's output, laid out in that run's room and flow: a caller that measures sizes from
                // the extent this call takes, never from rects in it.
                lazy::kept_output(&ctx, id)
            };
            #[cfg(feature = "testing")]
            count_memo_run(&ctx, name, id, MemoRun::Replayed, real);
            return Shown::Replayed(kept, shift);
        }
        let mark = focus::mark(&ctx);
        let mut run = Run {
            recording: memo && real,
            output: None,
            #[cfg(feature = "testing")]
            same_output: None,
        };
        let edits = if run.recording { lazy::edits(&ctx) } else { 0 };
        let seen = if run.recording { crate::runtime::inputs::seen(&ctx) } else { Default::default() };
        if run.recording {
            open_record(&ctx, audited.is_some());
        }
        if memo {
            crate::layout::open_view_fill(&ctx, id);
        }
        let mut child = alloc::child(ui, UiBuilder::new().max_rect(available).layout(layout).scope_id(id));
        // Its extent in the body around it comes from its content's measure, so a change inside reaches that body. The
        // content runs in the env of `ui`, as the direct children of the body around it: a plain `Lazy` keeps their
        // line, cells and shrink step, and a memoized view starts its own room on top of it.
        let out = layout_memory::boundary(&ctx, false, || Env::run_in(&mut child, env, |child| add(child, &mut run)));
        let rect = child.min_rect();
        // The child `Ui` writes its widget rect when it drops: the record holds it as written.
        drop(child);
        let fills = if memo { crate::layout::take_view_fill(&ctx, id) } else { None };
        let record = run.recording.then(|| replay::close(&ctx, place.layer));
        if let Some(shift) = audited {
            return place_audit_run(ui, Audited { name, id, view, rect, record: record.as_ref(), shift }, out, &run);
        }
        #[cfg(feature = "testing")]
        if let Some(record) = &record
            && crate::runtime::audit::on(&ctx)
        {
            crate::runtime::audit::name_callback(&ctx, name, record);
        }
        if real {
            layout_memory::backstop(&ctx, id, rect.size());
        }
        if memo {
            place_unit(ui, (shrink, fills.filter(|_| !real)), id, rect, rtl);
        } else {
            crate::layout::close_flow_start_in(ui);
            alloc::advance_past(ui, rect);
        }
        if view.is_some() || memo {
            let registered = real.then(|| record.as_ref().map_or_else(|| focus::transcript(&ctx, mark), |record| record.focus.clone()));
            lazy::shown(&ctx, owner, id, sizing, lazy::Measured { size: rect.size(), key, room: available, flow: place.flow, fills }, registered);
        }
        if let Some(view) = view
            && real
        {
            lazy::place(&ctx, &view, id, rect);
        }
        if let Some(record) = record {
            // The key from before the run: a run that wrote through its props moved the key, so the view runs once more,
            // from the written value, before it replays.
            lazy::ran(&ctx, id, record, place, rect, lazy::Ran { key, edits, seen, view, output: run.output.take() });
        }
        #[cfg(feature = "testing")]
        if memo {
            count_memo_run(&ctx, name, id, MemoRun::Ran, real);
        }
        // The name is read by the count trials only.
        let _ = name;
        Shown::Ran(out)
    }
}

/// Places the audit run of a view that would have replayed, as a replay places it, and compares the run with the kept
/// run replayed at the place `shift` names (`runtime::audit`). It keeps the kept run: every later pass compares with
/// what a replay adds.
fn place_audit_run<R>(ui: &mut Ui, audited: Audited<'_>, out: R, run: &Run) -> Shown<R> {
    let Audited { name, id, view, rect, record, shift } = audited;
    let ctx = ui.ctx().clone();
    layout_memory::backstop(&ctx, id, rect.size());
    place_view(ui, rect);
    if let Some(view) = view {
        lazy::place(&ctx, &view, id, rect);
    }
    #[cfg(feature = "testing")]
    {
        if let Some(record) = record {
            lazy::audit(&ctx, name, id, crate::runtime::audit::Shown { record, rect, output: run.output.as_ref() }, run.same_output, &shift);
        }
        count_memo_run(&ctx, name, id, MemoRun::Replayed, true);
    }
    let _ = (name, record, run, shift);
    Shown::Ran(out)
}

/// What the runner hands its audit step.
struct Audited<'a> {
    name: &'static str,
    id: egui::Id,
    view: Option<lazy::View>,
    rect: Rect,
    record: Option<&'a replay::Record>,
    /// Where the replay would have added the kept run.
    shift: egui::RecordShift,
}

/// Whether the replay audit runs a child that would replay in a pass whose runs are `real` (test builds only). A pass
/// egui will discard paints nothing, so a run there is no shown run to compare with: the child replays.
fn audits(ctx: &egui::Context, real: bool) -> bool {
    #[cfg(feature = "testing")]
    return real && !ctx.will_discard() && crate::runtime::audit::runs(ctx);
    #[cfg(not(feature = "testing"))]
    {
        let _ = (ctx, real);
        false
    }
}

/// Opens the record around a recording run, an audit run's when `audited`.
fn open_record(ctx: &egui::Context, audited: bool) {
    #[cfg(feature = "testing")]
    if audited {
        replay::open_audit(ctx);
        return;
    }
    let _ = audited;
    replay::open(ctx);
}

/// Where content of `size` starts in `region`: at its start edge, the right one in a right-to-left layout.
fn at_start(region: Rect, rtl: bool, size: Vec2) -> Pos2 {
    Pos2::new(if rtl { region.max.x - size.x } else { region.min.x }, region.min.y)
}

/// `region` cut to `width` at most, from its start edge.
fn at_most(region: Rect, rtl: bool, width: f32) -> Rect {
    let size = Vec2::new(width.min(region.width()), region.height());
    Rect::from_min_size(at_start(region, rtl, size), size)
}

/// `region` cut to `share` at most along `axis`, from its start edge: the room of a memoized view that fills the body
/// around it.
fn share_of(region: Rect, rtl: bool, axis: Axis, share: f32) -> Rect {
    match axis {
        Axis::X => at_most(region, rtl, share),
        _ => Rect::from_min_size(region.min, Vec2::new(region.width(), share.min(region.height()))),
    }
}

/// Places the extent `rect` of the memoized view `id` in flow in `ui` (`place_view`) as one unit of the body around it,
/// and returns the rect it took. `fills` is the axis the body shares which the view's content filled in a measuring
/// run: the view is then one of the children that fill the body, as a child shown there directly is. It takes its
/// share along the axis once the body knows it; in the first sweep of the body's measuring run it is counted and takes
/// nothing there (`counting_fills`), outside the least measure, where it takes what its content took. In a shrinking
/// row it is a unit that does not narrow below what it takes, as a body that fills the row is.
///
/// Any other view reports its extent to the shrinking row around it: what it takes, in the row's counting sweep, and
/// its least, in the sweep that asks the row's body units for theirs. Like a body unit, it narrows from what it takes
/// in no room down to what it takes with every leaf at its least.
fn place_unit(ui: &mut Ui, (shrink, fills): (Option<Shrink>, Option<Axis>), id: egui::Id, rect: Rect, rtl: bool) -> Rect {
    let ctx = ui.ctx().clone();
    let env = Env::of(ui);
    let Some(axis) = fills.filter(|axis| env.shares() == Some(*axis)) else {
        let took = place_view(ui, rect);
        match shrink {
            Some(Shrink::Count(row)) => crate::layout::report_shrink(&ctx, row, id, rect.width(), rect.width(), None, (false, None)),
            Some(Shrink::Least(row)) => crate::layout::report_least(&ctx, row, id, rect.width()),
            _ => {}
        }
        return took;
    };
    let content = rect.size();
    let along = match crate::layout::fill_share(ui, axis) {
        Some(share) => Some(share),
        None if crate::layout::counting_fills(ui, axis) && !env.least => {
            crate::layout::count_fill_view(ui);
            Some(0.0)
        }
        None => None,
    };
    let size = match (axis, along) {
        (Axis::X, Some(along)) => Vec2::new(along, content.y),
        (_, Some(along)) => Vec2::new(content.x, along),
        (_, None) => content,
    };
    let took = place_view(ui, Rect::from_min_size(at_start(rect, rtl, size), size));
    if let (Axis::X, Some(Shrink::Count(row))) = (axis, shrink) {
        crate::layout::report_shrink(&ctx, row, id, size.x, size.x, Some((size.x, size.x)), (false, None));
    }
    took
}

/// Places a memoized view's extent `rect` in flow in `ui`, as its run and its replay both do, and returns `rect`: in the
/// `Grid` or spread `Row` body that encloses `ui` directly the view takes the next cell at its extent, as its content
/// starts a layout room of its own and takes none; elsewhere the flow moves past `rect`.
fn place_view(ui: &mut Ui, rect: Rect) -> Rect {
    if take_cell(ui, rect.size()).is_none() {
        crate::layout::close_flow_start_in(ui);
        alloc::advance_past(ui, rect);
    }
    rect
}

/// The room of the memoized view `id` in the flow region `room` of `ui`. Its content starts a layout room of its own
/// (`Env::start_room`), so it would sit at the top of the unwrapped `Row` around it; as a child placed in flow
/// (`place_in_cell`), a view of one line sits with its centre on the row's line instead: its room moves down by the
/// whole pixels that put the centre of the extent it took (in this pass's measuring run, else its last run) there. A
/// view taller than that reaches, and a view no run showed yet, stays at the row's top. The room keeps its size, so a
/// kept run replays moved with it.
fn on_row_line(ui: &Ui, id: egui::Id, room: Rect) -> Rect {
    let Some(centre) = Env::of(ui).row_line.and_then(|line| line.centre) else {
        return room;
    };
    let Some(size) = lazy::last_extent(ui.ctx(), id) else {
        return room;
    };
    let pixels_per_point = ui.ctx().pixels_per_point();
    let above = ((centre - size.y / 2.0) * pixels_per_point).round() / pixels_per_point;
    if above.is_finite() && above > 0.0 { room.translate(Vec2::new(0.0, above)) } else { room }
}

/// What a child came to in a pass.
pub(super) enum Shown<R> {
    /// It ran and returned this.
    Ran(R),
    /// It took the extent of its kept run, and added that run again in a real run; with what a memoized view's run
    /// returned, as the run left it, and the shift the replay moved the run by (none in a measuring run).
    Replayed(Option<lazy::Output>, egui::RecordShift),
    /// It is out of view in its scroll area.
    OutOfView,
}

/// The run of a child: whether it records what it adds for a replay.
pub(super) struct Run {
    recording: bool,
    /// What a memoized view's run returned, which its replays return.
    pub(super) output: Option<lazy::Output>,
    /// Whether two outputs of a memoized view are equal, for the replay audit.
    #[cfg(feature = "testing")]
    pub(super) same_output: Option<crate::runtime::audit::SameOutput>,
}
