//! Whether a window fits its monitor.
//!
//! egui doesn't expose the monitor's work area (the screen less the taskbar, Dock or panel), only
//! the monitor's size. [`TASKBAR`] is the room kept free at the monitor's bottom edge.

use egui::Vec2;

/// Room kept free at the monitor's bottom edge for a taskbar or Dock egui can't see (the Windows
/// taskbar is 40–48 pt tall; a bottom Dock about as much).
// pc: crates/ui-egui/src/work_area.rs:17 `TASKBAR`.
pub const TASKBAR: f32 = 48.0;

/// Whether a window of outer size `outer` fits `monitor`: `outer.x <= monitor.x + 0.5` and
/// `outer.y <= monitor.y - TASKBAR + 0.5`.
// pc: crates/ui-egui/src/work_area.rs:100 `fits`.
pub fn fits(outer: Vec2, monitor: Vec2) -> bool {
    outer.x <= monitor.x + 0.5 && outer.y <= monitor.y - TASKBAR + 0.5
}
