//! A frame driver for trials with child viewports: the context opens OS viewports (not embedded), and a renderer
//! like eframe's runs each child's pass nested inside its parent's UI closure, with events and window facts the trial
//! gives it. As an integration does, the window gets what changed in its builder as commands before the pass. The
//! windows share one texture store, as eframe's renderer: each pass's texture changes reach it when the window paints
//! (a child as soon as its pass ends, the root after its run), and a change to a texture the store has not allocated
//! panics, as egui-wgpu's renderer does.

use craft_host::target::ShellInfo;
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::env::RootEnv;
use craft_ui::runtime::frame::{FrameInput, WindowInput, input_pending, set_input};
use craft_ui::testing::Frames;
use craft_ui::testing::access::{AccessTree, tree_of};
use craft_ui::testing::windows::FakeSystem;
use craft_ui::testing::windows::{FullOutput, RawInput};
use egui::{Context, Event, HoveredFile, Pos2, Rect, TextureId, TexturesDelta, Ui, Vec2, ViewportBuilder, ViewportCommand, ViewportId, ViewportInfo};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::sync::Arc;

const ROOT_SIZE: Vec2 = Vec2::new(800.0, 600.0);
const CHILD_SIZE: Vec2 = Vec2::new(400.0, 300.0);

thread_local! {
    /// The events of the running frame's child passes (the renderer runs on the trial's thread), each for the child it
    /// is addressed to, or else for the first child pass.
    static CHILD_EVENTS: RefCell<Vec<(Option<ViewportId>, Event)>> = const { RefCell::new(Vec::new()) };
    /// The files hovering over and dropped on the child's window in the next child pass.
    static CHILD_FILES: RefCell<(Vec<HoveredFile>, Vec<String>)> = const { RefCell::new((Vec::new(), Vec::new())) };
    /// The window facts every child opened from the root reports.
    static CHILD_INFO: RefCell<ViewportInfo> = RefCell::new(ViewportInfo::default());
    /// The window facts of each child opened from another child (a popup window): it opens where its builder puts it
    /// and follows its own commands.
    static OWN_INFO: RefCell<Vec<(ViewportId, ViewportInfo)>> = const { RefCell::new(Vec::new()) };
    /// The window facts the root reports.
    static ROOT_INFO: RefCell<ViewportInfo> = RefCell::new(ViewportInfo::default());
    /// The children whose windows are open: those that ran a pass and are still shown. Like an integration, every pass's
    /// input lists all the windows.
    static CHILDREN: RefCell<Vec<ViewportId>> = const { RefCell::new(Vec::new()) };
    /// The window commands every pass's output carried since the last frame.
    static COMMANDS: RefCell<Vec<(ViewportId, ViewportCommand)>> = const { RefCell::new(Vec::new()) };
    /// The AccessKit tree each viewport's last pass produced, in pass order.
    static TREES: RefCell<Vec<(ViewportId, AccessTree)>> = const { RefCell::new(Vec::new()) };
    /// The shapes each pass of the frame painted, nested shapes flattened.
    static SHAPES: RefCell<Vec<(ViewportId, Vec<egui::Shape>)>> = const { RefCell::new(Vec::new()) };
    /// The time every pass of the frame runs at.
    static NOW: Cell<f64> = const { Cell::new(0.0) };
    /// Each child's builder as its window has it: a later builder patches it, and what changed goes to the window as
    /// commands, as an integration sends them.
    static BUILDERS: RefCell<Vec<(ViewportId, ViewportBuilder)>> = const { RefCell::new(Vec::new()) };
    /// The textures the shared renderer has allocated.
    static TEXTURES: RefCell<HashSet<TextureId>> = RefCell::new(HashSet::new());
}

/// Paints `viewport`'s texture changes into the shared store, in order.
fn paint_textures(viewport: ViewportId, delta: &mut TexturesDelta) {
    TEXTURES.with(|textures| {
        let mut textures = textures.borrow_mut();
        for (id, images) in delta.set.drain() {
            for image in images {
                if image.is_whole() {
                    textures.insert(id);
                } else {
                    assert!(
                        textures.contains(&id),
                        "{viewport:?} updates {id:?}, which the renderer has not allocated yet: an earlier change to it has not reached the renderer"
                    );
                }
            }
        }
        for id in delta.free.drain() {
            textures.remove(&id);
        }
    });
}

