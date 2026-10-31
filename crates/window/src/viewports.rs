//! Show each open window as an egui immediate viewport.

use std::collections::HashMap;

use egui::{Context, Ui, ViewportBuilder, ViewportCommand, ViewportId};

use crate::list::{WindowId, WindowList};
use crate::place::{CASCADE_STEP, Rect, cascade, monitor_of};
use crate::{Frame, decorate};

/// Something that happened to a window this frame. The app decides what follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowEvent {
    /// The window got focus. The driver has already made it the list's active window.
    Focused(WindowId),
    /// The user asked to close the window (its close button, Alt+F4, ⌘W through the desktop).
    ///
    /// A child window stays open until the app closes it in the [`WindowList`], so the app can ask
    /// to save first. For the window in the root viewport, eframe closes the whole app unless the
    /// app sends [`ViewportCommand::CancelClose`] to the root in the same frame: do that before
    /// adopting another window ([`Viewports::adopt`]) or prompting.
    CloseRequested(WindowId),
}

/// What the driver remembers about a window between frames.
#[derive(Default)]
struct Seen {
    focused: bool,
    inner_size: Option<egui::Vec2>,
    monitor_size: Option<egui::Vec2>,
    /// Physical pixels per point: the window reports its rectangle in points of its own scale.
    scale: Option<f32>,
    /// The cascade position chosen when the window was created; repeated every frame so egui sees
    /// an unchanged builder.
    position: Option<egui::Pos2>,
    /// The root viewport's title, sent as a command when it changes.
    title: Option<String>,
}

/// Shows the windows of a [`WindowList`] as egui immediate viewports, one native window each.
///
/// Call [`Viewports::show`] once per frame from the root viewport's UI. The root viewport stays
/// the app's: when the app maps it to a window with [`Viewports::set_root`], that window's content
/// is drawn into the root UI, and every other window gets its own viewport. Close and quit policy
/// stays with the app; the driver reports [`WindowEvent`]s.
pub struct Viewports {
    frame: Frame,
    root: Option<WindowId>,
    step: f32,
    bounds: Option<Rect>,
    monitors: Option<Box<dyn Fn() -> Vec<Rect>>>,
    seen: HashMap<WindowId, Seen>,
}

impl Viewports {
    /// `frame` is the app's frame decision ([`crate::Choice::probe`]), applied to every new
    /// window's builder.
    pub fn new(frame: Frame) -> Self {
        Self { frame, root: None, step: CASCADE_STEP, bounds: None, monitors: None, seen: HashMap::new() }
    }

    /// The cascade offset for new windows, in points. Defaults to [`CASCADE_STEP`].
    pub fn with_step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// The area new windows cascade within, when the app knows the work area. Without it the
    /// driver uses the monitor the previous window is on: from [`Viewports::set_monitors`], or
    /// else guessed from the size egui reports.
    pub fn set_bounds(&mut self, bounds: Option<Rect>) {
        self.bounds = bounds;
    }

    /// How to list the desktop's monitors, in physical pixels: `eframe`'s
    /// `CreationContext::winit_window()` gives them through `available_monitors()`. New windows
    /// then cascade on the real monitor of the window before them, whatever the monitors' sizes,
    /// places and scales; egui reports only a monitor's size, which can't tell any of that.
    /// Asked only when a new window is placed.
    pub fn set_monitors(&mut self, monitors: impl Fn() -> Vec<Rect> + 'static) {
        self.monitors = Some(Box::new(monitors));
    }

    /// The window drawn in the root viewport.
    pub fn root(&self) -> Option<WindowId> {
        self.root
    }

    /// Draw `id` in the root viewport (or nothing, with `None`). The window's own viewport, if it
    /// had one, closes on the next frame.
    pub fn set_root(&mut self, id: Option<WindowId>) {
        if let Some(id) = id {
            // The root is a different native window: forget the child's focus and title.
            self.seen.insert(id, Seen::default());
        }
        self.root = id;
    }

