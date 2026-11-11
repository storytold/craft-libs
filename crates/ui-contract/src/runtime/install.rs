//! `runtime::install`: adds the runtime plugin once and sets the egui options the runtime owns.

use crate::runtime::egui::{focus, input, plugin};
use crate::runtime::{commands, frame, windows};
use crate::style::{DuplicateStyle, StyleReplacements};
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
}

impl InstallSpec {
    /// A spec for `shell_info` and `window_system`, with no ghost-layer painter.
    pub fn new(shell_info: ShellInfo, window_system: Arc<dyn WindowSystem>) -> Self {
        InstallSpec { shell_info, window_system, placement: None, frame_layers: None, looks: StyleReplacements::new(), frame: None }
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

    /// Sets the root window's decided frame (`craft_window::policy::decide_for`).
    pub fn frame(mut self, decision: Decision) -> Self {
        self.frame = Some(decision);
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
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::AlreadyInstalled => f.write_str("the runtime is already installed on this context"),
            InstallError::DuplicateStyle(DuplicateStyle(key)) => write!(f, "two style replacements for the style key {key}"),
            InstallError::Commands(error) => write!(f, "command registry: {error}"),
        }
    }
}

impl std::error::Error for InstallError {}

/// Adds the runtime plugin to `ctx` and sets the egui options the runtime owns: focus on press is the
/// runtime's, interface scale is a command, and quit goes through the close guard. A second call on the same
/// context is an error.
pub fn install(ctx: &Context, spec: InstallSpec) -> Result<(), InstallError> {
    if plugin::is_registered(ctx) {
        return Err(InstallError::AlreadyInstalled);
    }
    let looks = spec.looks.freeze().map_err(InstallError::DuplicateStyle)?;
    commands::install(ctx).map_err(InstallError::Commands)?;
    looks.store(ctx);
    focus::never_surrender_focus(ctx);
    input::clear_shortcuts(ctx);
    windows::install(ctx, spec.shell_info, spec.window_system, spec.placement, spec.frame);
    frame::set_frame_layers(ctx, spec.frame_layers);
    frame::seed_hooked_passes(ctx);
    plugin::register(ctx);
    Ok(())
}
