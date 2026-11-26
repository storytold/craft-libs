//! Viewport mounts: opening a mount's OS viewport and its close requests.

use super::host::monitors;
use super::native::{WindowAction, act, route_hit_test};
use super::placement::{record_placement, saved_placement};
use super::shell::{app_id, base_shell, os_of, refresh};
use super::spec::{WindowLevel, WindowSpec, frame_of_spec};
use super::{Record, refuse_close_under_modal, shared};
use crate::runtime::egui::viewports;
use crate::runtime::frame::PerViewport;
use craft_behavior::placement::{Defaults, place};
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
    /// Whether the caller keeps the window at `size` once it opened: a placed window or one sized by its content does;
    /// a resizable window's size is the user's.
    pub follows_size: bool,
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
    /// Whether the app moves the window from its move band, from the container's overlay (`Overlay::moved_by_app`).
    pub moved_by_app: bool,
}

/// Opens the OS viewport of a mount and runs `body` once for each pass it gets: with the saved placement, the OS
/// close request turned into a request to the container, and the placement saved after the pass.
///
/// The window opens at the caller's place (saved, placed or measured). From then on the window has two writers, the
/// OS and the caller, and only one writes at a time: a caller's place goes out as a command only where it differs
/// from the place the window is known at ([`keep_placed`]), and the OS's moves and resizes become the known place
/// that the caller records. A caller that follows the window so never sends it its own place back a pass late. A
/// resizable window's size goes out only to open it (`ViewportOpen::follows_size`).
pub(crate) fn show_viewport(ctx: &Context, open: &ViewportOpen<'_>, mut body: impl FnMut(&mut Ui)) {
    let viewport = viewports::viewport_id(open.id);
    let opened = PerViewport::<Record>::with(ctx, viewport, |record| record.seen.is_some());
    let (size, pos) = saved_geometry(ctx, open, opened).map_or((open.size, None), |(size, pos)| (Some(size), pos));
    let pos = pos.or(open.position);
    if opened {
        keep_placed(ctx, viewport, pos, size.filter(|_| open.follows_size));
    }
    let spec = open.spec;
    let cancel_to = spec.hands_on_cancel.then(|| crate::runtime::egui::pass::current_viewport(ctx));
    let state = shared(ctx, |shared| shared.app);
    let os = os_of(ctx);
    let (chrome, captions) = frame_of_spec(ctx, spec);
    let standing = PerViewport::<Record>::with(ctx, viewport, |record| {
        record.kind = Some(spec.kind);
        record.chrome = Some(chrome);
        record.captions = captions.clone();
        record.resizable = Some(open.resizable);
        record.min_size = open.limits.0;
        record.moved_by_app = open.moved_by_app;
        record.cancel_to = cancel_to;
        record.awaiting_first_paint |= spec.shown_after_first_paint && record.facts.is_none();
        if record.title.as_deref() != Some(open.title) {
            record.title = Some(open.title.to_owned());
            // A routed window keeps its handle: the title the system shows changes after this pass.
            if record.sink.is_none() {
                record.native = None;
                record.searched = None;
            }
        }
        record.level.get_or_insert(spec.level).standing(viewport, state, os)
    });
    if standing.lowered {
        shared(ctx, |shared| shared.lowered = true);
    }
    // A palette also yields to an app-modal or popup layer of the root window: hidden while one shows.
    let yields = shared(ctx, |shared| shared.palettes_yield)
        && PerViewport::<Record>::with(ctx, viewport, |record| record.level == Some(WindowLevel::Palette));
    let hidden = !standing.shown || yields;
    let app = app_id(ctx);
    let opens = viewports::Spec {
        title: open.title,
        // The builder carries the place only until the window opens: egui would send each change of it as a command.
        size: size.filter(|_| !opened),
        pos: pos.filter(|_| !opened),
        chrome,
        // A tool window has no OS caption buttons; its panel's title row carries a close action.
        os_buttons: !(spec.kind == WindowKind::Tool && captions.is_none() && chrome != WindowChrome::Native),
        min_size: open.limits.0,
        // egui sends a limit only when it has one: an open window always carries its maximum, unbounded when the
        // caller has none, so a lifted maximum reaches the OS.
        max_size: open.limits.1.or(opened.then_some(Vec2::INFINITY)),
        level: standing.level,
        resizable: open.resizable,
        taskbar: spec.taskbar,
        // The builder's visibility opens a window the app's state hides already hidden, and only opens it: egui would
        // send each later change of it as a command beside the one below.
        hidden: PerViewport::<Record>::with(ctx, viewport, |record| record.awaiting_first_paint || (hidden && record.facts.is_none())),
        active: spec.takes_focus,
        passes_pointer: spec.passes_pointer,
        app_id: &app,
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
    // A window opened hidden appears once its first pass ran; a window the app's state hides goes and comes back with
    // it. The builder's visibility only opens the window: egui sends no change of it once the window is open.
    let (reveal, changed) = PerViewport::<Record>::with(ctx, viewport, |record| {
        let first = std::mem::take(&mut record.awaiting_first_paint);
        let changed = std::mem::replace(&mut record.hidden, hidden) != hidden;
        (first || changed, changed)
    });
    if hidden && changed {
        viewports::set_visible(ctx, viewport, false);
    } else if !hidden && reveal {
        viewports::set_visible(ctx, viewport, true);
    }
}

/// What a layer shown in a popup window asks for ([`show_popup`]).
pub(crate) struct PopupOpen<'a> {
    /// The layer's id; the popup's viewport derives from it.
    pub id: Id,
    /// How the window opens: a [`WindowSpec::popup`].
    pub spec: &'a WindowSpec,
    /// The window's place in desktop points (outer top-left corner and inner size) when the layer's size is known
    /// before its pass; otherwise the caller places it after the pass ([`place_popup`]).
    pub rect: Option<Rect>,
}

