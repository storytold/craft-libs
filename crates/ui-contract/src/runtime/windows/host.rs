//! WindowHost, the multi-window driver: window facts, monitors and the pointer of a drag across windows.

use super::hit::{CaptionRegions, HitSink, with_exclusive};
use super::native::{WindowAction, act};
use super::placement::placement_of;
use super::shell::{base_shell, shell_of, window_info};
use super::spec::frame_of;
use super::{Record, keep_root_frame, shared};
use crate::runtime::egui::access::{AccessTree, Declared, NodeKey};
use crate::runtime::egui::injected::{self, InjectedPointer, Released};
use crate::runtime::egui::input_filter;
use crate::runtime::egui::screenshot::{ScreenshotFacts, ScreenshotSlot};
use crate::runtime::egui::{pass, screenshot, viewports};
use crate::runtime::frame::PerViewport;
use craft_behavior::placement::Placement;
use craft_behavior::{Monitor, PointerButton};
use craft_host::target::{Caps, ShellInfo, WindowState};
use craft_keymap::{Chord, Key, Mods};
use craft_window::desktop::Desktop;
use craft_window::policy::{Decision, Frame, LayoutWatch};
use egui::{Context, Id, Pos2, Rect, Vec2, ViewportId};

/// What the OS tells about a window, in desktop points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WindowFacts {
    /// The inner rect.
    pub inner: Rect,
    /// The outer rect, where the platform tells.
    pub outer: Option<Rect>,
    /// Physical pixels per point of the monitor the window is on: the platform's report, else the scale of the
    /// monitor the window lies on. `None` for a window on no known monitor.
    pub scale: Option<f32>,
    /// Normal, minimized, maximized or full screen.
    pub state: WindowState,
    /// Whether the window has OS focus.
    pub focused: bool,
}

/// A key equivalent the platform's menu took from the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyEquivalent {
    /// The press of `chord`. The platform delivers the key-up itself.
    Press(Chord),
    /// A press and its release of `chord`: a click that stands for the key, so no key-up follows.
    Tap(Chord),
    /// The platform's copy command: egui's copy event.
    Copy,
    /// The platform's cut command: egui's cut event.
    Cut,
}

/// The multi-window driver of one context.
#[derive(Clone)]
pub struct WindowHost {
    ctx: Context,
}

impl WindowHost {
    /// The window host of `ctx`.
    pub fn of(ctx: &Context) -> Self {
        WindowHost { ctx: ctx.clone() }
    }

    /// The viewport the running pass is for.
    pub fn current(&self) -> ViewportId {
        pass::current_viewport(&self.ctx)
    }

    /// The shell as the running viewport sees it.
    pub fn shell(&self) -> ShellInfo {
        shell_of(&self.ctx, pass::current_viewport(&self.ctx))
    }

    /// Whether OS viewports can open: the capability is on and the integration does not embed them.
    pub fn viewports_available(&self) -> bool {
        base_shell(&self.ctx).caps.contains(Caps::VIEWPORTS) && viewports::available(&self.ctx)
    }

    /// Whether a viewport mount opened in the running viewport's pass shows in an OS window of its own, and not in
    /// its fallback layer: OS viewports are available and the running window is neither full screen nor, on Wayland,
    /// maximized.
    pub fn opens_windows(&self) -> bool {
        super::opens_over(&self.ctx, pass::current_viewport(&self.ctx))
    }

    /// Whether the platform reports where windows are on the desktop and moves them there; false on Wayland.
    pub fn positions_reported(&self) -> bool {
        base_shell(&self.ctx).window_positions()
    }

    /// Whether the running viewport's window is drawn; a minimized or covered window runs no UI pass, so this reads
    /// false only from the app's `logic`.
    pub fn visible(&self) -> bool {
        viewports::current(&self.ctx).visible
    }

    /// Runs and paints the next pass even while the window is hidden; holds for one frame.
    pub fn paint_while_hidden(&self) {
        viewports::paint_while_hidden(&self.ctx, crate::runtime::egui::pass::current_viewport(&self.ctx));
    }

