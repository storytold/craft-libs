//! The window host's fixtures: install inputs that open no OS windows, and the scripted system.

use crate::runtime::InstallSpec;
use craft_host::InlineJobs;
use craft_host::target::ShellInfo;
use std::sync::Arc;

pub use crate::runtime::egui::fixture::{FullOutput, RawInput, dragged_id, record_input_hooks};

pub use craft_window::system::{FakeSystem, HeldButtons, NoWindows};

/// An install spec for the running build's shell and a system with no OS services, with the stories' command sets.
/// Background work runs inline, so what a pass asks for lands in the next pass on every machine and a settled frame
/// is the same on every run.
pub fn install_spec() -> InstallSpec {
    InstallSpec::new(ShellInfo::default(), Arc::new(NoWindows)).stories().jobs(Arc::new(InlineJobs))
}

/// Makes the shell of `ctx` run on `os`, for a test of the OS matrix: the key table, the chords shown, the chrome,
/// the capabilities of a native build and egui's own OS all follow it.
pub fn set_os(ctx: &egui::Context, os: craft_host::target::Os) {
    crate::runtime::windows::set_os(ctx, os);
}

/// The window actions the platform applies out of reach of egui's commands (`SetOpacity`) since the last call.
pub fn applied_actions(ctx: &egui::Context) -> Vec<(egui::ViewportId, crate::runtime::windows::WindowAction)> {
    crate::runtime::windows::take_sent(ctx)
}

/// Whether the app runs the move of `viewport`'s window from its move band (`Overlay::moved_by_app`), as the window
/// host knows it in the running pass.
pub fn moved_by_app(ctx: &egui::Context, viewport: egui::ViewportId) -> bool {
    crate::runtime::windows::app_moves(ctx, viewport)
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
