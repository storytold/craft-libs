//! `app::run`, the eframe entry point, and `native_options`: the root window opens where it was and the runtime is
//! installed on eframe's context. Behind the `app` feature because eframe is a heavy, platform-specific dependency.
//!
//! Apps with their own eframe or wgpu setup call `runtime::install` and `runtime::frame::set_input` themselves.

use crate::runtime::InstallSpec;
use crate::runtime::frame::{FrameInput, FrameOutput, request_repaint_after, set_input, take_output};
use crate::runtime::windows::{self, WindowAction, WindowHost};
use craft_behavior::placement::Defaults;
use craft_host::target::{Os, ShellInfo, WindowState};
use craft_window::desktop::fits;
use craft_window::policy::Decision;
use craft_window::restore::Store;
use craft_window::system::{NoWindows, WindowSystem};
use egui::{Context, Ui, ViewportId};
use std::sync::{Arc, Mutex};
use web_time::Instant;

/// What `run` needs of the app.
#[non_exhaustive]
pub struct AppSpec {
    /// The root window's title.
    pub title: String,
    /// The app id: the Wayland app id and the name eframe stores the app under.
    pub app_id: String,
    /// The platform the app runs on.
    pub shell_info: ShellInfo,
    /// The first window's size and the smallest window.
    pub defaults: Defaults,
    /// Where window placement is kept.
    pub placement: Arc<Mutex<Store>>,
    /// The OS services the window host reads: the system pointer and its warp.
    pub window_system: Arc<dyn WindowSystem>,
    /// Who draws the root window's frame (`craft_window::policy::decide_for`). Unset: the OS draws it.
    pub frame: Option<Decision>,
}

impl AppSpec {
    /// A spec for `app_id` titled `title`, with the running build's shell and an in-memory placement store.
    pub fn new(app_id: impl Into<String>, title: impl Into<String>, defaults: Defaults) -> Self {
        AppSpec {
            title: title.into(),
            app_id: app_id.into(),
            shell_info: ShellInfo::default(),
            defaults,
            placement: Arc::new(Mutex::new(Store::in_memory())),
            window_system: Arc::new(NoWindows),
            frame: None,
        }
    }

    /// Sets the placement store, shared by every window of the app.
    pub fn placement(mut self, store: Arc<Mutex<Store>>) -> Self {
        self.placement = store;
        self
    }

    /// Sets the OS services the window host reads.
    pub fn window_system(mut self, window_system: Arc<dyn WindowSystem>) -> Self {
        self.window_system = window_system;
        self
    }

    /// Sets the decision on who draws the root window's frame. The native options decorate the window by it and
    /// the runtime reads the window's chrome from it.
    pub fn frame(mut self, decision: Decision) -> Self {
        self.frame = Some(decision);
        self
    }

    /// Sets the shell info.
    pub fn shell_info(mut self, shell_info: ShellInfo) -> Self {
        self.shell_info = shell_info;
        self
    }
}

/// What the app decides when the root window is asked to close.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CloseDecision {
    /// The window may close; the platform quit takes it from there.
    #[default]
    Close,
    /// The window stays open, brought forward, while the app asks (for example about unsaved documents) and closes
    /// it with `WindowAction::Close` once the answer allows.
    Hold,
}

/// The app `run` drives.
pub trait ShellApp {
    /// What the runtime reads before each pass.
    fn frame_input(&mut self) -> FrameInput;

    /// Shows the root window.
    fn ui(&mut self, ui: &mut Ui);

    /// Runs before each frame, also while the window is minimized or hidden and `ui` does not run.
    /// `WindowHost::visible` tells which.
    fn logic(&mut self, _ctx: &Context) {}

    /// Runs each frame with the clock's reading, which layer 8 cannot take itself. Returns when the app next needs a
    /// frame without input (a pending save, a retry), or `None`. The driver asks for a frame at that instant.
    fn tick(&mut self, _now: Instant) -> Option<Instant> {
        None
    }

    /// Decides what a close request of the root window does. Runs in `logic`, so it also runs for a hidden window.
    fn close_requested(&mut self) -> CloseDecision {
        CloseDecision::Close
    }

    /// Receives what the runtime accumulated since the last frame: fired commands, announcements, the chrome toggle.
    fn output(&mut self, _output: FrameOutput) {}
}

/// eframe's `App` over a `ShellApp`: inputs go in before the pass and outputs come out at the start of the next
/// frame. `logic` carries the close path, which must run while the window is hidden.
struct Driver<A> {
    app: A,
    /// The `movable` flag last set on the native window.
    movable: Option<bool>,
    /// Whether the first fit of the root window to its monitor has run.
    fitted: bool,
}

/// Maximizes the root window, once its monitor is known, when it is larger than the monitor's usable area (Photoshop
/// opens maximized on a small display). Never on macOS, which keeps new windows inside the screen's visible frame
/// itself, and never for a window that is already maximized or in full screen. Returns whether the monitor was
/// known, which is when the fit has run.
fn fit_first(ctx: &Context) -> bool {
    let host = WindowHost::of(ctx);
    let monitor = ctx.input(|input| input.viewport().monitor_size);
    let (Some(facts), Some(monitor)) = (host.facts(ViewportId::ROOT), monitor) else { return false };
    let outer = facts.outer.unwrap_or(facts.inner);
    if windows::os_of(ctx) != Os::Mac
        && facts.state == WindowState::Normal
        && monitor.is_finite()
        && outer.is_finite()
        && !fits(outer.size(), monitor)
    {
        host.act(ViewportId::ROOT, WindowAction::SetMaximized(true));
    }
    true
}

