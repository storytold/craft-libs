//! The OS window of a viewport: finding it, sending it window commands, routing its hit test and the OS move hand-off.

use super::hit::{HitSink, ResizeEdge, hit_sink_installed};
use super::host::scale_of;
#[cfg(doc)]
use super::logic;
use super::shell::{base_shell, os_of, window_info};
use super::spec::WindowLevel;
use super::{HandOff, Record, SEARCH_EVERY, shared};
use crate::runtime::egui::{pass, viewports};
use crate::runtime::frame::PerViewport;
use craft_host::target::{Caps, DoubleClick, WindowState};
use craft_window::native::NativeWindow;
use craft_window::policy::Frame;
use egui::{Context, Pos2, Vec2, ViewportId};
use std::sync::Arc;

/// A window command `act` sends.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum WindowAction {
    /// Closes the window; the root window's close goes through the close guard.
    Close,
    /// Minimizes.
    Minimize,
    /// Maximizes, or restores a maximized window.
    ToggleMaximize,
    /// Enters or leaves full screen.
    ToggleFullscreen,
    /// Enters full screen (`true`) or leaves it. Unlike [`WindowAction::ToggleMaximize`] it is not refused while the
    /// window is in full screen, so a sequence of actions can be sent in one pass.
    SetFullscreen(bool),
    /// Maximizes (`true`) or restores the window, whatever state it reports.
    SetMaximized(bool),
    /// Raises the window and gives it OS focus.
    Focus,
    /// The OS starts moving the window with the pointer.
    StartMove,
    /// The OS starts resizing the window from `edge`.
    StartResize(ResizeEdge),
    /// Moves the window's outer top-left corner to a desktop position in points.
    SetOuterPosition(Pos2),
    /// Sizes the window's inner area in points.
    SetInnerSize(Vec2),
    /// Changes the window's level.
    SetLevel(WindowLevel),
    /// Changes the window's title.
    SetTitle(String),
    /// Sets the window's opacity, 1 being opaque (a dock window fades while its drag highlights a drop target).
    /// The platform applies it where it can: a layered window on Windows, `NSWindow.alphaValue` on macOS. The
    /// action returns whether the platform applied it.
    SetOpacity(f32),
}

/// What a double click on a window's free move band (its title bar, or the strip a modal leaves over it) asks of the
/// window in `frame`: an integrated frame maximizes or restores; a macOS inset frame follows the system setting
/// `setting`; a native frame is the OS's.
pub fn double_click_action(frame: &Frame, setting: DoubleClick, maximized: bool) -> Option<WindowAction> {
    match (frame, setting) {
        (Frame::Native, _) => None,
        (Frame::Integrated(_), _) | (Frame::Inset(_), DoubleClick::Zoom) => Some(WindowAction::SetMaximized(!maximized)),
        (Frame::Inset(_), DoubleClick::Minimize) => Some(WindowAction::Minimize),
        _ => None,
    }
}

/// Sends `action` to `viewport`. It is the only sender of window commands: it checks the capabilities and the
/// window's state; a root `Close` reaches the app's close decision and the platform quit in [`logic`]. Returns
/// whether a command went out.
pub fn act(ctx: &Context, viewport: ViewportId, action: WindowAction) -> bool {
    if let WindowAction::SetOpacity(opacity) = action {
        return set_opacity(ctx, viewport, opacity);
    }
    let base = base_shell(ctx);
    let commands = base.caps.contains(Caps::WINDOW_COMMANDS);
    let window = window_info(ctx, viewport, &base);
    let resizable = PerViewport::<Record>::with(ctx, viewport, |record| record.resizable != Some(false));
    let allowed = match action {
        WindowAction::Close => true,
        WindowAction::Focus => commands || base.caps.contains(Caps::VIEWPORTS),
        WindowAction::Minimize => commands && window.state != WindowState::Minimized,
        WindowAction::ToggleMaximize => commands && window.state != WindowState::Fullscreen,
        WindowAction::StartResize(_) => commands && resizable,
        WindowAction::ToggleFullscreen
        | WindowAction::SetFullscreen(_)
        | WindowAction::SetMaximized(_)
        | WindowAction::StartMove
        | WindowAction::SetOuterPosition(_)
        | WindowAction::SetInnerSize(_)
        | WindowAction::SetLevel(_)
        | WindowAction::SetTitle(_) => commands,
        WindowAction::SetOpacity(_) => false,
    };
    if !allowed {
        return false;
    }
    match action {
        WindowAction::Close => viewports::close(ctx, viewport),
        WindowAction::Focus => viewports::raise(ctx, viewport),
        WindowAction::Minimize => viewports::command(ctx, viewport, egui::ViewportCommand::Minimized(true)),
        WindowAction::ToggleMaximize => viewports::command(ctx, viewport, egui::ViewportCommand::Maximized(window.state != WindowState::Maximized)),
        WindowAction::ToggleFullscreen => {
            viewports::command(ctx, viewport, egui::ViewportCommand::Fullscreen(window.state != WindowState::Fullscreen))
        }
        WindowAction::SetFullscreen(on) => viewports::command(ctx, viewport, egui::ViewportCommand::Fullscreen(on)),
        WindowAction::SetMaximized(on) => viewports::command(ctx, viewport, egui::ViewportCommand::Maximized(on)),
        WindowAction::StartMove => {
            viewports::command(ctx, viewport, egui::ViewportCommand::StartDrag);
            hand_off(ctx, viewport);
        }
        WindowAction::StartResize(edge) => {
            viewports::command(ctx, viewport, egui::ViewportCommand::BeginResize(viewports::resize_direction(edge)));
            hand_off(ctx, viewport);
        }
        WindowAction::SetOuterPosition(pos) => viewports::command(ctx, viewport, egui::ViewportCommand::OuterPosition(pos)),
        WindowAction::SetInnerSize(size) => viewports::command(ctx, viewport, egui::ViewportCommand::InnerSize(size)),
        WindowAction::SetLevel(level) => {
            // A window opened by `show_viewport` gets its level with the next build of its viewport; any other (the
            // root) is told now.
            let opened = PerViewport::<Record>::with(ctx, viewport, |record| {
                record.level = Some(level);
                record.kind.is_some()
            });
            if opened {
                ctx.request_repaint_of(viewport);
            } else {
                let active = !shared(ctx, |shared| shared.away);
                viewports::command(ctx, viewport, egui::ViewportCommand::WindowLevel(level.for_egui(active, os_of(ctx))));
            }
        }
        WindowAction::SetTitle(title) => viewports::command(ctx, viewport, egui::ViewportCommand::Title(title)),
        WindowAction::SetOpacity(_) => {}
    }
    true
}

