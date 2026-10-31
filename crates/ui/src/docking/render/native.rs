//! Floating windows in their own OS windows (feature `native-windows`). Where the backend has
//! viewports, every floating window whose panels allow it is drawn with
//! `show_viewport_immediate`. Palettes stay on top of the application's other windows, and
//! drop to the normal level while none of its windows has focus, so they stay shown without
//! covering other applications; viewers are normal windows that come forward when the
//! application is activated. Where egui embeds viewports (wasm, tests) windows stay
//! in-window at their in-window rect, and so do they on Wayland, unless the app turns native
//! windows on ([`crate::docking::DockArea::native_windows`]). Tabs, groups and whole windows
//! drag between OS windows (see `cross.rs`), and so do the app's drags in panel bodies
//! (`body_drag.rs`).

use std::{fmt::Debug, hash::Hash, sync::Arc};

use egui::{
    Color32, Context, CursorIcon, Event, Id, Key, LayerId, Modifiers, Order, Pos2, RawInput, Rect, ResizeDirection, Sense, Ui, UiBuilder, Vec2,
    ViewportBuilder, ViewportClass, ViewportCommand, ViewportId, ViewportInfo, WindowLevel, pos2, vec2,
};

use super::Renderer;
use super::body_drag::{BodyRoute, Held, Queued};
use super::cross::{DragPointer, Surface};
use crate::docking::chrome::{ChromeStyle, TitleBar};
pub(in crate::docking) use crate::docking::placement::BAR_HEIGHT;
use crate::docking::placement::{kept_reachable, monitor_of, restore, sanitized};
use crate::docking::{Action, Drag, Floating, GroupDrag, GroupGesture, Node, WindowFiles, WindowKind};

/// Smallest native window.
const MIN_SIZE: [f32; 2] = [160.0, 100.0];
/// How long none of the application's windows has focus before the application counts as
/// inactive, so focus moving between our own windows does not.
const AWAY_DELAY: f64 = 0.25;
/// Whether normal windows are raised when the application is activated. macOS brings an
/// application's windows forward itself: all of them on activation from the Dock or the
/// application switcher, the clicked one alone on a click, as its own windows do.
const RAISE_ON_ACTIVATION: bool = !cfg!(target_os = "macos");
/// The level of a palette while the application is inactive, behind other applications'
/// windows. On Windows `WindowLevel::Normal` is `HWND_NOTOPMOST`, which puts the window in
/// front of every window that is not topmost, the activated application's too; `HWND_BOTTOM`
/// sends it behind them without activating it.
pub(in crate::docking) const INACTIVE_PALETTE: WindowLevel =
    if cfg!(target_os = "windows") { WindowLevel::AlwaysOnBottom } else { WindowLevel::Normal };
/// How long reports of a native window's place may lag a move the dock area commanded.
const MOVE_WAIT: f64 = 0.5;
/// How far the resize bands reach into a native window from its edges, where the window has
/// no frame of its own to resize from.
const EDGE: f32 = 6.0;
/// How far a corner's grab runs along each of its two edges.
const CORNER: f32 = 16.0;
/// macOS keeps the window's frame, and AppKit resizes the window from its edges and corners.
/// Elsewhere the window is undecorated and resizes from our bands, which hand the resize to
/// the window system (`BeginResize`).
const OS_RESIZES: bool = cfg!(target_os = "macos");
/// How far AppKit's corner resize area reaches into a titled, resizable window along each
/// edge. AppKit takes a press there before the content sees it, and shows its resize cursor.
/// Measured on macOS 26 as a 19 pt square at each corner, 62 % of which would lie inside a
/// square-cornered window: about 15 pt inside and 4 pt outside
/// (<https://noheger.at/blog/2026/01/11/the-struggle-of-resizing-windows-on-macos-tahoe/>).
/// There is no published measurement for earlier releases.
pub(in crate::docking) const APPKIT_CORNER: f32 = 15.0;
/// How far AppKit's edge resize area reaches into the content between the corners: a few
/// points; most of it lies outside the window.
const APPKIT_EDGE: f32 = 4.0;

/// Where the × of a native window's bar sits and where it takes clicks, in points from the
/// window's top and trailing edges, mirrored in a right-to-left layout.
#[derive(Clone, Copy, Debug)]
pub(in crate::docking) struct CloseLayout {
    /// From the window's top edge to the top of the glyph.
    pub glyph_top: f32,
    /// From the window's trailing edge to the glyph's trailing side.
    pub glyph_trailing: f32,
    /// Width and height of the glyph.
    pub glyph_size: f32,
    /// How far the hit area reaches past the glyph to the left (to the right when mirrored).
    pub hit_left: f32,
    /// How far the hit area reaches past the glyph to the right (to the left when mirrored).
    pub hit_right: f32,
    /// How far the hit area reaches above the glyph.
    pub hit_up: f32,
    /// How far the hit area reaches below the glyph. Past the bar's bottom it takes clicks
    /// from the tab strip under the bar.
    pub hit_down: f32,
}

/// The × on macOS, where macOS puts the close button of its own windows: centered in the
/// 18 pt bar, its trailing side 8 pt in. AppKit's corner resize area ([`APPKIT_CORNER`])
/// covers its trailing half and takes presses there, so the hit area reaches away from the
/// corner: left of the glyph, and down to the bar's bottom.
pub(in crate::docking) const MACOS_CLOSE: CloseLayout =
    CloseLayout { glyph_top: 4.5, glyph_trailing: 8.0, glyph_size: 9.0, hit_left: 10.0, hit_right: 0.0, hit_up: 0.0, hit_down: 4.5 };

/// The × elsewhere: an 18 pt square the height of the bar, its trailing side 7 pt in so the
/// side's resize band ([`EDGE`]) and the corner's grab run down beside it, with the glyph at
/// its center.
pub(in crate::docking) const OTHER_CLOSE: CloseLayout =
    CloseLayout { glyph_top: 4.5, glyph_trailing: 11.5, glyph_size: 9.0, hit_left: 4.5, hit_right: 4.5, hit_up: 4.5, hit_down: 4.5 };

/// Opacity of the native window that a bar drag of the area `area` moves, by the rule for every
/// floating window ([`super::DRAG_OPACITY`]): faded while the drag highlights a drop target, as
/// the last frame drew it ([`super::cross::HIGHLIGHTED`]), opaque over empty space, for a move
/// that cannot dock and while Ctrl/Cmd keeps the drop floating. The window stays above the main
/// window, so the highlighted target shows only through it.
#[cfg(any(target_os = "macos", test))]
pub(in crate::docking) fn drag_opacity(ctx: &Context, area: Id) -> f64 {
    let dragging = ctx.data(|d| d.get_temp::<WindowDrag>(area.with("window-drag"))).is_some();
    let highlighted = ctx.data(|d| d.get_temp::<bool>(area.with(super::cross::HIGHLIGHTED))).unwrap_or(false);
    if dragging && highlighted { f64::from(super::DRAG_OPACITY) } else { 1.0 }
}

/// Sets the opacity of the AppKit windows titled `title`, never one titled `main` (the main
/// window's title). egui has no viewport command for a window's opacity, and a transparent
/// surface would need the main window transparent too (eframe picks one alpha mode for every
/// viewport), so this goes to AppKit, which fades the content, corners and shadow together.
/// The window is found by title: its native handle is a raw `NSView` pointer, and turning that
/// into an `NSWindow` needs `unsafe`, which the workspace forbids. Two floats with one title
/// fade together.
#[cfg(target_os = "macos")]
fn set_window_opacity(title: &str, main: Option<&str>, opacity: f64) {
    if title.is_empty() || main == Some(title) {
        return;
    }
    let Some(mtm) = objc2::MainThreadMarker::new() else { return };
    let title = objc2_foundation::NSString::from_str(title);
    for window in objc2_app_kit::NSApplication::sharedApplication(mtm).windows().iter() {
        if window.title().isEqualToString(&title) && window.alphaValue() != opacity {
            window.setAlphaValue(opacity);
        }
    }
}

/// What the window system tells of the desktop pointer, and whether it moves a native window
/// for a bar drag itself.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::docking) enum WindowSystem {
    /// The running one: AppKit on macOS tells the cursor, the button and the modifiers, and
    /// moves a window for `StartDrag`; elsewhere none of these is told.
    #[default]
    Native,
    /// A stand-in (`docking::testing::NativeHost`).
    #[cfg(any(test, feature = "testing"))]
    Host {
        /// The cursor on the desktop, where told.
        cursor: Option<Pos2>,
        /// Whether the primary button is down, where told.
        held: Option<bool>,
        /// It moves a window for `StartDrag`, as AppKit does.
        moves: bool,
    },
}

impl WindowSystem {
    /// Where the cursor is on the desktop, in egui's desktop points.
    fn cursor(self, ctx: &Context) -> Option<Pos2> {
        match self {
            Self::Native => system_cursor(ctx),
            #[cfg(any(test, feature = "testing"))]
            Self::Host { cursor, .. } => cursor,
        }
        .filter(|p| p.is_finite())
    }

    /// Whether the primary button is down now. egui can report a release a frame after it.
    fn primary_held(self) -> Option<bool> {
        match self {
            Self::Native => system_primary_held(),
            #[cfg(any(test, feature = "testing"))]
            Self::Host { held, .. } => held,
        }
    }