    /// Issues one screenshot of `viewport`: a fresh slot bounded to `max_image_bytes` for its one image, and a pass of
    /// the viewport even while its window is hidden. The bound covers the single retained image; the application
    /// reserves its aggregate budget before asking.
    pub fn screenshot(&self, viewport: ViewportId, max_image_bytes: usize) -> ScreenshotSlot {
        screenshot::issue(&self.ctx, viewport, max_image_bytes)
    }

    /// Whether `viewport`'s window is open this pass, measured or not.
    pub fn is_open(&self, viewport: ViewportId) -> bool {
        viewports::reported(&self.ctx).iter().any(|(id, _)| *id == viewport)
    }

    /// What a screenshot of `viewport` measures this pass; `None` when the viewport is not running.
    pub fn screenshot_facts(&self, viewport: ViewportId) -> Option<ScreenshotFacts> {
        screenshot::facts(&self.ctx, viewport)
    }

    /// Queues a pointer move to `at` in `viewport` with `mods` held, delivered on a pass of its own; the modifiers stay
    /// held after it, and a button held by earlier injected input stays down, so the move drags. With `rest` (seconds)
    /// the input counts as processed only once the pointer has rested that long. As a window never reports a NaN or
    /// infinite pointer, a non-finite `at` here and in every `inject_*` below queues nothing; a release there releases
    /// where the injected pointer is.
    pub fn inject_move(&self, viewport: ViewportId, at: Pos2, mods: Mods, rest: f64) {
        injected::inject_move(&self.ctx, viewport, at, mods, rest);
    }

    /// Queues a move to `at`, then `count` presses and releases of `button` with `mods` held, one pass each. With
    /// `hold` the last press stays down until [`inject_release`](Self::inject_release). A button egui has no
    /// counterpart for queues nothing.
    pub fn inject_click(&self, viewport: ViewportId, at: Pos2, button: PointerButton, count: u32, mods: Mods, hold: bool) {
        injected::inject_click(&self.ctx, viewport, at, button, count, mods, hold);
    }

    /// Queues `key` pressed with `mods` held, as the platform reports them, and released unless `hold`.
    pub fn inject_key(&self, viewport: ViewportId, key: Key, mods: Mods, hold: bool) {
        injected::inject_key(&self.ctx, viewport, key, mods, hold);
    }

    /// Queues a move to `at`, then a wheel turn of `delta` in `unit` with `mods` held. A non-finite `delta` queues
    /// nothing.
    pub fn inject_wheel(&self, viewport: ViewportId, at: Pos2, delta: Vec2, unit: egui::MouseWheelUnit, mods: Mods) {
        // PLAN: `injected::inject_wheel` (injected.rs:477) takes `unit` after `delta` once shell-w1 writes it (sh.K4)
        injected::inject_wheel(&self.ctx, viewport, at, delta, unit, mods);
    }

    /// Turns AccessKit on, idempotent and off by default; from then the settled tree of every finished pass is kept
    /// per viewport (`access_tree`).
    pub fn keep_access_trees(&self) {
        self.ctx.enable_accesskit();
    }

    /// The access tree of `viewport`'s latest settled pass, a host fact of the window as its screenshot facts are;
    /// `None` while AccessKit is off or before the viewport has run.
    pub fn access_tree(&self, viewport: ViewportId) -> Option<std::sync::Arc<crate::runtime::egui::access::AccessTree>> {
        crate::runtime::egui::access::settled_tree(&self.ctx, viewport)
            .map(|update| std::sync::Arc::new(crate::runtime::egui::access::AccessTree::new(update)))
    }

    /// Queues the release of what `released` names, at `at` or where the injected pointer is.
    pub fn inject_release(&self, viewport: ViewportId, at: Option<Pos2>, released: Released) {
        injected::inject_release(&self.ctx, viewport, at, released);
    }

    /// Queues one pointer step in `viewport` as a device would send it, one pass per event, with the pen's reading
    /// for a pen: its pass reads the reading in place of the tablet's.
    pub fn inject_pointer(&self, viewport: ViewportId, step: InjectedPointer) {
        injected::inject_pointer(&self.ctx, viewport, step);
    }

    /// Queues `key` held down in `viewport` with no key event, on a pass of its own: no shortcut or key stage sees it,
    /// only [`injected_holds`](Self::injected_holds), until [`inject_let_go`](Self::inject_let_go). It ends where
    /// injected input is dropped: a withdrawal, a window focus loss, a cancel, the end of the injection session.
    pub fn inject_hold(&self, viewport: ViewportId, key: Key) {
        injected::inject_hold(&self.ctx, viewport, key, true);
    }

