//! The OS window of a viewport: finding it, sending it window commands, routing its hit test and the OS move hand-off.

use super::hit::{HitSink, ResizeEdge, hit_sink_installed};
use super::host::scale_of;
#[cfg(doc)]
use super::logic;
use super::shell::{base_shell, os_of, window_info};
use super::spec::{Standing, WindowLevel};
use super::{HandOff, Record, SEARCH_EVERY, shared};
use crate::runtime::egui::{pass, viewports};
use crate::runtime::frame::{PerViewport, Stored};
use craft_host::target::{Caps, DoubleClick, Os, WindowState};
use craft_window::native::NativeWindow;
use craft_window::policy::Frame;
use egui::{Context, Pos2, Rect, Vec2, ViewportId};
use std::sync::Arc;

/// The last title the host set for a viewport.
#[derive(Default)]
struct SentTitle(Option<String>);

impl Stored for SentTitle {}

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
    /// Brings the window in front of the app's other windows without giving it focus, as the window holding an
    /// app-modal layer comes in front of the palettes the modal lowered.
    Raise,
    /// The OS starts moving the window with the pointer.
    StartMove,
    /// The OS starts resizing the window from `edge`.
    StartResize(ResizeEdge),
    /// Moves the window's outer top-left corner to a desktop position in points.
    SetOuterPosition(Pos2),
    /// Sizes the window's inner area in points; its outer top-left corner stays where it is known to be.
    SetInnerSize(Vec2),
    /// Restores a minimized window.
    Restore,
    /// Sets the smallest inner size the window can be given, in points.
    SetMinInnerSize(Vec2),
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
/// window in `frame`: an integrated or macOS inset frame follows the system setting `setting` (zoom, the default,
/// maximizes or restores; GNOME's setting comes from `craft_window::desktop::double_click`); a native frame is the
/// OS's.
pub fn double_click_action(frame: &Frame, setting: DoubleClick, maximized: bool) -> Option<WindowAction> {
    match (frame, setting) {
        (Frame::Native, _) => None,
        (Frame::Integrated(_) | Frame::Inset(_), DoubleClick::Zoom) => Some(WindowAction::SetMaximized(!maximized)),
        (Frame::Integrated(_) | Frame::Inset(_), DoubleClick::Minimize) => Some(WindowAction::Minimize),
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
        | WindowAction::Restore
        | WindowAction::SetMinInnerSize(_)
        | WindowAction::SetLevel(_)
        | WindowAction::SetTitle(_)
        | WindowAction::Raise => commands,
        WindowAction::SetOpacity(_) => false,
    };
    if !allowed {
        return false;
    }
    match action {
        WindowAction::Close => viewports::close(ctx, viewport),
        WindowAction::Focus => viewports::raise(ctx, viewport),
        WindowAction::Raise => viewports::raise_above(ctx, viewport, standing(ctx, viewport, level_of(ctx, viewport)).level),
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
            if base.caps.contains(Caps::OS_MOVES_TRACKED) {
                shared(ctx, |shared| shared.tracked_move = Some(viewport));
            }
        }
        WindowAction::StartResize(edge) => {
            viewports::command(ctx, viewport, egui::ViewportCommand::BeginResize(viewports::resize_direction(edge)));
            hand_off(ctx, viewport);
        }
        WindowAction::SetOuterPosition(pos) => {
            viewports::command(ctx, viewport, egui::ViewportCommand::OuterPosition(pos));
            PerViewport::<Record>::with(ctx, viewport, |record| record.known = record.known.map(|known| Rect::from_min_size(pos, known.size())));
        }
        WindowAction::SetInnerSize(size) => {
            viewports::command(ctx, viewport, egui::ViewportCommand::InnerSize(size));
            let known = PerViewport::<Record>::with(ctx, viewport, |record| {
                record.known = record.known.map(|known| Rect::from_min_size(known.min, size));
                record.known
            });
            // AppKit keeps a window's bottom-left corner when its size changes (`setContentSize`), so a new height
            // moves its top edge: the window goes back to the top-left corner its caller knows it at, which a position
            // sent before the size has already set.
            if base.os == Os::Mac
                && let Some(known) = known
            {
                viewports::command(ctx, viewport, egui::ViewportCommand::OuterPosition(known.min));
            }
        }
        WindowAction::Restore => viewports::command(ctx, viewport, egui::ViewportCommand::Minimized(false)),
        WindowAction::SetMinInnerSize(size) => viewports::command(ctx, viewport, egui::ViewportCommand::MinInnerSize(size)),
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
                viewports::command(ctx, viewport, egui::ViewportCommand::WindowLevel(standing(ctx, viewport, level).level));
            }
        }
        WindowAction::SetTitle(title) => {
            // X11 rejects a title holding a NUL, so the host drops it before the compare: a title that differs only by
            // NULs is the one already sent.
            let title: String = title.chars().filter(|c| *c != '\0').collect();
            // `ViewportInfo::title` is empty on X11 and Wayland, so the host remembers what it set itself.
            let fresh = PerViewport::<SentTitle>::with(ctx, viewport, |sent| {
                let fresh = sent.0.as_deref() != Some(title.as_str());
                if fresh {
                    sent.0 = Some(title.clone());
                }
                fresh
            });
            if fresh {
                viewports::command(ctx, viewport, egui::ViewportCommand::Title(title));
            }
        }
        WindowAction::SetOpacity(_) => {}
    }
    true
}

/// The level `viewport`'s window was opened with or last set to.
fn level_of(ctx: &Context, viewport: ViewportId) -> WindowLevel {
    PerViewport::<Record>::with(ctx, viewport, |record| record.level.unwrap_or_default())
}

/// How `viewport`'s window at `level` stands in the app's current state.
fn standing(ctx: &Context, viewport: ViewportId, level: WindowLevel) -> Standing {
    level.standing(viewport, shared(ctx, |shared| shared.app), os_of(ctx))
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

/// Routes the Windows hit test of `viewport`'s window to its hit sink, once, when the OS window exists and the app
/// draws its frame. A platform without the routing leaves the sink uninstalled, so the app moves and resizes the
/// window itself.
pub(super) fn route_hit_test(ctx: &Context, viewport: ViewportId) {
    if hit_sink_installed(ctx, viewport) || PerViewport::<Record>::with(ctx, viewport, |record| record.moved_by_app) {
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

/// The OS took the press that started a move or resize of `viewport`'s window: [`input_hook`] ends it once the system
/// tells the button is up, or on the next input where it does not tell.
fn hand_off(ctx: &Context, viewport: ViewportId) {
    let pass = pass::pass_nr(ctx, viewport);
    PerViewport::<Record>::with(ctx, viewport, |record| record.handoff = Some(HandOff { pass, released: None }));
}
