//! Viewport mounts: opening a mount's OS viewport and its close requests.

use super::native::{WindowAction, act, route_hit_test};
use super::placement::{record_placement, saved_placement};
use super::shell::{os_of, refresh};
use super::spec::{WindowSpec, frame_of_spec};
use super::{Record, refuse_close_under_modal, shared};
use crate::runtime::egui::viewports;
use crate::runtime::frame::PerViewport;
use craft_host::target::{Os, WindowChrome, WindowKind};
use egui::{Context, Id, Pos2, Rect, Ui, Vec2, ViewportId};

/// What a viewport mount asks for.
pub(crate) struct ViewportOpen<'a> {
    /// The container's id.
    pub id: Id,
    /// The window title.
    pub title: &'a str,
    /// The size to open with when nothing is saved.
    pub size: Option<Vec2>,
    /// The key the placement is saved under.
    pub save: Option<&'a str>,
    /// How the window opens.
    pub spec: &'a WindowSpec,
    /// The smallest and largest inner size, from the container's overlay.
    pub limits: (Option<Vec2>, Option<Vec2>),
    /// The outer position in desktop points when nothing is saved, from the container's overlay.
    pub position: Option<Pos2>,
    /// Whether the user can resize the window, from the container's overlay.
    pub resizable: bool,
}

/// Opens the OS viewport of a mount and runs `body` once for each pass it gets: with the saved placement, the OS
/// close request turned into a request to the container, and the placement saved after the pass.
///
/// The window opens at the caller's place (saved, placed or measured). From then on the window has two writers, the
/// OS and the caller, and only one writes at a time: a caller's place goes out as a command only where it differs
/// from the place the window is known at ([`keep_placed`]), and the OS's moves and resizes become the known place
/// that the caller records. A caller that follows the window so never sends it its own place back a pass late.
pub(crate) fn show_viewport(ctx: &Context, open: &ViewportOpen<'_>, mut body: impl FnMut(&mut Ui)) {
    let viewport = viewports::viewport_id(open.id);
    let saved = open.save.and_then(|key| saved_placement(ctx, key));
    let (size, pos) = match &saved {
        Some(placement) => (Some(Vec2::from(placement.normal.size)), placement.position().map(Pos2::from)),
        None => (open.size, None),
    };
    let pos = pos.or(open.position);
    let opened = PerViewport::<Record>::with(ctx, viewport, |record| record.seen.is_some());
    if opened {
        keep_placed(ctx, viewport, pos, size);
    }
    let spec = open.spec;
    let active = !shared(ctx, |shared| shared.away);
    let os = os_of(ctx);
    let (chrome, captions) = frame_of_spec(ctx, spec);
    let level = PerViewport::<Record>::with(ctx, viewport, |record| {
        record.kind = Some(spec.kind);
        record.chrome = Some(chrome);
        record.captions = captions.clone();
        record.resizable = Some(open.resizable);
        record.awaiting_first_paint |= spec.shown_after_first_paint && record.facts.is_none();
        if record.title.as_deref() != Some(open.title) {
            record.title = Some(open.title.to_owned());
            // A routed window keeps its handle: the title the system shows changes after this pass.
            if record.sink.is_none() {
                record.native = None;
                record.searched = None;
            }
        }
        record.level.get_or_insert(spec.level).for_egui(active, os)
    });
    let opens = viewports::Spec {
        title: open.title,
        // The builder carries the place only until the window opens: egui would send each change of it as a command.
        size: size.filter(|_| !opened),
        pos: pos.filter(|_| !opened),
        chrome,
        // A tool window has no OS caption buttons; its panel's title row carries a close action.
        os_buttons: !(spec.kind == WindowKind::Tool && captions.is_none() && chrome != WindowChrome::Native),
        min_size: open.limits.0,
        max_size: open.limits.1,
        level,
        resizable: open.resizable,
        taskbar: spec.taskbar,
        hidden: PerViewport::<Record>::with(ctx, viewport, |record| record.awaiting_first_paint),
    };
    viewports::show_immediate(ctx, viewport, &opens, |ui| {
        let ctx = ui.ctx().clone();
        let facts = viewports::current(&ctx);
        refresh(&ctx, viewport, facts);
        note_seen(&ctx, viewport, &facts);
        if viewports::current(&ctx).close_requested && !refuse_close_under_modal(&ctx) {
            viewports::cancel_close(&ctx);
            shared(&ctx, |shared| {
                if !shared.close_requests.contains(&open.id) {
                    shared.close_requests.push(open.id);
                }
            });
        }
        body(ui);
        if chrome == WindowChrome::Integrated && os == Os::Windows {
            route_hit_test(&ctx, viewport);
        }
        if let Some(key) = open.save {
            record_placement(&ctx, viewport, key, &at_known(&ctx, viewport, viewports::current(&ctx)));
        }
    });
    if PerViewport::<Record>::with(ctx, viewport, |record| std::mem::take(&mut record.awaiting_first_paint)) {
        viewports::show(ctx, viewport);
    }
}

