//! `runtime::install`: adds the runtime plugin once and sets the egui options the runtime owns.

use crate::runtime::egui::{clipboard, input, plugin};
use crate::runtime::env::{self, SystemReader, system};
use crate::runtime::pen::PenSource;
use crate::runtime::{commands, frame, pen, windows};
use crate::style::{DuplicateStyle, StyleReplacements};
use craft_host::Jobs;
use craft_host::target::ShellInfo;
use craft_window::policy::Decision;
use craft_window::restore::Store;
use craft_window::system::WindowSystem;
use egui::Context;

use std::sync::{Arc, Mutex};

/// What `install` takes.
#[non_exhaustive]
pub struct InstallSpec {
    /// The platform and what it offers; `runtime::windows` owns it from install on.
    pub shell_info: ShellInfo,
    /// The OS services the window host reads in `set_input`.
    pub window_system: Arc<dyn WindowSystem>,
    /// Where viewport mounts keep their window placement; none keeps placement in memory only.
    pub placement: Option<Arc<Mutex<Store>>>,
    /// The end hook's ghost-layer painter, called once per pass. The runtime names no contract type, so it
    /// holds an erased function; `craft_ui::install` fills it with the overlay host's painter.
    pub frame_layers: Option<fn(&Context)>,
    /// The app's style replacements, frozen at install.
    pub looks: StyleReplacements,
    /// The frame decided for the root window; `runtime::windows` takes its chrome and captions and keeps the
    /// caption layout current on focus gain.
    pub frame: Option<Decision>,
    /// Where the root's `System` choices read the OS's appearance. `new` gives a reader with no desktop watcher,
    /// which answers only the colour scheme egui's integration reports; a host passes one it started.
    pub system: SystemReader,
    /// The modifiers the wheel zooms and scrolls sideways with, written to egui's input options at install.
    pub wheel: WheelModifiers,
    /// The app id every OS window carries (dock icon, Wayland compositor rules); empty carries none.
    pub app_id: String,
    /// The tablet readings the pointer feed pairs with each pointer move; none reports no pen.
    pub pen: Option<PenSource>,
    /// Whether the stories' command sets (`#[command(story)]`) load: the gallery and the story fixtures show
    /// stories, an app does not, so a story's keys never reach an app's keymap.
    pub stories: bool,
    /// The clipboard the host installs (system, browser event, memory); without one the runtime uses
    /// `Clipboard::new(EguiClipboard)`.
    pub clipboard: Option<craft_platform::clipboard::Clipboard>,
    /// Where the context's background work (document faces, texture loads) runs while no host is installed; none
    /// runs it on `craft_host::default_jobs()`. A host's jobs come first.
    pub jobs: Option<Arc<dyn Jobs>>,
}

/// The modifiers the mouse wheel zooms and scrolls sideways with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WheelModifiers {
    /// Held with the wheel, the wheel zooms.
    pub zoom: egui::Modifiers,
    /// Held with the wheel, a vertical wheel scrolls sideways.
    pub sideways: egui::Modifiers,
}

impl WheelModifiers {
    /// The wheel zooms with `zoom` held and scrolls sideways with `sideways` held.
    pub const fn new(zoom: egui::Modifiers, sideways: egui::Modifiers) -> Self {
        Self { zoom, sideways }
    }
}

impl Default for WheelModifiers {
    fn default() -> Self {
        WheelModifiers { zoom: egui::Modifiers::COMMAND, sideways: egui::Modifiers::SHIFT }
    }
}

impl InstallSpec {
    /// A spec for `shell_info` and `window_system`, with no ghost-layer painter.
    pub fn new(shell_info: ShellInfo, window_system: Arc<dyn WindowSystem>) -> Self {
        InstallSpec {
            shell_info,
            window_system,
            placement: None,
            frame_layers: None,
            looks: StyleReplacements::new(),
            frame: None,
            system: SystemReader::unwatched(),
            wheel: WheelModifiers::default(),
            app_id: String::new(),
            pen: None,
            stories: false,
            clipboard: None,
            jobs: None,
        }
    }

    /// Runs the context's background work on `jobs` while no host is installed.
    pub fn jobs(mut self, jobs: Arc<dyn Jobs>) -> Self {
        self.jobs = Some(jobs);
        self
    }

    /// Installs the host's clipboard (system, browser event, memory) in place of the runtime's own.
    pub fn clipboard(mut self, clipboard: craft_platform::clipboard::Clipboard) -> Self {
        self.clipboard = Some(clipboard);
        self
    }

    /// Loads the stories' command sets too, for an install that shows stories.
    pub fn stories(mut self) -> Self {
        self.stories = true;
        self
    }