    /// Move window `id` into the root viewport, for when the root's own window closes while
    /// others stay open. The root window takes `id`'s position and size and gets focus; `id`'s
    /// data and [`WindowId`] are unchanged, so its UI state carries over.
    pub fn adopt<T>(&mut self, ctx: &Context, windows: &WindowList<T>, id: WindowId) {
        let inner_size = self.seen.get(&id).and_then(|s| s.inner_size);
        self.set_root(Some(id));
        if let Some(rect) = windows.outer_rect(id) {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::OuterPosition(egui::pos2(rect.x, rect.y)));
        }
        if let Some(size) = inner_size {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::InnerSize(size));
        }
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Focus);
    }

    /// The egui viewport that shows `id`: [`ViewportId::ROOT`] for the root's window.
    pub fn viewport_id(&self, id: WindowId) -> ViewportId {
        if self.root == Some(id) { ViewportId::ROOT } else { child_viewport_id(id) }
    }

    /// The window a viewport shows, e.g. `ctx.viewport_id()` inside a window's UI.
    pub fn window_of(&self, viewport: ViewportId) -> Option<WindowId> {
        if viewport == ViewportId::ROOT {
            return self.root;
        }
        self.seen.keys().copied().find(|&id| self.root != Some(id) && child_viewport_id(id) == viewport)
    }

    /// Bring window `id` to the front.
    pub fn focus(&self, ctx: &Context, id: WindowId) {
        ctx.send_viewport_cmd_to(self.viewport_id(id), ViewportCommand::Focus);
    }

    /// Show every open window for this frame. Call from the root viewport's UI.
    ///
    /// - `builder` gives a window's viewport builder: its title (e.g. from [`crate::WindowMenu`]),
    ///   size and icon. The driver applies the frame decision ([`decorate`]) after it, and places
    ///   a new window by [`cascade`] from the most recently focused window. For the root's window
    ///   only the title is used.
    /// - `content` draws a window into its viewport's top-level [`Ui`]; `ui.ctx()` is that
    ///   viewport's context, so per-viewport input (`close_requested`, IME) and commands apply to
    ///   that window.
    ///
    /// Returns this frame's focus changes and close requests, in drawing order.
    pub fn show<T>(
        &mut self,
        ui: &mut Ui,
        windows: &mut WindowList<T>,
        mut builder: impl FnMut(WindowId, &T) -> ViewportBuilder,
        mut content: impl FnMut(&mut Ui, WindowId, &mut T),
    ) -> Vec<WindowEvent> {
        self.seen.retain(|id, _| windows.contains(*id));
        let ctx = ui.ctx().clone();
        let mut events = Vec::new();

        if let Some(root) = self.root.filter(|&id| windows.contains(id)) {
            if let Some(title) = windows.get(root).and_then(|data| builder(root, data).title) {
                let seen = self.seen.entry(root).or_default();
                if seen.title.as_ref() != Some(&title) {
                    ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Title(title.clone()));
                    seen.title = Some(title);
                }
            }
            if let Some(data) = windows.get_mut(root) {
                content(ui, root, data);
            }
            self.record(Snapshot::read(&ctx), windows, root, &mut events);
        }

        let children: Vec<WindowId> = windows.ids().filter(|&id| self.root != Some(id)).collect();
        for id in children {
            let Some(data) = windows.get(id) else { continue };
            let mut viewport = decorate(builder(id, data), &self.frame);
            let position = match self.seen.get(&id) {
                Some(seen) => seen.position,
                None => self.place(windows, id, &viewport),
            };
            if let Some(position) = position {
                viewport = viewport.with_position(position);
            }
            self.seen.entry(id).or_default().position = position;

            let Some(data) = windows.get_mut(id) else { continue };
            let snapshot = ctx.show_viewport_immediate(child_viewport_id(id), viewport, |child_ui, _class| {
                content(child_ui, id, data);
                Snapshot::read(child_ui.ctx())
            });
            self.record(snapshot, windows, id, &mut events);
        }
        events
    }

    /// Take in window `id`'s viewport state for this frame.
    fn record<T>(&mut self, snapshot: Snapshot, windows: &mut WindowList<T>, id: WindowId, events: &mut Vec<WindowEvent>) {
        let Snapshot { close, focused, outer, inner, monitor, scale } = snapshot;
        windows.set_outer_rect(id, outer.map(Rect::from));
        let seen = self.seen.entry(id).or_default();
        seen.inner_size = inner.map(|r| r.size()).or(seen.inner_size);
        seen.monitor_size = monitor.or(seen.monitor_size);
        seen.scale = Some(scale);
        if focused && !seen.focused {
            windows.focus(id);
            events.push(WindowEvent::Focused(id));
        }
        seen.focused = focused;
        if close {
            events.push(WindowEvent::CloseRequested(id));
        }
    }

    /// Where a new window goes, if the previous window's position is known.
    fn place<T>(&self, windows: &WindowList<T>, id: WindowId, viewport: &ViewportBuilder) -> Option<egui::Pos2> {
        let previous = windows.cascade_origin(Some(id))?;
        let previous_id = windows.recent().find(|&w| w != id && windows.outer_rect(w).is_some());
        // The new window's outer size: its requested inner size plus the previous window's frame.
        let frame = previous_id
            .and_then(|w| self.seen.get(&w))
            .and_then(|s| s.inner_size)
            .map(|inner| egui::vec2(previous.width - inner.x, previous.height - inner.y))
            .unwrap_or_default();
        let size = viewport.inner_size.map(|s| s + frame).unwrap_or(egui::vec2(previous.width, previous.height));
        let seen = previous_id.and_then(|w| self.seen.get(&w));
        let bounds = self
            .bounds
            .or_else(|| self.real_monitor(previous, seen.and_then(|s| s.scale)))
            .or_else(|| Some(monitor_bounds(previous, seen?.monitor_size?)))?;
        let placed = cascade(previous, [size.x, size.y], bounds, self.step);
        Some(egui::pos2(placed.x, placed.y))
    }

    /// The real monitor of the window at `rect`, in that window's points. A window's rectangle is
    /// in points of its own scale, so the monitor is picked in physical pixels.
    fn real_monitor(&self, rect: Rect, scale: Option<f32>) -> Option<Rect> {
        let monitors = self.monitors.as_ref()?();
        let scale = scale.filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0);
        Some(monitor_of(rect.scaled(scale), &monitors)?.scaled(1.0 / scale))
    }
}

