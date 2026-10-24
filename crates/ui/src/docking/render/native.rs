//! Floating windows in their own OS windows (feature `native-windows`). Where the backend has
//! viewports, every floating window whose panels allow it is drawn with
//! `show_viewport_immediate`, always on top of other windows and hidden while none of the
//! application's windows has focus. Where egui embeds viewports (wasm, tests) windows stay
//! in-window at their in-window rect. Tabs, groups and whole windows drag between OS windows
//! (see `cross.rs`).

use std::{fmt::Debug, hash::Hash};

use egui::{
    Context, CursorIcon, Event, Id, Key, Modifiers, Pos2, RawInput, Rect, ResizeDirection, Sense, Ui, Vec2, ViewportBuilder, ViewportClass,
    ViewportCommand, ViewportId, ViewportInfo, WindowLevel, pos2, vec2,
};

use super::Renderer;
use super::cross::{DragPointer, Surface};
use crate::docking::chrome::{ChromeStyle, TitleBar};
pub(in crate::docking) use crate::docking::placement::BAR_HEIGHT;
use crate::docking::placement::{kept_reachable, monitor_of, restore, sanitized};
use crate::docking::{Action, Drag, Floating, GroupDrag, GroupGesture, Node, WindowFiles};

/// Smallest native window.
const MIN_SIZE: [f32; 2] = [160.0, 100.0];
/// How long floating windows stay after the application's last window lost focus, so focus
/// moving between our own windows does not hide them.
const HIDE_DELAY: f64 = 0.25;
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

/// Opacity of a native window while its bar or only tab drags it: the window stays above the
/// main window, so the drop preview under the pointer shows only through it.
#[cfg(target_os = "macos")]
const DRAG_OPACITY: f64 = 0.5;

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
    /// Desktop position of the window's content as last commanded.
    pub to: Pos2,
    /// Desktop position of the window's content when the drag began, for Escape.
    pub start: Pos2,
    /// The drag began on the window's only tab or group, so it docks whatever
    /// [`crate::docking::DockStyle::title_bar_docks`] says.
    pub docks: bool,
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
}

/// Whether two rects differ by more than rounding.
fn differs(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).any(|(a, b)| (a - b).abs() > 0.5)
}