    /// Sets the store viewport mounts save their window placement in.
    pub fn placement(mut self, store: Arc<Mutex<Store>>) -> Self {
        self.placement = Some(store);
        self
    }

    /// Sets the app's style replacements.
    pub fn looks(mut self, looks: impl Into<StyleReplacements>) -> Self {
        self.looks = looks.into();
        self
    }

    /// Sets the reader of the OS's appearance; the host starts it (`SystemReader::start`) with a wake that repaints.
    pub fn system(mut self, reader: SystemReader) -> Self {
        self.system = reader;
        self
    }

    /// Sets the modifiers the wheel zooms and scrolls sideways with.
    pub fn wheel(mut self, modifiers: WheelModifiers) -> Self {
        self.wheel = modifiers;
        self
    }

    /// Sets the root window's decided frame (`craft_window::policy::decide_for`).
    pub fn frame(mut self, decision: Decision) -> Self {
        self.frame = Some(decision);
        self
    }

    /// Sets the app id every OS window carries (dock icon, Wayland compositor rules).
    pub fn app_id(mut self, id: impl Into<String>) -> Self {
        self.app_id = id.into();
        self
    }

    /// Sets the tablet source the pointer feed reads.
    pub fn pen(mut self, source: PenSource) -> Self {
        self.pen = Some(source);
        self
    }

    /// Sets the end hook's ghost-layer painter.
    pub fn frame_layers(mut self, painter: Option<fn(&Context)>) -> Self {
        self.frame_layers = painter;
        self
    }
}

/// Why `install` refused a spec.
#[derive(Debug)]
#[non_exhaustive]
pub enum InstallError {
    /// The context already has the runtime plugin.
    AlreadyInstalled,
    /// Two style replacements name one style key.
    DuplicateStyle(DuplicateStyle),
    /// A command set's definitions are unusable, or two commands share an id.
    Commands(commands::CommandDefError),
    /// `install` ran inside a pass: install the runtime before the first pass.
    DuringPass,
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::AlreadyInstalled => f.write_str("the runtime is already installed on this context"),
            InstallError::DuplicateStyle(DuplicateStyle(key)) => write!(f, "two style replacements for the style key {key}"),
            InstallError::Commands(error) => write!(f, "command registry: {error}"),
            InstallError::DuringPass => f.write_str("install the runtime before the first pass"),
        }
    }
}

impl std::error::Error for InstallError {}

/// The shell with what the running browser offers: a browser build captures a screen colour only where the browser
/// has the EyeDropper, so `Caps::SCREEN_CAPTURE` is read here, once for every app. Native capabilities are the
/// build's own (`Caps::native`).
fn with_browser_caps(shell: ShellInfo) -> ShellInfo {
    #[cfg(target_arch = "wasm32")]
    {
        let mut shell = shell;
        if craft_platform::screen::available() {
            shell.caps.insert(craft_host::target::Caps::SCREEN_CAPTURE);
        }
        shell
    }
    #[cfg(not(target_arch = "wasm32"))]
    shell
}

/// Adds the runtime plugin to `ctx` and sets the egui options the runtime owns: the interface scale is the app's
/// (`RootEnv::scale`), quit is a command that goes through the close guard, and the wheel's modifiers are the spec's. A second call on the same context is an error,
/// checked first so an app that installs again (a language change) gets that answer; a call inside a pass is one too.
pub fn install(ctx: &Context, spec: InstallSpec) -> Result<(), InstallError> {
    if plugin::is_registered(ctx) {
        return Err(InstallError::AlreadyInstalled);
    }
    // The hooks of a pass that is running would see a runtime that was never there at its begin.
    if frame::in_pass(ctx) {
        return Err(InstallError::DuringPass);
    }
    let looks = spec.looks.freeze().map_err(InstallError::DuplicateStyle)?;
    commands::install(ctx, spec.stories).map_err(InstallError::Commands)?;
    looks.store(ctx);
    input::clear_shortcuts(ctx);
    input::set_wheel_modifiers(ctx, spec.wheel.zoom, spec.wheel.sideways);
    system::install(ctx, spec.system);
    pen::install(ctx, spec.pen);
    env::install_jobs(ctx, spec.jobs);
    clipboard::install(ctx, spec.clipboard);
    windows::install(ctx, with_browser_caps(spec.shell_info), spec.window_system, spec.placement, spec.frame, spec.app_id);
    frame::set_frame_layers(ctx, spec.frame_layers);
    frame::seed_hooked_passes(ctx);
    plugin::register(ctx);
    Ok(())
}
