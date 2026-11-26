//! `WindowHost`: the one multi-window driver. It owns `ShellInfo`, refreshes each viewport's `WindowInfo`, releases
//! a press after an OS resize (H4), raises a modal's window for a blocked press (H5), opens viewport mounts, keeps
//! window placement in `craft_window::restore::Store`, routes a root close through the platform quit from the app's
//! `logic`, and maps pointers to desktop space for drags that cross windows.

use crate::runtime::cursor;
use crate::runtime::egui::input::{self as raw_input, RawInput};
use crate::runtime::egui::{injected, pass, repaint, viewports};
use crate::runtime::frame::PerViewport;
use crate::runtime::ime;
use crate::runtime::overlay;
use crate::runtime::pass_state;
use craft_behavior::{Cursor, CursorIcon, CursorPriority};
use craft_host::target::{Backend, Caps, EdgeInsets, Os, ShellInfo, WindowChrome, WindowKind, WindowState};
use craft_window::desktop::Desktop;
use craft_window::native::NativeWindow;
use craft_window::policy::{Captions, Decision, Frame, LayoutWatch};
use craft_window::restore::Store;
use craft_window::system::{HeldButtons, WindowSystem};
use egui::{Context, Id, Pos2, Rect, Vec2, ViewportId};
use std::sync::{Arc, Mutex};

pub use crate::runtime::egui::injected::{InjectedPointer, Released};
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
pub(crate) use host::{clear_source_pointer, note_moved_pointer, note_source_pointer, system_pointer};
#[cfg(windows)]
#[doc(hidden)]
pub use native::route_root;
pub use native::{WindowAction, act, double_click_action};
use placement::record_placement;
pub(crate) use shell::base_shell;
#[doc(hidden)]
pub use shell::os_of;
#[doc(hidden)]
pub use shell::shell_of;
use shell::{refresh, window_info};
#[cfg(feature = "testing")]
pub(crate) use shell::{set_chrome, set_os};
use spec::{AppState, chrome_of};
pub use spec::{WindowLevel, WindowSpec};
pub(crate) use viewport::{PopupOpen, ViewportOpen, close_popup, place_popup, show_popup, show_viewport, take_close_request, unplaced};

/// Seconds between writes of a changed placement store.
const FLUSH_SECONDS: f64 = 1.0;

/// Seconds after an app-modal layer opens until its window takes the keyboard from a window it blocks: an OS dialog
/// the app opens with the modal (a file dialog) takes the focus a little later, and a focus request sent before then
/// could bring the modal's window over it.
const MODAL_FOCUS_WAIT: f64 = 0.2;

/// Install-time inputs and the cross-window records of one context.
#[derive(Default)]
pub(crate) struct Shared {
    /// The opacity requests since the last [`take_sent`], for trials: the platform applies them out of reach.
    #[cfg(feature = "testing")]
    sent: Vec<(ViewportId, WindowAction)>,
    shell: Option<ShellInfo>,
    /// The app id from `InstallSpec::app_id`, which every OS window carries; empty when the app set none.
    app_id: String,
    system: Option<Arc<dyn WindowSystem>>,
    store: Option<Arc<Mutex<Store>>>,
    /// The system pointer read in `set_input`.
    system_pointer: Option<Pos2>,
    /// The mouse buttons and the command modifier as the system told them in `set_input`; `None` where it does not.
    system_buttons: Option<HeldButtons>,
    system_command: Option<bool>,
    /// The window the OS moves for a drag it was handed where the platform tracks such moves
    /// ([`Caps::OS_MOVES_TRACKED`]), until the move ends.
    tracked_move: Option<ViewportId>,
    /// The pointer of the viewport a drag started in, in desktop points, for platforms that tell no system pointer.
    source_pointer: Option<Pos2>,
    /// Containers whose viewport the OS asked to close in this root frame.
    close_requests: Vec<Id>,
    last_flush: f64,
    /// The root window's frame and its caption layout watch, from `InstallSpec::frame`.
    root_frame: Option<Frame>,
    layouts: Option<LayoutWatch>,
    /// The desktop the installed frame decision was made for (`Decision::desktop`); `None` without a decision.
    desktop: Option<Desktop>,
    /// Whether the root window had focus in the last root pass.
    root_focused: bool,
    /// The app's state when the last frame's windows had all reported, which its palettes follow.
    app: AppState,
    /// Whether a palette window stood lowered under an app-modal layer in this frame.
    lowered: bool,
    /// Where the raise of the modal's window above the palettes it lowered stands.
    raise: Raise,
    /// The pass time at which the modal's window takes the keyboard from a window it blocks, until then.
    modal_focus: Option<f64>,
    /// Whether the root window showed an app-modal or popup layer in the last root pass: palette windows are hidden
    /// meanwhile, and one that opens meanwhile opens hidden.
    palettes_yield: bool,
    /// The app's windows from the one focused longest ago to the one focused last: the stacking order of
    /// overlapping windows.
    stacking: Vec<ViewportId>,
    /// The windows that got a primary press, each with the root frame of the press, in this root frame and the last.
    presses: Vec<(ViewportId, u64)>,
}