/// Applies `opacity` to the OS window. A faded window is told again in every pass, because the window system's own
/// style changes can drop the layered style; an opaque one only when it was faded. Returns whether the platform applied
/// it now.
fn set_opacity(ctx: &Context, viewport: ViewportId, opacity: f32) -> bool {
    let opacity = opacity.clamp(0.0, 1.0);
    let before = PerViewport::<Record>::with(ctx, viewport, |record| record.opacity.replace(opacity));
    if before != Some(opacity) {
        #[cfg(feature = "testing")]
        shared(ctx, |shared| shared.sent.push((viewport, WindowAction::SetOpacity(opacity))));
    }
    let faded = opacity < 1.0 || before.is_some_and(|before| before < 1.0);
    faded && native_of(ctx, viewport).is_some_and(|native| native.set_opacity(opacity))
}

/// The OS window of `viewport`, found once its window exists: by its title among the thread's windows, when exactly
/// one starts at the reported client origin. A failed search repeats every `SEARCH_EVERY` passes. The root window's is
/// set by [`route_root`].
fn native_of(ctx: &Context, viewport: ViewportId) -> Option<NativeWindow> {
    let pass = pass::pass_nr(ctx, viewport);
    let (known, title, searched, facts) =
        PerViewport::<Record>::with(ctx, viewport, |record| (record.native.clone(), record.title.clone(), record.searched, record.facts));
    if known.is_some() || searched.is_some_and(|last| pass < last + SEARCH_EVERY) {
        return known;
    }
    let (title, inner) = (title?, facts?.inner?);
    let scale = scale_of(ctx, &facts?)?;
    let origin = [(inner.min.x * scale).round() as i32, (inner.min.y * scale).round() as i32];
    let found = NativeWindow::titled(&title, Some(origin));
    PerViewport::<Record>::with(ctx, viewport, |record| {
        record.searched = found.is_none().then_some(pass);
        record.native = found.clone();
    });
    found
}

/// Records that the component of `viewport`'s window runs its own drag from the window's move band, as the floating
/// dock group does to dock its window: the Windows hit test then leaves the band to the app, whose press starts that
/// drag and the OS move together. Runs in each pass of the component.
#[doc(hidden)]
pub fn keeps_app_drag(ctx: &Context, viewport: ViewportId) {
    PerViewport::<Record>::with(ctx, viewport, |record| record.app_drag = true);
}

/// Routes the Windows hit test of `viewport`'s window to its hit sink, once, when the OS window exists and the app
/// draws its frame. A platform without the routing leaves the sink uninstalled, so the app moves and resizes the
/// window itself.
pub(super) fn route_hit_test(ctx: &Context, viewport: ViewportId) {
    if hit_sink_installed(ctx, viewport) || PerViewport::<Record>::with(ctx, viewport, |record| record.app_drag) {
        return;
    }
    let Some(native) = native_of(ctx, viewport) else { return };
    let sink = HitSink::linked(ctx, viewport);
    if native.route_hits(Arc::new(sink.clone())) {
        PerViewport::<Record>::with(ctx, viewport, |record| record.sink = Some(sink));
    }
}

/// Gives the root window its OS window (`handle` finds it from eframe's window handle; it runs until the window is
/// known) and routes its hit test while the app draws the frame. Runs each pass: a frame that becomes integrated later
/// is routed then.
#[cfg(windows)]
#[doc(hidden)]
pub fn route_root(ctx: &Context, handle: impl FnOnce() -> Option<NativeWindow>) {
    if PerViewport::<Record>::with(ctx, ViewportId::ROOT, |record| record.native.is_none()) {
        let Some(native) = handle() else { return };
        PerViewport::<Record>::with(ctx, ViewportId::ROOT, |record| record.native = Some(native));
    }
    if matches!(shared(ctx, |shared| shared.root_frame.clone()), Some(Frame::Integrated(_))) {
        route_hit_test(ctx, ViewportId::ROOT);
    }
}

/// The OS took the press that started a move or resize of `viewport`'s window: [`input_hook`] ends it on the next
/// input.
fn hand_off(ctx: &Context, viewport: ViewportId) {
    let pass = pass::pass_nr(ctx, viewport);
    PerViewport::<Record>::with(ctx, viewport, |record| record.handoff = Some(HandOff { pass, released: None }));
}