/// Seconds between two frames.
const FRAME_SECONDS: f64 = 1.0 / 60.0;

/// Runs `f` on the facts `id` reports: its own for a child opened from another child, else the ones every child
/// opened from the root shares.
fn with_facts<R>(id: ViewportId, f: impl FnOnce(&mut ViewportInfo) -> R) -> R {
    let own = OWN_INFO.with(|own| {
        let mut own = own.borrow_mut();
        own.iter().position(|(held, _)| *held == id).map(|at| own.swap_remove(at))
    });
    match own {
        Some((_, mut info)) => {
            let out = f(&mut info);
            OWN_INFO.with(|own| own.borrow_mut().push((id, info)));
            out
        }
        None => CHILD_INFO.with(|info| f(&mut info.borrow_mut())),
    }
}

/// The window system's answer to a move or a resize: the children's facts follow the commands (the fake window
/// has no decoration, so its inner and outer rects are one). The children opened from the root share one set of facts.
fn os_applies(id: ViewportId, command: &ViewportCommand) {
    if id == ViewportId::ROOT {
        return;
    }
    with_facts(id, |info| {
        let current = info.outer_rect.or(info.inner_rect).unwrap_or_else(|| Rect::from_min_size(Pos2::ZERO, CHILD_SIZE));
        let moved = match command {
            ViewportCommand::OuterPosition(pos) => Rect::from_min_size(*pos, current.size()),
            ViewportCommand::InnerSize(size) => Rect::from_min_size(current.min, *size),
            _ => return,
        };
        info.outer_rect = Some(moved);
        info.inner_rect = Some(moved);
    });
}

fn keep_commands(output: &FullOutput) {
    COMMANDS.with(|kept| {
        let mut kept = kept.borrow_mut();
        for (id, viewport) in &output.viewport_output {
            for command in &viewport.commands {
                os_applies(*id, command);
                kept.push((*id, command.clone()));
            }
        }
    });
}

/// The window facts of every window the integration knows: the root and the children that ran, each with the
/// facts the trial gave it or its own.
fn all_viewports() -> egui::ViewportIdMap<ViewportInfo> {
    let mut all = egui::ViewportIdMap::default();
    all.insert(ViewportId::ROOT, ROOT_INFO.with(|info| info.borrow().clone()));
    CHILDREN.with(|children| {
        for id in children.borrow().iter() {
            all.insert(*id, with_facts(*id, |info| info.clone()));
        }
    });
    all
}

/// Patches the window of `ids.this` with `builder`, as an integration does before the child's pass: the fields that
/// changed go to the window as commands. The first builder opens the window; a window opened from another child opens
/// where the builder puts it, with focus unless the builder opens it inactive.
fn patch_window(ids: egui::ViewportIdPair, builder: ViewportBuilder) {
    let id = ids.this;
    let commands = BUILDERS.with(|kept| {
        let mut kept = kept.borrow_mut();
        match kept.iter_mut().find(|(held, _)| *held == id) {
            Some((_, window)) => window.patch(builder).0,
            None => {
                if ids.parent != ViewportId::ROOT {
                    let rect = Rect::from_min_size(builder.position.unwrap_or(Pos2::ZERO), builder.inner_size.unwrap_or(CHILD_SIZE));
                    let scale = CHILD_INFO.with(|info| info.borrow().native_pixels_per_point);
                    let info = ViewportInfo {
                        focused: Some(builder.active.unwrap_or(true)),
                        inner_rect: Some(rect),
                        outer_rect: Some(rect),
                        native_pixels_per_point: scale,
                        ..ViewportInfo::default()
                    };
                    OWN_INFO.with(|own| own.borrow_mut().push((id, info)));
                }
                kept.push((id, builder));
                Vec::new()
            }
        }
    });
    COMMANDS.with(|kept| {
        for command in commands {
            os_applies(id, &command);
            kept.borrow_mut().push((id, command));
        }
    });
}