/// One viewport's window record.
#[derive(Default)]
pub(crate) struct Record {
    facts: Option<viewports::Facts>,
    /// The measured traffic-lights inset, kept until the scale changes or full screen ends.
    lights: craft_window::mac::Cache,
    /// What the OS reserves at the window's edges, once measured.
    insets: Option<EdgeInsets>,
    kind: Option<WindowKind>,
    chrome: Option<WindowChrome>,
    /// The caption buttons the app draws in this window.
    captions: Option<Captions>,
    /// The level the window was opened with or last set to; a palette's egui level follows the app's state.
    level: Option<WindowLevel>,
    /// Whether the window is hidden because the app's state hides it (a palette while the main window is minimized or
    /// the root window shows an app-modal or popup layer).
    hidden: bool,
    /// The buttons that are down as the viewport's events left the input hook.
    buttons_down: Vec<egui::PointerButton>,
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
    /// The app runs the window's move from its move band (a dock window's title row, whose move docks the window), so a
    /// press on the band must reach it and the operating system's move loop is not started from the hit test. Set from
    /// the mount's `ViewportOpen::moved_by_app` before each of the window's passes, its first included.
    moved_by_app: bool,
    /// While the app moves the window: the desktop pointer, at the pressed point of the window where the move last put
    /// it. The window's reported place lags the move.
    moved_pointer: Option<Pos2>,
    /// The `movable` flag last set on the OS window.
    movable_applied: Option<bool>,
    /// Where a window `show_viewport` opened is as far as its caller knows: its outer top-left corner and inner size,
    /// as its own pass last saw them, or as [`act`] last commanded. A caller that records the window's place records
    /// this one, so a caller that follows the window places it here and sends it nothing.
    known: Option<Rect>,
    /// The place the window's own pass last saw.
    seen: Option<Rect>,
    /// The smallest inner size its caller asks for, as the window's builder carries it.
    min_size: Option<Vec2>,
    /// For a popup window ([`show_popup`]): the window its layer was opened from, while the layer shows.
    opener: Option<ViewportId>,
    /// The window an Escape nothing in this one takes goes to: the one it was opened from, where its spec says so
    /// (`WindowSpec::hands_on_cancel`).
    cancel_to: Option<ViewportId>,
}

/// The raise of the window holding an app-modal layer above the palettes the modal lowered. The raise goes out in the
/// frame after a palette was built lowered: a raise in the same frame may reach the OS before the palette's new level
/// does, and the palette, made an ordinary window after it, would stand in front again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Raise {
    /// No palette has stood lowered under the open modal yet, or no modal is open.
    #[default]
    Waiting,
    /// A palette stood lowered in the last frame: the modal's window is raised in this one.
    Due,
    /// The modal's window was raised.
    Done,
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
    pass_state::with::<Shared, _>(ctx, f)
}

