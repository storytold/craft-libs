//! `WindowHost`: the one multi-window driver. It owns `ShellInfo`, refreshes each viewport's `WindowInfo`, releases
//! a press after an OS resize (H4), raises a modal's window for a blocked press (H5), opens viewport mounts, keeps
//! window placement in `craft_window::restore::Store`, routes a root close through the platform quit from the app's
//! `logic`, and maps pointers to desktop space for drags that cross windows.

use crate::runtime::cursor;
use crate::runtime::egui::input::{self as raw_input, RawInput};
use crate::runtime::egui::memory::Service;
use crate::runtime::egui::{pass, viewports};
use crate::runtime::frame::PerViewport;
use crate::runtime::overlay;
use craft_behavior::{Cursor, CursorIcon, CursorPriority};
use craft_host::target::{Caps, EdgeInsets, ShellInfo, WindowChrome, WindowKind};
use craft_window::native::NativeWindow;
use craft_window::policy::{Captions, Decision, Frame, LayoutWatch};
use craft_window::restore::Store;
use craft_window::system::WindowSystem;
use egui::{Context, Id, Pos2, Rect, Vec2, ViewportId};
use std::sync::{Arc, Mutex};

pub use crate::runtime::egui::screenshot::{ScreenshotError, ScreenshotFacts, ScreenshotSlot};

mod hit;
mod host;
mod native;
mod placement;
mod shell;
mod spec;
mod viewport;

pub use hit::{CaptionRegions, HitSink, ResizeEdge, WindowHit};
pub(crate) use hit::{hit_sink_installed, note_exclusive};
use hit::{publish_regions, replay_caption_events};
pub use host::{KeyEquivalent, WindowFacts, WindowHost};
pub(crate) use host::{clear_source_pointer, note_source_pointer};
#[doc(hidden)]
pub use native::keeps_app_drag;
#[cfg(windows)]
#[doc(hidden)]
pub use native::route_root;
pub use native::{WindowAction, act, double_click_action};
use placement::record_placement;
#[doc(hidden)]
pub use shell::os_of;
#[doc(hidden)]
pub use shell::shell_of;
use shell::{base_shell, refresh, window_info};
#[cfg(feature = "testing")]
pub(crate) use shell::{set_chrome, set_os};
use spec::chrome_of;
pub use spec::{WindowLevel, WindowSpec};
pub(crate) use viewport::{ViewportOpen, show_viewport, take_close_request};

/// Seconds between writes of a changed placement store.
const FLUSH_SECONDS: f64 = 1.0;

/// Install-time inputs and the cross-window records of one context.
#[derive(Default)]
struct Shared {
    /// The opacity requests since the last [`take_sent`], for trials: the platform applies them out of reach.
    #[cfg(feature = "testing")]
    sent: Vec<(ViewportId, WindowAction)>,
    shell: Option<ShellInfo>,
    system: Option<Arc<dyn WindowSystem>>,
    store: Option<Arc<Mutex<Store>>>,
    /// The system pointer read in `set_input`.
    system_pointer: Option<Pos2>,
    /// The pointer of the viewport a drag started in, in desktop points, for platforms that tell no system pointer.
    source_pointer: Option<Pos2>,
    /// Containers whose viewport the OS asked to close in this root frame.
    close_requests: Vec<Id>,
    last_flush: f64,
    /// The root window's frame and its caption layout watch, from `InstallSpec::frame`.
    root_frame: Option<Frame>,
    layouts: Option<LayoutWatch>,
    /// Whether the root window had focus in the last root pass.
    root_focused: bool,
    /// Whether no window of the app had focus when the last frame's windows had all reported.
    away: bool,
    /// The app's windows from the one focused longest ago to the one focused last: the stacking order of
    /// overlapping windows.
    stacking: Vec<ViewportId>,
}