/// Whether the OS asked the viewport of container `id` to close in this root frame; once.
pub(crate) fn take_close_request(ctx: &Context, id: Id) -> bool {
    shared(ctx, |shared| {
        let before = shared.close_requests.len();
        shared.close_requests.retain(|requested| *requested != id);
        shared.close_requests.len() != before
    })
}

/// How far a caller's place may differ from the known place before it is a change: the OS rounds to pixels, and a
/// placement records the position in pixels.
const ROUNDING: f32 = 0.5;

/// The place of a window with `facts`: its outer top-left corner and its inner size.
fn place_of(facts: &viewports::Facts) -> Option<Rect> {
    let inner = facts.inner?;
    Some(Rect::from_min_size(facts.outer.unwrap_or(inner).min, inner.size()))
}

/// In the window's own pass: a place the window reports that differs from the one it reported before is its known
/// place, as its caller reads it in this pass.
fn note_seen(ctx: &Context, viewport: ViewportId, facts: &viewports::Facts) {
    let Some(place) = place_of(facts) else { return };
    PerViewport::<Record>::with(ctx, viewport, |record| {
        if record.seen != Some(place) {
            record.seen = Some(place);
            record.known = Some(place);
        }
    });
}

/// Sends the window to the caller's position `pos` and inner size `size` where they differ from its known place by
/// more than rounding. Nothing goes out while a move gesture moves the window (it sends the window's place itself),
/// nor while the window is minimized, maximized or full screen, when a caller keeps its normal place.
fn keep_placed(ctx: &Context, viewport: ViewportId, pos: Option<Pos2>, size: Option<Vec2>) {
    let (known, held) = PerViewport::<Record>::with(ctx, viewport, |record| {
        let state = record.facts.is_some_and(|facts| facts.minimized || facts.maximized || facts.fullscreen);
        (record.known, state || record.moved_pointer.is_some())
    });
    let Some(known) = known.filter(|_| !held) else { return };
    let differs = |delta: Vec2| delta.x.abs() > ROUNDING || delta.y.abs() > ROUNDING;
    if let Some(pos) = pos.filter(|pos| differs(*pos - known.min)) {
        act(ctx, viewport, WindowAction::SetOuterPosition(pos));
    }
    if let Some(size) = size.filter(|size| differs(*size - known.size())) {
        act(ctx, viewport, WindowAction::SetInnerSize(size));
    }
}

/// `facts` with the window at its known place, where it has one: the place its caller records.
pub(super) fn at_known(ctx: &Context, viewport: ViewportId, facts: viewports::Facts) -> viewports::Facts {
    let known = PerViewport::<Record>::with(ctx, viewport, |record| record.known);
    let (Some(known), Some(inner)) = (known, facts.inner) else { return facts };
    let outer = facts.outer.unwrap_or(inner);
    let shift = known.min - outer.min;
    let grow = known.size() - inner.size();
    viewports::Facts {
        inner: Some(Rect::from_min_size(inner.min + shift, known.size())),
        outer: facts.outer.map(|outer| Rect::from_min_size(outer.min + shift, outer.size() + grow)),
        ..facts
    }
}
