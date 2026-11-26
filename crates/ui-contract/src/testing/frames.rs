//! A fixed-rate frame driver over a context.

use crate::runtime::commands::CommandInput;
use crate::runtime::egui::access::AccessTree;
use crate::runtime::egui::{areas, fixture, plugin};
use crate::runtime::env::RootEnv;
use crate::runtime::frame::{FrameInput, announce, set_input};
use craft_host::Host;
use egui::{Context, Event, Ui, Vec2};
use std::sync::Arc;
use std::time::Duration;

const DT: f64 = 1.0 / 60.0;

/// One context, advanced frame by frame at 60 Hz.
#[non_exhaustive]
pub struct Frames {
    /// The context the frames run on.
    pub ctx: Context,
    time: f64,
    /// Whether a frame leaves the clock where it is: only [`Frames::advance`] moves it on.
    still: bool,
    /// The time of the last frame these frames ran, to tell when another driver of the context has run it since.
    given: f64,
    root: RootEnv,
    cursor: egui::CursorIcon,
    cursor_image: Option<egui::CustomCursorImage>,
    repaint_delay: Duration,
    host: Option<Arc<Host>>,
    commands: CommandInput,
    /// The modifiers the app holds down for the user in every following frame.
    latched: craft_keymap::Mods,
    tree: Option<AccessTree>,
    ime: Option<fixture::ImeSeen>,
    copied: Vec<String>,
    opened: Vec<(String, bool)>,
    texts: Vec<fixture::PaintedText>,
    bounds: Vec<egui::Rect>,
    overshoot: Vec<f32>,
    modifiers: egui::Modifiers,
    /// The modifiers the context was last told about.
    told: egui::Modifiers,
    /// The OS files hovering over the window in every following frame.
    files: fixture::OsFiles,
    /// The size of the egui screen rect: the window every press has to land in.
    screen: Vec2,
    /// The window's pixels per point; egui's default when `None`.
    pixels_per_point: Option<f32>,
    /// What the frames painted since the last `take_paint`, when they are recorded.
    paint: Option<Recorded>,
}

impl Default for Frames {
    fn default() -> Self {
        Self::new()
    }
}

/// What recorded frames painted since the last `take_paint`. What nobody took when the frames drop goes unrendered,
/// its texture changes with it.
#[derive(Default)]
struct Recorded(fixture::FullOutput);

impl Drop for Recorded {
    fn drop(&mut self) {
        self.0.textures_delta.clear();
    }
}

impl Frames {
    /// The size of the egui screen rect unless `screen` sets another.
    const SCREEN: Vec2 = Vec2::new(800.0, 600.0);

    /// The size of the egui screen rect in every following frame: `room` where it is larger than the default
    /// 800 by 600, so a press anywhere in the room lands inside the window.
    pub fn screen(&mut self, room: Vec2) {
        self.screen = room.max(Self::SCREEN);
    }

    /// A fresh context with the runtime installed.
    pub fn new() -> Self {
        let mut frames = Self::uninstalled();
        frames.install();
        frames
    }

    /// The window's pixels per point in every following frame.
    pub fn pixels_per_point(&mut self, pixels_per_point: f32) {
        self.pixels_per_point = Some(pixels_per_point);
    }

    /// Keeps what every following frame paints, for [`Frames::take_paint`].
    pub fn record_paint(&mut self) {
        self.paint.get_or_insert_with(Recorded::default);
    }

    /// What the frames painted since the last call, as a renderer takes it: the last frame's shapes and every
    /// texture change since. `None` unless [`Frames::record_paint`] was called.
    pub fn take_paint(&mut self) -> Option<fixture::FullOutput> {
        self.paint.as_mut().map(|paint| std::mem::take(&mut paint.0))
    }

    /// Frames on `ctx`, a context the caller made (an app's own, which its code reads outside the frames), with the
    /// runtime installed on it unless it already is, as an app installs it: without the stories' command sets. The
    /// context keeps its own fonts and options.
    pub fn over(ctx: Context) -> Self {
        let frames = Self::on(ctx);
        if !plugin::is_registered(&frames.ctx) {
            let installed = crate::install(&frames.ctx, crate::runtime::InstallSpec { stories: false, ..super::install_spec() });
            debug_assert!(installed.is_ok(), "the context takes the runtime once");
        }
        frames
    }