/// One viewport's window record.
#[derive(Default)]
struct Record {
    facts: Option<viewports::Facts>,
    /// The measured traffic-lights inset, kept until the scale changes or full screen ends.
    lights: craft_window::mac::Cache,
    /// What the OS reserves at the window's edges, once measured.
    insets: Option<EdgeInsets>,
    kind: Option<WindowKind>,
    chrome: Option<WindowChrome>,
    /// The caption buttons the app draws in this window.
    captions: Option<Captions>,
    /// The level the window was opened with or last set to; a palette's egui level follows the app's activity.
    level: Option<WindowLevel>,
    /// H4: whether the primary button is down as the viewport's events left the input hook.
    primary_down: bool,
    /// Whether the user can resize the window; unknown for a window not opened by `show_viewport`.
    resizable: Option<bool>,
    /// Whether the window was opened hidden and its first pass has not run yet.
    awaiting_first_paint: bool,
    /// The last OS move or resize the window was handed.
    handoff: Option<HandOff>,
    /// The geometry last written to the placement store, so an unchanged window writes nothing.
    placed: Option<Geometry>,
    last_pos: Option<Pos2>,
    last_inner_size: Option<Vec2>,
    /// The window procedure's hit test, once the platform layer installed it.
    sink: Option<HitSink>,
    /// The title the window was opened with, which finds its OS window.
    title: Option<String>,
    /// The OS window, once found.
    native: Option<NativeWindow>,
    /// The pass of the last failed search for the OS window, so the search repeats only every `SEARCH_EVERY` passes.
    searched: Option<u64>,
    /// The opacity the window was last set to; 1 is opaque.
    opacity: Option<f32>,
    /// The window's component runs its own drag from the move band (a dock window's title row), so a press on the
    /// band must reach it and the operating system's move loop is not started from the hit test.
    app_drag: bool,
}

/// Passes between searches for an OS window that was not found.
const SEARCH_EVERY: u64 = 20;

/// An OS move or resize started in the pass `pass`: the OS keeps the press and sends no release. `released` is the
/// pass whose input the runtime ended the press in.
#[derive(Clone, Copy)]
struct HandOff {
    pass: u64,
    released: Option<u64>,
}

/// The window facts a placement keeps.
#[derive(Clone, Copy, PartialEq)]
struct Geometry {
    inner: Rect,
    outer: Option<Rect>,
    scale: Option<f32>,
    maximized: bool,
    fullscreen: bool,
}

fn shared<R>(ctx: &Context, f: impl FnOnce(&mut Shared) -> R) -> R {
    Service::<Shared>::run(ctx, f)
}

/// The opacity requests made since the last call: a trial's window action stream, as the platform applies them where
/// no egui command shows them.
#[cfg(feature = "testing")]
pub(crate) fn take_sent(ctx: &Context) -> Vec<(ViewportId, WindowAction)> {
    shared(ctx, |shared| std::mem::take(&mut shared.sent))
}

/// Whether `viewport`'s component declared its own drag from the move band.
#[cfg(feature = "testing")]
pub(crate) fn keeps_app_drag_of(ctx: &Context, viewport: ViewportId) -> bool {
    PerViewport::<Record>::with(ctx, viewport, |record| record.app_drag)
}

/// Called by `install`: keeps the shell, the OS services and the placement store.
pub(crate) fn install(ctx: &Context, shell: ShellInfo, system: Arc<dyn WindowSystem>, store: Option<Arc<Mutex<Store>>>, frame: Option<Decision>) {
    let layouts = frame.as_ref().map(LayoutWatch::new);
    let shell = match &frame {
        Some(decision) => {
            let window = (*shell.window).clone().chrome(chrome_of(&decision.frame));
            shell.window(window)
        }
        None => shell,
    };
    shared(ctx, |shared| {
        shared.shell = Some(shell);
        shared.system = Some(system);
        shared.store = store;
        shared.root_frame = frame.map(|decision| decision.frame);
        shared.layouts = layouts;
    });
}

/// Before the pass, from `set_input`, outside every egui lock: reads the OS services and refreshes the window
/// info of every viewport the last root input reported.
pub(crate) fn set_input(ctx: &Context) {
    let system = shared(ctx, |shared| shared.system.clone());
    let pointer = system.and_then(|system| system.desktop_pointer()).map(|[x, y]| Pos2::new(x, y));
    for (viewport, facts) in viewports::reported(ctx) {
        refresh(ctx, viewport, facts);
    }
    shared(ctx, |shared| shared.system_pointer = pointer);
}

