//! `WindowSystem`: the OS calls the window host makes outside egui (desktop pointer, mouse button and modifier state,
//! pointer warp, monitors). egui-free. `SystemWindows` is the running platform's, `FakeSystem` the scripted
//! implementation for tests.

use craft_behavior::Monitor;
use craft_host::target::DoubleClick;
use std::sync::Mutex;

/// OS services the multi-window driver reads. Every method runs in the UI's `set_input`, outside every egui lock.
pub trait WindowSystem: Send + Sync {
    /// The system pointer in desktop points, or `None` where the platform does not tell (Wayland, the web).
    fn desktop_pointer(&self) -> Option<[f32; 2]>;

    /// Whether the primary mouse button is down now, or `None` where the platform does not tell. Read while the OS
    /// moves a window for a drag and sends the app no pointer events.
    fn primary_held(&self) -> Option<bool>;

    /// Whether the platform's command modifier (Cmd on macOS, Ctrl elsewhere) is down now, or `None` where the platform
    /// does not tell. Read while the OS moves a window for a drag and sends the app no key events.
    fn command_held(&self) -> Option<bool>;

    /// Moves the system pointer to `to` in desktop points. Returns whether the platform moved it.
    fn warp_pointer(&self, to: [f32; 2]) -> bool;

    /// The connected monitors.
    fn monitors(&self) -> Vec<Monitor>;

    /// What the user set a double click on a window's title bar to do, or `None` where the platform has no such
    /// setting for an app-drawn bar. The runtime reads it once, at install.
    fn title_bar_double_click(&self) -> Option<DoubleClick> {
        None
    }
}

/// The title-bar double click the global default `AppleActionOnDoubleClick` names (System Settings › Desktop & Dock ›
/// "Double-click a window's title bar to"): `Minimize` minimizes and `None` does nothing. Anything else zooms: unset
/// (never changed), `Maximize`, and `Fill`, which macOS 15 added and zooming comes closest to.
pub fn mac_double_click(value: Option<&str>) -> DoubleClick {
    match value.map(str::trim) {
        Some("Minimize") => DoubleClick::Minimize,
        Some("None") => DoubleClick::Nothing,
        _ => DoubleClick::Zoom,
    }
}

/// A platform with none of the services: the pointer is unknown, cannot be moved, and no monitors are listed.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoWindows;

impl WindowSystem for NoWindows {
    fn desktop_pointer(&self) -> Option<[f32; 2]> {
        None
    }

    fn primary_held(&self) -> Option<bool> {
        None
    }

    fn command_held(&self) -> Option<bool> {
        None
    }

    fn warp_pointer(&self, _: [f32; 2]) -> bool {
        false
    }

    fn monitors(&self) -> Vec<Monitor> {
        Vec::new()
    }
}

/// The running platform's services. AppKit tells the pointer, the primary button and Cmd on macOS (with the
/// `appkit` feature); elsewhere none is told. It warps no pointer and lists no monitors.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemWindows;

impl WindowSystem for SystemWindows {
    /// AppKit measures from the bottom-left of the primary screen (the one with the menu bar, first in
    /// `NSScreen::screens`), y up; winit flips window positions against that screen's height, whichever screen a
    /// window is on, so the pointer is flipped the same way.
    fn desktop_pointer(&self) -> Option<[f32; 2]> {
        #[cfg(all(feature = "appkit", target_os = "macos"))]
        {
            use objc2_app_kit::{NSEvent, NSScreen};
            // eframe runs the app on the main thread; anywhere else AppKit is out of reach.
            let mtm = objc2::MainThreadMarker::new()?;
            let primary = NSScreen::screens(mtm).firstObject()?;
            let at = NSEvent::mouseLocation();
            Some([at.x as f32, (primary.frame().size.height - at.y) as f32])
        }
        #[cfg(not(all(feature = "appkit", target_os = "macos")))]
        {
            None
        }
    }

    fn primary_held(&self) -> Option<bool> {
        #[cfg(all(feature = "appkit", target_os = "macos"))]
        {
            Some(objc2_app_kit::NSEvent::pressedMouseButtons() & 1 != 0)
        }
        #[cfg(not(all(feature = "appkit", target_os = "macos")))]
        {
            None
        }
    }

    fn command_held(&self) -> Option<bool> {
        #[cfg(all(feature = "appkit", target_os = "macos"))]
        {
            use objc2_app_kit::{NSEvent, NSEventModifierFlags};
            Some(NSEvent::modifierFlags_class().contains(NSEventModifierFlags::Command))
        }
        #[cfg(not(all(feature = "appkit", target_os = "macos")))]
        {
            None
        }
    }

    fn warp_pointer(&self, _: [f32; 2]) -> bool {
        false
    }

    fn monitors(&self) -> Vec<Monitor> {
        Vec::new()
    }

    /// The global default `AppleActionOnDoubleClick` on macOS (with the `appkit` feature), read through
    /// `NSUserDefaults`; elsewhere none is told.
    fn title_bar_double_click(&self) -> Option<DoubleClick> {
        #[cfg(all(feature = "appkit", target_os = "macos"))]
        {
            use objc2_foundation::{NSString, NSUserDefaults};
            let value = NSUserDefaults::standardUserDefaults().stringForKey(&NSString::from_str("AppleActionOnDoubleClick"));
            Some(mac_double_click(value.map(|value| value.to_string()).as_deref()))
        }
        #[cfg(not(all(feature = "appkit", target_os = "macos")))]
        {
            None
        }
    }
}

/// A scripted system: the pointer, the button and modifier state and the monitors are set by the test, warps are
/// recorded.
#[derive(Debug, Default)]
pub struct FakeSystem {
    state: Mutex<FakeState>,
}

#[derive(Debug, Default)]
struct FakeState {
    pointer: Option<[f32; 2]>,
    primary: Option<bool>,
    command: Option<bool>,
    double_click: Option<DoubleClick>,
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

    /// Sets whether the primary button is down; `None` where the platform does not tell.
    pub fn set_primary_held(&self, held: Option<bool>) {
        self.with(|state| state.primary = held);
    }

    /// Sets whether the command modifier is down; `None` where the platform does not tell.
    pub fn set_command_held(&self, held: Option<bool>) {
        self.with(|state| state.command = held);
    }

    /// Sets the title-bar double click the user chose; `None` where the platform has no such setting.
    pub fn set_title_bar_double_click(&self, double_click: Option<DoubleClick>) {
        self.with(|state| state.double_click = double_click);
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

    fn primary_held(&self) -> Option<bool> {
        self.with(|state| state.primary)
    }

    fn command_held(&self) -> Option<bool> {
        self.with(|state| state.command)
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

    fn title_bar_double_click(&self) -> Option<DoubleClick> {
        self.with(|state| state.double_click)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values System Settings writes map to their actions; macOS 15's Fill and an unset or unknown value zoom.
    #[test]
    fn the_mac_setting_names_its_action() {
        assert_eq!(mac_double_click(Some("Minimize")), DoubleClick::Minimize);
        assert_eq!(mac_double_click(Some("None")), DoubleClick::Nothing);
        for zooms in [None, Some("Maximize"), Some("Fill"), Some("")] {
            assert_eq!(mac_double_click(zooms), DoubleClick::Zoom, "{zooms:?} zooms");
        }
    }
}