/// Desktop position of a viewport's content, and its offset from the window's outer
/// position, where the window system reports both.
fn content_of(info: &ViewportInfo) -> Option<(Pos2, Vec2)> {
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

/// Whether floating windows show: while one of the application's windows has focus, and until
/// [`HIDE_DELAY`] after `last_focused`, when one last had it.
pub(in crate::docking) fn floats_shown(focused: bool, now: f64, last_focused: f64) -> bool {
    focused || now - last_focused < HIDE_DELAY
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
        let native = !ctx.embed_viewports();
        let origin = ctx.input(|i| i.viewport().inner_rect.map(|r| r.min)).filter(|p| p.is_finite()).unwrap_or(Pos2::ZERO);
        let visibility = if native { self.float_visibility(ctx) } else { (true, false) };
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
            self.map_desktop(ctx, &natives, visibility.0);
            // The main window owns drags of the panels it draws.
            let id = self.id;
            let dragged = ctx.data(|d| {
                d.get_temp::<Drag<P>>(id.with("drag")).map(|d| d.panel).or(d.get_temp::<GroupDrag<P>>(id.with("group-drag")).map(|d| d.panel))
            });
            if dragged.is_some_and(|panel| !natives.iter().any(|(w, ..)| w.contains(&panel))) {
                let origin = self.cross.main.map(|m| m.min);
                self.record_pointer(ctx, None, origin, None);
            }
        }
        for (window, rect, viewport) in natives {
            if let Some(key) = self.native_window(ctx, window, rect, viewport, visibility) {
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

    /// The main window and the shown native windows on the desktop, front first: the window
    /// focused most recently first. A window whose position is unknown is left out.
    fn map_desktop(&mut self, ctx: &Context, natives: &[(&Floating<P>, [f32; 4], ViewportId)], visible: bool) {
        let key = self.id.with("native-order");
        let mut order = ctx.data(|d| d.get_temp::<Vec<u64>>(key)).unwrap_or_default();
        order.retain(|id| natives.iter().any(|(w, ..)| w.id == *id));
        let mut windows = Vec::new();
        for (window, rect, viewport) in natives {
            let info = ctx.input(|i| i.raw.viewports.get(viewport).cloned());
            // A new window opens in front.
            if info.as_ref().is_some_and(|i| i.focused == Some(true)) || !order.contains(&window.id) {
                order.retain(|id| *id != window.id);
                order.insert(0, window.id);
            }
            let [x, y, w, h] = *rect;
            let placed = match info {
                None => Some(Rect::from_min_size(pos2(x, y), vec2(w, h))),
                Some(info) => placed(&info).map(|[x, y, w, h]| Rect::from_min_size(pos2(x, y), vec2(w, h))),
            };
            if let Some(placed) = placed.filter(|r| r.is_finite() && visible) {
                windows.push((window.id, placed));
            }
        }
        windows.sort_by_key(|(id, _)| order.iter().position(|o| o == id));
        ctx.data_mut(|d| d.insert_temp(key, order));
        self.cross.windows = windows;
        self.cross.main = ctx.input(|i| i.viewport().outer_rect.and(i.viewport().inner_rect)).filter(|r| r.is_finite());
    }

    /// Whether floating windows show this frame, and whether that changed since the last.
    fn float_visibility(&mut self, ctx: &Context) -> (bool, bool) {
        let (focused, now) = ctx.input(|i| {
            let infos = || i.raw.viewports.values();
            // A backend that reports no focus at all keeps the windows.
            let focused = i.focused || infos().any(|v| v.focused == Some(true)) || infos().all(|v| v.focused.is_none());
            (focused, i.time)
        });
        self.cross.focus_lost = crate::focus::turned_false(ctx, self.id.with("app-focus"), focused);
        let key = self.id.with("native-visibility");
        let (last, was) = ctx.data(|d| d.get_temp::<(f64, bool)>(key)).unwrap_or((now, true));
        let last = if focused { now } else { last };
        let visible = floats_shown(focused, now, last);
        if visible && !focused {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((HIDE_DELAY - (now - last)).max(0.0)));
        }
        ctx.data_mut(|d| d.insert_temp(key, (last, visible)));
        (visible, visible != was)
    }

    /// One floating window in its own viewport. Returns the key of its [`Place`].
    fn native_window(&mut self, ctx: &Context, window: &Floating<P>, rect: [f32; 4], viewport: ViewportId, visibility: (bool, bool)) -> Option<Id> {
        let (visible, toggled) = visibility;
        let first = window.panels().next()?.clone();
        let key = self.id.with(("native-place", viewport));
        let now = ctx.input(|i| i.time);
        let drag_key = self.id.with("window-drag");
        let moving = |ctx: &Context| ctx.data(|d| d.get_temp::<WindowDrag>(drag_key)).is_some_and(|d| d.window == window.id);
        let info = ctx.input(|i| i.raw.viewports.get(&viewport).cloned());
        let frame = info.as_ref().and_then(content_of).map_or(Vec2::ZERO, |(_, frame)| frame);
        let mut place = match ctx.data(|d| d.get_temp::<Place>(key)) {
            Some(mut place) => {
                // The application changed the window's place (undo, reset, a reloaded layout):
                // the OS window moves there, kept where its bar can be grabbed.
                if !moving(ctx)
                    && let Some(native) = window.native.filter(|native| differs(*native, place.known))
                {
                    let target = kept_reachable(native, &self.monitors);
                    place_window(ctx, viewport, target, frame);
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
                Place { spawn, known: spawn, pending: Some(now + MOVE_WAIT), corrected: false }
            }
        };
        let spawn = place.spawn;
        let title = crate::docking::dock_model::first_active(&window.root).map(|p| self.label_of(p)).unwrap_or_default();
        let builder = ViewportBuilder::default()
            .with_title(title.clone())
            .with_position([spawn[0], spawn[1]])
            .with_inner_size([spawn[2], spawn[3]])
            .with_min_inner_size(MIN_SIZE)
            .with_window_level(WindowLevel::AlwaysOnTop)
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
            .with_resizable(window.panels().all(|p| self.permissions(p).movable));
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_decorations(false);
        if toggled || (info.is_none() && !visible) {
            ctx.send_viewport_cmd_to(viewport, ViewportCommand::Visible(visible));
        }
        if visible && self.activation.as_ref().is_some_and(|panel| window.contains(panel)) {
            ctx.send_viewport_cmd_to(viewport, ViewportCommand::Focus);
        }
        let (targets, docks, windows) = (self.targets.len(), self.dock_targets.len(), self.frame.windows.len());
        ctx.with_plugin(|input: &mut NativeInput| input.windows.insert(viewport, self.id));
        let report = ctx.show_viewport_immediate(viewport, builder, |ui, class| {
            let full = ui.max_rect();
            ui.painter().rect_filled(full, 0.0, self.style.floating_background);
            let buttons = self.native_bar(ui, window);
            let body = Rect::from_min_max(pos2(full.left(), (full.top() + BAR_HEIGHT).min(full.bottom())), full.max);
            self.cross.pointed = None;
            self.floating_body(ui, window, body, full);
            if debug_close() {
                debug_outlines(ui, full, &buttons);
            }
            // An embedded fallback reads the main window's input: its close request, place,
            // keys and resizes are not this window's.
            if !matches!(class, ViewportClass::Immediate | ViewportClass::Deferred) {
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
            self.paint_cross(ui, &mut surface);
            self.forward_events(ui);
            self.report_files(ui, window, viewport);
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
        // Set every frame from the drag state, so any way a drag ends (a drop, Escape, a focus
        // loss, the window closing) brings the window back to full opacity. AppKit finds the
        // window by title, so a window sharing the dragged one's title stays faded with it.
        #[cfg(target_os = "macos")]
        {
            let main = ctx.input(|i| i.raw.viewports.get(&ViewportId::ROOT).and_then(|v| v.title.clone()));
            let dragged = ctx.data(|d| d.get_temp::<WindowDrag>(drag_key)).and_then(|drag| self.layout.floating.iter().find(|w| w.id == drag.window));
            let faded = dragged.and_then(|w| crate::docking::dock_model::first_active(&w.root)).is_some_and(|p| self.label_of(p) == title);
            set_window_opacity(&title, main.as_deref(), if faded { DRAG_OPACITY } else { 1.0 });
        }
        if let Some(placed) = report.placed.filter(|_| !moving) {
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
            // Like the bar's ×: its panels close and reopen from the application's menu.
            self.output.actions.push(Action::CloseWindow { panel: first });
            return Some(key);
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
        if movable && response.bar.drag_started() {
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
        if self.follow_window_drag(ui, window.id) {
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
    /// or, where it reports none (Wayland), by handing the move to it. `docks`: the drag began
    /// on the window's only tab or group.
    fn start_window_drag(&mut self, ui: &Ui, window: u64, docks: bool) {
        let press = ui.input(|i| i.pointer.press_origin().or(i.pointer.latest_pos()));
        match (ui.input(|i| content_of(i.viewport())), press) {
            (Some((origin, _)), Some(press)) => {
                let drag = WindowDrag { window, grab: press.to_vec2(), to: origin, start: origin, docks };
                ui.ctx().data_mut(|d| d.insert_temp(self.id.with("window-drag"), drag));
            }
            _ => ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag),
        }
    }

    /// Move the window by `OuterPosition` while the button is held, so the pressed point stays
    /// under the pointer. Escape puts it back; losing focus leaves it. Returns whether the
    /// window is moving.
    fn follow_window_drag(&mut self, ui: &Ui, window: u64) -> bool {
        let key = self.id.with("window-drag");
        let Some(mut drag) = ui.ctx().data(|d| d.get_temp::<WindowDrag>(key)).filter(|d| d.window == window) else { return false };
        let content = ui.input(|i| content_of(i.viewport()));
        let frame = content.map_or(Vec2::ZERO, |(_, frame)| frame);
        // A pointer move of this frame is relative to where the window was when the window
        // system sent it, which is where it reports the window now: a move commanded earlier
        // may not have landed yet. Without one the window stays, since the last position egui
        // keeps (button events included) was measured against an older place.
        let moved = ui.input(|i| i.events.iter().rev().find_map(|e| if let Event::PointerMoved(p) = e { Some(*p) } else { None }));
        let mut step = None;
        if let (Some((origin, _)), Some(local)) = (content, moved.filter(|p| p.is_finite())) {
            let to = origin + (local.to_vec2() - drag.grab);
            if to.distance(drag.to) > 0.0 {
                (drag.to, step) = (to, Some(to));
                ui.ctx().data_mut(|d| d.insert_temp(key, drag));
            }
        }
        // The pointer on the desktop is the pressed point of the window where it was last
        // put, so the origin is that place, less the local pointer.
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
            return true;
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
        true
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
        self.output.window_files.push(WindowFiles { window: window.id, viewport, panel, pos, hovered, dropped });
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
/// window's input. eframe runs `App::raw_input_hook` for the main window only.
#[derive(Default)]
struct NativeInput {
    hooks: std::collections::HashMap<Id, egui::mutex::Mutex<InputHook>>,
    /// The dock area of each native window shown this frame.
    windows: std::collections::HashMap<ViewportId, Id>,
}

pub(in crate::docking) type InputHook = Box<dyn FnMut(&Context, &mut RawInput) + Send>;

impl egui::Plugin for NativeInput {
    fn debug_name(&self) -> &'static str {
        "craft-ui docking native input"
    }

    fn input_hook(&mut self, ctx: &Context, input: &mut RawInput) {
        if let Some(hook) = self.windows.get(&input.viewport_id).and_then(|area| self.hooks.get(area)) {
            (hook.lock())(ctx, input);
        }
    }
}

/// Set or clear the input hook of the dock area `area` for this frame, and forget its native
/// windows until they show again.
pub(in crate::docking) fn set_input_hook(ctx: &Context, area: Id, hook: Option<InputHook>) {
    if hook.is_some() && ctx.plugin_opt::<NativeInput>().is_none() {
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
