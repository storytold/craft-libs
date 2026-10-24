//! Drive a [`DockArea`] with real egui input in application tests (feature `testing`).
//!
//! ```ignore
//! let mut driver = DockDriver::new(app_layout(), Panels::default());
//! let bar = driver.hit(&Hit::DragBar(PanelId::Layers)).unwrap();
//! driver.drag(bar.center(), egui::pos2(2.0, 300.0));
//! driver.settle();
//! assert!(driver.hit(&Hit::ColumnBar(PanelId::Layers)).is_some());
//! ```

use std::{fmt::Debug, hash::Hash};

use egui::{Context, Event, Id, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, UiBuilder, epaint::ClippedShape, vec2};

use super::{DockArea, DockContent, DockOutput, DockStyle, Hit, Layout, Monitor};

/// Pointer travel that starts a drag before moving to its destination.
const NUDGE: egui::Vec2 = egui::Vec2::new(8.0, 6.0);

type StyleFn = Box<dyn Fn(&mut DockStyle)>;

/// A [`DockArea`] in a headless egui context, with its layout and content. Each frame advances
/// the clock by 1/60 s, so double-clicks register. [`Self::step`] applies what a frame emits,
/// the way an application's command path would.
pub struct DockDriver<P, C> {
    pub ctx: Context,
    pub layout: Layout<P>,
    pub content: C,
    /// The area's id, for [`DockArea::hit_rects`] and the free docking functions.
    pub id: Id,
    /// The window, 800x600 by default.
    pub screen: Rect,
    /// Where the area draws inside the window; the whole window by default.
    pub area: Rect,
    /// Whether the window has focus. Losing it cancels gestures (C2).
    pub focused: bool,
    /// Modifiers held for the following events.
    pub modifiers: Modifiers,
    /// Passed to [`DockArea::auto_collapse_iconic`].
    pub auto_collapse: bool,
    /// Passed to [`DockArea::maximized`].
    pub maximized: Option<P>,
    /// Passed to [`DockArea::raise_panel`] on the next frame only.
    pub raise: Option<P>,
    /// Passed to [`DockArea::float_bounds`].
    pub float_bounds: Option<Rect>,
    /// Passed to [`DockArea::monitors`].
    pub monitors: Vec<Monitor>,
    /// Passed to [`DockArea::locked`].
    pub locked: bool,
    /// The stand-in for OS windows, in use after [`Self::with_native_host`].
    #[cfg(feature = "native-windows")]
    host: Option<std::rc::Rc<std::cell::RefCell<NativeHost>>>,
    #[cfg(feature = "native-windows")]
    no_host: std::cell::RefCell<NativeHost>,
    style: Option<StyleFn>,
    shapes: Vec<ClippedShape>,
    time: f64,
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static, C: DockContent<P>> DockDriver<P, C> {
    /// A driver whose first frames have settled, so every control has its rect.
    pub fn new(layout: Layout<P>, content: C) -> Self {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let mut driver = Self {
            ctx: Context::default(),
            layout,
            content,
            id: Id::new("dock-driver"),
            screen,
            area: screen,
            focused: true,
            modifiers: Modifiers::NONE,
            auto_collapse: true,
            maximized: None,
            raise: None,
            float_bounds: None,
            monitors: Vec::new(),
            locked: false,
            #[cfg(feature = "native-windows")]
            host: None,
            #[cfg(feature = "native-windows")]
            no_host: std::cell::RefCell::new(NativeHost::new(Pos2::ZERO)),
            style: None,
            shapes: Vec::new(),
            time: 0.0,
        };
        driver.settle();
        driver
    }

    /// Adjust the style each frame, after [`DockStyle::from_ui`]. Settles again.
    pub fn with_style(mut self, style: impl Fn(&mut DockStyle) + 'static) -> Self {
        self.style = Some(Box::new(style));
        self.settle();
        self
    }

    /// Draw in `area` of the window. Settles again.
    pub fn with_area(mut self, area: Rect) -> Self {
        self.area = area;
        self.settle();
        self
    }

    /// Render one frame with `events` and return its output. Nothing is applied.
    pub fn frame(&mut self, events: Vec<Event>) -> DockOutput<P> {
        let mut output = DockOutput::default();
        self.time += 1.0 / 60.0;
        #[allow(unused_mut)]
        let mut input = RawInput {
            screen_rect: Some(self.screen),
            events: std::iter::once(Event::ModifiersChanged(self.modifiers)).chain(events).collect(),
            focused: self.focused,
            time: Some(self.time),
            ..Default::default()
        };
        #[cfg(feature = "native-windows")]
        if let Some(host) = &self.host {
            let mut host = host.borrow_mut();
            (host.time, host.screen, host.modifiers) = (self.time, self.screen, self.modifiers);
            host.frames += 1;
            input.viewports = host.infos();
            input.focused = host.focus == Some(egui::ViewportId::ROOT);
        }
        let (layout, content, area, id) = (&self.layout, &mut self.content, self.area, self.id);
        let monitors = self.monitors.clone();
        let (style_fn, maximized, raise, auto, bounds, locked) =
            (&self.style, self.maximized.clone(), self.raise.take(), self.auto_collapse, self.float_bounds, self.locked);
        let mut full = self.ctx.run_ui(input, |ui| {
            let mut style = DockStyle::from_ui(ui);
            if let Some(adjust) = style_fn {
                adjust(&mut style);
            }
            ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
                let mut dock = DockArea::new(id).maximized(maximized.clone()).auto_collapse_iconic(auto).monitors(monitors.clone()).locked(locked);
                if let Some(panel) = raise.clone() {
                    dock = dock.raise_panel(panel);
                }
                if let Some(bounds) = bounds {
                    dock = dock.float_bounds(bounds);
                }
                output = dock.show_content(ui, layout, &style, content);
            });
        });
        full.textures_delta.clear();
        #[cfg(feature = "native-windows")]
        if let Some(host) = &self.host {
            let mut host = host.borrow_mut();
            host.apply(&full);
            host.land();
        }
        self.shapes = full.shapes;
        output
    }

    /// Draw native floating windows in a stand-in for OS windows (feature `native-windows`):
    /// the main window's content sits at `origin` on the desktop, and each native window runs
    /// its own frame on this context with its own input and desktop rect. Settles again.
    #[cfg(feature = "native-windows")]
    pub fn with_native_host(mut self, origin: Pos2) -> Self {
        let host = std::rc::Rc::new(std::cell::RefCell::new(NativeHost::new(origin)));
        let shared = host.clone();
        Context::set_immediate_viewport_renderer(move |ctx, viewport| NativeHost::render(&shared, ctx, viewport));
        self.ctx.set_embed_viewports(false);
        self.host = Some(host);
        self.settle();
        self
    }

    /// The stand-in for OS windows: without [`Self::with_native_host`], one with no windows
    /// that nothing uses.
    #[cfg(feature = "native-windows")]
    pub fn host(&self) -> std::cell::RefMut<'_, NativeHost> {
        self.host.as_deref().unwrap_or(&self.no_host).borrow_mut()
    }

    /// The viewport of the floating window with this [`super::Floating::id`].
    #[cfg(feature = "native-windows")]
    pub fn viewport(&self, window: u64) -> egui::ViewportId {
        DockArea::<P>::window_viewport(self.id, window)
    }

    /// Give `events`, in its own coordinates, to the native window of the floating window
    /// `window` on the next frame, and step.
    #[cfg(feature = "native-windows")]
    pub fn window_step(&mut self, window: u64, events: Vec<Event>) -> DockOutput<P> {
        let viewport = self.viewport(window);
        if let Some(target) = self.host().windows.get_mut(&viewport) {
            target.events.extend(events);
        }
        self.step(Vec::new())
    }

    /// Press and release at `pos` in a native window; the output is the release frame's.
    #[cfg(feature = "native-windows")]
    pub fn window_click(&mut self, window: u64, pos: Pos2) -> DockOutput<P> {
        self.window_step(window, vec![Event::PointerMoved(pos), self.button(pos, true)]);
        self.window_step(window, vec![Event::PointerMoved(pos), self.button(pos, false)])
    }

    /// In a native window, press at `from`, move past the drag threshold and on to `to`, in
    /// its coordinates at each frame, without releasing. The window system keeps sending the
    /// pointer to the window where it was pressed, inside it or not.
    #[cfg(feature = "native-windows")]
    pub fn window_hold(&mut self, window: u64, from: Pos2, to: Pos2) -> DockOutput<P> {
        self.window_step(window, vec![Event::PointerMoved(from), self.button(from, true)]);
        self.window_step(window, vec![Event::PointerMoved(from + NUDGE)]);
        self.window_step(window, vec![Event::PointerMoved(to)])
    }

    /// Release at `pos` in a native window, in its coordinates.
    #[cfg(feature = "native-windows")]
    pub fn window_release(&mut self, window: u64, pos: Pos2) -> DockOutput<P> {
        self.window_step(window, vec![Event::PointerMoved(pos), self.button(pos, false)])
    }

    /// The desktop point `point` in the coordinates of the native window of `window`, as the
    /// host placed it.
    #[cfg(feature = "native-windows")]
    pub fn window_local(&self, window: u64, point: Pos2) -> Option<Pos2> {
        let viewport = self.viewport(window);
        self.host().windows.get(&viewport).map(|w| point - w.rect.min.to_vec2())
    }

    /// Render one frame and apply its actions, all or none, with the content's permissions. A
    /// rejected batch is reported in the output's `error`.
    pub fn step(&mut self, events: Vec<Event>) -> DockOutput<P> {
        let mut output = self.frame(events);
        if output.error.is_none()
            && let Err(error) = self.layout.apply_all(output.actions.clone(), |p| self.content.permissions(p))
        {
            output.error = Some(error);
        }
        output
    }

    /// Empty frames until moved widgets report their rects; egui reports them a frame late.
    pub fn settle(&mut self) {
        for _ in 0..3 {
            self.step(Vec::new());
        }
    }

    pub fn move_to(&mut self, pos: Pos2) -> DockOutput<P> {
        self.step(vec![Event::PointerMoved(pos)])
    }

    pub fn press(&mut self, pos: Pos2) -> DockOutput<P> {
        self.step(vec![Event::PointerMoved(pos), self.button(pos, true)])
    }

    pub fn release(&mut self, pos: Pos2) -> DockOutput<P> {
        self.step(vec![Event::PointerMoved(pos), self.button(pos, false)])
    }

    /// Press and release at `pos`; the output is the release frame's.
    pub fn click(&mut self, pos: Pos2) -> DockOutput<P> {
        self.press(pos);
        self.release(pos)
    }

    /// Press at `from`, move past the drag threshold and on to `to`, without releasing.
    pub fn hold(&mut self, from: Pos2, to: Pos2) -> DockOutput<P> {
        self.press(from);
        self.move_to(from + NUDGE);
        self.move_to(to)
    }

    /// A whole drag; the output is the release frame's.
    pub fn drag(&mut self, from: Pos2, to: Pos2) -> DockOutput<P> {
        self.hold(from, to);
        self.release(to)
    }

    /// Press and release `key` in one frame.
    pub fn key(&mut self, key: Key) -> DockOutput<P> {
        let modifiers = self.modifiers;
        self.step(vec![
            Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
            Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
        ])
    }

    /// Give or take the window's focus for this and the following frames.
    pub fn focus(&mut self, focused: bool) -> DockOutput<P> {
        self.focused = focused;
        self.step(vec![Event::WindowFocused(focused)])
    }

    /// The pointer leaves the window.
    pub fn leave(&mut self) -> DockOutput<P> {
        self.step(vec![Event::PointerGone])
    }

    /// Every gesture source of the last frame.
    pub fn hits(&self) -> Vec<(Hit<P>, Rect)> {
        DockArea::<P>::hit_rects(&self.ctx, self.id)
    }

    /// The rect of one gesture source in the last frame.
    pub fn hit(&self, hit: &Hit<P>) -> Option<Rect> {
        self.hits().into_iter().find(|(h, _)| h == hit).map(|(_, rect)| rect)
    }

    /// The shapes of the last frame.
    pub fn shapes(&self) -> &[ClippedShape] {
        &self.shapes
    }

    /// Whether the last frame filled `rect` (an insertion line or a split preview).
    pub fn filled(&self, rect: Rect) -> bool {
        self.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Rect(shape) => shape.rect == rect && shape.fill != egui::Color32::TRANSPARENT,
            _ => false,
        })
    }

    /// Whether the last frame filled a polygon with these corners, each within half a point.
    pub fn filled_polygon(&self, corners: &[Pos2]) -> bool {
        self.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Path(path) => {
                path.fill != egui::Color32::TRANSPARENT
                    && path.points.len() == corners.len()
                    && path.points.iter().zip(corners).all(|(a, b)| a.distance(*b) <= 0.5)
            }
            _ => false,
        })
    }

    /// Whether the last frame outlined `rect` without filling it.
    pub fn outlined(&self, rect: Rect) -> bool {
        self.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Rect(shape) => shape.rect == rect && shape.fill == egui::Color32::TRANSPARENT && shape.stroke.width > 0.0,
            _ => false,
        })
    }

    fn button(&self, pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: self.modifiers }
    }
}