    /// Whether Ctrl or Cmd is down now. AppKit sends no key events while it moves a window, so
    /// egui's modifiers can be stale then.
    fn command_held(self) -> Option<bool> {
        match self {
            Self::Native => system_command_held(),
            #[cfg(any(test, feature = "testing"))]
            Self::Host { .. } => None,
        }
    }

    /// Whether it moves a native window for a bar drag (`StartDrag`), and tells the cursor
    /// and the button meanwhile: AppKit does, smoothly, while a window moved from the app by
    /// `OuterPosition` jitters there.
    fn moves_windows(self) -> bool {
        match self {
            Self::Native => cfg!(target_os = "macos"),
            #[cfg(any(test, feature = "testing"))]
            Self::Host { moves, .. } => moves,
        }
    }
}

/// AppKit's cursor, in the desktop points egui reports window positions in. AppKit measures
/// from the bottom-left of the primary screen (the one with the menu bar, first in
/// `NSScreen::screens`), y up; winit flips window positions against that screen's height,
/// whichever screen a window is on, so the cursor is flipped the same way. egui's points are
/// AppKit's divided by the zoom factor.
#[cfg(target_os = "macos")]
fn system_cursor(ctx: &Context) -> Option<Pos2> {
    let mtm = objc2::MainThreadMarker::new()?;
    let primary = objc2_app_kit::NSScreen::screens(mtm).firstObject()?;
    let at = objc2_app_kit::NSEvent::mouseLocation();
    let height = primary.frame().size.height;
    Some(pos2(at.x as f32, (height - at.y) as f32) / ctx.zoom_factor())
}

#[cfg(not(target_os = "macos"))]
fn system_cursor(_: &Context) -> Option<Pos2> {
    None
}

/// AppKit's primary button state, current when egui's lags.
#[cfg(target_os = "macos")]
fn system_primary_held() -> Option<bool> {
    Some(objc2_app_kit::NSEvent::pressedMouseButtons() & 1 != 0)
}

#[cfg(not(target_os = "macos"))]
fn system_primary_held() -> Option<bool> {
    None
}

/// AppKit's Ctrl and Cmd state, current while it sends egui no key events.
#[cfg(target_os = "macos")]
fn system_command_held() -> Option<bool> {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags};
    Some(NSEvent::modifierFlags_class().intersects(NSEventModifierFlags::Command | NSEventModifierFlags::Control))
}

#[cfg(not(target_os = "macos"))]
fn system_command_held() -> Option<bool> {
    None
}

/// Whether native windows outline their ×'s hit area, and on macOS AppKit's resize areas:
/// `CRAFT_DOCK_DEBUG_CLOSE=1`, read once.
fn debug_close() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("CRAFT_DOCK_DEBUG_CLOSE").is_ok_and(|v| v == "1"))
}

/// A drag of a native window's bar, in the context's data under the area's `"window-drag"`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::docking) struct WindowDrag {
    /// [`Floating::id`] of the dragged window.
    pub window: u64,
    /// Where the pointer pressed, in the window's local points.
    pub grab: Vec2,
    /// Desktop position of the window's content as last commanded, or under `os`, the
    /// cursor less `grab`.
    pub to: Pos2,
    /// Desktop position of the window's content when the drag began, for Escape.
    pub start: Pos2,
    /// The drag began on the window's only tab or group, so it docks whatever
    /// [`crate::docking::DockStyle::title_bar_docks`] says.
    pub docks: bool,
    /// The window system moves the window ([`WindowSystem::moves_windows`]).
    pub os: bool,
    /// The cursor has gone past the click distance from the press. Under `os` the drag starts
    /// on the press, and until then it is a click, which docks nothing.
    pub moved: bool,
}

/// What the dock area knows of a shown native window's place, in the context's data under
/// the area's `("native-place", viewport)`. Rects are desktop rects of the window's content.
#[derive(Clone, Copy, Debug)]
struct Place {
    /// Where it opened. The builder keeps it, so egui sends no move of its own.
    spawn: [f32; 4],
    /// As last reported or commanded. [`Floating::native`] follows it; a `native` that differs
    /// was changed by the application (undo, reset, a reloaded layout), and the window moves.
    known: [f32; 4],
    /// Until when reports that differ from `known` are a commanded move that has not landed.
    pending: Option<f64>,
    /// The window system put the frame, not the content, where it opened: corrected once.
    corrected: bool,
    /// The height of its bar while minimized, as it was last told. `known` keeps the expanded
    /// size meanwhile.
    collapsed: Option<f32>,
}

/// Whether two rects differ by more than rounding.
fn differs(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).any(|(a, b)| (a - b).abs() > 0.5)
}

/// Desktop position of a viewport's content, and its offset from the window's outer
/// position, where the window system reports both.
pub(super) fn content_of(info: &ViewportInfo) -> Option<(Pos2, Vec2)> {
    let (outer, inner) = (info.outer_rect?, info.inner_rect?);
    (outer.min.is_finite() && inner.min.is_finite()).then(|| (inner.min, inner.min - outer.min))
}

/// The viewport of the native window `window` of the dock area `area`. The `n`th window with
/// an id an earlier window of the same frame took gets its own.
pub(in crate::docking) fn viewport_id(area: Id, window: u64, n: usize) -> ViewportId {
    match n {
        0 => ViewportId::from_hash_of((area, "native", window)),
        n => ViewportId::from_hash_of((area, "native", window, n)),
    }
}

/// The desktop rect of a native viewport's content. None when the backend reports no
/// positions.
fn placed(info: &ViewportInfo) -> Option<[f32; 4]> {
    let (min, _) = content_of(info)?;
    let size = info.inner_rect?.size();
    size.is_finite().then(|| sanitized([min.x, min.y, size.x, size.y]))
}

/// What a native window's frame reports back to the main window.
struct Report<P> {
    closed: bool,
    placed: Option<[f32; 4]>,
    /// From the window's outer position to its content.
    frame: Vec2,
    monitor_size: Option<Vec2>,
    /// Its drop targets, for drags from other windows.
    surface: Option<Surface<P>>,
}