    /// A fresh context without the runtime, for frames that ran before `install`.
    pub fn uninstalled() -> Self {
        Self::on(fixture::bare_context())
    }

    fn on(ctx: Context) -> Self {
        Frames {
            ctx,
            time: 0.0,
            still: false,
            given: 0.0,
            root: RootEnv::default(),
            cursor: egui::CursorIcon::Default,
            cursor_image: None,
            repaint_delay: Duration::MAX,
            host: None,
            commands: CommandInput::new(),
            latched: craft_keymap::Mods::NONE,
            tree: None,
            ime: None,
            copied: Vec::new(),
            opened: Vec::new(),
            texts: Vec::new(),
            bounds: Vec::new(),
            overshoot: Vec::new(),
            modifiers: egui::Modifiers::NONE,
            told: egui::Modifiers::NONE,
            files: fixture::OsFiles::default(),
            screen: Self::SCREEN,
            pixels_per_point: None,
            paint: None,
        }
    }

    /// Installs the runtime on the context.
    pub fn install(&mut self) {
        let installed = crate::install(&self.ctx, super::install_spec());
        debug_assert!(installed.is_ok(), "the context takes the runtime once");
    }

    /// The host every following frame is given.
    pub fn host(&mut self, host: Arc<Host>) {
        self.host = Some(host);
    }

    fn input(&self) -> FrameInput {
        let input = FrameInput::new(self.root.clone()).commands(self.commands.clone()).latched(self.latched);
        match &self.host {
            Some(host) => input.host(host.clone()),
            None => input,
        }
    }

    /// The root values every following frame is given.
    pub fn root(&mut self, root: RootEnv) {
        self.root = root;
    }

    /// The modifiers the app holds down for the user in every following frame.
    pub fn latched(&mut self, mods: craft_keymap::Mods) {
        self.latched = mods;
    }

    /// The command input every following frame is given.
    pub fn commands(&mut self, commands: CommandInput) {
        self.commands = commands;
    }

    /// One frame with `events`; `ui` runs once per pass. Returns the number of passes.
    pub fn frame(&mut self, events: Vec<Event>, ui: impl FnMut(&mut Ui)) -> usize {
        self.frame_in_viewport(egui::ViewportId::ROOT, events, ui)
    }

    /// One frame of `viewport`, a child window of the root while the root stays live, with `events`; `ui` runs once
    /// per pass. Returns the number of passes. The child opens with its first frame and stays open, listed in the root's
    /// frames too, until [`close_viewport`](Self::close_viewport) or a `ViewportCommand::Close` for it.
    pub fn frame_in_viewport(&mut self, viewport: egui::ViewportId, events: Vec<Event>, mut ui: impl FnMut(&mut Ui)) -> usize {
        // The app's input is the root's, once per app frame: a child window's frame runs inside it.
        if viewport == egui::ViewportId::ROOT {
            set_input(&self.ctx, self.input());
        }
        let mut passes = 0;
        self.run(viewport, events, |root| {
            passes += 1;
            ui(root);
        });
        passes
    }

    /// One frame with `events`, as [`frame`](Self::frame); returns whether the frame sent a window a close command.
    pub fn frame_closing(&mut self, events: Vec<Event>, ui: impl FnMut(&mut Ui)) -> bool {
        set_input(&self.ctx, self.input());
        self.run(egui::ViewportId::ROOT, events, ui)
    }

    /// One frame with no `set_input`, as an app that never calls it would drive it: the root values and the
    /// interface scale stay as they were. Returns whether the frame sent a close command.
    pub fn frame_without_input(&mut self, events: Vec<Event>, ui: impl FnMut(&mut Ui)) -> bool {
        self.run(egui::ViewportId::ROOT, events, ui)
    }

