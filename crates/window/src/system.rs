//! `WindowSystem`: the OS calls the window host makes outside egui (desktop pointer, pointer warp, monitors).
//! egui-free. `FakeSystem` is the scripted implementation for tests.

use craft_behavior::Monitor;
use std::sync::Mutex;

/// OS services the multi-window driver reads. Every method runs in the UI's `set_input`, outside every egui lock.
pub trait WindowSystem: Send + Sync {
    /// The system pointer in desktop points, or `None` where the platform does not tell (Wayland, the web).
    fn desktop_pointer(&self) -> Option<[f32; 2]>;

    /// Moves the system pointer to `to` in desktop points. Returns whether the platform moved it.
    fn warp_pointer(&self, to: [f32; 2]) -> bool;

    /// The connected monitors.
    fn monitors(&self) -> Vec<Monitor>;
}

/// A platform with none of the services: the pointer is unknown, cannot be moved, and no monitors are listed.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoWindows;

impl WindowSystem for NoWindows {
    fn desktop_pointer(&self) -> Option<[f32; 2]> {
        None
    }

    fn warp_pointer(&self, _: [f32; 2]) -> bool {
        false
    }

    fn monitors(&self) -> Vec<Monitor> {
        Vec::new()
    }
}

/// A scripted system: the pointer and monitors are set by the test, warps are recorded.
#[derive(Debug, Default)]
pub struct FakeSystem {
    state: Mutex<FakeState>,
}

#[derive(Debug, Default)]
struct FakeState {
    pointer: Option<[f32; 2]>,
    monitors: Vec<Monitor>,
    warps: Vec<[f32; 2]>,
}

impl FakeSystem {
    /// A system with no pointer and no monitors.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the desktop pointer.
    pub fn set_pointer(&self, pointer: Option<[f32; 2]>) {
        self.with(|state| state.pointer = pointer);
    }

    /// Sets the monitors.
    pub fn set_monitors(&self, monitors: Vec<Monitor>) {
        self.with(|state| state.monitors = monitors);
    }

    /// The targets of every warp so far.
    pub fn warps(&self) -> Vec<[f32; 2]> {
        self.with(|state| state.warps.clone())
    }

    fn with<R>(&self, f: impl FnOnce(&mut FakeState) -> R) -> R {
        // A poisoned lock still holds consistent plain data.
        let mut guard = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut guard)
    }
}

impl WindowSystem for FakeSystem {
    fn desktop_pointer(&self) -> Option<[f32; 2]> {
        self.with(|state| state.pointer)
    }

    fn warp_pointer(&self, to: [f32; 2]) -> bool {
        self.with(|state| {
            state.warps.push(to);
            state.pointer = Some(to);
        });
        true
    }

    fn monitors(&self) -> Vec<Monitor> {
        self.with(|state| state.monitors.clone())
    }
}