impl<P> Default for Report<P> {
    fn default() -> Self {
        Self { closed: false, placed: None, frame: Vec2::ZERO, monitor_size: None, surface: None }
    }
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Renderer<'_, P> {
    /// Show the native windows and return the windows to draw in-window.
    pub(super) fn native_windows<'w>(&mut self, ctx: &Context, windows: Vec<&'w Floating<P>>) -> Vec<&'w Floating<P>> {
        let native = !ctx.embed_viewports() && self.native_windows.unwrap_or_else(|| !wayland(ctx, env_session()));
        let origin = ctx.input(|i| i.viewport().inner_rect.map(|r| r.min)).filter(|p| p.is_finite()).unwrap_or(Pos2::ZERO);
        let (active, activated) = if native { self.app_activity(ctx) } else { (true, false) };
        self.cross.on = native;
        let mut shown = Vec::new();
        let mut in_window = Vec::new();
        let mut ids = Vec::new();
        let mut natives = Vec::new();
        for window in windows {
            // The application's own policy: a locked area keeps its native windows.
            let allowed = window.panels().all(|p| self.content.allow_native(p) && self.content.permissions(p).floatable);
            if !native || !allowed || window.panels().next().is_none() {
                in_window.push(window);
                continue;
            }
            // Every float is native: one still in-window opens over its place in the app window.
            let rect = window.native.unwrap_or_else(|| {
                let [x, y, w, h] = window.rect;
                sanitized([origin.x + x, origin.y + y, w, h])
            });
            let n = ids.iter().filter(|id| **id == window.id).count();
            ids.push(window.id);
            natives.push((window, rect, viewport_id(self.id, window.id, n)));
        }
        if native {
            #[cfg(debug_assertions)]
            self.warn_rebuilt(ctx, &natives);
            self.escape_elsewhere(ctx, None);
            // A window closed or docked during its own bar drag ends the drag.
            let key = self.id.with("window-drag");
            if ctx.data(|d| d.get_temp::<WindowDrag>(key)).is_some_and(|d| !natives.iter().any(|(w, ..)| w.id == d.window)) {
                ctx.data_mut(|d| d.remove::<WindowDrag>(key));
            }
            self.map_desktop(ctx, &natives, activated);
            self.raise_next(ctx, &natives);
            // The main window owns drags of the panels it draws.
            let id = self.id;
            let dragged = ctx.data(|d| {
                d.get_temp::<Drag<P>>(id.with("drag")).map(|d| d.panel).or(d.get_temp::<GroupDrag<P>>(id.with("group-drag")).map(|d| d.panel))
            });
            if dragged.is_some_and(|panel| !natives.iter().any(|(w, ..)| w.contains(&panel))) {
                let origin = self.cross.main.map(|m| m.min);
                self.record_pointer(ctx, None, origin, None);
            }
            self.cross.viewports = natives.iter().map(|(w, _, viewport)| (w.id, *viewport)).collect();
        }
        self.map_body_route(ctx, native);
        for (window, rect, viewport) in natives {
            if let Some(key) = self.native_window(ctx, window, rect, viewport, active) {
                shown.push(key);
            }
        }
        // Forget where windows that are no longer native were told to open.
        let key = self.id.with("native-shown");
        let previous = ctx.data_mut(|d| d.remove_temp::<Vec<Id>>(key)).unwrap_or_default();
        ctx.data_mut(|d| {
            for id in previous.iter().filter(|id| !shown.contains(id)) {
                d.remove::<Place>(*id);
            }
            d.insert_temp(key, shown);
        });
        in_window
    }

    /// Debug builds: warn once when for [`REBUILT_FRAMES`] frames running, every native window
    /// is new, with the panels of the last frame's. The app builds its layout anew each frame
    /// without [`Floating::with_id`], so each frame opens new OS windows. A layout loaded
    /// again does it once.
    #[cfg(debug_assertions)]
    fn warn_rebuilt(&self, ctx: &Context, natives: &[(&Floating<P>, [f32; 4], ViewportId)]) {
        let (key, warned) = (self.id.with("native-previous"), self.id.with("native-rebuilt"));
        let current: Vec<(u64, Vec<P>)> = natives.iter().map(|(w, ..)| (w.id, w.panels().cloned().collect())).collect();
        let (runs, previous) = ctx.data_mut(|d| d.remove_temp::<(usize, Vec<(u64, Vec<P>)>)>(key)).unwrap_or_default();
        let runs = if rebuilt(&previous, &current) { runs + 1 } else { 0 };
        if runs >= REBUILT_FRAMES && !ctx.data(|d| d.get_temp::<bool>(warned).unwrap_or(false)) {
            log::warn!(
                "craft-ui docking: the native floating windows get new ids every frame, so each frame opens new OS windows. \
                 Keep the layout across frames, or give its windows ids with Floating::with_id."
            );
            ctx.data_mut(|d| d.insert_temp(warned, true));
        }
        ctx.data_mut(|d| d.insert_temp(key, (runs, current)));
    }

    /// The main window and the shown native windows on the desktop, front first: palettes
    /// above the rest, each the window focused most recently first. A window whose position is
    /// unknown is left out. `activated`: the application was just activated, so the main window
    /// and its normal windows are raised (see [`Self::raise_next`]).
    fn map_desktop(&mut self, ctx: &Context, natives: &[(&Floating<P>, [f32; 4], ViewportId)], activated: bool) {
        let key = self.id.with("native-order");
        // Focus order, front first; `None` is the main window.
        let mut order = ctx.data(|d| d.get_temp::<Vec<Option<u64>>>(key)).unwrap_or_default();
        order.retain(|id| id.is_none_or(|id| natives.iter().any(|(w, ..)| w.id == id)));
        // The main window comes to the front when it gains focus, and on a press in it: the
        // window system raises it then, though it may have kept focus.
        let main_key = self.id.with("native-main-focused");
        let (main_focused, pressed) = ctx.input(|i| (i.viewport().focused == Some(true), i.pointer.any_pressed()));
        let gained = main_focused && !ctx.data(|d| d.get_temp::<bool>(main_key)).unwrap_or(false);
        ctx.data_mut(|d| d.insert_temp(main_key, main_focused));
        if gained || pressed || !order.contains(&None) {
            let front = order.contains(&None);
            order.retain(Option::is_some);
            if front { order.insert(0, None) } else { order.push(None) }
        }
        let mut windows = Vec::new();
        for (window, rect, viewport) in natives {
            let info = ctx.input(|i| i.raw.viewports.get(viewport).cloned());
            // A new window opens in front.
            if info.as_ref().is_some_and(|i| i.focused == Some(true)) || !order.contains(&Some(window.id)) {
                order.retain(|id| *id != Some(window.id));
                order.insert(0, Some(window.id));
            }
            let [x, y, w, h] = *rect;
            let placed = match info {
                None => Some(Rect::from_min_size(pos2(x, y), vec2(w, h))),
                Some(info) => placed(&info).map(|[x, y, w, h]| Rect::from_min_size(pos2(x, y), vec2(w, h))),
            };
            let normal = self.normal_window(window);
            if let Some(placed) = placed.filter(|r| r.is_finite()) {
                windows.push((window.id, placed, normal));
            }
        }
        if activated && RAISE_ON_ACTIVATION {
            // The back one first, so each lands in front of those raised before it, and the
            // window just focused, at the front of the order, last.
            // While input is blocked the main window, with the modal, ends in front.
            if self.input_blocked {
                order.retain(Option::is_some);
                order.insert(0, None);
            }
            let queue: Vec<Option<u64>> = order
                .iter()
                .rev()
                .filter(|id| id.is_none_or(|id| natives.iter().any(|(w, ..)| w.id == id && self.normal_window(w))))
                .copied()
                .collect();
            // Blocked, the main window comes in front of a palette the app was activated through.
            if self.input_blocked || queue.iter().any(Option::is_some) {
                ctx.data_mut(|d| d.insert_temp(self.id.with("native-raise"), queue));
            }
        }
        let rank = |id: Option<u64>| order.iter().position(|o| *o == id);
        windows.sort_by_key(|(id, _, normal)| (*normal, rank(Some(*id))));
        self.cross.front = windows.iter().filter(|(id, _, normal)| !normal || rank(Some(*id)) < rank(None)).count();
        self.cross.windows = windows.into_iter().map(|(id, rect, _)| (id, rect)).collect();
        ctx.data_mut(|d| d.insert_temp(key, order));
        self.cross.main = ctx.input(|i| i.viewport().outer_rect.and(i.viewport().inner_rect)).filter(|r| r.is_finite());
    }

    /// Raise the next window [`Self::map_desktop`] queued when the application was activated,
    /// the main window (`None`) or a normal window: one per pass, since egui sends the commands
    /// of different viewports in no fixed order. Toggling the window level raises a window without focusing it: on Windows
    /// `HWND_NOTOPMOST` puts the window in front of every window that is not topmost, and on
    /// X11 the window manager raises a window as it takes `_NET_WM_STATE_ABOVE`. winit has no
    /// plain raise. `ViewportCommand::Focus` would take focus from the window the user
    /// activated, and egui-winit ignores it for a window that has focus, so focus could not be
    /// handed back. `Visible(true)` does nothing to a shown window.
    fn raise_next(&self, ctx: &Context, natives: &[(&Floating<P>, [f32; 4], ViewportId)]) {
        let key = self.id.with("native-raise");
        let Some(mut queue) = ctx.data_mut(|d| d.remove_temp::<Vec<Option<u64>>>(key)) else { return };
        // A window that became a palette since activation keeps its `AlwaysOnTop` level.
        let viewport = |id: Option<u64>| match id {
            None => Some(ViewportId::ROOT),
            Some(id) => natives.iter().find(|(w, ..)| w.id == id && self.normal_window(w)).map(|(.., v)| *v),
        };
        queue.retain(|id| viewport(*id).is_some());
        if queue.is_empty() {
            return;
        }
        if let Some(viewport) = viewport(queue.remove(0)) {
            ctx.send_viewport_cmd_to(viewport, ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop));
            ctx.send_viewport_cmd_to(viewport, ViewportCommand::WindowLevel(WindowLevel::Normal));
        }
        if !queue.is_empty() {
            ctx.data_mut(|d| d.insert_temp(key, queue));
            ctx.request_repaint();
        }
    }

    /// Whether the application is active this frame: one of its windows has focus, or one had it
    /// less than [`AWAY_DELAY`] ago. And whether it was just activated: one of its windows has
    /// focus after none had it for that long. Losing focus again stops the raise of normal
    /// windows that activation started.
    fn app_activity(&mut self, ctx: &Context) -> (bool, bool) {
        let (focused, now) = ctx.input(|i| {
            let infos = || i.raw.viewports.values();
            // A backend that reports no focus at all keeps the windows.
            let focused = i.focused || infos().any(|v| v.focused == Some(true)) || infos().all(|v| v.focused.is_none());
            (focused, i.time)
        });
        self.cross.focus_lost = crate::focus::turned_false(ctx, self.id.with("app-focus"), focused);
        let key = self.id.with("native-last-focused");
        let last = if focused { now } else { ctx.data(|d| d.get_temp::<f64>(key)).unwrap_or(now) };
        let active = focused || now - last < AWAY_DELAY;
        if active && !focused {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((AWAY_DELAY - (now - last)).max(0.0)));
        }
        ctx.data_mut(|d| d.insert_temp(key, last));
        let away_key = self.id.with("native-away");
        let away = ctx.data(|d| d.get_temp::<bool>(away_key)).unwrap_or(false);
        ctx.data_mut(|d| {
            d.insert_temp(away_key, !focused && (away || !active));
            if !focused {
                d.remove::<Vec<Option<u64>>>(self.id.with("native-raise"));
            }
        });
        (active, focused && away)
    }

    /// A window holding a viewer is a normal window, so it stacks with the main window; the rest
    /// are palettes.
    fn normal_window(&self, window: &Floating<P>) -> bool {
        window.panels().any(|p| self.content.window_kind(p) == WindowKind::Normal)
    }

    /// One floating window in its own viewport. `active`: the application is active (see
    /// [`Self::app_activity`]). Returns the key of its [`Place`].
    fn native_window(&mut self, ctx: &Context, window: &Floating<P>, rect: [f32; 4], viewport: ViewportId, active: bool) -> Option<Id> {
        let normal = self.normal_window(window);
        let first = window.panels().next()?.clone();
        let key = self.id.with(("native-place", viewport));
        let now = ctx.input(|i| i.time);
        let drag_key = self.id.with("window-drag");
        let moving = |ctx: &Context| ctx.data(|d| d.get_temp::<WindowDrag>(drag_key)).is_some_and(|d| d.window == window.id);
        let info = ctx.input(|i| i.raw.viewports.get(&viewport).cloned());
        let frame = info.as_ref().and_then(content_of).map_or(Vec2::ZERO, |(_, frame)| frame);
        // A minimized window shows its bar and what `floating_body` draws below it.
        let collapsed = window.minimized.then(|| BAR_HEIGHT + self.minimized_body(window));
        let shown = |[x, y, w, h]: [f32; 4]| [x, y, w, collapsed.unwrap_or(h)];
        let mut place = match ctx.data(|d| d.get_temp::<Place>(key)) {
            Some(mut place) => {
                // The application changed the window's place (undo, reset, a reloaded layout):
                // the OS window moves there, kept where its bar can be grabbed.
                if !moving(ctx)
                    && let Some(native) = window.native.filter(|native| differs(*native, place.known))
                {
                    let target = kept_reachable(native, &self.monitors);
                    place_window(ctx, viewport, shown(target), frame);
                    (place.known, place.pending) = (target, Some(now + MOVE_WAIT));
                }
                place
            }
            None => {
                // A window of a loaded layout opens at its saved place, moved onto a connected
                // monitor. One made this session opens where it was put.
                let seen_key = self.id.with("native-seen");
                let mut seen = ctx.data(|d| d.get_temp::<std::collections::HashSet<u64>>(seen_key)).unwrap_or_default();
                let spawn = if window.native.is_some() && !seen.contains(&window.id) {
                    let (main, size) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().monitor_size));
                    restore(rect, window.monitor.as_ref(), &self.monitors, main, size)
                } else {
                    sanitized(rect)
                };
                seen.insert(window.id);
                ctx.data_mut(|d| d.insert_temp(seen_key, seen));
                Place { spawn, known: spawn, pending: Some(now + MOVE_WAIT), corrected: false, collapsed: None }
            }
        };
        // Minimized or expanded: the window keeps its top-left corner and width, and its
        // height is fixed while it shows only its bar.
        // The limits go first: the OS clamps the new size to the limits it has.
        if place.collapsed != collapsed {
            let (min, max) = match collapsed {
                Some(height) => (vec2(MIN_SIZE[0], height), vec2(1_000_000.0, height)),
                None => (Vec2::from(MIN_SIZE), Vec2::INFINITY),
            };
            ctx.send_viewport_cmd_to(viewport, ViewportCommand::MinInnerSize(min));
            ctx.send_viewport_cmd_to(viewport, ViewportCommand::MaxInnerSize(max));
            place_window(ctx, viewport, shown(place.known), frame);
            (place.collapsed, place.pending) = (collapsed, Some(now + MOVE_WAIT));
        }
        let spawn = place.spawn;
        let title = crate::docking::dock_model::first_active(&window.root).map(|p| self.label_of(p)).unwrap_or_default();
        let builder = ViewportBuilder::default()
            .with_title(title.clone())
            .with_position([spawn[0], spawn[1]])
            .with_inner_size([spawn[2], spawn[3]])
            .with_min_inner_size(MIN_SIZE)
            // A blocked palette could cover the modal, and its bar moves nothing: the main
            // window can come in front of it at the normal level. While the application is
            // inactive, its palettes stay shown behind other applications' windows
            // (`INACTIVE_PALETTE`). egui sends the level only when it changes.
            .with_window_level(match (normal || self.input_blocked, active) {
                (true, _) => WindowLevel::Normal,
                (false, true) => WindowLevel::AlwaysOnTop,
                (false, false) => INACTIVE_PALETTE,
            })
            .with_taskbar(false);
        // The bar replaces the OS title bar. macOS keeps the frame, with its title bar and
        // buttons hidden: AppKit resizes a movable window from its edges and corners with its
        // own cursors, and draws its rounded corners and standard shadow.
        #[cfg(target_os = "macos")]
        let builder = builder
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_fullsize_content_view(true)
            .with_titlebar_buttons_shown(false)
            .with_resizable(!self.input_blocked && window.panels().all(|p| self.permissions(p).movable));
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_decorations(false);
        // While the application is inactive, raising a palette leaves focus with the application
        // in use.
        if (active || normal) && !self.input_blocked && self.activation.as_ref().is_some_and(|panel| window.contains(panel)) {
            ctx.send_viewport_cmd_to(viewport, ViewportCommand::Focus);
        }
        let (targets, docks, windows) = (self.targets.len(), self.dock_targets.len(), self.frame.windows.len());
        let blocked = self.input_blocked;
        ctx.with_plugin(|input: &mut NativeInput| input.windows.insert(viewport, self.id));
        let report = ctx.show_viewport_immediate(viewport, builder, |ui, class| {
            // `cancel_drag` reaches this viewport's drag here.
            let cancelled = ui.ctx().data(|d| d.get_temp::<Vec<Id>>(crate::docking::surface::native_cancel_key(self.id)));
            if ui.ctx().dragged_id().is_some_and(|id| cancelled.is_some_and(|ids| ids.contains(&id))) {
                ui.ctx().stop_dragging();
            }
            let full = ui.max_rect();
            // An embedded fallback reads the main window's input: its close request, place,
            // keys and resizes are not this window's.
            let own = matches!(class, ViewportClass::Immediate | ViewportClass::Deferred);
            let released = if blocked && own { block_input(ui) } else { Vec::new() };
            ui.painter().rect_filled(full, 0.0, self.style.floating_background);
            let buttons = self.native_bar(ui, window);
            let body = Rect::from_min_max(pos2(full.left(), (full.top() + BAR_HEIGHT).min(full.bottom())), full.max);
            self.cross.pointed = None;
            if own {
                crate::ime::window::begin_window(ui.ctx());
            }
            self.floating_body(ui, window, body, full);
            if own {
                self.content.window_drawn(ui);
                crate::ime::window::finish_window(ui.ctx());
            }
            if own && !blocked && ui.ctx().text_edit_focused() {
                let _ = body_input(ui.ctx()).lock().typing_next.insert(viewport);
            }
            self.cross.held |= ui.ctx().dragged_id().is_some_and(|id| self.owned_interactions.contains(&id));
            if debug_close() {
                debug_outlines(ui, full, &buttons);
            }
            if !own {
                return Report::default();
            }
            if window.panels().all(|p| self.permissions(p).movable) {
                if OS_RESIZES {
                    leave_cursor_to_os(ui, full);
                } else {
                    resize_edges(ui, self.id.with(("native-edges", window.id)), full, &buttons);
                }
            }
            let mut surface = Surface {
                window: window.id,
                origin: None,
                bounds: full,
                targets: self.targets.split_off(targets),
                dock_targets: self.dock_targets.split_off(docks),
                windows: self.frame.windows.split_off(windows),
            };
            surface.origin = self.window_gestures(ui, window);
            if blocked {
                (surface.targets, surface.dock_targets, surface.windows) = Default::default();
                cover(ui, self.id.with(("native-blocked", window.id)), full, self.style.blocked_dim);
                if !released.is_empty() {
                    self.output.forwarded_modifiers = Some(ui.input(|i| i.modifiers));
                    self.output.forwarded_events.extend(released);
                }
            } else {
                self.paint_cross(ui, &mut surface);
                self.route_body_drag(ui.ctx(), Some(window.id));
                self.forward_events(ui);
                self.report_files(ui, window, viewport);
            }
            ui.input(|i| Report {
                closed: i.viewport().close_requested(),
                placed: placed(i.viewport()),
                frame: content_of(i.viewport()).map_or(Vec2::ZERO, |(_, frame)| frame),
                monitor_size: i.viewport().monitor_size,
                surface: Some(surface),
            })
        });
        // Targets in another OS window are in its coordinates. An embedded fallback drew its
        // targets in the main window's.
        if report.surface.is_some() {
            self.targets.truncate(targets);
            self.dock_targets.truncate(docks);
            self.frame.windows.truncate(windows);
        }
        self.cross.surfaces.extend(report.surface);
        if let Some(to) = self.cross.moved.take() {
            (place.known[0], place.known[1], place.pending) = (to.x, to.y, Some(now + MOVE_WAIT));
        }
        let moving = moving(ctx);
        // Set every frame from the drag state and the last frame's preview, so leaving a target
        // or any way a drag ends (a drop, Escape, a focus loss, the window closing) brings the
        // window back to full opacity. AppKit finds the window by title, so a window sharing
        // the dragged one's title stays faded with it.
        #[cfg(target_os = "macos")]
        {
            let main = ctx.input(|i| i.raw.viewports.get(&ViewportId::ROOT).and_then(|v| v.title.clone()));
            let drag = ctx.data(|d| d.get_temp::<WindowDrag>(drag_key));
            let dragged = drag.as_ref().and_then(|drag| self.layout.floating.iter().find(|w| w.id == drag.window));
            let this = dragged.and_then(|w| crate::docking::dock_model::first_active(&w.root)).is_some_and(|p| self.label_of(p) == title);
            let opacity = if this { drag_opacity(ctx, self.id) } else { 1.0 };
            set_window_opacity(&title, main.as_deref(), opacity);
        }
        // The collapsed height a minimized window reports is not the size to keep.
        let reported = report.placed.map(|[x, y, w, h]| [x, y, w, if place.collapsed.is_some() { place.known[3] } else { h }]);
        if let Some(placed) = reported.filter(|_| !moving) {
            let waiting = place.pending.is_some_and(|until| now < until);
            if !differs(placed, place.known) {
                place.pending = None;
            } else if waiting {
                // The window system put the window's frame where its content should be.
                let outer = pos2(placed[0], placed[1]) - report.frame;
                if !place.corrected && report.frame.length() > 0.5 && outer.distance(pos2(place.known[0], place.known[1])) < 0.5 {
                    place.corrected = true;
                    ctx.send_viewport_cmd_to(viewport, ViewportCommand::OuterPosition(pos2(place.known[0], place.known[1]) - report.frame));
                }
            } else {
                (place.known, place.pending) = (placed, None);
            }
        }
        ctx.data_mut(|d| d.insert_temp(key, place));
        if report.closed {
            // Like the bar's ×: its panels close and reopen from the application's menu. A
            // window the application would refuse to close stays open.
            if !blocked && window.panels().all(|p| self.permissions(p).closable) {
                self.output.actions.push(Action::CloseWindow { panel: first });
                return Some(key);
            }
            ctx.send_viewport_cmd_to(viewport, ViewportCommand::CancelClose);
        }
        // The layout follows the window's place, once a bar drag is over.
        let known = place.known;
        if !moving && window.native.is_none_or(|native| differs(native, known)) {
            self.output.actions.push(Action::SetNative { panel: first.clone(), rect: Some(known) });
            // The in-window rect follows, for builds that draw the window in-window.
            if window.panels().all(|p| self.permissions(p).movable) {
                let origin = ctx.input(|i| i.viewport().inner_rect.map(|r| r.min)).filter(|p| p.is_finite()).unwrap_or(Pos2::ZERO);
                let [x, y, w, h] = known;
                self.output.actions.push(Action::MoveFloating { panel: first.clone(), rect: [x - origin.x, y - origin.y, w, h] });
            }
        }
        let monitor = monitor_of(&self.monitors, known).cloned();
        let size = match monitor {
            Some(_) => None,
            None => report.monitor_size.filter(|s| s.is_finite() && s.x > 0.0 && s.y > 0.0).map(|s| [s.x, s.y]),
        };
        if (monitor.is_some() || size.is_some()) && (monitor != window.monitor || size != window.monitor_size) {
            self.output.actions.push(Action::SetMonitor { panel: first, monitor, monitor_size: size });
        }
        Some(key)
    }

    /// The bar at the top of a native window: its title and ×. Dragging it moves the window.
    /// Returns the rects of its buttons.
    fn native_bar(&mut self, ui: &mut Ui, window: &Floating<P>) -> Vec<Rect> {
        let Some(first) = window.panels().next().cloned() else { return Vec::new() };
        let full = ui.max_rect();
        let style = ChromeStyle::from_dock(self.style);
        let rect = Rect::from_min_size(full.min, vec2(full.width(), BAR_HEIGHT.min(full.height())));
        ui.painter().rect_filled(rect, 0.0, style.title_background);
        let (close_hit, close_glyph) = close_rects(rect, if OS_RESIZES { MACOS_CLOSE } else { OTHER_CLOSE }, style.right_to_left);
        let movable = window.panels().all(|p| self.permissions(p).movable);
        let closable = self.style.close_button && window.panels().all(|p| self.permissions(p).closable);
        let (move_label, close_label) = (self.style.move_window_label.clone(), self.style.close_window_label.clone());
        let title = crate::docking::dock_model::first_active(&window.root).map(|p| self.label_of(p)).unwrap_or_default();
        // On macOS the bar keeps the arrow, as native title bars do: AppKit's resize areas
        // cover its ends and top edge, and a cursor set over them flickers with AppKit's.
        let mut bar = TitleBar::new(self.id.with(("native-bar", window.id)), &move_label).title(title).movable(movable && !OS_RESIZES);
        if closable {
            bar = bar.close(&close_label).close_at(close_hit, close_glyph);
        }
        let response = bar.show(ui, rect, &style);
        if response.close.as_ref().is_some_and(|close| close.clicked()) {
            self.output.actions.push(Action::CloseWindow { panel: first });
        }
        // Where the window system moves the window, it takes the press, as a title bar's;
        // a drag that egui would only start past the click distance would lose the way there.
        let start = if self.window_system.moves_windows() {
            response.bar.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed())
        } else {
            response.bar.drag_started()
        };
        if movable && start {
            self.start_window_drag(ui, window.id, false);
        }
        response.close.iter().chain(&response.menu).map(|r| r.rect).collect()
    }

    /// After a native window's tree drew: a drag of its only tab, or of its only group, moves
    /// the window as its bar does. Then the window follows a bar drag, and writes the pointer
    /// of a drag it owns. Returns the desktop position of the window's content, where known.
    fn window_gestures(&mut self, ui: &Ui, window: &Floating<P>) -> Option<Pos2> {
        let ctx = ui.ctx();
        let (drag_key, group_key) = (self.id.with("drag"), self.id.with("group-drag"));
        let tab = ctx.data(|d| d.get_temp::<Drag<P>>(drag_key)).filter(|d| window.contains(&d.panel));
        let group = ctx.data(|d| d.get_temp::<GroupDrag<P>>(group_key)).filter(|d| window.contains(&d.panel));
        let lone = window.panels().count() == 1 && tab.is_some();
        let whole = matches!(window.root, Node::Tabs { .. }) && group.as_ref().is_some_and(|g| g.gesture == GroupGesture::Group);
        if (lone || whole) && window.panels().all(|p| self.permissions(p).movable) {
            ctx.data_mut(|d| {
                d.remove::<Drag<P>>(drag_key);
                d.remove::<GroupDrag<P>>(group_key);
            });
            self.start_window_drag(ui, window.id, true);
        }
        self.escape_elsewhere(ctx, Some(window.id));
        let origin = ui.input(|i| content_of(i.viewport())).map(|(origin, _)| origin);
        if let Some(origin) = self.follow_window_drag(ui, window.id, origin) {
            return origin;
        }
        let owns = ctx.data(|d| d.get_temp::<Drag<P>>(drag_key).map(|d| d.panel).or(d.get_temp::<GroupDrag<P>>(group_key).map(|d| d.panel)));
        if owns.is_some_and(|panel| window.contains(&panel)) && self.record_pointer(ctx, Some(window.id), origin, None).escape {
            // The drag's cancel, not a shortcut.
            ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        }
        origin
    }

    /// Start moving the window with the pointer, from the position the window system reports,
    /// or by handing the move to the window system: where it moves windows smoothly (macOS),
    /// tracking the drag, and where it reports no positions (Wayland), leaving it alone. `docks`:
    /// the drag began on the window's only tab or group.
    fn start_window_drag(&mut self, ui: &Ui, window: u64, docks: bool) {
        let ctx = ui.ctx();
        let key = self.id.with("window-drag");
        if ctx.data(|d| d.get_temp::<WindowDrag>(key)).is_some_and(|d| d.window == window) {
            return;
        }
        let os = self.window_system.moves_windows();
        // The window system keeps the window where it is under the pointer now.
        let press =
            ui.input(|i| if os { i.pointer.latest_pos().or(i.pointer.press_origin()) } else { i.pointer.press_origin().or(i.pointer.latest_pos()) });
        match (ui.input(|i| content_of(i.viewport())), press) {
            (Some((origin, _)), Some(press)) => {
                let drag = WindowDrag { window, grab: press.to_vec2(), to: origin, start: origin, docks, os, moved: !os || docks };
                ctx.data_mut(|d| d.insert_temp(key, drag));
                if os {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
            }
            _ => ctx.send_viewport_cmd(ViewportCommand::StartDrag),
        }
    }

    /// Move the window by `OuterPosition` while the button is held, so the pressed point stays
    /// under the pointer. Escape puts it back; losing focus leaves it. Where the window system
    /// moves the window, follow it instead ([`Self::follow_os_drag`]). Returns the desktop
    /// position of the window's content while it moves, `origin` as reported or where it is.
    fn follow_window_drag(&mut self, ui: &Ui, window: u64, origin: Option<Pos2>) -> Option<Option<Pos2>> {
        let key = self.id.with("window-drag");
        let mut drag = ui.ctx().data(|d| d.get_temp::<WindowDrag>(key)).filter(|d| d.window == window)?;
        if drag.os {
            return Some(self.follow_os_drag(ui, drag, origin));
        }
        let content = ui.input(|i| content_of(i.viewport()));
        let frame = content.map_or(Vec2::ZERO, |(_, frame)| frame);
        // A pointer move of this frame is relative to where the window was when the window
        // system sent it, which is where it reports the window now: a move commanded earlier
        // may not have landed yet. Without one the window stays, since the last position egui
        // keeps (button events included) was measured against an older place.
        let moved = ui.input(|i| i.events.iter().rev().find_map(|e| if let Event::PointerMoved(p) = e { Some(*p) } else { None }));
        let to = content.zip(moved.filter(|p| p.is_finite())).map(|((origin, _), local)| origin + (local.to_vec2() - drag.grab));
        let mut step = None;
        if let Some(to) = to.filter(|to| to.distance(drag.to) > 0.0) {
            (drag.to, step) = (to, Some(to));
            ui.ctx().data_mut(|d| d.insert_temp(key, drag));
        }
        // The pointer on the desktop is the pressed point of the window where it was last
        // put, so the origin is that place, less the local pointer. Targets are hit there.
        let local = ui.input(|i| i.pointer.latest_pos()).filter(|p| p.is_finite());
        let origin = local.map(|local| drag.to + drag.grab - local.to_vec2()).or(content.map(|(origin, _)| origin));
        let pointer = self.record_pointer(ui.ctx(), Some(window), origin, Some(&drag));
        let move_to = |r: &mut Self, to: Pos2| {
            ui.ctx().send_viewport_cmd(ViewportCommand::OuterPosition(to - frame));
            r.cross.moved = Some(to);
        };
        if pointer.escape || self.cross.focus_lost {
            if pointer.escape {
                move_to(self, drag.start);
                ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
            }
            ui.ctx().data_mut(|d| d.remove::<WindowDrag>(key));
            return Some(origin);
        }
        if let Some(to) = step {
            move_to(self, to);
        }
        if !pointer.down {
            ui.ctx().data_mut(|d| d.remove::<WindowDrag>(key));
            // Left with too little of its bar on a monitor to grab it: back onto the nearest.
            if let Some(size) = ui.input(|i| i.viewport().inner_rect.map(|r| r.size())) {
                let [x, y, ..] = kept_reachable([drag.to.x, drag.to.y, size.x, size.y], &self.monitors);
                if pos2(x, y).distance(drag.to) > 0.5 {
                    move_to(self, pos2(x, y));
                }
            }
            self.end_window_drag(drag, pointer);
        }
        Some(origin)
    }

    /// A bar drag that the window system carries out (`StartDrag`): the window stays under the
    /// cursor at the pressed point, so it is where the cursor is less the grab. Its reported
    /// place lags the move, and AppKit tells the cursor fresh each frame. The drag ends on the
    /// first frame on which the window system or egui reports the release, and then once.
    /// Escape does not end it, as with Finder's windows: AppKit sends no keys meanwhile.
    /// Returns where the window's content is.
    fn follow_os_drag(&mut self, ui: &Ui, mut drag: WindowDrag, origin: Option<Pos2>) -> Option<Pos2> {
        let ctx = ui.ctx();
        let key = self.id.with("window-drag");
        let local = ui.input(|i| i.pointer.latest_pos()).filter(|p| p.is_finite());
        let cursor = self.window_system.cursor(ctx).or_else(|| Some(origin? + local?.to_vec2()));
        if let Some(cursor) = cursor {
            drag.to = cursor - drag.grab;
            drag.moved |= drag.to.distance(drag.start) > ctx.options(|o| o.input_options.max_click_dist);
        }
        let (egui_down, released) = ui.input(|i| (i.pointer.primary_down(), i.pointer.primary_released()));
        let ended = self.window_system.primary_held() == Some(false) || released || !egui_down;
        let command =
            self.window_system.command_held().unwrap_or_else(|| ui.input(|i| i.modifiers.command || i.modifiers.ctrl || i.modifiers.mac_cmd));
        if self.cross.focus_lost || ended {
            ctx.data_mut(|d| d.remove::<WindowDrag>(key));
        } else {
            ctx.data_mut(|d| d.insert_temp(key, drag));
        }
        // A click on the bar is no drop, and the window's reports say where it is.
        if !drag.moved {
            return origin;
        }
        self.cross.moved = Some(drag.to);
        if self.cross.focus_lost {
            return Some(drag.to);
        }
        let pointer = DragPointer {
            owner: Some(drag.window),
            origin: Some(drag.to),
            local: Some(drag.grab.to_pos2()),
            down: !ended,
            released: ended,
            escape: false,
            command,
            window: Some(drag.window),
            docks: drag.docks,
        };
        self.write_pointer(ctx, pointer);
        if ended {
            // Left with too little of its bar on a monitor to grab it: back onto the nearest.
            if let Some(size) = ui.input(|i| i.viewport().inner_rect.map(|r| r.size())) {
                let [x, y, ..] = kept_reachable([drag.to.x, drag.to.y, size.x, size.y], &self.monitors);
                if pos2(x, y).distance(drag.to) > 0.5 {
                    let frame = ui.input(|i| content_of(i.viewport())).map_or(Vec2::ZERO, |(_, frame)| frame);
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos2(x, y) - frame));
                    self.cross.moved = Some(pos2(x, y));
                }
            }
            self.end_window_drag(drag, pointer);
        }
        Some(drag.to)
    }

    /// Release of a bar drag. The window already sits where it was dragged; over a target in
    /// another window it docks there once every window has drawn (`cross.rs`).
    fn end_window_drag(&mut self, drag: WindowDrag, pointer: DragPointer) {
        self.cross.release = Some((drag, pointer));
    }

    /// Files dragged over or dropped on the window this frame, with the panel under the pointer.
    fn report_files(&mut self, ui: &Ui, window: &Floating<P>, viewport: ViewportId) {
        let (hovered, dropped, pos) = ui.input(|i| (i.raw.hovered_files.clone(), i.raw.dropped_files.clone(), i.pointer.latest_pos()));
        if hovered.is_empty() && dropped.is_empty() {
            return;
        }
        // The window system may send no pointer moves while files are dragged in.
        let Some(panel) = self.cross.pointed.clone().or_else(|| crate::docking::dock_model::first_active(&window.root).cloned()) else { return };
        // A window whose own pass is discarded draws twice: keep one report, with the latest
        // hovered files and every dropped one.
        if let Some(files) = self.output.window_files.iter_mut().find(|f| f.viewport == viewport) {
            files.panel = panel;
            files.pos = pos;
            files.hovered = hovered;
            files.dropped.extend(dropped);
        } else {
            self.output.window_files.push(WindowFiles { window: window.id, viewport, panel, pos, hovered, dropped });
        }
    }

    /// Keys, text and clipboard events the window's widgets left over, for the application's
    /// shortcuts. Nothing while a widget of the window has keyboard focus: its keys are its own.
    fn forward_events(&mut self, ui: &Ui) {
        if ui.memory(|m| m.focused().is_some()) {
            return;
        }
        let events = ui.input(|i| {
            i.events
                .iter()
                .filter(|e| matches!(e, Event::Key { .. } | Event::Text(_) | Event::Copy | Event::Cut | Event::Paste(_)))
                .cloned()
                .collect::<Vec<_>>()
        });
        if !events.is_empty() {
            self.output.forwarded_modifiers = Some(ui.input(|i| i.modifiers));
            self.output.forwarded_events.extend(events);
        }
    }
}