/// The opacity requests made since the last call: a trial's window action stream, as the platform applies them where
/// no egui command shows them.
#[cfg(feature = "testing")]
pub(crate) fn take_sent(ctx: &Context) -> Vec<(ViewportId, WindowAction)> {
    shared(ctx, |shared| std::mem::take(&mut shared.sent))
}

/// Whether the OS resizes `viewport`'s window from a frame of its own: it draws the frame (macOS `Inset`, a titled
/// window under the app's bar) and the window is resizable. A window its caller sizes opens not resizable, so the OS
/// leaves it to the app's grips.
pub(crate) fn os_resizes(ctx: &Context, viewport: ViewportId) -> bool {
    let chrome = shell_of(ctx, viewport).window.chrome;
    chrome == WindowChrome::Inset && PerViewport::<Record>::with(ctx, viewport, |record| record.resizable != Some(false))
}

/// The window an Escape nothing in the running viewport's window takes goes to (`WindowSpec::hands_on_cancel`).
pub(crate) fn cancel_to(ctx: &Context) -> Option<ViewportId> {
    PerViewport::<Record>::with_existing(ctx, pass::current_viewport(ctx), |record| record.cancel_to).flatten()
}

/// Whether the app runs the move of `viewport`'s window (`Overlay::moved_by_app`).
pub(crate) fn app_moves(ctx: &Context, viewport: ViewportId) -> bool {
    PerViewport::<Record>::with(ctx, viewport, |record| record.moved_by_app)
}

/// Called by `install`: keeps the shell, the app id, the OS services and the placement store.
pub(crate) fn install(
    ctx: &Context,
    shell: ShellInfo,
    system: Arc<dyn WindowSystem>,
    store: Option<Arc<Mutex<Store>>>,
    frame: Option<Decision>,
    app_id: String,
) {
    // The watch may read the desktop's settings, so it is built outside the lock.
    let layouts = frame.as_ref().map(LayoutWatch::new);
    shared(ctx, |shared| {
        shared.shell = Some(shell);
        shared.app_id = app_id;
        shared.system = Some(system);
        shared.store = store;
        match frame.zip(layouts) {
            Some((decision, layouts)) => keep_root_frame(shared, decision, layouts),
            None => {
                shared.desktop = None;
                shared.root_frame = None;
                shared.layouts = None;
            }
        }
    });
}

/// Keeps `decision` as the root window's frame: the frame and desktop [`WindowHost::frame`] and
/// [`WindowHost::desktop`] read, the caption layout watch `layouts` built for it, and the root shell window's chrome.
/// `install` and [`WindowHost::set_root_frame`] both keep their decision through it.
fn keep_root_frame(shared: &mut Shared, decision: Decision, layouts: LayoutWatch) {
    shared.shell = shared.shell.take().map(|shell| {
        let window = (*shell.window).clone().chrome(chrome_of(&decision.frame));
        shell.window(window)
    });
    shared.desktop = Some(decision.desktop);
    shared.root_frame = Some(decision.frame);
    shared.layouts = Some(layouts);
}

/// Before the pass, from `set_input`, outside every egui lock: reads the OS services and refreshes the window
/// info of every viewport the last root input reported. The system tells the pointer in its own points; egui's are
/// those divided by the interface zoom.
pub(crate) fn set_input(ctx: &Context) {
    let system = shared(ctx, |shared| shared.system.clone());
    let zoom = ctx.zoom_factor();
    let pointer = system.as_ref().and_then(|system| system.desktop_pointer()).map(|[x, y]| Pos2::new(x, y) / zoom);
    let buttons = system.as_ref().and_then(|system| system.buttons_held());
    let command = system.as_ref().and_then(|system| system.command_held());
    let mut discard = false;
    for (viewport, facts) in viewports::reported(ctx) {
        refresh(ctx, viewport, facts);
        discard |= ime::take_discard(ctx, viewport);
    }
    shared(ctx, |shared| {
        shared.system_pointer = pointer;
        shared.system_buttons = buttons;
        shared.system_command = command;
    });
    if discard && let Some(system) = &system {
        system.discard_marked_text();
    }
}