/// The window OS files hover over and drop on in [`Windows::frame_with_files`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilesOver {
    /// The root viewport's window.
    Root,
    /// The child viewport's window.
    Child,
}

/// A dropped file that exists only as a path.
#[derive(Debug)]
struct DroppedPath(std::path::PathBuf);

impl egui::DroppedFile for DroppedPath {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|error| error.to_string())
    }

    #[cfg(target_arch = "wasm32")]
    fn bytes_async(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + '_>> {
        Box::pin(std::future::ready(Err("there is no file system on the web".to_owned())))
    }
}

fn render_child(ctx: &Context, viewport: egui::ImmediateViewport<'_>) {
    patch_window(viewport.ids, viewport.builder);
    let mut callback = viewport.viewport_ui_cb;
    let this = viewport.ids.this;
    let events = CHILD_EVENTS.with(|events| {
        let mut events = events.borrow_mut();
        let (own, others): (Vec<_>, Vec<_>) = events.drain(..).partition(|(to, _)| to.is_none_or(|to| to == this));
        *events = others;
        own.into_iter().map(|(_, event)| event).collect()
    });
    let (hovered_files, dropped) = CHILD_FILES.with(|files| std::mem::take(&mut *files.borrow_mut()));
    let dropped_files = dropped.into_iter().map(|name| Arc::new(DroppedPath(name.into())) as egui::DroppedFileHandle).collect();
    let mut info = with_facts(viewport.ids.this, |info| info.clone());
    info.parent = Some(viewport.ids.parent);
    let mut input = RawInput {
        viewport_id: viewport.ids.this,
        events,
        hovered_files,
        dropped_files,
        // The pass lays out in the window as large as its facts say, as the OS gives it.
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, info.inner_rect.map_or(CHILD_SIZE, |inner| inner.size()))),
        ..RawInput::default()
    };
    input.time = Some(NOW.with(Cell::get));
    CHILDREN.with(|children| {
        let mut children = children.borrow_mut();
        if !children.contains(&viewport.ids.this) {
            children.push(viewport.ids.this);
        }
    });
    input.viewports = all_viewports();
    input.viewports.insert(viewport.ids.this, info);
    let mut output = ctx.run_ui(input, |ui| callback(ui));
    paint_textures(viewport.ids.this, &mut output.textures_delta);
    keep_commands(&output);
    keep_tree(&mut output, viewport.ids.this);
}

fn keep_tree(output: &mut FullOutput, viewport: ViewportId) {
    let tree = tree_of(output);
    TREES.with(|kept| kept.borrow_mut().push((viewport, tree)));
    let mut shapes = Vec::new();
    for clipped in &output.shapes {
        flatten(&clipped.shape, &mut shapes);
    }
    SHAPES.with(|kept| kept.borrow_mut().push((viewport, shapes)));
}

fn flatten(shape: &egui::Shape, into: &mut Vec<egui::Shape>) {
    match shape {
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| flatten(shape, into)),
        shape => into.push(shape.clone()),
    }
}

/// An OS window as macOS runs egui's window commands on it through winit, in desktop points with y down: a position
/// moves the frame's top-left corner at its current size (`setFrameOrigin`); a size keeps the frame's bottom-left
/// corner (`setContentSize`), so a change of height moves the top edge; the largest size in force clamps the current
/// size the same way, and every size the window takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppKitWindow {
    /// Where the window is.
    pub rect: Rect,
    max: Option<Vec2>,
}

impl AppKitWindow {
    /// A window at `rect` whose largest size is `max`, the limit the commands of earlier frames left in force.
    pub fn new(rect: Rect, max: Option<Vec2>) -> Self {
        AppKitWindow { rect, max }
    }

    /// Runs `commands` in order.
    pub fn run(&mut self, commands: &[ViewportCommand]) {
        for command in commands {
            match command {
                ViewportCommand::OuterPosition(to) => self.rect = Rect::from_min_size(*to, self.rect.size()),
                ViewportCommand::InnerSize(size) => self.content_size(*size),
                ViewportCommand::MinInnerSize(least) => self.content_size(self.rect.size().max(*least)),
                ViewportCommand::MaxInnerSize(most) => {
                    self.max = Some(*most);
                    self.content_size(self.rect.size());
                }
                _ => {}
            }
        }
    }