    /// One frame with no events; returns the shapes it painted, nested shapes flattened.
    pub fn frame_shapes(&mut self, mut ui: impl FnMut(&mut Ui)) -> Vec<egui::Shape> {
        set_input(&self.ctx, self.input());
        self.tick();
        fixture::run_frame_shapes(&self.ctx, self.time, self.screen, &mut ui)
    }

    /// The modifiers held in every following frame.
    pub fn hold(&mut self, modifiers: egui::Modifiers) {
        self.modifiers = modifiers;
    }

    /// The modifiers [`Frames::hold`] keeps.
    pub(crate) fn held(&self) -> egui::Modifiers {
        self.modifiers
    }

    /// Files named `names` are dragged over the window from the OS, in this frame and every following one until they
    /// drop or leave.
    pub fn hover_files(&mut self, names: &[&str]) {
        self.files.hovered = names.iter().map(|name| ((*name).to_owned(), String::new())).collect();
    }

    /// Like `hover_files`, with the media type the OS reports for each file while it hovers.
    pub fn hover_typed_files(&mut self, files: &[(&str, &str)]) {
        self.files.hovered = files.iter().map(|(name, mime)| ((*name).to_owned(), (*mime).to_owned())).collect();
    }

    /// The hovering files leave the window without a drop.
    pub fn leave_files(&mut self) {
        self.files.hovered.clear();
    }

    /// The hovering files drop on the window in the next frame; `names` are what the OS reports as dropped.
    pub fn drop_files(&mut self, names: &[&str]) {
        self.files.hovered.clear();
        self.files.dropped = names.iter().map(|name| (*name).to_owned()).collect();
    }

    /// The hovering files drop on the window in the next frame as a web page drops them, with neither path nor bytes:
    /// the page reads the file `name` asynchronously, and its `bytes` arrive after that frame. The fixture opens the read
    /// in the drop frame and posts the file into the file inbox after it, as the wasm read does; the pass after it
    /// delivers the drop once.
    pub fn read_dropped(&mut self, name: &str, bytes: &[u8]) {
        self.files.hovered.clear();
        self.files.read.push((name.to_owned(), bytes.to_vec()));
    }

    /// The hovering files drop on the window in the next frame as a browser on the desktop hands a file over: by the
    /// relative name `name`, which locates no file, with its `bytes` at hand.
    pub fn drop_with_bytes(&mut self, name: &str, bytes: &[u8]) {
        self.files.hovered.clear();
        self.files.given.push((name.to_owned(), bytes.to_vec()));
    }

    /// Keeps the clock still: a frame no longer moves it on, so time passes only where a test calls
    /// [`advance`](Self::advance). Held-pointer repeats, hover delays and animations then wait for the test.
    pub fn still_clock(&mut self) {
        self.still = true;
    }

    /// Moves the clock on by `seconds` before the next frame.
    pub fn advance(&mut self, seconds: f64) {
        self.time += seconds;
    }

    /// Closes the child window `viewport`, as its user would: later frames no longer list it.
    pub fn close_viewport(&mut self, viewport: egui::ViewportId) {
        fixture::close_viewport(&self.ctx, viewport);
    }

    /// `n` frames with no events.
    pub fn idle(&mut self, n: usize, mut ui: impl FnMut(&mut Ui)) {
        for _ in 0..n {
            self.frame(Vec::new(), &mut ui);
        }
    }

    /// Moves the clock on one frame. When another driver of the context (an app's own harness) has run it past the
    /// last frame these frames gave it, the clock first catches up with the context's, so it never takes its time
    /// back; otherwise it keeps its own time, [`advance`](Self::advance) steps included.
    fn tick(&mut self) {
        let context = self.ctx.input(|i| i.time);
        if context > self.given {
            self.time = self.time.max(context);
        }
        if !self.still {
            self.time += DT;
        }
        self.given = self.time;
    }