/// The system's bit of an egui pointer button.
fn held_of(button: egui::PointerButton) -> HeldButtons {
    match button {
        egui::PointerButton::Primary => HeldButtons::PRIMARY,
        egui::PointerButton::Secondary => HeldButtons::SECONDARY,
        egui::PointerButton::Middle => HeldButtons::MIDDLE,
        egui::PointerButton::Extra1 => HeldButtons::EXTRA1,
        egui::PointerButton::Extra2 => HeldButtons::EXTRA2,
    }
}

/// The least change of an inner size, in points, that counts as a resize: far under one physical pixel at any scale
/// (a quarter point at 400 %), far over the float rounding of a size the platform reports again in points from its
/// pixels as the window moves (290.4 coming back as 290.40002 at 125 %).
const RESIZE_LEAST: f32 = 0.05;

/// Whether a window's inner size changed from `before` to `now` by more than float rounding.
fn size_changed(before: egui::Vec2, now: egui::Vec2) -> bool {
    (before.x - now.x).abs() > RESIZE_LEAST || (before.y - now.y).abs() > RESIZE_LEAST
}

/// H4, in the input hook. The OS takes the button release of a move or resize it runs, so the press stays down in
/// egui and every later press is lost. Two cases end it with a release of the primary button, in any chrome: a move
/// or resize [`act`] handed the OS, when the input brings no release of its own (where the OS sends one, a second
/// would turn into a click), once the system tells the button is up, or where it does not tell, on the first input
/// after the hand-off; and, where the OS resizes undecorated windows from their edges
/// ([`Caps::OS_RESIZES_UNDECORATED`]), a size change of a window the OS may resize under a held primary press (the
/// size of a window that is not resizable is the app's own). While the OS runs a tracked move
/// ([`Caps::OS_MOVES_TRACKED`]) it sends no key events, so every window's modifiers are the system's command modifier
/// alone. A wheel turn with no pointer egui knows of is placed at the system pointer ([`locate_wheel`]). A button the
/// system reports up that egui still holds, in a frame with no button event and with no injected step holding it, is
/// released once, at the pointer's last position in the frame: the window never received its release, and a release at
/// the press position would be a click. Where the system tells nothing, nothing is released. Reads only the raw input,
/// the system's pointer, button and modifier state, the injected holds and the viewport's record. Returns whether it
/// added an event.
pub(crate) fn input_hook(ctx: &Context, input: &mut RawInput) -> bool {
    let viewport = input.viewport_id;
    replay_caption_events(ctx, viewport, input);
    let located = locate_wheel(ctx, input);
    let base = base_shell(ctx);
    let undecorated = base.caps.contains(Caps::OS_RESIZES_UNDECORATED) && window_info(ctx, viewport, &base).chrome == WindowChrome::Integrated;
    let size = viewports::inner_size_in(input, viewport);
    let pass = pass::pass_nr(ctx, viewport);
    let os_released =
        input.events.iter().any(|event| matches!(event, egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, .. }));
    let no_button_event = !input.events.iter().any(|event| matches!(event, egui::Event::PointerButton { .. }));
    let (buttons, command, tracked) = shared(ctx, |shared| (shared.system_buttons, shared.system_command, shared.tracked_move));
    let primary_held = buttons.is_some_and(|held| held.contains(HeldButtons::PRIMARY));
    if tracked.is_some()
        && let Some(command) = command
    {
        let mac = base.os == Os::Mac;
        let modifiers = egui::Modifiers { command, mac_cmd: command && mac, ctrl: command && !mac, ..egui::Modifiers::NONE };
        input.events.push(egui::Event::ModifiersChanged(modifiers));
    }
    let lost: Vec<egui::PointerButton> = match buttons {
        Some(held) if no_button_event => {
            let down = PerViewport::<Record>::with(ctx, viewport, |record| record.buttons_down.clone());
            down.into_iter().filter(|button| !held.contains(held_of(*button)) && !injected::held_button(ctx, viewport, *button)).collect()
        }
        _ => Vec::new(),
    };
    let (released, handed_off) = PerViewport::<Record>::with(ctx, viewport, |record| {
        let resized =
            record.resizable != Some(false) && matches!((record.last_inner_size, size), (Some(before), Some(now)) if size_changed(before, now));
        record.last_inner_size = size.or(record.last_inner_size);
        let handed_off = record.handoff.is_some_and(|handoff| handoff.released.is_none() && handoff.pass < pass) && !primary_held;
        if handed_off {
            record.handoff = record.handoff.map(|handoff| HandOff { released: Some(pass), ..handoff });
        }
        let primary = egui::PointerButton::Primary;
        // A lost primary release already ends the press.
        let release =
            record.buttons_down.contains(&primary) && !lost.contains(&primary) && ((undecorated && resized) || (handed_off && !os_released));
        if !lost.is_empty() {
            let at = input
                .events
                .iter()
                .rev()
                .find_map(|event| match event {
                    egui::Event::PointerMoved(pos) => Some(*pos),
                    _ => None,
                })
                .or(record.last_pos)
                .unwrap_or(Pos2::ZERO);
            for button in &lost {
                raw_input::push_release(input, *button, at);
            }
        }
        if release {
            raw_input::push_release(input, primary, record.last_pos.unwrap_or(Pos2::ZERO));
        }
        raw_input::follow_buttons(&input.events, &mut record.buttons_down, &mut record.last_pos);
        (release || !lost.is_empty(), handed_off)
    });
    if handed_off && tracked == Some(viewport) {
        shared(ctx, |shared| shared.tracked_move = None);
    }
    let pressed =
        input.events.iter().any(|event| matches!(event, egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: true, .. }));
    if pressed {
        let frame = pass::root_frame_nr(ctx);
        shared(ctx, |shared| {
            shared.presses.retain(|(_, at)| at + 1 >= frame);
            shared.presses.push((viewport, frame));
        });
    }
    released || located
}