    /// Queues the release of a key [`inject_hold`](Self::inject_hold) holds, on a pass of its own.
    pub fn inject_let_go(&self, viewport: ViewportId, key: Key) {
        injected::inject_hold(&self.ctx, viewport, key, false);
    }

    /// The keys [`inject_hold`](Self::inject_hold) holds in `viewport`, as of its running pass.
    pub fn injected_holds(&self, viewport: ViewportId) -> Vec<Key> {
        injected::holds(&self.ctx, viewport)
    }

    /// Holds Caps Lock on or off in place of the platform's state; `None` returns to the platform's.
    pub fn set_caps_lock(&self, on: Option<bool>) {
        injected::set_caps_lock(&self.ctx, on);
    }

    /// The Caps Lock state injected input holds, when it holds one.
    pub fn injected_caps_lock(&self) -> Option<bool> {
        injected::caps_lock(&self.ctx)
    }

    /// Whether the running pass's input carries injected input: what it triggers acts for an automation client.
    pub fn injected_pass(&self) -> bool {
        injected::in_pass(&self.ctx)
    }

    /// Adds `equivalent` to the next input of `viewport`: a key equivalent the platform's menu took from the window
    /// reaches the application as the key press the user made.
    pub fn key_equivalent(&self, viewport: ViewportId, equivalent: KeyEquivalent) {
        input_filter::add_key_equivalent(&self.ctx, viewport, equivalent);
    }

    /// Records whether the platform's menu of `viewport` is open. While it is, the window's next press or Escape is
    /// taken from the input once (it closes the menu); see [`take_menu_close`](Self::take_menu_close).
    pub fn menu_open(&self, viewport: ViewportId, open: bool) {
        input_filter::set_menu_open(&self.ctx, viewport, open);
    }

    /// True once after a press or Escape reached `viewport`'s input: the platform's menu is to close.
    pub fn take_menu_close(&self, viewport: ViewportId) -> bool {
        input_filter::take_menu_close(&self.ctx, viewport)
    }

    /// Queues `text` as typed text.
    pub fn inject_text(&self, viewport: ViewportId, text: &str) {
        injected::inject_text(&self.ctx, viewport, text);
    }

    /// Queues `event` as the platform's input method would deliver it (a preedit or a commit), behind earlier
    /// injected input.
    pub fn inject_ime(&self, viewport: ViewportId, event: egui::ImeEvent) {
        injected::inject_ime(&self.ctx, viewport, event);
    }

    /// Hovers the files over `at` and, with `drop`, drops them in the following injected step, through the same
    /// hovered and dropped file input a window's drag session fills.
    pub fn inject_files(&self, viewport: ViewportId, at: Option<Pos2>, paths: Vec<std::path::PathBuf>, drop: bool) {
        injected::inject_files(&self.ctx, viewport, at, paths, drop);
    }

    /// Queues one AccessKit action request (click, focus or context menu) on `node` as one injected step behind
    /// earlier injected input.
    pub fn inject_access(&self, viewport: ViewportId, node: NodeKey, action: Declared) {
        injected::inject_access(&self.ctx, viewport, node, action);
    }

    /// Turns AccessKit output on, so [`access_tree`](Self::access_tree) and
    /// [`access_tree_view`](Self::access_tree_view) have each viewport's settled tree from then on.
    pub fn keep_access_tree(&self) {
        crate::runtime::egui::access::keep_trees(&self.ctx);
    }

    /// [`access_tree`](Self::access_tree) as a tree to walk: `None` before the viewport's first pass or while
    /// AccessKit is off ([`keep_access_tree`](Self::keep_access_tree) turns it on).
    pub fn access_tree_view(&self, viewport: ViewportId) -> Option<AccessTree> {
        crate::runtime::egui::access::kept_tree(&self.ctx, viewport)
    }

