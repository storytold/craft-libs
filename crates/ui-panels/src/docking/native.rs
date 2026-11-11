//! Native dock windows: a floating dock group that shows in an OS window of its own.
//!
//! A native group is a [`FloatingGroup`](super::FloatingGroup) mounted in a viewport. The container does the window
//! work: its title row moves the window, its grips resize it, its limits are the window's, and a close is a
//! `close_requested` report. What is left is policy, in these functions: whether a window of the dock model gets an
//! OS window, what kind of window it is, where it opens on the connected monitors, and what the dock model records
//! when the OS moves or resizes it. `DockArea` and `FloatingGroup` call them. A native window keeps the
//! [`Placement`](craft_behavior::placement::Placement) app windows keep, and opens where [`place`] puts it.

use super::{DockConfig, FloatingGroupOutput, PanelSpec};
use crate::runtime::drag;
use crate::runtime::windows::{WindowAction, WindowHost, WindowSpec};
use craft_behavior::GesturePhase;
use craft_behavior::dock::{Action, Floating};
use craft_behavior::placement::{Defaults, monitor_of, place};
use craft_host::target::WindowKind;
use egui::{Context, Pos2, Rect, Vec2, ViewportId};

/// How far the OS-reported size of a window may differ from the one the dock model keeps before the difference is a
/// resize: the OS rounds to pixels, and a model that records its own rounding would re-send it for ever.
const SETTLED: f32 = 0.5;

/// How a floating window of the dock model shows.
#[derive(Clone, Debug)]
#[non_exhaustive]
#[doc(hidden)]
pub struct Opening {
    /// The rect the group is placed at: on the desktop for an OS window, in the app window otherwise.
    pub rect: Rect,
    /// The OS window to open, or `None` where the group shows in the app window.
    pub spec: Option<WindowSpec>,
}

/// Whether the groups of panels with `specs` may open OS windows: the dock's `config` turns them on, the platform
/// opens them and every panel allows it.
pub(crate) fn allowed(ctx: &Context, config: &DockConfig, specs: &[PanelSpec]) -> bool {
    config.native_windows && WindowHost::of(ctx).viewports_available() && specs.iter().all(|spec| spec.allow_native)
}

/// The window a group of panels with `specs` opens: a document window when any panel is a document, else a tool
/// window (a palette level, no taskbar entry).
pub fn spec_for(specs: &[PanelSpec]) -> WindowSpec {
    if specs.iter().any(|spec| spec.window_kind == WindowKind::Document) { WindowSpec::document() } else { WindowSpec::tool() }
}

/// Where the saved placement of `window` opens on the desktop, in desktop points: [`place`] against the connected
/// monitors, with the main window's monitor as the home of a window whose position is unknown. A window with no
/// position opens where its group sits in the main window. `None` for a window that is not native.
pub fn restore<P>(ctx: &Context, window: &Floating<P>) -> Option<Rect> {
    let saved = window.native.as_ref()?;
    let host = WindowHost::of(ctx);
    let monitors = host.monitors();
    let main = host.facts(ViewportId::ROOT);
    let home = main.and_then(|facts| {
        let (scale, outer) = (facts.scale?, facts.outer.unwrap_or(facts.inner));
        let on = monitor_of(&monitors, Rect::from_min_max(outer.min * scale, outer.max * scale))?;
        monitors.iter().position(|monitor| monitor == on)
    });
    let placed = place(Some(saved), &monitors, home, &Defaults::new(window.rect.size().into(), [1.0; 2]));
    let at = placed.position().map(Pos2::from).or_else(|| main.map(|facts| facts.inner.min + window.rect.min.to_vec2()));
    Some(Rect::from_min_size(at.unwrap_or(window.rect.min), Vec2::from(placed.normal.size)))
}

/// How `window` shows: in an OS window at its restored rect when it is native and `config`, the platform and its
/// panels allow one, else in the app window at its own rect. `specs` are its panels' specs.
#[doc(hidden)]
pub fn opening<P>(ctx: &Context, config: &DockConfig, window: &Floating<P>, specs: &[PanelSpec]) -> Opening {
    if allowed(ctx, config, specs)
        && let Some(rect) = restore(ctx, window)
    {
        return Opening { rect, spec: Some(spec_for(specs)) };
    }
    Opening { rect: window.rect, spec: None }
}

/// The action that records where the OS put the native window `window`, shown in `viewport`: its placement, as an
/// app window's is recorded. `None` while the model already holds what the OS reports, so a window at rest costs no
/// action.
#[doc(hidden)]
pub fn reported<P: Clone>(ctx: &Context, window: &Floating<P>, viewport: ViewportId) -> Option<Action<P>> {
    let panel = window.panels().next()?;
    let next = WindowHost::of(ctx).placement(viewport, window.native.clone())?;
    let settled = window.native.as_ref().is_some_and(|saved| {
        let near = (0..2).all(|i| (saved.normal.size[i] - next.normal.size[i]).abs() <= SETTLED);
        let mut next = next.clone();
        next.normal = saved.normal;
        near && next == *saved
    });
    (!settled).then(|| Action::set_native(panel.clone(), Some(next)))
}

/// Raises the window of the native group that `output` reports on when the user began moving or resizing it this
/// pass, so a palette comes to the front of the windows it overlaps. False when nothing was raised.
pub(crate) fn raise(ctx: &Context, output: &FloatingGroupOutput) -> bool {
    let began = output.placed.is_some_and(|change| change.phase == GesturePhase::Begin);
    let host = WindowHost::of(ctx);
    began && host.viewport_of(output.response.id).is_some_and(|viewport| host.act(viewport, WindowAction::Focus))
}

/// Fades the native window of the group that `output` reports on while its drag highlights a drop target, and makes
/// it opaque again otherwise: over empty space, for a move that cannot dock and while Ctrl or Cmd keeps the drop
/// floating (the target then takes no drop and shows no mark). Every way a drag ends brings the opacity back, because
/// this runs in each pass of the group. The group runs in its parent's pass, so the window is the group's own, not the
/// running viewport.
pub(crate) fn fade(ctx: &Context, output: &FloatingGroupOutput) {
    let host = WindowHost::of(ctx);
    let Some(viewport) = host.viewport_of(output.response.id) else { return };
    let highlighted = drag::moves(ctx, viewport) && drag::mark(ctx).is_some();
    host.act(viewport, WindowAction::SetOpacity(if highlighted { craft_theme::Opacity::MOVING } else { 1.0 }));
}

/// Tells the window host that the native window of the group `id` is dragged by its title row from the app (the group
/// docks its window), so the Windows hit test leaves that row to the app.
pub(crate) fn keeps_app_drag(ctx: &Context, id: egui::Id) {
    if let Some(viewport) = WindowHost::of(ctx).viewport_of(id) {
        crate::runtime::windows::keeps_app_drag(ctx, viewport);
    }
}
