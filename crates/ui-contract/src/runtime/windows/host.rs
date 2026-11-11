//! WindowHost, the multi-window driver: window facts, monitors and the pointer of a drag across windows.

use super::hit::{CaptionRegions, HitSink, with_exclusive};
use super::native::{WindowAction, act};
use super::placement::placement_of;
use super::shell::{base_shell, shell_of, window_info};
use super::spec::frame_of;
use super::{Record, shared};
use crate::runtime::egui::input_filter;
use crate::runtime::egui::screenshot::{ScreenshotFacts, ScreenshotSlot};
use crate::runtime::egui::{injected, pass, screenshot, viewports};
use crate::runtime::frame::PerViewport;
use craft_behavior::placement::Placement;
use craft_behavior::{Monitor, PointerButton};
use craft_host::target::{Caps, ShellInfo, WindowState};
use craft_keymap::{Chord, Key, Mods};
use craft_window::policy::Frame;
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

    /// What a screenshot of the running viewport measures this pass.
    pub fn screenshot_facts(&self) -> ScreenshotFacts {
        screenshot::facts(&self.ctx)
    }

    /// Queues a pointer move to `at` in `viewport`, delivered on a pass of its own.
    pub fn inject_move(&self, viewport: ViewportId, at: Pos2) {
        injected::inject_move(&self.ctx, viewport, at);
    }

    /// Queues a move to `at`, then `count` presses and releases of `button`, one pass each; a button egui has no
    /// counterpart for queues nothing.
    pub fn inject_click(&self, viewport: ViewportId, at: Pos2, button: PointerButton, count: u32) {
        injected::inject_click(&self.ctx, viewport, at, button, count);
    }

    /// Queues `key` pressed and released with `mods` held, as the platform reports them.
    pub fn inject_key(&self, viewport: ViewportId, key: Key, mods: Mods) {
        injected::inject_key(&self.ctx, viewport, key, mods);
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

    /// Drops the injected queue of `viewport` (the document changed); a pending queue counts as withdrawn
    /// ([`take_injected_withdrawn`](Self::take_injected_withdrawn)).
    pub fn withdraw_injected(&self, viewport: ViewportId) {
        injected::withdraw(&self.ctx, viewport);
    }

    /// Drops every injected queue and step in flight; true when anything was pending.
    pub fn cancel_injected(&self) -> bool {
        injected::cancel(&self.ctx)
    }

    /// Whether no injected step is queued or awaits its pass: the application may answer its waiters.
    pub fn injected_idle(&self) -> bool {
        injected::idle(&self.ctx)
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

    /// `previous` (or a new record) updated to where `viewport`'s window is now, recorded as an app window's saved
    /// placement is. `None` before the window has reported its rect and scale.
    #[doc(hidden)]
    pub fn placement(&self, viewport: ViewportId, previous: Option<Placement>) -> Option<Placement> {
        let facts = PerViewport::<Record>::with(&self.ctx, viewport, |record| record.facts)?;
        placement_of(&self.ctx, &facts, previous)
    }

    /// The connected monitors, in desktop pixels; none where the platform does not list them.
    /// [`Monitor::points_at`] gives one in the points of a window ([`WindowFacts::scale`]).
    pub fn monitors(&self) -> Vec<Monitor> {
        monitors(&self.ctx)
    }

    /// Places `viewport`'s outer rect at `outer`: its position, and the inner size that leaves the window's
    /// decoration as it is. False when a command could not go out.
    pub fn set_outer(&self, viewport: ViewportId, outer: Rect) -> bool {
        let decoration = self.facts(viewport).and_then(|facts| facts.outer.map(|current| current.size() - facts.inner.size())).unwrap_or(Vec2::ZERO);
        let moved = act(&self.ctx, viewport, WindowAction::SetOuterPosition(outer.min));
        let sized = act(&self.ctx, viewport, WindowAction::SetInnerSize((outer.size() - decoration).max(Vec2::ZERO)));
        moved && sized
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

    /// Who draws the running viewport's frame, and the caption buttons it shows.
    pub fn frame(&self) -> Frame {
        let viewport = pass::current_viewport(&self.ctx);
        if viewport == ViewportId::ROOT
            && let Some(frame) = shared(&self.ctx, |shared| shared.root_frame.clone())
        {
            return frame;
        }
        let captions = PerViewport::<Record>::with(&self.ctx, viewport, |record| record.captions.clone());
        frame_of(window_info(&self.ctx, viewport, &base_shell(&self.ctx)).chrome, captions)
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
        let inner = PerViewport::<Record>::with(&self.ctx, viewport, |record| record.facts.and_then(|facts| facts.inner))?;
        inner.contains(desktop).then(|| Pos2::ZERO + (desktop - inner.min))
    }
}

/// The connected monitors in physical pixels; none where the platform does not list them.
pub(super) fn monitors(ctx: &Context) -> Vec<Monitor> {
    let system = shared(ctx, |shared| shared.system.clone());
    system.map(|system| system.monitors()).unwrap_or_default()
}

/// The scale of a window with `facts`: the platform's report, else the scale of the monitor the window's top-left
/// corner lies on. A corner is on a monitor when its pixel position at that monitor's scale is inside the monitor.
pub(super) fn scale_of(ctx: &Context, facts: &viewports::Facts) -> Option<f32> {
    facts.native_scale.or_else(|| {
        let corner = facts.outer.or(facts.inner)?.min;
        monitors(ctx).iter().find(|monitor| monitor.rect_px().contains(corner * monitor.scale)).map(|monitor| monitor.scale)
    })
}

/// Records the pointer of the viewport a drag runs in, `local` in that viewport's coordinates, as a desktop
/// position: the fallback where the platform tells no system pointer. `None` keeps the last position.
pub(crate) fn note_source_pointer(ctx: &Context, viewport: ViewportId, local: Option<Pos2>) {
    let inner = PerViewport::<Record>::with(ctx, viewport, |record| record.facts.and_then(|facts| facts.inner));
    if let Some(desktop) = local.zip(inner).map(|(local, inner)| inner.min + local.to_vec2()) {
        shared(ctx, |shared| shared.source_pointer = Some(desktop));
    }
}

/// Forgets the drag's pointer once the session ends.
pub(crate) fn clear_source_pointer(ctx: &Context) {
    shared(ctx, |shared| shared.source_pointer = None);
}