/// #1518: after the user switches back to a window, the platform sends no pointer move until the mouse moves, so a
/// wheel turn finds no pointer and scrolls nothing. When `input` turns the wheel, brings no pointer move or leave of its
/// own, and egui knows no pointer in the viewport, a move to the system pointer (read in [`set_input`]) goes before
/// the wheel, where that pointer lies inside the window. It acts on this input's own wheel event only: a rerun pass of
/// the frame has no events, and the next pass knows the pointer, so the move is added once. Returns whether it added it.
fn locate_wheel(ctx: &Context, input: &mut RawInput) -> bool {
    let wheel = input.events.iter().any(|event| matches!(event, egui::Event::MouseWheel { .. }));
    let moved = input.events.iter().any(|event| matches!(event, egui::Event::PointerMoved(_) | egui::Event::PointerGone));
    if !wheel || moved {
        return false;
    }
    let viewport = input.viewport_id;
    // egui pushes the viewport after this hook, so its pointer is read by id.
    if ctx.input_for(viewport, |state| state.pointer.latest_pos()).is_some() {
        return false;
    }
    let Some(inner) = input.viewports.get(&viewport).and_then(|info| info.inner_rect) else { return false };
    let Some(desktop) = shared(ctx, |shared| shared.system_pointer) else { return false };
    let local = Pos2::ZERO + (desktop - inner.min);
    if !Rect::from_min_size(Pos2::ZERO, inner.size()).contains(local) {
        return false;
    }
    input.events.insert(0, egui::Event::PointerMoved(local));
    true
}