/// A viewport's state for this frame, read inside its own pass (egui's input is per viewport).
struct Snapshot {
    close: bool,
    focused: bool,
    outer: Option<egui::Rect>,
    inner: Option<egui::Rect>,
    monitor: Option<egui::Vec2>,
    scale: f32,
}

impl Snapshot {
    fn read(ctx: &Context) -> Self {
        ctx.input(|i| {
            let v = i.viewport();
            Self {
                close: v.close_requested(),
                focused: v.focused.unwrap_or(false),
                outer: v.outer_rect,
                inner: v.inner_rect,
                monitor: v.monitor_size,
                scale: i.pixels_per_point(),
            }
        })
    }
}

/// The viewport id of a window that isn't in the root viewport.
pub fn child_viewport_id(id: WindowId) -> ViewportId {
    ViewportId::from_hash_of(("craft_window", id.get()))
}

/// How far past its monitor's corner a window's frame can reach, in points: on Windows a maximized
/// window's corner is about 8 points outside its monitor.
const FRAME_OVERSHOOT: f32 = 16.0;

/// The monitor to assume when the app doesn't list them ([`Viewports::set_monitors`]). egui reports
/// a monitor's size but not its origin. Assume monitors of that size tile the desktop and take the
/// tile holding the window's top-left corner, moved in by [`FRAME_OVERSHOOT`] so a maximized
/// window's corner doesn't count as the monitor before its own. That is right for a single monitor
/// and for side-by-side monitors of equal size, and only a guess for any other desktop.
fn monitor_bounds(window: Rect, monitor: egui::Vec2) -> Rect {
    let tile = |pos: f32, len: f32| if len > 0.0 { ((pos + FRAME_OVERSHOOT) / len).floor() * len } else { 0.0 };
    Rect::new(tile(window.x, monitor.x), tile(window.y, monitor.y), monitor.x, monitor.y)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use egui::{RawInput, ViewportEvent, ViewportInfo};

    use super::*;
    use crate::Layout;

    thread_local! {
        /// The builders the test backend received, by viewport.
        static BUILDERS: RefCell<HashMap<ViewportId, ViewportBuilder>> = RefCell::new(HashMap::new());
        /// The viewport infos the test backend hands each viewport's pass.
        static INFOS: RefCell<egui::ViewportIdMap<ViewportInfo>> = RefCell::new(egui::ViewportIdMap::default());
    }

    /// A multi-viewport backend like eframe's: each immediate viewport runs a nested pass with its
    /// own input.
    fn backend() -> Context {
        BUILDERS.with(|b| b.borrow_mut().clear());
        INFOS.with(|i| i.borrow_mut().clear());
        let ctx = Context::default();
        ctx.set_embed_viewports(false);
        Context::set_immediate_viewport_renderer(|ctx, viewport| {
            let id = viewport.ids.this;
            BUILDERS.with(|b| b.borrow_mut().insert(id, viewport.builder.clone()));
            let mut input = RawInput { viewport_id: id, ..RawInput::default() };
            input.viewports = INFOS.with(|i| i.borrow().clone());
            input.viewports.entry(id).or_default().parent = Some(viewport.ids.parent);
            let mut cb = viewport.viewport_ui_cb;
            let mut out = ctx.run_ui(input, |ui| cb(ui));
            out.textures_delta.clear(); // not painted
        });
        ctx
    }

    fn info(viewport: ViewportId, edit: impl FnOnce(&mut ViewportInfo)) {
        INFOS.with(|i| edit(i.borrow_mut().entry(viewport).or_default()));
    }

    /// One root frame showing `windows`; returns the events and which windows drew where.
    fn frame(ctx: &Context, driver: &mut Viewports, windows: &mut WindowList<&'static str>) -> (Vec<WindowEvent>, Vec<(WindowId, ViewportId)>) {
        let mut input = RawInput { viewports: INFOS.with(|i| i.borrow().clone()), ..RawInput::default() };
        input.viewports.entry(ViewportId::ROOT).or_default();
        let mut drawn = Vec::new();
        let mut events = Vec::new();
        let mut out = ctx.run_ui(input, |ui| {
            events = driver.show(
                ui,
                windows,
                |_, name| ViewportBuilder::default().with_title(*name).with_inner_size([400.0, 300.0]),
                |ui, id, _| drawn.push((id, ui.ctx().viewport_id())),
            );
        });
        out.textures_delta.clear(); // not painted
        INFOS.with(|i| i.borrow_mut().values_mut().for_each(|v| v.events.clear()));
        (events, drawn)
    }

    #[test]
    fn each_window_gets_its_own_viewport_and_close_is_reported_per_window() {
        let ctx = backend();
        let mut driver = Viewports::new(Frame::App(Layout::default()));
        let mut windows = WindowList::new();
        let a = windows.open("a");
        let b = windows.open("b");

        let (events, drawn) = frame(&ctx, &mut driver, &mut windows);
        assert!(events.is_empty());
        let (va, vb) = (child_viewport_id(a), child_viewport_id(b));
        assert_eq!(drawn, [(a, va), (b, vb)]);
        assert_ne!(va, vb);
        assert_eq!(driver.window_of(vb), Some(b));

        let builders = BUILDERS.with(|b| b.borrow().clone());
        for v in [va, vb] {
            assert_eq!(builders.get(&v).and_then(|b| b.decorations), Some(false), "the app draws the frame");
        }
        assert_eq!(builders.get(&va).and_then(|b| b.title.clone()).as_deref(), Some("a"));

        info(vb, |v| v.events.push(ViewportEvent::Close));
        let (events, drawn) = frame(&ctx, &mut driver, &mut windows);
        assert_eq!(events, [WindowEvent::CloseRequested(b)]);
        assert_eq!(drawn.len(), 2, "the window stays until the app closes it");
    }

    #[test]
    fn focus_is_reported_once_and_makes_the_window_active() {
        let ctx = backend();
        let mut driver = Viewports::new(Frame::Desktop);
        let mut windows = WindowList::new();
        let a = windows.open("a");
        let _b = windows.open("b");
        info(child_viewport_id(a), |v| v.focused = Some(true));
        let (events, _) = frame(&ctx, &mut driver, &mut windows);
        assert_eq!(events, [WindowEvent::Focused(a)]);
        assert_eq!(windows.active(), Some(a));
        let (events, _) = frame(&ctx, &mut driver, &mut windows);
        assert!(events.is_empty(), "still focused: no new event");
    }

    #[test]
    fn the_root_window_draws_in_the_root_viewport() {
        let ctx = backend();
        let mut driver = Viewports::new(Frame::Desktop);
        let mut windows = WindowList::new();
        let a = windows.open("a");
        let b = windows.open("b");
        driver.set_root(Some(a));
        info(ViewportId::ROOT, |v| v.events.push(ViewportEvent::Close));
        let (events, drawn) = frame(&ctx, &mut driver, &mut windows);
        assert_eq!(drawn, [(a, ViewportId::ROOT), (b, child_viewport_id(b))]);
        assert_eq!(events, [WindowEvent::CloseRequested(a)]);

        // The root's window closes; the root adopts b, which loses its own viewport.
        windows.close(a);
        driver.adopt(&ctx, &windows, b);
        let (_, drawn) = frame(&ctx, &mut driver, &mut windows);
        assert_eq!(drawn, [(b, ViewportId::ROOT)]);
        assert_eq!(driver.window_of(ViewportId::ROOT), Some(b));
    }

    #[test]
    fn a_new_window_cascades_from_the_last_one() {
        let ctx = backend();
        let mut driver = Viewports::new(Frame::Desktop);
        let mut windows = WindowList::new();
        let a = windows.open("a");
        let va = child_viewport_id(a);
        info(va, |v| {
            v.outer_rect = Some(egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(400.0, 330.0)));
            v.inner_rect = Some(egui::Rect::from_min_size(egui::pos2(100.0, 80.0), egui::vec2(400.0, 300.0)));
            v.monitor_size = Some(egui::vec2(1000.0, 800.0));
        });
        frame(&ctx, &mut driver, &mut windows);
        let b = windows.open("b");
        frame(&ctx, &mut driver, &mut windows);
        let position = BUILDERS.with(|m| m.borrow().get(&child_viewport_id(b)).and_then(|b| b.position));
        assert_eq!(position, Some(egui::pos2(128.0, 78.0)));
        let position = BUILDERS.with(|m| m.borrow().get(&va).and_then(|b| b.position));
        assert_eq!(position, None, "the first window is placed by the desktop");
    }

    /// Opens a second window from one at `outer` on `monitor_size`, in `scale`, and returns where
    /// the second window goes.
    fn opened_from(outer: egui::Rect, monitor_size: egui::Vec2, scale: f32, driver: &mut Viewports) -> Option<egui::Pos2> {
        let ctx = backend();
        let mut windows = WindowList::new();
        let a = windows.open("a");
        let va = child_viewport_id(a);
        info(va, |v| {
            v.outer_rect = Some(outer);
            v.inner_rect = Some(outer);
            v.monitor_size = Some(monitor_size);
            v.native_pixels_per_point = Some(scale);
        });
        frame(&ctx, driver, &mut windows);
        let b = windows.open("b");
        frame(&ctx, driver, &mut windows);
        BUILDERS.with(|m| m.borrow().get(&child_viewport_id(b)).and_then(|b| b.position))
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h))
    }

    /// A monitor whose place isn't a multiple of its size: no guess from the size can find it.
    #[test]
    fn a_new_window_stays_on_a_monitor_of_another_size_and_place() {
        let mut driver = Viewports::new(Frame::Desktop);
        driver.set_monitors(|| vec![Rect::new(0.0, 0.0, 2560.0, 1440.0), Rect::new(2560.0, 200.0, 1920.0, 1080.0)]);
        // A guess takes 1920–3840 for the monitor, so the step past 3840 would wrap to the first one.
        let place = opened_from(rect(3500.0, 300.0, 400.0, 300.0), egui::vec2(1920.0, 1080.0), 1.0, &mut driver);
        assert_eq!(place, Some(egui::pos2(3528.0, 328.0)));
    }

    /// A monitor below a wider one, offset sideways: the step past its bottom edge wraps to the
    /// monitor's own top, not to a line of the size's multiples.
    #[test]
    fn a_new_window_wraps_at_the_edges_of_the_real_monitor() {
        let mut driver = Viewports::new(Frame::Desktop);
        driver.set_monitors(|| vec![Rect::new(0.0, 0.0, 3440.0, 1440.0), Rect::new(718.0, 1440.0, 1920.0, 1080.0)]);
        let place = opened_from(rect(900.0, 2200.0, 400.0, 300.0), egui::vec2(1920.0, 1080.0), 1.0, &mut driver);
        assert_eq!(place, Some(egui::pos2(928.0, 1440.0)));
    }

    #[test]
    fn a_new_window_beside_a_maximized_one_stays_on_its_monitor() {
        let mut driver = Viewports::new(Frame::Desktop);
        driver.set_monitors(|| vec![Rect::new(0.0, 0.0, 3840.0, 1600.0)]);
        let place = opened_from(rect(-8.0, -8.0, 3856.0, 1576.0), egui::vec2(3840.0, 1600.0), 1.0, &mut driver);
        assert_eq!(place, Some(egui::pos2(20.0, 20.0)));
    }

    /// Most of the window past the right edge: its centre is off the monitor, its monitor isn't.
    #[test]
    fn a_new_window_from_one_hanging_off_the_edge_stays_on_the_monitor() {
        let mut driver = Viewports::new(Frame::Desktop);
        driver.set_monitors(|| vec![Rect::new(0.0, 0.0, 1920.0, 1080.0)]);
        let place = opened_from(rect(1600.0, 100.0, 800.0, 600.0), egui::vec2(1920.0, 1080.0), 1.0, &mut driver);
        assert_eq!(place, Some(egui::pos2(0.0, 128.0)));
    }

    /// A window off every monitor: the new one still opens on one.
    #[test]
    fn a_new_window_from_one_off_screen_opens_on_a_monitor() {
        let mut driver = Viewports::new(Frame::Desktop);
        driver.set_monitors(|| vec![Rect::new(0.0, 0.0, 1920.0, 1080.0)]);
        let place = opened_from(rect(-1920.0, -1080.0, 800.0, 600.0), egui::vec2(1920.0, 1080.0), 1.0, &mut driver);
        assert_eq!(place, Some(egui::pos2(0.0, 0.0)));
    }

    /// At twice the scale, on the second monitor: physically at (3000, 100), and that monitor is
    /// 960–2880 in the window's points. Read as if points were pixels the window is on the first
    /// monitor, and a guess from the size takes 0–1920; either would wrap the new window to 0.
    #[test]
    fn a_new_window_stays_on_a_monitor_of_another_scale() {
        let mut driver = Viewports::new(Frame::Desktop);
        driver.set_monitors(|| vec![Rect::new(0.0, 0.0, 1920.0, 1080.0), Rect::new(1920.0, 0.0, 3840.0, 2160.0)]);
        let place = opened_from(rect(1500.0, 50.0, 400.0, 300.0), egui::vec2(1920.0, 1080.0), 2.0, &mut driver);
        assert_eq!(place, Some(egui::pos2(1528.0, 78.0)));
    }

    /// `set_bounds` is the app's own word: it wins over the monitors.
    #[test]
    fn set_bounds_wins_over_the_monitors() {
        let mut driver = Viewports::new(Frame::Desktop);
        driver.set_monitors(|| vec![Rect::new(0.0, 0.0, 1920.0, 1080.0)]);
        driver.set_bounds(Some(Rect::new(100.0, 100.0, 1000.0, 800.0)));
        let place = opened_from(rect(5.0, 5.0, 400.0, 300.0), egui::vec2(1920.0, 1080.0), 1.0, &mut driver);
        assert_eq!(place, Some(egui::pos2(100.0, 100.0)));
    }

    /// An app that doesn't list its monitors: a maximized window's frame doesn't move the guess
    /// to a monitor that isn't there, and a window past the right edge stays on its monitor.
    #[test]
    fn without_the_monitors_a_maximized_window_still_finds_its_own() {
        let monitor = egui::vec2(1920.0, 1080.0);
        let mut driver = Viewports::new(Frame::Desktop);
        assert_eq!(opened_from(rect(-8.0, -8.0, 1936.0, 1056.0), monitor, 1.0, &mut driver), Some(egui::pos2(20.0, 20.0)));
        let mut driver = Viewports::new(Frame::Desktop);
        assert_eq!(opened_from(rect(1600.0, 100.0, 800.0, 600.0), monitor, 1.0, &mut driver), Some(egui::pos2(0.0, 128.0)));
        assert_eq!(monitor_bounds(Rect::new(-1928.0, -8.0, 1936.0, 1056.0), monitor), Rect::new(-1920.0, 0.0, 1920.0, 1080.0));
    }

    #[test]
    fn monitor_bounds_follow_the_window() {
        let monitor = egui::vec2(1920.0, 1080.0);
        assert_eq!(monitor_bounds(Rect::new(2000.0, 10.0, 1.0, 1.0), monitor), Rect::new(1920.0, 0.0, 1920.0, 1080.0));
        assert_eq!(monitor_bounds(Rect::new(10.0, 10.0, 1.0, 1.0), monitor), Rect::new(0.0, 0.0, 1920.0, 1080.0));
    }
}
