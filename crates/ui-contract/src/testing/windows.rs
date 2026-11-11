//! The window host's fixtures: install inputs that open no OS windows, and the scripted system.

use crate::runtime::InstallSpec;
use craft_host::target::ShellInfo;
use std::sync::Arc;

pub use crate::runtime::egui::fixture::{FullOutput, RawInput, dragged_id, record_input_hooks};

pub use craft_window::system::{FakeSystem, NoWindows};

/// An install spec for the running build's shell and a system with no OS services.
pub fn install_spec() -> InstallSpec {
    InstallSpec::new(ShellInfo::default(), Arc::new(NoWindows))
}

/// The window actions the platform applies out of reach of egui's commands (`SetOpacity`) since the last call.
pub fn applied_actions(ctx: &egui::Context) -> Vec<(egui::ViewportId, crate::runtime::windows::WindowAction)> {
    crate::runtime::windows::take_sent(ctx)
}

/// Whether the window of `viewport` has a component that runs its own drag from the window's move band.
pub fn keeps_app_drag(ctx: &egui::Context, viewport: egui::ViewportId) -> bool {
    crate::runtime::windows::keeps_app_drag_of(ctx, viewport)
}

/// Starts the drag of the window container `source` (what the title row of a dock window does), with the pointer
/// released by the caller's events as in a real drag.
pub fn begin_window_drag(ctx: &egui::Context, source: egui::Id) {
    crate::runtime::drag::begin_window_drag(ctx, source);
}

/// Shows (or removes) the mark of a drop target in another window over the running drag.
pub fn show_drop_mark(ctx: &egui::Context, over: Option<egui::Rect>) {
    crate::runtime::drag::show_mark(ctx, over.map(crate::contract::DropMark::Into));
}