    /// Changes the open root window's frame to `decision`'s and returns true: the window gets the commands
    /// [`policy::reframe`](craft_window::policy::reframe) gives, and [`frame`](Self::frame),
    /// [`desktop`](Self::desktop) and the root shell's chrome follow at once. A change of caption buttons alone sends
    /// no command, and the decision in force changes nothing. A change to or from the inset frame cannot reach an open
    /// window: it keeps the current decision and returns false. Call it when the app's decision changes (the user's
    /// title bar setting); between calls the caption layout follows the desktop.
    pub fn set_root_frame(&self, decision: Decision) -> bool {
        let (installed, desktop) = shared(&self.ctx, |shared| (shared.root_frame.clone(), shared.desktop.clone()));
        let current = installed.unwrap_or_else(|| frame_in(&self.ctx, ViewportId::ROOT));
        let Some(commands) = craft_window::policy::reframe(&current, &decision.frame) else { return false };
        if current == decision.frame && desktop.as_ref() == Some(&decision.desktop) {
            return true;
        }
        // The watch may read the desktop's settings, so it is built outside the lock.
        let layouts = LayoutWatch::new(&decision);
        shared(&self.ctx, |shared| keep_root_frame(shared, decision, layouts));
        for command in commands {
            viewports::command(&self.ctx, ViewportId::ROOT, command);
        }
        self.ctx.request_repaint_of(ViewportId::ROOT);
        true
    }

    /// Withdraws the injected input of `viewport` (the document changed) at its next input hook: a step whose pass
    /// has not run and the queue behind it; what is dropped counts as withdrawn
    /// ([`take_injected_withdrawn`](Self::take_injected_withdrawn)). Called from the raw input hook, so every
    /// request queued before the framework's input hook is dropped with the rest.
    pub fn withdraw_injected(&self, viewport: ViewportId) {
        injected::withdraw(&self.ctx, viewport);
    }

    /// Drops every injected queue and step in flight, and the keys [`inject_hold`](Self::inject_hold) holds; true
    /// when anything was pending.
    pub fn cancel_injected(&self) -> bool {
        injected::cancel(&self.ctx)
    }

    /// Ends the injection session: the automation connection that fed it is gone. Drops every queue and step in
    /// flight, queues the release of every button, key and modifier injected input holds (they run on each
    /// viewport's next pass), and returns Caps Lock to the platform's state.
    pub fn end_injection(&self) {
        injected::end(&self.ctx);
    }

    /// Whether no injected step is queued or awaits its pass: the application may answer its waiters.
    pub fn injected_idle(&self) -> bool {
        injected::idle(&self.ctx)
    }

    /// [`injected_idle`](Self::injected_idle) asked at the end of a pass, after its UI ran: the step the running pass
    /// carried counts as run, unless egui runs the pass again. Outside a pass it is `injected_idle`.
    pub fn injected_idle_at_pass_end(&self) -> bool {
        injected::idle_at_pass_end(&self.ctx)
    }

    /// Whether an injected step was withdrawn since the last call.
    pub fn take_injected_withdrawn(&self) -> bool {
        injected::take_withdrawn(&self.ctx)
    }

    /// Asks for one pass of `viewport` after `seconds`; a negative time means now, a non-finite one asks for
    /// nothing. An idle window draws no frames, so a timer that must fire asks for its own.
    pub fn repaint_in(&self, viewport: ViewportId, seconds: f64) {
        if seconds.is_finite() {
            crate::runtime::egui::repaint::request_after(&self.ctx, viewport, seconds);
        }
    }

    /// Sends `action` to `viewport`; see [`act`].
    pub fn act(&self, viewport: ViewportId, action: WindowAction) -> bool {
        act(&self.ctx, viewport, action)
    }

    /// The system pointer in desktop points, as the platform tells it or, failing that, as the window a drag
    /// started in saw it.
    pub fn desktop_pointer(&self) -> Option<Pos2> {
        shared(&self.ctx, |shared| shared.system_pointer.or(shared.source_pointer))
    }

    /// The main window's native handle (the Win32 `HWND` value, read through the safe `HasWindowHandle` API), `None` on
    /// other platforms and where no window is open. A driver dialog takes it as its owner.
    pub fn main_window_handle(&self) -> Option<std::num::NonZeroIsize> {
        // The root window's OS window comes from eframe's handle (`route_root`), read through `HasWindowHandle`.
        PerViewport::<Record>::with(&self.ctx, ViewportId::ROOT, |record| record.native.clone())?.raw_handle()
    }

