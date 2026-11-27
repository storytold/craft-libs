//! Native dock windows: a floating dock group that shows in an OS window of its own.
//!
//! A native group is a [`FloatingGroup`](super::FloatingGroup) mounted in a viewport. The container does the window
//! work: its title row moves the window, its grips resize it, its limits are the window's, and the OS close is the
//! group's Close action. What is left is policy, in these functions: whether a window of the dock model gets an
//! OS window, what kind of window it is, where it opens on the connected monitors, and what the dock model records
//! when the OS moves or resizes it. `DockArea` and `FloatingGroup` call them. A native window keeps the
//! [`Placement`](craft_behavior::placement::Placement) app windows keep, and opens where [`place`] puts it.

use super::{DockConfig, FloatingGroupOutput, PanelSpec};
use crate::runtime::windows::{WindowAction, WindowHost, WindowSpec};
use crate::runtime::{drag, overlay, pointer};
use craft_behavior::dock::{Action, Floating};
use craft_behavior::placement::{Defaults, monitor_of, place};
use craft_behavior::{Buttons, GesturePhase};
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
    /// The group shows in the app window until the drag that moves it there ends, and in an OS window after.
    pub waits: bool,
}

/// Whether the groups of panels with `specs` open OS windows: the dock's `config` turns them on, the platform opens
/// them over the dock's window this pass (the rule the group's mount falls back to a layer by) and reports where they
/// are, and every panel allows it.
pub(crate) fn allowed(ctx: &Context, config: &DockConfig, specs: &[PanelSpec]) -> bool {
    opens_os_windows(ctx, config) && specs.iter().all(|spec| spec.allow_native)
}

/// Whether the dock opens its floating windows in OS windows: `config` asks for them, and the window host opens OS
/// windows and reports where they are. A panel may still keep to the app window (`PanelSpec::allow_native`).
#[doc(hidden)]
pub fn opens_os_windows(ctx: &Context, config: &DockConfig) -> bool {
    let host = WindowHost::of(ctx);
    config.native_windows && host.opens_windows() && host.positions_reported()
}

/// The display scale the OS window `viewport` lays out at: its monitor's scale with the app's zoom, as the window
/// reported it (`WindowHost::facts`). `None` before it reported one.
pub fn pixels_per_point(ctx: &Context, viewport: ViewportId) -> Option<f32> {
    WindowHost::of(ctx).facts(viewport)?.scale.map(|scale| scale * ctx.zoom_factor())
}

/// The window a group of panels with `specs` opens: a document window when any panel is a document, else a tool
/// window (a palette level, no taskbar entry). An Escape nothing in it takes is the Cancel of the window the dock is in.
pub fn spec_for(specs: &[PanelSpec]) -> WindowSpec {
    let document = specs.iter().any(|spec| spec.window_kind == WindowKind::Document);
    let spec = if document { WindowSpec::document() } else { WindowSpec::tool() };
    spec.hands_on_cancel(true)
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
///
/// A window opens no OS window while a drag moves it in the app window: the OS gives an OS window no press that went
/// down in another window, so the drag would end. `continues` says that the window takes over the held press in this
/// pass (`DockArea::continue_move`), which a window never placed on the desktop does in the app window.
#[doc(hidden)]
pub fn opening<P>(ctx: &Context, config: &DockConfig, window: &Floating<P>, specs: &[PanelSpec], group: egui::Id, continues: bool) -> Opening {
    let layer = Opening { rect: window.rect, spec: None, waits: false };
    if !allowed(ctx, config, specs) {
        return layer;
    }
    let open = WindowHost::of(ctx).viewport_of(group).is_some();
    if !open {
        let takes_over = continues && window.native.is_none() && pointer::frame(ctx).buttons.contains(Buttons::PRIMARY);
        if takes_over || pointer::pressed_part(ctx, group).is_some() || drag::sourced_by(ctx, group) {
            return Opening { waits: true, ..layer };
        }
    }
    let recorded = window.native.as_ref().filter(|_| open).and_then(|native| {
        let at = native.position()?;
        Some(Rect::from_min_size(Pos2::from(at), Vec2::from(native.normal.size)))
    });
    Opening { rect: recorded.unwrap_or_else(|| restore(ctx, window)), spec: Some(spec_for(specs)), waits: false }
}

/// Asks for the pass in which a group that waits to open its OS window ([`Opening::waits`]) opens it: the next one once
/// the press that moved it has ended, or the one after a target in another window had its chance to take the drop.
#[doc(hidden)]
pub fn waited(ctx: &Context, opening: &Opening, output: &FloatingGroupOutput) {
    let ended = output.placed.is_some_and(|change| matches!(change.phase, GesturePhase::Commit | GesturePhase::Cancel));
    if opening.waits && (ended || pointer::pressed_part(ctx, output.response.id).is_none()) {
        crate::runtime::frame::request_repaint(ctx);
    }
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

/// Brings the group that `output` reports on to the front of the windows it overlaps when `asked`
/// ([`FloatingGroup::raise`](super::FloatingGroup::raise)), and an OS window also when the user began moving or
/// resizing it this pass (that press already brings a layer forward). An OS window is raised and focused; a layer's
/// family comes in front of the other layers at the end of the pass.
pub(crate) fn raise(ctx: &Context, output: &FloatingGroupOutput, asked: bool) {
    let host = WindowHost::of(ctx);
    match host.viewport_of(output.response.id) {
        Some(viewport) => {
            let began = output.placed.is_some_and(|change| change.phase == GesturePhase::Begin);
            if asked || began {
                host.act(viewport, WindowAction::Focus);
            }
        }
        None if asked => overlay::raise(ctx, output.response.id),
        None => {}
    }
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