/// Input hooks of dock areas for their native windows, run by egui before it reads each
/// window's input. eframe runs `App::raw_input_hook` for the main window only. App drags in
/// panel bodies cross windows here too (`body_drag.rs`).
#[derive(Default)]
pub(super) struct NativeInput {
    hooks: std::collections::HashMap<Id, egui::mutex::Mutex<InputHook>>,
    /// The dock area of each native window shown this frame.
    windows: std::collections::HashMap<ViewportId, Id>,
}

/// Each dock area's app drag in a panel body, the events waiting for a window's next input, and
/// the windows whose text fields have the keyboard. Kept outside [`NativeInput`]: egui calls
/// [`egui::Plugin::input_hook`] with the plugin locked, and an app's input hook may call
/// [`crate::docking::cancel_drag`] or [`crate::docking::send_keys`], which must reach this state
/// without locking the plugin again. The lock here is dropped before that hook runs.
#[derive(Default)]
pub(super) struct NativeBodyInput {
    /// Each dock area's app drag in a panel body.
    pub routes: std::collections::HashMap<Id, BodyRoute>,
    /// Pointer events routed to each window, for its next input.
    pub queued: Queued,
    /// Owners' held releases, for their input in a later main-window pass.
    pub held: Held,
    /// The main window's passes so far.
    pub pass: u64,
    /// The windows that had a focused text field in the last main-window pass, and those
    /// noted so far in this one.
    typing: std::collections::HashSet<ViewportId>,
    typing_next: std::collections::HashSet<ViewportId>,
    /// The main window's completed passes when `typing` was last taken from `typing_next`.
    typing_at: u64,
    /// Key and clipboard events for each window's next input, and the pass they were queued in.
    keys: std::collections::HashMap<ViewportId, (u64, Vec<Event>)>,
}