    fn content_size(&mut self, size: Vec2) {
        let size = self.max.map_or(size, |max| size.min(max));
        self.rect = Rect::from_min_size(Pos2::new(self.rect.min.x, self.rect.max.y - size.y), size);
    }
}

/// What a frame sent out.
#[non_exhaustive]
pub struct Out {
    /// The window commands of every pass of the frame.
    pub commands: Vec<(ViewportId, ViewportCommand)>,
    /// How each viewport asked to open or change its window.
    pub builders: Vec<(ViewportId, ViewportBuilder)>,
    /// The AccessKit tree each viewport's pass produced, in pass order: the children's first, as they run inside the
    /// root's pass.
    pub trees: Vec<(ViewportId, AccessTree)>,
    /// The shapes each viewport's pass painted, nested shapes flattened, in the order of `trees`.
    pub shapes: Vec<(ViewportId, Vec<egui::Shape>)>,
}

/// A context with the runtime installed whose child viewports are OS windows.
#[non_exhaustive]
pub struct Windows {
    /// The installed context.
    pub ctx: Context,
    root_info: RefCell<ViewportInfo>,
    root: RefCell<RootEnv>,
    app_modal: Cell<Option<ViewportId>>,
    window_input: Cell<WindowInput>,
}

impl Default for Windows {
    fn default() -> Self {
        Self::new()
    }
}

impl Windows {
    /// The running build's shell and a scripted system.
    pub fn new() -> Self {
        Self::with(InstallSpec::new(ShellInfo::default(), Arc::new(FakeSystem::new())))
    }

    /// A context installed with `spec`.
    pub fn with(spec: InstallSpec) -> Self {
        let ctx = Frames::uninstalled().ctx;
        ctx.set_embed_viewports(false);
        assert!(craft_ui::install(&ctx, spec).is_ok(), "the context takes the runtime once");
        Context::set_immediate_viewport_renderer(render_child);
        CHILD_EVENTS.with(|events| events.borrow_mut().clear());
        CHILD_FILES.with(|files| *files.borrow_mut() = Default::default());
        CHILD_INFO.with(|info| *info.borrow_mut() = ViewportInfo::default());
        OWN_INFO.with(|own| own.borrow_mut().clear());
        CHILDREN.with(|children| children.borrow_mut().clear());
        COMMANDS.with(|kept| kept.borrow_mut().clear());
        BUILDERS.with(|kept| kept.borrow_mut().clear());
        TEXTURES.with(|textures| textures.borrow_mut().clear());
        NOW.with(|now| now.set(0.0));
        Windows {
            ctx,
            root_info: RefCell::new(ViewportInfo::default()),
            root: RefCell::new(RootEnv::default()),
            app_modal: Cell::new(None),
            window_input: Cell::new(WindowInput::Live),
        }
    }

    /// The window facts child passes report from now on.
    pub fn child_info(&self, info: ViewportInfo) {
        CHILD_INFO.with(|kept| *kept.borrow_mut() = info);
    }

    /// The window facts `viewport` reports: its own for a child opened from another child, else those
    /// [`child_info`](Self::child_info) gave.
    pub fn facts(&self, viewport: ViewportId) -> ViewportInfo {
        with_facts(viewport, |info| info.clone())
    }

    /// The window in which the app's own egui code shows a modal (`FrameInput::app_modal`), from the next frame on.
    pub fn app_modal(&self, viewport: Option<ViewportId>) {
        self.app_modal.set(viewport);
    }

    /// The window-input policy every following frame's input gives, as a host's `FrameInput::window_input` does.
    pub fn window_input(&self, policy: WindowInput) {
        self.window_input.set(policy);
    }

    /// The window facts the root viewport reports from now on.
    pub fn root_info(&self, info: ViewportInfo) {
        *self.root_info.borrow_mut() = info;
    }