impl<A: ShellApp> eframe::App for Driver<A> {
    fn raw_input_hook(&mut self, ctx: &Context, _: &mut crate::runtime::egui::input::RawInput) {
        let output = take_output(ctx);
        self.app.output(output);
        set_input(ctx, self.app.frame_input());
    }

    fn logic(&mut self, ctx: &Context, _: &mut eframe::Frame) {
        self.app.logic(ctx);
        let now = crate::runtime::motion::now();
        if let Some(due) = self.app.tick(now) {
            request_repaint_after(ctx, due.saturating_duration_since(now).as_secs_f64());
        }
        if windows::root_close_requested(ctx) && self.app.close_requested() == CloseDecision::Hold {
            windows::hold_close(ctx);
        }
        windows::logic(ctx);
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        #[cfg(windows)]
        windows::route_root(ui.ctx(), || craft_window::native::NativeWindow::of_handle(&*frame));
        #[cfg(not(windows))]
        let _ = frame;
        // The monitor is known only once the window exists, so the fit runs from the first passes, and once.
        if !self.fitted {
            self.fitted = fit_first(ui.ctx());
        }
        self.app.ui(ui);
        super::title_bar::sync_movable(ui.ctx(), &mut self.movable);
    }
}

/// The native options for the root window: its saved placement, else the defaults.
#[cfg(not(target_arch = "wasm32"))]
pub fn native_options(spec: &AppSpec) -> eframe::NativeOptions {
    use craft_behavior::placement::place;
    use craft_window::policy::apply;
    use craft_window::restore::MAIN;
    use egui::{Pos2, ViewportBuilder};
    let saved = spec.placement.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(MAIN);
    // The monitors are not known before the window exists, so the saved geometry is kept as it is.
    let placed = place(saved.as_ref(), &[], None, &spec.defaults);
    let scale = placed.scale.max(f32::EPSILON);
    let mut viewport = ViewportBuilder::default()
        .with_title(spec.title.clone())
        .with_app_id(spec.app_id.clone())
        .with_inner_size(placed.normal.size)
        .with_min_inner_size(spec.defaults.min)
        .with_maximized(placed.maximized)
        .with_fullscreen(placed.fullscreen);
    if let Some([x, y]) = placed.position_px {
        viewport = viewport.with_position(Pos2::new(x as f32 / scale, y as f32 / scale));
    }
    if let Some(decision) = &spec.frame {
        viewport = apply(viewport, &decision.frame);
    }
    eframe::NativeOptions { viewport, ..eframe::NativeOptions::default() }
}

/// Installs the runtime on eframe's context, then builds the app with `make`. An install error (two style
/// replacements for one style key) ends the start with it.
fn create<A: ShellApp + 'static>(
    spec: AppSpec,
    make: impl FnOnce(&Context) -> A + 'static,
) -> impl FnOnce(&eframe::CreationContext<'_>) -> Result<Box<dyn eframe::App>, Box<dyn std::error::Error + Send + Sync>> {
    move |cc| {
        let mut install = InstallSpec::new(spec.shell_info, spec.window_system).placement(spec.placement);
        if let Some(decision) = spec.frame {
            install = install.frame(decision);
        }
        crate::install(&cc.egui_ctx, install).map_err(|error| format!("the runtime could not be installed: {error}"))?;
        // The suite's interface face; an app that installs its own fonts in `make` replaces it.
        crate::fonts::UiFonts::new(&crate::fonts::I18nScripts).install(&cc.egui_ctx);
        Ok(Box::new(Driver { app: make(&cc.egui_ctx), movable: None, fitted: false }))
    }
}

/// Opens the root window and runs `make`'s app until it closes. The runtime is installed on eframe's context
/// before `make` runs; an install error ends the run with it.
#[cfg(not(target_arch = "wasm32"))]
pub fn run<A: ShellApp + 'static>(spec: AppSpec, make: impl FnOnce(&Context) -> A + 'static) -> eframe::Result {
    let options = native_options(&spec);
    let title = spec.app_id.clone();
    eframe::run_native(&title, options, Box::new(create(spec, make)))
}

/// Runs `make`'s app on `canvas`. The runtime is installed on eframe's context before `make` runs. The returned
/// runner owns the running app; dropping it does not stop the app, `destroy` does.
#[cfg(target_arch = "wasm32")]
pub async fn start_web<A: ShellApp + 'static>(
    canvas: eframe::web_sys::HtmlCanvasElement,
    spec: AppSpec,
    make: impl FnOnce(&Context) -> A + 'static,
) -> Result<eframe::WebRunner, eframe::wasm_bindgen::JsValue> {
    let runner = eframe::WebRunner::new();
    runner.start(canvas, eframe::WebOptions::default(), Box::new(create(spec, make))).await?;
    Ok(runner)
}
