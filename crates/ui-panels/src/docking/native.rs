//! Native dock windows: a floating dock group that shows in an OS window of its own.
//!
//! A native group is a [`FloatingGroup`](super::FloatingGroup) mounted in a viewport. The container does the window
//! work: its title row moves the window, its grips resize it, its limits are the window's, and the OS close is the
//! group's Close action. What is left is policy, in these functions: whether a window of the dock model gets an
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

/// Whether the groups of panels with `specs` open OS windows: the dock's `config` turns them on, the platform opens
/// them over the dock's window this pass (the rule the group's mount falls back to a layer by) and reports where they
/// are, and every panel allows it.
pub(crate) fn allowed(ctx: &Context, config: &DockConfig, specs: &[PanelSpec]) -> bool {
    let host = WindowHost::of(ctx);
    config.native_windows && host.opens_windows() && host.positions_reported() && specs.iter().all(|spec| spec.allow_native)
}

/// The window a group of panels with `specs` opens: a document window when any panel is a document, else a tool
/// window (a palette level, no taskbar entry).
pub fn spec_for(specs: &[PanelSpec]) -> WindowSpec {
    if specs.iter().any(|spec| spec.window_kind == WindowKind::Document) { WindowSpec::document() } else { WindowSpec::tool() }
}

/// The rect in the main window's points of `desktop`, a rect in desktop points: where a window at `desktop` would sit
/// in the app window, as [`restore`] opens a window from its rect there. `None` while the main window has not
/// reported its place.
pub fn in_app(ctx: &Context, desktop: Rect) -> Option<Rect> {
    let main = WindowHost::of(ctx).facts(ViewportId::ROOT)?;
    Some(desktop.translate(-main.inner.min.to_vec2()))
}

/// Where `window` opens on the desktop, in desktop points: its saved placement [`place`]d against the connected
/// monitors, with the main window's monitor as the home of a window whose position is unknown. A window with no
/// position, and one never shown in an OS window, opens where its group sits in the main window.
pub fn restore<P>(ctx: &Context, window: &Floating<P>) -> Rect {
    let host = WindowHost::of(ctx);
    let monitors = host.monitors();
    let main = host.facts(ViewportId::ROOT);
    let home = main.and_then(|facts| {
        let (scale, outer) = (facts.scale?, facts.outer.unwrap_or(facts.inner));
        let on = monitor_of(&monitors, Rect::from_min_max(outer.min * scale, outer.max * scale))?;
        monitors.iter().position(|monitor| monitor == on)
    });
    let placed = place(window.native.as_ref(), &monitors, home, &Defaults::new(window.rect.size().into(), [1.0; 2]));
    let at = placed.position().map(Pos2::from).or_else(|| main.map(|facts| facts.inner.min + window.rect.min.to_vec2()));
    Rect::from_min_size(at.unwrap_or(window.rect.min), Vec2::from(placed.normal.size))
}

/// How `window` shows: in an OS window where `config`, the platform and its panels allow one, else in the app window
/// at its own rect. `specs` are its panels' specs; `group` is the id of the group that shows it. A window that opens
/// opens at its [`restore`]d rect; an open window stays at the placement the dock model recorded for it, so the
/// monitor fit of `restore` never sends back a place the user gave it.
#[doc(hidden)]
pub fn opening<P>(ctx: &Context, config: &DockConfig, window: &Floating<P>, specs: &[PanelSpec], group: egui::Id) -> Opening {
    if !allowed(ctx, config, specs) {
        return Opening { rect: window.rect, spec: None };
    }
    let open = WindowHost::of(ctx).viewport_of(group).is_some();
    let recorded = window.native.as_ref().filter(|_| open).and_then(|native| {
        let at = native.position()?;
        Some(Rect::from_min_size(Pos2::from(at), Vec2::from(native.normal.size)))
    });
    Opening { rect: recorded.unwrap_or_else(|| restore(ctx, window)), spec: Some(spec_for(specs)) }
}

/// The action that records where the OS put the native window `window`, shown in `viewport`: its placement, as an
/// app window's is recorded, with the rect `kept` gives for the rect the OS shows the window at (outer top-left corner
/// and inner size, in desktop points), so a collapsed window keeps its expanded rect
/// ([`WindowColumn::kept`](super::WindowColumn::kept)). `None` while the model already holds what the OS reports, so a
/// window at rest costs no action.
#[doc(hidden)]
pub fn reported<P: Clone>(ctx: &Context, window: &Floating<P>, viewport: ViewportId, kept: impl FnOnce(Rect) -> Rect) -> Option<Action<P>> {
    let panel = window.panels().next()?;
    let mut next = WindowHost::of(ctx).placement(viewport, window.native.clone())?;
    if let (false, false, Some(at)) = (next.maximized, next.fullscreen, next.position()) {
        let rect = kept(Rect::from_min_size(Pos2::from(at), Vec2::from(next.normal.size)));
        next.normal.size = rect.size().into();
        next.normal.pos = rect.min.into();
        next.position_px = Some([(rect.min.x * next.scale).round() as i32, (rect.min.y * next.scale).round() as i32]);
    }
    let settled = window.native.as_ref().is_some_and(|saved| {
        let near = (0..2).all(|i| (saved.normal.size[i] - next.normal.size[i]).abs() <= SETTLED);
        let mut next = next.clone();
        next.normal = saved.normal;
        near && next == *saved
    });
    (!settled).then(|| Action::set_native(panel.clone(), Some(next)))
}

/// Sends the native window shown in `viewport`, which the dock model records at `recorded` (its outer top-left
/// corner and inner size), to `rect` where it differs by more than rounding: the place the dock gives a window it
/// sizes itself (a column with steps). The window's next report is that place, which [`reported`] records.
#[doc(hidden)]
pub fn send_to(ctx: &Context, viewport: ViewportId, recorded: Rect, rect: Rect) {
    let differs = |delta: Vec2| delta.x.abs() > SETTLED || delta.y.abs() > SETTLED;
    let host = WindowHost::of(ctx);
    if differs(rect.min - recorded.min) {
        host.act(viewport, WindowAction::SetOuterPosition(rect.min));
    }
    if differs(rect.size() - recorded.size()) {
        host.act(viewport, WindowAction::SetInnerSize(rect.size()));
    }
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