/// H4, in the input hook. The OS takes the button release of a move or resize it runs, so the press stays down in
/// egui and every later press is lost. Two cases end it with a release of the primary button, in any chrome:
/// the first input after [`act`] handed the OS a move or resize, when the input brings no release of its own (where
/// the OS sends one, a second would turn into a click); and, where the OS resizes undecorated windows from their
/// edges ([`Caps::OS_RESIZES_UNDECORATED`]), a size change under a held primary press. Reads only the raw input and
/// the viewport's record. Returns whether it added an event.
pub(crate) fn input_hook(ctx: &Context, input: &mut RawInput) -> bool {
    let viewport = input.viewport_id;
    replay_caption_events(ctx, viewport, input);
    let base = base_shell(ctx);
    let undecorated = base.caps.contains(Caps::OS_RESIZES_UNDECORATED) && window_info(ctx, viewport, &base).chrome == WindowChrome::Integrated;
    let size = viewports::inner_size_in(input, viewport);
    let pass = pass::pass_nr(ctx, viewport);
    let os_released =
        input.events.iter().any(|event| matches!(event, egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, .. }));
    PerViewport::<Record>::with(ctx, viewport, |record| {
        let resized = matches!((record.last_inner_size, size), (Some(before), Some(now)) if before != now);
        record.last_inner_size = size.or(record.last_inner_size);
        let handed_off = record.handoff.is_some_and(|handoff| handoff.released.is_none() && handoff.pass < pass);
        if handed_off {
            record.handoff = record.handoff.map(|handoff| HandOff { released: Some(pass), ..handoff });
        }
        let release = record.primary_down && ((undecorated && resized) || (handed_off && !os_released));
        if release {
            raw_input::push_primary_release(input, record.last_pos.unwrap_or(Pos2::ZERO));
        }
        raw_input::follow_primary(&input.events, &mut record.primary_down, &mut record.last_pos);
        release
    })
}

/// The begin hook (H5), where the viewport is current: refreshes its window info, and raises the modal's window
/// when the input hook dropped a press under a modal in another viewport.
pub(crate) fn begin(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    refresh(ctx, viewport, viewports::current(ctx));
    reshow_cursor(ctx, viewport);
    if overlay::take_dropped_press(ctx, viewport)
        && let Some(modal) = blocking_modal(ctx)
    {
        act(ctx, modal, WindowAction::Focus);
    }
}

/// The window whose app-modal layer blocks the running viewport: one other than it with a modal open. `None`
/// when no app-modal is open or the only one is in this window. A blocked window drops presses and keys (H5),
/// shows the blocked cursor and refuses to close.
pub(crate) fn blocking_modal(ctx: &Context) -> Option<ViewportId> {
    overlay::modal_viewport(ctx, pass::current_viewport(ctx))
}

/// Refuses the close request of the running viewport while an app-modal layer blocks it, and raises the modal's
/// window. True when it refused.
fn refuse_close_under_modal(ctx: &Context) -> bool {
    let Some(modal) = blocking_modal(ctx) else { return false };
    viewports::cancel_close(ctx);
    act(ctx, modal, WindowAction::Focus);
    true
}

/// The compositor owns the pointer during an OS move or resize and sets its own cursor, and egui-winit only sends a
/// cursor that changed, so afterwards the window keeps the compositor's cursor. On the first pointer motion after the
/// hand-off's press was ended, asks for the default cursor for one pass, so the cursor the pointer is over is sent
/// again on the next.
fn reshow_cursor(ctx: &Context, viewport: ViewportId) {
    if !ctx.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::PointerMoved(_)))) {
        return;
    }
    let pass = pass::pass_nr(ctx, viewport);
    let due = PerViewport::<Record>::with(ctx, viewport, |record| match record.handoff {
        Some(HandOff { released: Some(released), .. }) if released < pass => record.handoff.take().is_some(),
        _ => false,
    });
    if due {
        cursor::request(ctx, Cursor::System(CursorIcon::Default), CursorPriority::Drag);
        ctx.request_repaint_of(viewport);
    }
}