/// OS windows for [`DockDriver::with_native_host`] (feature `native-windows`).
#[cfg(feature = "native-windows")]
pub struct NativeHost {
    /// Desktop position of the main window's content.
    pub origin: Pos2,
    /// The monitor size egui reports for every window.
    pub monitor_size: Option<egui::Vec2>,
    /// The window with focus: the main window (`ViewportId::ROOT`, the default), a native
    /// window, or `None` while another application is active.
    pub focus: Option<egui::ViewportId>,
    /// Windows report no position, as on Wayland.
    pub no_position: bool,
    /// From a native window's outer position to its content, as a title bar puts it. The
    /// window opens with its outer position where it was asked to, and `OuterPosition` moves
    /// that.
    pub frame: egui::Vec2,
    /// `OuterPosition` lands a frame late, at the end of the frame after the command. That
    /// frame still reports the old place, and gets pointer events relative to it.
    pub lag: bool,
    pub windows: std::collections::HashMap<egui::ViewportId, HostWindow>,
    /// Every command sent to a native window, in order.
    pub commands: Vec<(egui::ViewportId, egui::ViewportCommand)>,
    /// Moves waiting to land under `lag`: window, content position, frame sent.
    late: Vec<(egui::ViewportId, Pos2, usize)>,
    frames: usize,
    /// The driver's modifiers: the keyboard's, whichever window has focus.
    modifiers: Modifiers,
    screen: Rect,
    time: f64,
}