/// Opens the popup window of a layer opened in the running viewport (its opener) and runs `body` in its pass. The
/// window is a viewport mount without a container: it opens at `open.rect` and follows it, and the opener gets the OS
/// focus back when it closes ([`close_popup`]).
pub(crate) fn show_popup(ctx: &Context, open: &PopupOpen<'_>, body: impl FnMut(&mut Ui)) {
    let opener = crate::runtime::egui::pass::current_viewport(ctx);
    PerViewport::<Record>::with(ctx, viewports::viewport_id(open.id), |record| record.opener = Some(opener));
    let mount = ViewportOpen {
        id: open.id,
        title: "",
        size: open.rect.map(|rect| rect.size()),
        follows_size: true,
        save: None,
        spec: open.spec,
        limits: (None, None),
        position: open.rect.map(|rect| rect.min),
        resizable: false,
        moved_by_app: false,
    };
    show_viewport(ctx, &mount, body);
}

/// Sends the popup window of layer `id` to `rect` (desktop points) where it is not there already: a layer whose size
/// its pass measured is placed once that pass ran.
pub(crate) fn place_popup(ctx: &Context, id: Id, rect: Rect) {
    let viewport = viewports::viewport_id(id);
    let known = PerViewport::<Record>::with(ctx, viewport, |record| record.known);
    if known.is_none_or(|known| differs(ctx, viewport, rect.min - known.min)) {
        act(ctx, viewport, WindowAction::SetOuterPosition(rect.min));
    }
    if known.is_none_or(|known| differs(ctx, viewport, rect.size() - known.size())) {
        act(ctx, viewport, WindowAction::SetInnerSize(rect.size()));
    }
    PerViewport::<Record>::with(ctx, viewport, |record| record.known = Some(rect));
}

/// The layer `id` is hidden: when it showed in a popup window that still has the OS focus, the window it was opened
/// from takes the focus back. Once per close; a layer that never showed in a popup does nothing.
pub(crate) fn close_popup(ctx: &Context, id: Id) {
    let viewport = viewports::viewport_id(id);
    let closed = PerViewport::<Record>::with_existing(ctx, viewport, |record| {
        let focused = record.facts.is_some_and(|facts| facts.focus_reported);
        record.opener.take().filter(|_| focused)
    });
    if let Some(opener) = closed.flatten() {
        act(ctx, opener, WindowAction::Focus);
    }
}

/// Fits saved geometry to the current display layout once, before the viewport opens.
fn saved_geometry(ctx: &Context, open: &ViewportOpen<'_>, opened: bool) -> Option<(Vec2, Option<Pos2>)> {
    let saved = saved_placement(ctx, open.save?)?.repaired()?;
    let positions = base_shell(ctx).window_positions();
    let placement = if opened {
        saved
    } else {
        let min = open.limits.0.map_or([1.0; 2], Into::into);
        let defaults = Defaults::new(saved.normal.size, min).with_no_position(!positions);
        place(Some(&saved), &monitors(ctx), None, &defaults)
    };
    let pos = placement.position().filter(|_| positions).map(Pos2::from);
    Some((placement.normal.size.into(), pos))
}

/// Whether the OS asked the viewport of container `id` to close in this root frame; once.
pub(crate) fn take_close_request(ctx: &Context, id: Id) -> bool {
    shared(ctx, |shared| {
        let before = shared.close_requests.len();
        shared.close_requests.retain(|requested| *requested != id);
        shared.close_requests.len() != before
    })
}

/// Whether container `id` shows in an OS window that has not reported its place yet: its body lays out in room the OS
/// has not given, so what it takes there is no size of its content.
pub(crate) fn unplaced(ctx: &Context, id: Id) -> bool {
    super::mount_window(ctx, id)
        && PerViewport::<Record>::with(ctx, viewports::viewport_id(id), |record| record.kind.is_some() && record.seen.is_none())
}

/// How far a caller's place may differ from the known place before it is a change, in physical pixels of the window's
/// display: the OS rounds to pixels, and a placement records the position in pixels.
const ROUNDING_PX: f32 = 0.5;

/// Whether `delta`, in points, moves the window `viewport` by more than rounding on its display: a whole pixel on
/// a 2x display is half a point.
fn differs(ctx: &Context, viewport: ViewportId, delta: Vec2) -> bool {
    let facts = PerViewport::<Record>::with(ctx, viewport, |record| record.facts);
    let scale = facts.and_then(|facts| super::host::scale_of(ctx, &facts)).unwrap_or(1.0);
    delta.x.abs() * scale > ROUNDING_PX || delta.y.abs() * scale > ROUNDING_PX
}

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
    if let Some(pos) = pos.filter(|pos| differs(ctx, viewport, *pos - known.min)) {
        act(ctx, viewport, WindowAction::SetOuterPosition(pos));
    }
    if let Some(size) = size.filter(|size| differs(ctx, viewport, *size - known.size())) {
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