    fn run(&mut self, viewport: egui::ViewportId, mut events: Vec<Event>, ui: impl FnMut(&mut Ui)) -> bool {
        self.tick();
        if self.told != self.modifiers {
            self.told = self.modifiers;
            events.insert(0, fixture::modifiers_changed(self.modifiers));
        }
        // A real pointer button event carries the modifiers held as it happens, and a new press reads its event's
        // modifiers (F-144): a generated press or release that names none (`pointer::press`) carries the held ones.
        if self.modifiers != egui::Modifiers::NONE {
            for event in &mut events {
                if let Event::PointerButton { modifiers, .. } = event
                    && *modifiers == egui::Modifiers::NONE
                {
                    *modifiers = self.modifiers;
                }
            }
        }
        let files = fixture::OsFiles {
            hovered: self.files.hovered.clone(),
            dropped: std::mem::take(&mut self.files.dropped),
            read: std::mem::take(&mut self.files.read),
            given: std::mem::take(&mut self.files.given),
        };
        let mut result = fixture::run_frame(&self.ctx, self.time, self.screen, self.pixels_per_point, viewport, events, files, ui);
        match &mut self.paint {
            Some(Recorded(paint)) => {
                paint.textures_delta.append(std::mem::take(&mut result.textures));
                paint.shapes = std::mem::take(&mut result.shapes);
                paint.pixels_per_point = result.pixels_per_point;
            }
            None => result.textures.clear(),
        }
        self.cursor = result.cursor;
        self.cursor_image = result.cursor_image;
        self.repaint_delay = result.repaint_delay;
        self.tree = result.tree;
        self.ime = result.ime;
        self.copied = result.copied;
        self.opened.extend(result.opened);
        self.texts = result.texts;
        self.bounds = result.bounds;
        self.overshoot = result.overshoot;
        result.closed
    }

    /// The IME output of the last frame: `None` when it wrote none.
    pub fn ime(&self) -> Option<&fixture::ImeSeen> {
        self.ime.as_ref()
    }

    /// The texts the last frame put on the clipboard.
    pub fn copied(&self) -> &[String] {
        &self.copied
    }

    /// Every URL the frames so far asked to open, in order, each with whether it asked for a new tab.
    pub fn opened(&self) -> &[(String, bool)] {
        &self.opened
    }

    /// The text runs the last frame painted.
    pub fn texts(&self) -> &[fixture::PaintedText] {
        &self.texts
    }

    /// The visual bounds of the shapes the last frame painted.
    pub fn shape_bounds(&self) -> &[egui::Rect] {
        &self.bounds
    }

    /// Half the stroke width of the shape behind each of `shape_bounds`.
    pub fn shape_overshoot(&self) -> &[f32] {
        &self.overshoot
    }

    /// The AccessKit tree the last frame produced.
    pub fn tree(&mut self) -> AccessTree {
        self.tree.take().unwrap_or_else(AccessTree::none)
    }

    /// The wait the last frame asked for before the next pass: `Duration::ZERO` for an immediate repaint,
    /// `None` when it asked for none.
    pub fn repaint_delay(&self) -> Option<Duration> {
        (self.repaint_delay != Duration::MAX).then_some(self.repaint_delay)
    }

    /// Whether anything asked for a pass since the last frame ended.
    pub fn repaint_requested(&self) -> bool {
        fixture::repaint_requested(&self.ctx)
    }

    /// The system cursor the last frame's output asked for.
    pub fn cursor_icon(&self) -> egui::CursorIcon {
        self.cursor
    }

    /// The cursor image the last frame's output asked for, which the integration shows instead of the system cursor.
    pub fn cursor_image(&self) -> Option<&egui::CustomCursorImage> {
        self.cursor_image.as_ref()
    }
}

/// Adds `text` to the next frame output's announcements, as the runtime's access service does.
pub fn announce_text(ctx: &Context, text: &str) {
    announce(ctx, text.to_owned());
}

/// Shows an egui `Area` and runs `body` on its `Ui`.
pub fn show_area<R>(ctx: &Context, name: &'static str, body: impl FnOnce(&mut Ui) -> R) -> R {
    areas::show_area(ctx, name, body)
}

/// The width of the widest layer above the background, 0 when there is none.
pub fn widest_layer(ctx: &Context) -> f32 {
    areas::widest_layer(ctx)
}