/// One OS window of a [`NativeHost`].
#[cfg(feature = "native-windows")]
#[derive(Debug)]
pub struct HostWindow {
    /// Desktop rect of its content: where it opened, then moved by `OuterPosition` and
    /// `InnerSize`.
    pub rect: Rect,
    pub visible: bool,
    /// The window level its builder asked for on its last frame.
    pub level: Option<egui::WindowLevel>,
    /// Frames it drew.
    pub frames: usize,
    /// Events for its next frame, in its own coordinates.
    pub events: Vec<Event>,
    /// Ask to close it on its next frame, as the OS close button does.
    pub close: bool,
    /// Shapes of its last frame.
    pub shapes: Vec<ClippedShape>,
}

#[cfg(feature = "native-windows")]
impl NativeHost {
    fn new(origin: Pos2) -> Self {
        Self {
            origin,
            monitor_size: None,
            focus: Some(egui::ViewportId::ROOT),
            no_position: false,
            frame: egui::Vec2::ZERO,
            lag: false,
            windows: Default::default(),
            commands: Vec::new(),
            late: Vec::new(),
            frames: 0,
            modifiers: Modifiers::NONE,
            screen: Rect::NOTHING,
            time: 0.0,
        }
    }

    /// What egui knows of every window.
    fn infos(&self) -> egui::ViewportIdMap<egui::ViewportInfo> {
        let info = |id: egui::ViewportId, rect: Rect| egui::ViewportInfo {
            inner_rect: Some(rect),
            outer_rect: (!self.no_position)
                .then(|| if id == egui::ViewportId::ROOT { rect } else { Rect::from_min_max(rect.min - self.frame, rect.max) }),
            focused: Some(self.focus == Some(id)),
            monitor_size: self.monitor_size,
            ..Default::default()
        };
        let main = Rect::from_min_size(self.origin, self.screen.size());
        std::iter::once((egui::ViewportId::ROOT, info(egui::ViewportId::ROOT, main)))
            .chain(self.windows.iter().map(|(id, window)| (*id, info(*id, window.rect))))
            .collect()
    }