    /// Moves the system pointer to `to` in desktop points; false where the platform cannot.
    pub fn warp_pointer(&self, to: Pos2) -> bool {
        let system = shared(&self.ctx, |shared| shared.system.clone());
        system.is_some_and(|system| system.warp_pointer([to.x, to.y]))
    }

    /// `viewport`'s window facts, once it has reported its inner rect.
    pub fn facts(&self, viewport: ViewportId) -> Option<WindowFacts> {
        let facts = PerViewport::<Record>::with(&self.ctx, viewport, |record| record.facts)?;
        let info = window_info(&self.ctx, viewport, &base_shell(&self.ctx));
        Some(WindowFacts { inner: facts.inner?, outer: facts.outer, scale: scale_of(&self.ctx, &facts), state: info.state, focused: facts.focused })
    }

    /// The window that has the keys: the one the platform reports focused, else the main window. A key the platform
    /// takes from the windows (a native menu's key equivalent) belongs to it.
    pub fn key_window(&self) -> ViewportId {
        viewports::key_window(&self.ctx)
    }

    /// Whether a text field has the keys in [`key_window`](Self::key_window), as its last pass ended: the focused
    /// stop types text, or egui focus is on a text edit the app drew itself.
    pub fn typing(&self) -> bool {
        crate::runtime::focus::typing(&self.ctx, self.key_window())
    }

    /// `previous` (or a new record) updated to where `viewport`'s window is, recorded as an app window's saved
    /// placement is: where it last reported itself, or where the app last sent it. A caller that places the window at
    /// this placement sends it nothing. `None` before the window has reported its rect and scale.
    #[doc(hidden)]
    pub fn placement(&self, viewport: ViewportId, previous: Option<Placement>) -> Option<Placement> {
        let facts = PerViewport::<Record>::with(&self.ctx, viewport, |record| record.facts)?;
        placement_of(&self.ctx, &super::viewport::at_known(&self.ctx, viewport, facts), previous)
    }

    /// The connected monitors, in desktop pixels; none where the platform does not list them.
    /// [`Monitor::points_at`] gives one in the points of a window ([`WindowFacts::scale`]).
    pub fn monitors(&self) -> Vec<Monitor> {
        monitors(&self.ctx)
    }

    /// The monitor `viewport`'s window lies on: the one its top-left corner is on, the rule its scale
    /// ([`WindowFacts::scale`]) falls back to. `None` before the window has reported its rect, or where the platform
    /// lists no monitor under the corner.
    pub fn monitor_of(&self, viewport: ViewportId) -> Option<Monitor> {
        let facts = PerViewport::<Record>::with(&self.ctx, viewport, |record| record.facts)?;
        monitor_of(&self.ctx, &facts)
    }

    /// Places `viewport`'s outer rect at `outer`: its position, and the inner size that leaves the window's
    /// decoration as it is. A minimized window is restored first, and a placement below the window's smallest inner
    /// size lowers that size to the placed one, so the window takes the place it is given. False when a command could
    /// not go out.
    pub fn set_outer(&self, viewport: ViewportId, outer: Rect) -> bool {
        let facts = self.facts(viewport);
        let decoration = facts.and_then(|facts| facts.outer.map(|current| current.size() - facts.inner.size())).unwrap_or(Vec2::ZERO);
        let inner = (outer.size() - decoration).max(Vec2::ZERO);
        let mut sent = true;
        if facts.is_some_and(|facts| facts.state == WindowState::Minimized) {
            sent &= act(&self.ctx, viewport, WindowAction::Restore);
        }
        let least = PerViewport::<Record>::with(&self.ctx, viewport, |record| record.min_size);
        if let Some(least) = least.filter(|least| inner.x < least.x || inner.y < least.y) {
            sent &= act(&self.ctx, viewport, WindowAction::SetMinInnerSize(least.min(inner)));
        }
        sent &= act(&self.ctx, viewport, WindowAction::SetOuterPosition(outer.min));
        sent & act(&self.ctx, viewport, WindowAction::SetInnerSize(inner))
    }

    /// The viewport of the container `id` while its window is open.
    pub fn viewport_of(&self, id: Id) -> Option<ViewportId> {
        let viewport = viewports::viewport_id(id);
        PerViewport::<Record>::with(&self.ctx, viewport, |record| record.kind.is_some()).then_some(viewport)
    }