/// The app drag routing and routed events shared by every native input hook of `ctx`.
pub(super) fn body_input(ctx: &Context) -> Arc<egui::mutex::Mutex<NativeBodyInput>> {
    ctx.data_mut(|data| data.get_temp_mut_or_default::<Arc<egui::mutex::Mutex<NativeBodyInput>>>(body_input_key()).clone())
}

/// The temp-data key for the context's native body-routing state.
///
/// All dock areas in a context deliberately use this one container: its `routes` map is keyed by
/// dock-area `Id`, and queued, held and key events by target viewport. The key is unique only so
/// this state does not collide with unrelated temporary data.
fn body_input_key() -> Id {
    Id::unique("craft-ui docking native body input")
}

pub(in crate::docking) type InputHook = Box<dyn FnMut(&Context, &mut RawInput) + Send>;

impl egui::Plugin for NativeInput {
    /// The egui plugin's diagnostic name, `craft-ui docking native input`.
    fn debug_name(&self) -> &'static str {
        "craft-ui docking native input"
    }

    /// Route this viewport's input without deadlocking.
    ///
    /// Only the root viewport advances the frame pass; queued, held and key entries older than the
    /// current pass are pruned. Events waiting for this viewport are taken before routing, so events
    /// enqueued by routing wait for its next input. A held release is only released once a later
    /// root pass exists. Earlier routed events are spliced before the window's own events, and any
    /// accepted route requests a root repaint. The dock area's application hook is called only
    /// after the routing mutex is dropped because it may cancel a drag and re-enter the shared
    /// body state.
    fn input_hook(&mut self, ctx: &Context, input: &mut RawInput) {
        let viewport = input.viewport_id;
        let body = body_input(ctx);
        let mut routed = false;
        {
            let mut body = body.lock();
            let body = &mut *body;
            if viewport == ViewportId::ROOT {
                body.pass += 1;
                // Every window draws in the main window's pass: events for one that has not
                // drawn since the last pass are for a window that is gone.
                let pass = body.pass;
                body.queued.retain(|_, (at, _)| *at + 1 >= pass);
                body.held.retain(|_, (at, _)| *at + 1 >= pass);
                body.keys.retain(|_, (at, _)| *at + 1 >= pass);
                body.typing = std::mem::take(&mut body.typing_next);
                body.typing_at = ctx.cumulative_pass_nr_for(ViewportId::ROOT);
                // eframe has run `App::raw_input_hook`, where an app's native menu reports its key
                // equivalents (macOS: AppKit's menu takes them before the key window sees them).
                // While a native window has the keyboard, the main window gets no keys of its own,
                // so these are the menu's: those a focused text field there acts on are its own.
                if let Some(window) = typing_window_of(&body.typing, &input.viewports) {
                    let (moved, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut input.events).into_iter().partition(text_key);
                    input.events = kept;
                    if !moved.is_empty() {
                        let entry = body.keys.entry(window).or_default();
                        entry.0 = pass;
                        entry.1.extend(moved);
                        ctx.request_repaint_of(window);
                    }
                }
            }
            // Taken before routing, which may queue more for this window's next input. A held
            // release waits for a later main-window pass: a discarded pass reruns within its own.
            let waiting = body.queued.remove(&viewport);
            let keys = body.keys.remove(&viewport).map(|(_, mut keys)| {
                text_clipboard_keys(ctx, viewport, &mut keys);
                keys
            });
            let held = if body.held.get(&viewport).is_some_and(|(at, _)| *at < body.pass) { body.held.remove(&viewport) } else { None };
            for route in body.routes.values_mut() {
                routed |= route.route(input, &mut body.queued, &mut body.held, body.pass);
            }
            // Before the window's own events: they happened earlier, and a held release must
            // not end a press that follows it.
            let earlier = held
                .into_iter()
                .flat_map(|(_, events)| events)
                .chain(waiting.into_iter().flat_map(|(_, events)| events.into_iter().map(|(_, e)| e)))
                .chain(keys.into_iter().flatten());
            input.events.splice(0..0, earlier);
        }
        if routed {
            ctx.request_repaint_of(ViewportId::ROOT);
        }
        // After the routing lock is dropped: the app's hook may call `cancel_drag`, which locks
        // the same body state through egui's temporary data, not this plugin.
        if let Some(hook) = self.windows.get(&input.viewport_id).and_then(|area| self.hooks.get(area)) {
            (hook.lock())(ctx, input);
        }
    }
}