/// The begin hook (H5), where the viewport is current: refreshes its window info, and raises the modal's window
/// when the input hook dropped a press under a modal in another viewport, or when this viewport gained focus while
/// a modal in another viewport blocks it (the modal's window then comes forward with the window the user clicked).
pub(crate) fn begin(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    // A viewport's first pass has no previous facts and is not a focus gain.
    let had_focus_before = PerViewport::<Record>::with(ctx, viewport, |record| record.facts.map(|facts| facts.focused));
    let facts = viewports::current(ctx);
    refresh(ctx, viewport, facts);
    reshow_cursor(ctx, viewport);
    let gained_focus = facts.focused && had_focus_before == Some(false);
    if (overlay::take_dropped_press(ctx, viewport) || gained_focus)
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

/// Whether container `id`'s viewport mount shows in an OS window of its own this pass: OS viewports are available,
/// and the window it opens from is neither full screen nor, on Wayland, maximized. A window opened over such a
/// window covers it; the compositor then sends the covered window no frames, and the immediate viewport, drawn inside
/// them, freezes. Otherwise the mount shows its fallback layer in the opener.
///
/// It reads only this pass's facts, so the mount, the body (which runs inside the child's pass) and the window steps
/// agree within a pass.
pub(crate) fn mount_window(ctx: &Context, id: Id) -> bool {
    let current = pass::current_viewport(ctx);
    let opener = if current == viewports::viewport_id(id) { viewports::parent(ctx) } else { current };
    opens_over(ctx, opener)
}

/// Whether a viewport mount opened from `opener`'s window shows in an OS window of its own: the rule of
/// [`mount_window`].
fn opens_over(ctx: &Context, opener: ViewportId) -> bool {
    if !WindowHost::of(ctx).viewports_available() {
        return false;
    }
    let shell = shell_of(ctx, opener);
    let covers = match shell.window.state {
        WindowState::Fullscreen => true,
        WindowState::Maximized => shell.backend == Backend::Wayland,
        _ => false,
    };
    !covers
}

/// Where the layers opened in a palette window show: in popup windows of their own, placed in desktop points.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PopupRoom {
    /// The palette window's inner top-left corner in desktop points: the origin of its viewport's points.
    pub origin: Pos2,
    /// The room its layers may take, in its viewport's points: the work area of the monitor the window is on, or where
    /// no monitor is listed, the area the app's windows other than popups span.
    pub room: Rect,
}

/// The room of the layers opened in the running viewport when it is a palette window (a dock window or a popup
/// window, both at [`WindowLevel::Palette`]) that opens OS windows ([`opens_over`]) where the platform places them:
/// every such layer shows in a popup window of its own. `None` in every other window, whose layers stay in it.
pub(crate) fn popup_room(ctx: &Context) -> Option<PopupRoom> {
    let viewport = pass::current_viewport(ctx);
    if !base_shell(ctx).window_positions() || !opens_over(ctx, viewport) {
        return None;
    }
    let facts = PerViewport::<Record>::with_existing(ctx, viewport, |record| record.facts.filter(|_| record.level == Some(WindowLevel::Palette)))??;
    let origin = facts.inner?.min;
    let scale = host::scale_of(ctx, &facts).unwrap_or(1.0).max(f32::EPSILON);
    let work = host::monitor_of(ctx, &facts).map(|monitor| {
        let (pos, size) = monitor.work_px.unwrap_or((monitor.pos_px, monitor.size_px));
        let min = Pos2::new(pos[0] as f32, pos[1] as f32) / scale;
        Rect::from_min_size(min, Vec2::new(size[0] as f32, size[1] as f32) / scale)
    });
    // The popup windows themselves don't count: a popup's room never depends on where a popup stands.
    let spanned = || {
        let popup = |viewport: ViewportId| PerViewport::<Record>::with_existing(ctx, viewport, |record| record.opener.is_some()).unwrap_or(false);
        viewports::reported(ctx)
            .iter()
            .filter(|(viewport, _)| !popup(*viewport))
            .filter_map(|(_, facts)| facts.inner)
            .fold(Rect::NOTHING, Rect::union)
    };
    let room = work.unwrap_or_else(spanned).translate(-origin.to_vec2());
    Some(PopupRoom { origin, room })
}