    /// The window under the desktop point `at`: where windows overlap, the focused one, else the one focused last,
    /// else the smallest.
    pub fn viewport_at(&self, at: Pos2) -> Option<ViewportId> {
        self.viewports_at(at).first().copied()
    }

    /// The windows under the desktop point `at`, the topmost first (the order of [`Self::viewport_at`]).
    pub fn viewports_at(&self, at: Pos2) -> Vec<ViewportId> {
        let stacking = shared(&self.ctx, |shared| shared.stacking.clone());
        let mut under: Vec<(ViewportId, WindowFacts)> = viewports::reported(&self.ctx)
            .into_iter()
            .filter_map(|(viewport, _)| self.facts(viewport).map(|facts| (viewport, facts)))
            .filter(|(_, facts)| facts.outer.unwrap_or(facts.inner).contains(at))
            .collect();
        let area = |facts: &WindowFacts| {
            let rect = facts.outer.unwrap_or(facts.inner);
            rect.width() * rect.height()
        };
        let height = |viewport: &ViewportId| stacking.iter().position(|stacked| stacked == viewport).map_or(0, |at| at + 1);
        under.sort_by(|(a_id, a), (b_id, b)| b.focused.cmp(&a.focused).then(height(b_id).cmp(&height(a_id))).then(area(a).total_cmp(&area(b))));
        under.into_iter().map(|(viewport, _)| viewport).collect()
    }

    /// Who draws the running viewport's frame, and the caption buttons it shows. An inset frame whose window was
    /// measured with the traffic lights at the right end says so ([`MacInset::lights_right`](craft_window::policy::MacInset)).
    pub fn frame(&self) -> Frame {
        let viewport = pass::current_viewport(&self.ctx);
        let frame = match shared(&self.ctx, |shared| shared.root_frame.clone()) {
            Some(frame) if viewport == ViewportId::ROOT => frame,
            _ => frame_in(&self.ctx, viewport),
        };
        match frame {
            Frame::Inset(inset) if !inset.lights_right => {
                let right = PerViewport::<Record>::with_existing(&self.ctx, viewport, |record| record.lights.lights_right()).unwrap_or(false);
                Frame::Inset(inset.with_lights_right(right))
            }
            frame => frame,
        }
    }

    /// The room the macOS traffic lights take over the running viewport's content: `leading` wide at the end they sit
    /// at, twice their centre's distance from the top high. `None` for any other frame and in full screen, where the
    /// lights are not over the content.
    pub fn lights_room(&self) -> Option<Rect> {
        let Frame::Inset(inset) = self.frame() else { return None };
        if shell_of(&self.ctx, pass::current_viewport(&self.ctx)).window.state == WindowState::Fullscreen {
            return None;
        }
        let window = self.ctx.viewport_rect();
        let size = Vec2::new(inset.leading.min(window.width()), (2.0 * inset.centre_from_top).min(window.height()));
        let min = if inset.lights_right { Pos2::new(window.right() - size.x, window.top()) } else { window.min };
        Some(Rect::from_min_size(min, size))
    }

    /// The desktop the app's windows open on, as the installed frame decision detected it; `None` when the app
    /// installed no decision (an embedded shell, a trial). The caption buttons take the desktop's icon theme from it.
    pub fn desktop(&self) -> Option<Desktop> {
        shared(&self.ctx, |shared| shared.desktop.clone())
    }

    /// Where the exclusive parts of `viewport`'s window lay in its last complete pass: the caption buttons, the edge
    /// zones and the drag areas. `None` while the window declared none.
    pub fn caption_regions(&self, viewport: ViewportId) -> Option<CaptionRegions> {
        with_exclusive(&self.ctx, viewport, |exclusive| exclusive.done.clone())
    }

    /// The hit-test sink of `viewport`'s window, made on the first call. The platform layer calls it once it has put
    /// its window procedure in place and holds the sink there; from then on the platform answers that window's move,
    /// resize and caption hits. The window's move and resize parts still register, as the exclusive parts the sink
    /// reads, and a press on them starts no app move.
    pub fn install_hit_sink(&self, viewport: ViewportId) -> HitSink {
        PerViewport::<Record>::with(&self.ctx, viewport, |record| record.sink.get_or_insert_with(|| HitSink::linked(&self.ctx, viewport)).clone())
    }