/// The native window in `typing` that has the keyboard, by the main window's `viewports`.
fn typing_window_of(typing: &std::collections::HashSet<ViewportId>, viewports: &egui::ViewportIdMap<ViewportInfo>) -> Option<ViewportId> {
    typing.iter().copied().find(|viewport| viewports.get(viewport).and_then(|info| info.focused) == Some(true))
}

/// Whether a text field acts on `event`: the clipboard events, and the command keys that copy,
/// paste, select all, undo, redo, move or delete in text. Other command keys are the app's
/// (save, close, new window, its own shortcuts).
fn text_key(event: &Event) -> bool {
    match event {
        Event::Copy | Event::Cut | Event::Paste(_) => true,
        Event::Key { key, modifiers: m, .. } if m.command && !m.alt => match key {
            Key::C | Key::X | Key::V | Key::A | Key::Backspace | Key::Delete => !m.shift,
            Key::Z | Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown => true,
            _ => false,
        },
        _ => false,
    }
}

/// A text field copies and pastes on `Copy`, `Cut` and `Paste` events, which egui-winit makes
/// from the window's own key presses. A command C, X or V press here is a menu's key
/// equivalent, which a text field ignores: give back C and X as `Copy` and `Cut`, and turn V
/// into a request for the clipboard text, which arrives in `viewport` as `Paste`.
fn text_clipboard_keys(ctx: &Context, viewport: ViewportId, events: &mut Vec<Event>) {
    events.retain_mut(|e| {
        let Event::Key { key, pressed: true, modifiers: m, .. } = *e else { return true };
        if !m.command || m.alt || m.shift {
            return true;
        }
        match key {
            Key::C => *e = Event::Copy,
            Key::X => *e = Event::Cut,
            Key::V => {
                ctx.send_viewport_cmd_to(viewport, ViewportCommand::RequestPaste);
                return false;
            }
            _ => {}
        }
        true
    });
}