/// The end hook: keeps the root window's placement, writes a changed placement store once a second, and drops the
/// close requests the viewport mounts did not take in this frame.
pub(crate) fn end(ctx: &Context) {
    publish_regions(ctx, pass::current_viewport(ctx));
    if pass::current_viewport(ctx) != ViewportId::ROOT {
        return;
    }
    shared(ctx, |shared| shared.close_requests.clear());
    follow_activity(ctx);
    follow_layout(ctx, viewports::current(ctx).focused);
    // The root window's placement is the app's `main` entry.
    if shared(ctx, |shared| shared.store.is_some()) {
        record_placement(ctx, ViewportId::ROOT, craft_window::restore::MAIN, &viewports::current(ctx));
    }
    flush_store(ctx);
}

/// Whether the root window was asked to close and nothing has cancelled it yet. Valid in the app's `logic`, which
/// eframe runs while the window is minimized or hidden too.
#[doc(hidden)]
pub fn root_close_requested(ctx: &Context) -> bool {
    root_close_asked(ctx) && blocking_modal(ctx).is_none()
}

/// Whether the OS asked the root window to close and nothing has cancelled it yet, whatever an app-modal layer in
/// another window says.
fn root_close_asked(ctx: &Context) -> bool {
    pass::current_viewport(ctx) == ViewportId::ROOT && viewports::current(ctx).close_requested && !viewports::close_cancelled(ctx)
}

/// Keeps the root window open and brings it forward, so the app can ask before it closes.
#[doc(hidden)]
pub fn hold_close(ctx: &Context) {
    viewports::cancel_close(ctx);
    viewports::raise(ctx, ViewportId::ROOT);
}

/// The close path of the root window, for the app's `logic`: a close request goes to the platform quit where the
/// platform has one (the window stays for the quit to close); otherwise egui goes on as it would. Call it last in
/// `logic`, after the app's own close decision, so it sees the decision's `CancelClose`. It belongs in `logic`
/// because eframe runs no UI pass for a minimized or hidden window and closes it itself; on macOS that close is the
/// Touch Bar crash `craft_quit` exists to avoid. `app::run` calls it; apps that call `install` themselves do too.
pub fn logic(ctx: &Context) {
    if root_close_asked(ctx) && !refuse_close_under_modal(ctx) && craft_quit::quit() {
        viewports::cancel_close(ctx);
    }
}

fn flush_store(ctx: &Context) {
    let now = pass::now(ctx);
    let store = shared(ctx, |shared| {
        if now - shared.last_flush < FLUSH_SECONDS {
            return None;
        }
        shared.last_flush = now;
        shared.store.clone()
    });
    if let Some(store) = store {
        let mut store = store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if store.is_dirty()
            && let Err(error) = store.save()
        {
            log::warn!("window placement not saved: {error}");
        }
    }
}

/// Keeps the app's activity and the stacking order of its windows current from the windows' focus. A palette's
/// window level follows the activity: the mounts build their viewports with the level of the moment, and a change
/// asks for the next frame. Runs at the end of the root pass, inside which every child window's pass of the frame
/// has run, so each window's focus is the one its own pass reported after its input batch. The root input's list of
/// windows holds each child as of its previous pass, so it is not read for focus. The app is inactive when no
/// window reports focus: the batch that took focus from one window gave it to no other.
fn follow_activity(ctx: &Context) {
    let windows = viewports::reported(ctx);
    let focused: Vec<ViewportId> = windows
        .iter()
        .map(|(viewport, _)| *viewport)
        .filter(|viewport| PerViewport::<Record>::with(ctx, *viewport, |record| record.facts.is_some_and(|facts| facts.focused)))
        .collect();
    let changed = shared(ctx, |shared| {
        shared.stacking.retain(|viewport| windows.iter().any(|(open, _)| open == viewport) && !focused.contains(viewport));
        shared.stacking.extend(&focused);
        let away = focused.is_empty();
        std::mem::replace(&mut shared.away, away) != away
    });
    if changed {
        ctx.request_repaint();
    }
}

/// On the root window's focus gain, re-reads the desktop's caption layout and gives the root frame the change.
fn follow_layout(ctx: &Context, focused: bool) {
    shared(ctx, |shared| {
        let gained = focused && !std::mem::replace(&mut shared.root_focused, focused);
        if !gained {
            return;
        }
        if let Some(layouts) = &mut shared.layouts
            && layouts.on_focus()
            && let Some(Frame::Integrated(captions)) = &mut shared.root_frame
        {
            captions.layout = layouts.layout().clone();
        }
    });
}