    /// The desktop pointer in the coordinates of `viewport`, when it lies inside that window.
    pub fn pointer_in(&self, viewport: ViewportId) -> Option<Pos2> {
        let desktop = self.desktop_pointer()?;
        let inner = self.inner_on_desktop(viewport)?;
        inner.contains(desktop).then(|| Pos2::ZERO + (desktop - inner.min))
    }

    /// What a point of `from`'s window adds to land at the same desktop place in `to`'s window, by the inner rects
    /// the windows last reported, as [`Self::pointer_in`] maps the desktop pointer.
    pub(crate) fn offset_between(&self, from: ViewportId, to: ViewportId) -> Option<Vec2> {
        Some(self.inner_on_desktop(from)?.min - self.inner_on_desktop(to)?.min)
    }

    /// The inner rect of `viewport`'s window on the desktop, as the window last reported it.
    fn inner_on_desktop(&self, viewport: ViewportId) -> Option<Rect> {
        PerViewport::<Record>::with(&self.ctx, viewport, |record| record.facts.and_then(|facts| facts.inner))
    }
}

/// The frame of `viewport`'s window as its chrome and the caption buttons it was opened with tell it, for a window
/// with no installed decision.
fn frame_in(ctx: &Context, viewport: ViewportId) -> Frame {
    let captions = PerViewport::<Record>::with(ctx, viewport, |record| record.captions.clone());
    frame_of(window_info(ctx, viewport, &base_shell(ctx)).chrome, captions)
}

/// The connected monitors in physical pixels; none where the platform does not list them.
pub(super) fn monitors(ctx: &Context) -> Vec<Monitor> {
    let system = shared(ctx, |shared| shared.system.clone());
    system.map(|system| system.monitors()).unwrap_or_default()
}

/// The monitor the top-left corner of a window with `facts` lies on. A corner is on a monitor when its pixel position
/// at that monitor's scale is inside the monitor.
pub(super) fn monitor_of(ctx: &Context, facts: &viewports::Facts) -> Option<Monitor> {
    let corner = facts.outer.or(facts.inner)?.min;
    monitors(ctx).into_iter().find(|monitor| monitor.rect_px().contains(corner * monitor.scale))
}

/// The scale of a window with `facts`: the platform's report, else the scale of its monitor ([`monitor_of`]).
pub(super) fn scale_of(ctx: &Context, facts: &viewports::Facts) -> Option<f32> {
    facts.native_scale.or_else(|| monitor_of(ctx, facts).map(|monitor| monitor.scale))
}

/// Records the pointer of the viewport a drag runs in, `local` in that viewport's coordinates, as a desktop
/// position: the fallback where the platform tells no system pointer. While the app moves that viewport's window the
/// move's own pointer counts ([`note_moved_pointer`]). `None` keeps the last position.
pub(crate) fn note_source_pointer(ctx: &Context, viewport: ViewportId, local: Option<Pos2>) {
    let (inner, moved) = PerViewport::<Record>::with(ctx, viewport, |record| (record.facts.and_then(|facts| facts.inner), record.moved_pointer));
    if let Some(desktop) = moved.or_else(|| local.zip(inner).map(|(local, inner)| inner.min + local.to_vec2())) {
        shared(ctx, |shared| shared.source_pointer = Some(desktop));
    }
}

/// Records the desktop pointer of a move the app runs of `viewport`'s window: the pressed point of the window where the
/// move last put it. `None` once the move ends.
/// The system's pointer in desktop points as the pass began, `None` where the platform does not tell it. Unlike
/// [`WindowHost::desktop_pointer`] it never falls back to a drag's own pointer, so a window moved after it does not
/// feed its own place back into the move.
pub(crate) fn system_pointer(ctx: &Context) -> Option<Pos2> {
    shared(ctx, |shared| shared.system_pointer)
}

pub(crate) fn note_moved_pointer(ctx: &Context, viewport: ViewportId, desktop: Option<Pos2>) {
    PerViewport::<Record>::with(ctx, viewport, |record| record.moved_pointer = desktop);
}

/// Forgets the drag's pointer once the session ends.
pub(crate) fn clear_source_pointer(ctx: &Context) {
    shared(ctx, |shared| shared.source_pointer = None);
}