    /// The app's root values every frame gives from now on, the defaults until then.
    pub fn root_env(&self, root: RootEnv) {
        *self.root.borrow_mut() = root;
    }

    /// Gives the next frame's pass of the child `viewport` these `events`, as a popup window's, which runs inside its
    /// opener's pass.
    pub fn send(&self, viewport: ViewportId, events: Vec<Event>) {
        CHILD_EVENTS.with(|kept| kept.borrow_mut().extend(events.into_iter().map(|event| (Some(viewport), event))));
    }

    /// One frame: `root` events go to the root viewport, `child` events to the first child pass, and each other child
    /// gets the events [`send`](Self::send) gave it. Returns what the frame sent out.
    pub fn frame(&self, root: Vec<Event>, child: Vec<Event>, ui: impl FnMut(&mut Ui)) -> Out {
        self.frame_with_files(root, child, FilesOver::Root, Vec::new(), Vec::new(), ui)
    }

    /// [`frame`](Self::frame) where the next pass of the window `over` also gets the OS files hovering over it and the
    /// files dropped on it. The other window gets none, as the OS sends them only to the window under the pointer.
    pub fn frame_with_files(
        &self,
        root: Vec<Event>,
        child: Vec<Event>,
        over: FilesOver,
        hovered: Vec<HoveredFile>,
        dropped: Vec<String>,
        ui: impl FnMut(&mut Ui),
    ) -> Out {
        CHILD_EVENTS.with(|events| events.borrow_mut().extend(child.into_iter().map(|event| (None, event))));
        let ((root_hovered, root_dropped), child_files) = match over {
            FilesOver::Root => ((hovered, dropped), Default::default()),
            FilesOver::Child => (Default::default(), (hovered, dropped)),
        };
        let root_dropped: Vec<egui::DroppedFileHandle> =
            root_dropped.into_iter().map(|name| Arc::new(DroppedPath(name.into())) as egui::DroppedFileHandle).collect();
        CHILD_FILES.with(|files| *files.borrow_mut() = child_files);
        TREES.with(|kept| kept.borrow_mut().clear());
        SHAPES.with(|kept| kept.borrow_mut().clear());
        if !input_pending(&self.ctx) {
            set_input(&self.ctx, FrameInput::new(self.root.borrow().clone()).app_modal(self.app_modal.get()).window_input(self.window_input.get()));
        }
        let mut input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, ROOT_SIZE)),
            events: root,
            hovered_files: root_hovered,
            dropped_files: root_dropped,
            ..RawInput::default()
        };
        input.time = Some(NOW.with(|now| {
            now.set(now.get() + FRAME_SECONDS);
            now.get()
        }));
        ROOT_INFO.with(|info| *info.borrow_mut() = self.root_info.borrow().clone());
        input.viewports = all_viewports();
        let mut output = self.ctx.run_ui(input, ui);
        CHILD_EVENTS.with(|events| events.borrow_mut().clear());
        // A child the frame no longer shows has closed: like an integration, the kit drops its window, so later inputs
        // stop listing it (and what its last pass left, such as a modal, stops counting), and a window opened again
        // opens anew.
        let shown = |id: &ViewportId| output.viewport_output.contains_key(id);
        CHILDREN.with(|children| children.borrow_mut().retain(shown));
        OWN_INFO.with(|own| own.borrow_mut().retain(|(id, _)| shown(id)));
        BUILDERS.with(|kept| kept.borrow_mut().retain(|(id, _)| shown(id)));
        paint_textures(ViewportId::ROOT, &mut output.textures_delta);
        keep_commands(&output);
        keep_tree(&mut output, ViewportId::ROOT);
        let builders = output.viewport_output.iter().map(|(id, viewport)| (*id, viewport.builder.clone())).collect();
        Out {
            commands: COMMANDS.with(|kept| std::mem::take(&mut *kept.borrow_mut())),
            builders,
            trees: TREES.with(|kept| std::mem::take(&mut *kept.borrow_mut())),
            shapes: SHAPES.with(|kept| std::mem::take(&mut *kept.borrow_mut())),
        }
    }
}