    /// Run one native window's frame.
    fn render(host: &std::cell::RefCell<Self>, ctx: &Context, mut viewport: egui::ImmediateViewport<'_>) {
        let id = viewport.ids.this;
        let input = {
            let mut host = host.borrow_mut();
            let builder = &viewport.builder;
            let rect = Rect::from_min_size(builder.position.unwrap_or_default() + host.frame, builder.inner_size.unwrap_or(vec2(300.0, 200.0)));
            let visible = builder.visible.unwrap_or(true);
            let level = builder.window_level;
            host.windows.entry(id).or_insert_with(|| HostWindow {
                rect,
                visible,
                level,
                frames: 0,
                events: Vec::new(),
                close: false,
                shapes: Vec::new(),
            });
            let mut viewports = host.infos();
            let (time, focused, modifiers) = (host.time, host.focus == Some(id), host.modifiers);
            let Some(window) = host.windows.get_mut(&id) else { return };
            window.frames += 1;
            window.level = level;
            if std::mem::take(&mut window.close)
                && let Some(info) = viewports.get_mut(&id)
            {
                info.events.push(egui::ViewportEvent::Close);
            }
            RawInput {
                viewport_id: id,
                viewports,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, window.rect.size())),
                events: std::iter::once(Event::ModifiersChanged(modifiers)).chain(std::mem::take(&mut window.events)).collect(),
                focused,
                time: Some(time),
                ..Default::default()
            }
        };
        let mut full = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
        full.textures_delta.clear();
        let mut host = host.borrow_mut();
        host.apply(&full);
        if let Some(window) = host.windows.get_mut(&id) {
            window.shapes = full.shapes;
        }
    }

    /// End a frame: moves sent in an earlier one land.
    fn land(&mut self) {
        let frames = self.frames;
        let (landed, late) = std::mem::take(&mut self.late).into_iter().partition::<Vec<_>, _>(|(.., sent)| *sent < frames);
        self.late = late;
        for (id, pos, _) in landed {
            if let Some(window) = self.windows.get_mut(&id) {
                window.rect = Rect::from_min_size(pos, window.rect.size());
            }
        }
    }

    /// Carry out the commands a frame sent to native windows.
    fn apply(&mut self, full: &egui::FullOutput) {
        for (id, output) in &full.viewport_output {
            if *id == egui::ViewportId::ROOT {
                continue;
            }
            for command in &output.commands {
                self.commands.push((*id, command.clone()));
                let Some(window) = self.windows.get_mut(id) else { continue };
                match command {
                    egui::ViewportCommand::OuterPosition(pos) if self.lag => self.late.push((*id, *pos + self.frame, self.frames)),
                    egui::ViewportCommand::OuterPosition(pos) => window.rect = Rect::from_min_size(*pos + self.frame, window.rect.size()),
                    egui::ViewportCommand::InnerSize(size) => window.rect = Rect::from_min_size(window.rect.min, *size),
                    egui::ViewportCommand::Visible(visible) => window.visible = *visible,
                    _ => {}
                }
            }
        }
    }
}