/// Whether a primary press landed, in this root frame (or in the last one, unless the popup window `popup` `appeared`
/// in this one), in a window of the app other than the running viewport (the popup's opener, whose own presses its
/// layer stack judges) and the popup window and the popup windows opened from it.
pub(crate) fn pressed_elsewhere(ctx: &Context, popup: ViewportId, appeared: bool) -> bool {
    let opener = pass::current_viewport(ctx);
    let frame = pass::root_frame_nr(ctx);
    let presses = shared(ctx, |shared| shared.presses.clone());
    presses.into_iter().filter(|(_, at)| *at == frame || (!appeared && at + 1 == frame)).any(|(viewport, _)| {
        if viewport == opener {
            return false;
        }
        // A press in a popup window opened from the popup is inside its family.
        !in_family(ctx, viewport, popup)
    })
}

/// Whether `viewport`'s window is `window` or a popup window opened from it, at any depth.
fn in_family(ctx: &Context, viewport: ViewportId, window: ViewportId) -> bool {
    let mut at = Some(viewport);
    while let Some(shown) = at {
        if shown == window {
            return true;
        }
        at = PerViewport::<Record>::with_existing(ctx, shown, |record| record.opener).flatten();
    }
    false
}

/// Whether no window of the app has focus, as the last frame's windows reported.
pub(crate) fn app_away(ctx: &Context) -> bool {
    shared(ctx, |shared| shared.app.away)
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

/// The end hook: sets the window's `movable` flag, keeps the root window's placement, writes a changed placement store
/// once a second, and drops the close requests the viewport mounts did not take in this frame.
pub(crate) fn end(ctx: &Context) {
    publish_regions(ctx, pass::current_viewport(ctx));
    sync_movable(ctx);
    if pass::current_viewport(ctx) != ViewportId::ROOT {
        return;
    }
    shared(ctx, |shared| shared.close_requests.clear());
    follow_activity(ctx);
    follow_overlays(ctx);
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

/// Sets the `movable` flag of the running viewport's OS window from this pass's [`viewports::movable`]: the one place
/// AppKit's flag is set, after the window's UI has run. A window is told apart by its viewport: the key window is the one
/// whose viewport has focus, so an unfocused viewport is left as it is. AppKit is only called when the flag changes.
/// Only a macOS window with a full-size content view has a flag to set; elsewhere this does nothing. A window whose
/// move the app runs (`Overlay::moved_by_app`) keeps AppKit's own flag: AppKit moves it for the move's hand-off and
/// resizes it from its edges only while it is movable.
fn sync_movable(ctx: &Context) {
    if !viewports::current(ctx).focus_reported {
        return;
    }
    let movable = viewports::movable(ctx);
    let viewport = pass::current_viewport(ctx);
    PerViewport::<Record>::with(ctx, viewport, |record| {
        if !record.moved_by_app && record.movable_applied != Some(movable) {
            craft_window::mac::set_movable(movable, &mut record.movable_applied);
        }
    });
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

/// Keeps the app's state and the stacking order of its windows current from the windows' facts. A palette's window
/// level and visibility follow the state: the mounts build their viewports with the state of the moment, and a change
/// asks for the next frame. Runs at the end of the root pass, inside which every child window's pass of the frame
/// has run, so each window's focus is the one its own pass reported after its input batch. The root input's list of
/// windows holds each child as of its previous pass, so it is not read for focus. The app is inactive when no
/// window reports focus: the batch that took focus from one window gave it to no other.
///
/// Once a palette has stood lowered under an app-modal layer, the modal's window is raised above it, once per modal.
/// A window that has the keyboard when a modal comes to block it drops the keys the modal needs, so the modal's window
/// takes the keyboard from it ([`focus_modal`]).
fn follow_activity(ctx: &Context) {
    let windows = viewports::reported(ctx);
    let focused: Vec<ViewportId> = windows
        .iter()
        .map(|(viewport, _)| *viewport)
        .filter(|viewport| PerViewport::<Record>::with(ctx, *viewport, |record| record.facts.is_some_and(|facts| facts.focused)))
        .collect();
    let modal = overlay::app_modal_viewport(ctx);
    let minimized = viewports::current(ctx).minimized;
    let now = pass::now(ctx);
    let (repaint, raise) = shared(ctx, |shared| {
        shared.stacking.retain(|viewport| windows.iter().any(|(open, _)| open == viewport) && !focused.contains(viewport));
        shared.stacking.extend(&focused);
        let app = AppState { away: focused.is_empty(), modal, minimized };
        let lowered = std::mem::take(&mut shared.lowered);
        let modal_changed = app.modal != shared.app.modal;
        if modal_changed {
            shared.modal_focus = app.modal.map(|_| now + MODAL_FOCUS_WAIT);
        }
        let (step, raise) = match shared.raise {
            _ if modal_changed => (Raise::Waiting, None),
            Raise::Waiting if lowered => (Raise::Due, None),
            Raise::Due => (Raise::Done, app.modal),
            step => (step, None),
        };
        shared.raise = step;
        let changed = std::mem::replace(&mut shared.app, app) != app;
        (changed || step == Raise::Due, raise)
    });
    if let Some(modal) = raise {
        act(ctx, modal, WindowAction::Raise);
    }
    if let Some(modal) = modal {
        focus_modal(ctx, modal, &windows);
    }
    if repaint {
        ctx.request_repaint();
    }
}

/// Gives the window of `modal`, the app's top modal, the OS focus when one of `windows` that it blocks still has it
/// [`MODAL_FOCUS_WAIT`] after the modal opened; once per modal. With the focus elsewhere by then (in the modal's own
/// window, in an OS dialog, in another app) nothing is sent.
fn focus_modal(ctx: &Context, modal: ViewportId, windows: &[(ViewportId, viewports::Facts)]) {
    let Some(due) = shared(ctx, |shared| shared.modal_focus) else { return };
    let blocked = |viewport: ViewportId| overlay::modal_viewport(ctx, viewport) == Some(modal) && !in_family(ctx, viewport, modal);
    let wait = due - pass::now(ctx);
    if wait > 0.0 {
        if windows.iter().any(|(viewport, _)| blocked(*viewport)) {
            repaint::request_after(ctx, ViewportId::ROOT, wait);
        }
        return;
    }
    shared(ctx, |shared| shared.modal_focus = None);
    let focused = |viewport: ViewportId| {
        PerViewport::<Record>::with_existing(ctx, viewport, |record| record.facts.is_some_and(|facts| facts.focus_reported)).unwrap_or(false)
    };
    if !focused(modal) && windows.iter().any(|(viewport, _)| focused(*viewport) && blocked(*viewport)) {
        act(ctx, modal, WindowAction::Focus);
    }
}

/// Records whether the root window shows an app-modal or popup layer: palette windows hide meanwhile, so a click on a
/// palette cannot bring it over the layer, and the palettes' mounts hide and show them as they do for the app's state
/// ([`show_viewport`]), asked for by a repaint when it changes. Runs at the end of the root pass, where the
/// layers of the running viewport are the root's. Focus moving between the app's windows does not touch it.
fn follow_overlays(ctx: &Context) {
    let shows = overlay::any_open(ctx, overlay::OpenFilter::Keys);
    if shared(ctx, |shared| std::mem::replace(&mut shared.palettes_yield, shows) != shows) {
        ctx.request_repaint();
    }
}

/// On the root window's focus gain, re-reads the desktop's caption layout and gives the root frame the change.
fn follow_layout(ctx: &Context, focused: bool) {
    let gained = shared(ctx, |shared| {
        let gained = focused && !std::mem::replace(&mut shared.root_focused, focused);
        if !gained {
            return false;
        }
        if let Some(layouts) = &mut shared.layouts
            && layouts.on_focus()
            && let Some(Frame::Integrated(captions)) = &mut shared.root_frame
        {
            captions.layout = layouts.layout().clone();
        }
        true
    });
    if gained {
        crate::runtime::env::system::set_active(ctx, true);
    }
}