/// See [`crate::docking::typing_window`].
pub(in crate::docking) fn typing_window(ctx: &Context) -> Option<ViewportId> {
    // Between frames (in `App::raw_input_hook`) a main-window pass has ended since the swap, and
    // `typing_next` is the whole of the last pass, which the next swap hands to the routing.
    let passes = ctx.cumulative_pass_nr_for(ViewportId::ROOT);
    let body = body_input(ctx);
    let typing = {
        let input = body.lock();
        if passes > input.typing_at { input.typing_next.clone() } else { input.typing.clone() }
    };
    ctx.input_for(ViewportId::ROOT, |i| typing_window_of(&typing, &i.raw.viewports))
}

/// See [`crate::docking::send_keys`].
pub(in crate::docking) fn send_keys(ctx: &Context, viewport: ViewportId, events: Vec<Event>) {
    if events.is_empty() {
        return;
    }
    {
        let body = body_input(ctx);
        let mut input = body.lock();
        let pass = input.pass;
        let entry = input.keys.entry(viewport).or_default();
        entry.0 = pass;
        entry.1.extend(events);
    }
    ctx.request_repaint_of(viewport);
}

/// Set or clear the input hook of the dock area `area` for this frame, and forget its native
/// windows until they show again.
pub(in crate::docking) fn set_input_hook(ctx: &Context, area: Id, hook: Option<InputHook>) {
    if ctx.plugin_opt::<NativeInput>().is_none() {
        ctx.add_plugin(NativeInput::default());
    }
    ctx.with_plugin(|input: &mut NativeInput| {
        input.windows.retain(|_, a| *a != area);
        match hook {
            Some(hook) => input.hooks.insert(area, egui::mutex::Mutex::new(hook)),
            None => input.hooks.remove(&area),
        }
    });
}

/// Frames running whose native windows are all new, with the last frame's panels, before
/// [`Renderer::warn_rebuilt`] warns.
#[cfg(debug_assertions)]
const REBUILT_FRAMES: usize = 3;

/// What the environment says of winit's backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::docking) enum Session {
    /// `WAYLAND_DISPLAY` or `WAYLAND_SOCKET` is set: winit picks Wayland unless the app forces
    /// X11 in code (`EventLoopBuilderExtX11::with_x11`); winit reads no variable that does.
    Wayland,
    /// None of those nor `DISPLAY` is set on a Unix desktop: a Wayland app started with only
    /// `WAYLAND_SOCKET`, which winit unsets as it connects, or no display at all (tests, CI).
    Unknown,
    /// X11 or another backend.
    Other,
}

/// The [`Session`] of a Unix desktop (`unix`) whose environment variable `name` is set and
/// not empty when `set(name)`.
pub(in crate::docking) fn session(unix: bool, set: impl Fn(&str) -> bool) -> Session {
    if !unix {
        Session::Other
    } else if set("WAYLAND_DISPLAY") || set("WAYLAND_SOCKET") {
        Session::Wayland
    } else if set("DISPLAY") {
        Session::Other
    } else {
        Session::Unknown
    }
}

/// This process's [`Session`], read once.
fn env_session() -> Session {
    static SESSION: std::sync::OnceLock<Session> = std::sync::OnceLock::new();
    *SESSION.get_or_init(|| {
        session(cfg!(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd")), |name| {
            std::env::var_os(name).is_some_and(|value| !value.is_empty())
        })
    })
}

/// Whether the main window runs on Wayland, given the environment's `session`. egui does not
/// name the backend, but on Wayland the main window never reports its outer rect, while under
/// X11 it does from the first frame on (eframe reads it as the window opens). A minimized X11
/// window reports none either, so once one has, the backend is not Wayland. Without a Wayland
/// variable, only a real OS window (one that reports its native pixels per point) counts.
pub(in crate::docking) fn wayland(ctx: &Context, session: Session) -> bool {
    let root = |test: fn(&ViewportInfo) -> bool| ctx.input(|i| i.raw.viewports.get(&ViewportId::ROOT).is_some_and(test));
    match session {
        Session::Other => return false,
        Session::Unknown if !root(|v| v.native_pixels_per_point.is_some()) => return false,
        Session::Wayland | Session::Unknown => {}
    }
    let key = Id::unique("craft-ui-dock-window-positions");
    if ctx.data(|d| d.get_temp::<bool>(key).unwrap_or(false)) {
        return false;
    }
    let reported = root(|v| v.outer_rect.is_some());
    if reported {
        ctx.data_mut(|d| d.insert_temp(key, true));
    }
    !reported
}

/// Whether every window of `current` is new to `previous`, and they hold the same panels:
/// windows, with their panels, as `(id, panels)`.
#[cfg(debug_assertions)]
fn rebuilt<P: Eq + Hash>(previous: &[(u64, Vec<P>)], current: &[(u64, Vec<P>)]) -> bool {
    !current.is_empty() && current.iter().all(|(id, _)| previous.iter().all(|(p, _)| p != id)) && panels(previous) == panels(current)
}

#[cfg(debug_assertions)]
fn panels<P: Eq + Hash>(windows: &[(u64, Vec<P>)]) -> std::collections::HashSet<&Vec<P>> {
    windows.iter().map(|(_, panels)| panels).collect()
}

/// The start of a blocked native window's frame: its widgets are disabled, and the pointer,
/// keys, text and scrolling are taken out of its input before anything reads them, so a panel
/// that reads input directly sees none either. Returns the key releases it took out, for the
/// app: a key pressed before the block would otherwise stay down in the app's shortcuts.
fn block_input(ui: &mut Ui) -> Vec<Event> {
    // Disabling fades what the window paints, its opaque background too; the cover dims it.
    let opacity = ui.opacity();
    ui.disable();
    ui.set_opacity(opacity);
    // Native reset acknowledgements are ordered control messages, even while widgets are blocked.
    let kept = |e: &Event| matches!(e, Event::WindowFocused(_) | Event::ImeResetCompleted { .. });
    let released = ui.input(|i| i.events.iter().filter(|e| matches!(e, Event::Key { pressed: false, .. })).cloned().collect());
    ui.input_mut(|i| {
        i.events.retain(kept);
        i.raw.events.retain(kept);
        // File drops are fields of their own, not events.
        i.raw.dropped_files.clear();
        i.raw.hovered_files.clear();
        i.keys_down.clear();
        i.smooth_scroll_delta = Vec2::ZERO;
        i.pointer = Default::default();
    });
    ui.memory_mut(|m| m.stop_text_input());
    released
}

/// The end of a blocked native window's frame: `dim` over its content `full`, in a layer above
/// its popups that takes the pointer, so a popup left open takes no click either.
fn cover(ui: &Ui, id: Id, full: Rect, dim: Color32) {
    let cover = Ui::new(ui.ctx().clone(), id, UiBuilder::new().layer_id(LayerId::new(Order::Tooltip, id)).max_rect(full));
    cover.painter().rect_filled(full, 0.0, dim);
    cover.interact(full, id.with("sink"), Sense::CLICK | Sense::DRAG);
}

/// Move and size a native window so its content is the desktop rect `to`. The size goes
/// first, so the move puts the top-left corner in place whichever corner the window system
/// keeps on a resize (AppKit's frame origin is the bottom left).
fn place_window(ctx: &Context, viewport: ViewportId, to: [f32; 4], frame: Vec2) {
    ctx.send_viewport_cmd_to(viewport, ViewportCommand::InnerSize(vec2(to[2], to[3])));
    ctx.send_viewport_cmd_to(viewport, ViewportCommand::OuterPosition(pos2(to[0], to[1]) - frame));
}

/// The resize bands of a movable native window without a frame of its own. Drawn after its
/// content, so they take the pointer at its edges; the bar's `buttons` are left out.
fn resize_edges(ui: &Ui, id: Id, full: Rect, buttons: &[Rect]) {
    for (i, (direction, band)) in resize_bands(full, buttons).into_iter().enumerate() {
        let response = ui.interact(band, id.with(i), Sense::drag()).on_hover_cursor(resize_cursor(direction));
        if response.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::BeginResize(direction));
        }
    }
}

/// Over AppKit's resize areas of the window's content `full`, the default cursor whatever the
/// content asked for: AppKit shows its resize cursor there, and any other cursor we set fights
/// it.
fn leave_cursor_to_os(ui: &Ui, full: Rect) {
    if ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| appkit_resize_areas(full).iter().any(|area| area.contains(p))) {
        ui.ctx().set_cursor_icon(CursorIcon::Default);
    }
}

/// AppKit's resize areas inside a window's content `rect`: a square at each corner and a strip
/// along each edge.
pub(in crate::docking) fn appkit_resize_areas(rect: Rect) -> [Rect; 8] {
    let (c, e) = (Vec2::splat(APPKIT_CORNER), APPKIT_EDGE);
    [
        Rect::from_min_size(rect.left_top(), c),
        Rect::from_min_size(rect.right_top() - vec2(c.x, 0.0), c),
        Rect::from_min_size(rect.left_bottom() - vec2(0.0, c.y), c),
        Rect::from_min_size(rect.right_bottom() - c, c),
        rect.with_max_y(rect.top() + e),
        rect.with_min_y(rect.bottom() - e),
        rect.with_max_x(rect.left() + e),
        rect.with_min_x(rect.right() - e),
    ]
}

/// Over a native window's content `full`: its bar's `buttons` outlined in red and, on macOS,
/// AppKit's resize areas in orange.
fn debug_outlines(ui: &Ui, full: Rect, buttons: &[Rect]) {
    let stroke = |color| egui::Stroke::new(1.0, color);
    for button in buttons {
        ui.painter().rect_stroke(*button, 0.0, stroke(egui::Color32::RED), egui::StrokeKind::Inside);
    }
    if OS_RESIZES {
        for area in appkit_resize_areas(full) {
            ui.painter().rect_stroke(area, 0.0, stroke(egui::Color32::ORANGE), egui::StrokeKind::Inside);
        }
    }
}

/// The hit area and glyph of the × in a native window's bar `bar`, laid out by `layout` from
/// the bar's top and trailing edges.
pub(in crate::docking) fn close_rects(bar: Rect, layout: CloseLayout, right_to_left: bool) -> (Rect, Rect) {
    let l = layout;
    let glyph = Rect::from_min_size(pos2(bar.right() - l.glyph_trailing - l.glyph_size, bar.top() + l.glyph_top), Vec2::splat(l.glyph_size));
    let hit = Rect::from_min_max(glyph.min - vec2(l.hit_left, l.hit_up), glyph.max + vec2(l.hit_right, l.hit_down));
    let mirror = |r: Rect| {
        if right_to_left {
            Rect::from_x_y_ranges((bar.left() + bar.right() - r.right())..=(bar.left() + bar.right() - r.left()), r.y_range())
        } else {
            r
        }
    };
    (mirror(hit), mirror(glyph))
}

/// The bands along the edges of a native window's content `rect` that resize it: an L at each
/// corner, [`EDGE`] deep and [`CORNER`] along both edges, and the edges between them. They
/// leave out the bar's `buttons`, which take their own clicks.
pub(in crate::docking) fn resize_bands(rect: Rect, buttons: &[Rect]) -> Vec<(ResizeDirection, Rect)> {
    use ResizeDirection as D;
    let (l, r, t, b, e, c) = (rect.left(), rect.right(), rect.top(), rect.bottom(), EDGE, CORNER);
    let band = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
    let bands = [
        (D::NorthWest, band(l, t, l + c, t + e)),
        (D::NorthWest, band(l, t, l + e, t + c)),
        (D::NorthEast, band(r - c, t, r, t + e)),
        (D::NorthEast, band(r - e, t, r, t + c)),
        (D::SouthWest, band(l, b - e, l + c, b)),
        (D::SouthWest, band(l, b - c, l + e, b)),
        (D::SouthEast, band(r - c, b - e, r, b)),
        (D::SouthEast, band(r - e, b - c, r, b)),
        (D::North, band(l + c, t, r - c, t + e)),
        (D::South, band(l + c, b - e, r - c, b)),
        (D::West, band(l, t + c, l + e, b - c)),
        (D::East, band(r - e, t + c, r, b - c)),
    ];
    let mut out = Vec::new();
    for (direction, band) in bands {
        let mut pieces = vec![band];
        for button in buttons {
            pieces = pieces.into_iter().flat_map(|piece| outside(piece, *button)).collect();
        }
        out.extend(pieces.into_iter().filter(|p| p.is_positive()).map(|p| (direction, p)));
    }
    out
}

/// The parts of `rect` outside `hole`: up to four rects.
fn outside(rect: Rect, hole: Rect) -> Vec<Rect> {
    let overlap = rect.intersect(hole);
    if !overlap.is_positive() {
        return vec![rect];
    }
    [
        Rect::from_min_max(rect.min, pos2(rect.right(), overlap.top())),
        Rect::from_min_max(pos2(rect.left(), overlap.bottom()), rect.max),
        Rect::from_min_max(pos2(rect.left(), overlap.top()), overlap.left_bottom()),
        Rect::from_min_max(overlap.right_top(), pos2(rect.right(), overlap.bottom())),
    ]
    .into_iter()
    .filter(|r| r.is_positive())
    .collect()
}

fn resize_cursor(direction: ResizeDirection) -> CursorIcon {
    use {CursorIcon as C, ResizeDirection as D};
    match direction {
        D::North => C::ResizeNorth,
        D::South => C::ResizeSouth,
        D::East => C::ResizeEast,
        D::West => C::ResizeWest,
        D::NorthEast => C::ResizeNorthEast,
        D::NorthWest => C::ResizeNorthWest,
        D::SouthEast => C::ResizeSouthEast,
        D::SouthWest => C::ResizeSouthWest,
    }
}
