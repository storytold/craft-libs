//! `WindowSystem`: the OS calls the window host makes outside egui (desktop pointer, mouse button and modifier state,
//! pointer warp, monitors). egui-free. `SystemWindows` is the running platform's, `FakeSystem` the scripted
//! implementation for tests.

use craft_behavior::Monitor;
use std::sync::Mutex;

/// The mouse buttons the OS reports down, with no egui types.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeldButtons(u8);

impl HeldButtons {
    /// No button.
    pub const NONE: HeldButtons = HeldButtons(0);
    /// The primary button.
    pub const PRIMARY: HeldButtons = HeldButtons(1);
    /// The secondary button.
    pub const SECONDARY: HeldButtons = HeldButtons(2);
    /// The middle button.
    pub const MIDDLE: HeldButtons = HeldButtons(4);
    /// The first extra button.
    pub const EXTRA1: HeldButtons = HeldButtons(8);
    /// The second extra button.
    pub const EXTRA2: HeldButtons = HeldButtons(16);

    /// Whether every button of `b` is down.
    pub fn contains(self, b: HeldButtons) -> bool {
        self.0 & b.0 == b.0
    }

    /// Adds the buttons of `b`.
    pub fn insert(&mut self, b: HeldButtons) {
        self.0 |= b.0;
    }
}

/// OS services the multi-window driver reads. Every method runs in the UI's `set_input`, outside every egui lock.
pub trait WindowSystem: Send + Sync {
    /// The system pointer in desktop points, or `None` where the platform does not tell (Wayland, the web).
    fn desktop_pointer(&self) -> Option<[f32; 2]>;

    /// Every mouse button down now, or `None` where the platform does not tell. Read while the OS moves a window for a
    /// drag and sends the app no pointer events, and to find a release the window never received.
    fn buttons_held(&self) -> Option<HeldButtons>;

    /// Asks the input method to drop its marked text. Returns whether it did.
    fn discard_marked_text(&self) -> bool;

    /// Whether the platform's command modifier (Cmd on macOS, Ctrl elsewhere) is down now, or `None` where the platform
    /// does not tell. Read while the OS moves a window for a drag and sends the app no key events.
    fn command_held(&self) -> Option<bool>;

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

    fn buttons_held(&self) -> Option<HeldButtons> {
        None
    }

    fn discard_marked_text(&self) -> bool {
        false
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
/// `appkit` feature); Windows tells the pointer; elsewhere none is told. It warps no pointer and lists no monitors.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemWindows;

impl WindowSystem for SystemWindows {
    /// AppKit measures from the bottom-left of the primary screen (the one with the menu bar, first in
    /// `NSScreen::screens`), y up; winit flips window positions against that screen's height, whichever screen a
    /// window is on, so the pointer is flipped the same way. Windows tells physical pixels, and winit gives a window's
    /// position as its physical pixels over the window's own scale, so the pointer is divided by the scale of the
    /// window under it: the app's window wherever the pointer is over one.
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
        #[cfg(windows)]
        {
            // Physical pixels whatever this thread's DPI awareness.
            let at = winsafe::GetPhysicalCursorPos().ok()?;
            let dpi = winsafe::HWND::WindowFromPhysicalPoint(at)?.GetDpiForWindow();
            // 0 is an invalid window; 96 DPI is a scale of 1.
            let scale = (dpi > 0).then(|| dpi as f32 / 96.0)?;
            Some([at.x as f32 / scale, at.y as f32 / scale])
        }
        #[cfg(not(any(all(feature = "appkit", target_os = "macos"), windows)))]
        {
            None
        }
    }

    /// AppKit's `pressedMouseButtons` has one bit per button in the order of [`HeldButtons`]'s constants.
    fn buttons_held(&self) -> Option<HeldButtons> {
        #[cfg(all(feature = "appkit", target_os = "macos"))]
        {
            Some(HeldButtons((objc2_app_kit::NSEvent::pressedMouseButtons() & 0x1f) as u8))
        }
        #[cfg(not(all(feature = "appkit", target_os = "macos")))]
        {
            None
        }
    }

    /// Not while the key window has a sheet attached: a file dialog's name field would lose its composition.
    fn discard_marked_text(&self) -> bool {
        #[cfg(all(feature = "appkit", target_os = "macos"))]
        {
            use objc2_app_kit::{NSApplication, NSTextInputContext};
            let discarded = || -> Option<()> {
                let mtm = objc2::MainThreadMarker::new()?;
                if NSApplication::sharedApplication(mtm).keyWindow().is_some_and(|window| window.attachedSheet().is_some()) {
                    return None;
                }
                NSTextInputContext::currentInputContext(mtm)?.discardMarkedText();
                Some(())
            };
            discarded().is_some()
        }
        #[cfg(not(all(feature = "appkit", target_os = "macos")))]
        {
            false
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
    buttons: Option<HeldButtons>,
    discards: usize,
    command: Option<bool>,
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

    /// Sets the buttons the test system reports down; `None` where the platform does not tell.
    pub fn set_buttons_held(&self, held: Option<HeldButtons>) {
        self.with(|state| state.buttons = held);
    }

    /// The count of asked discards of marked text.
    pub fn discards(&self) -> usize {
        self.with(|state| state.discards)
    }

    /// Sets whether the command modifier is down; `None` where the platform does not tell.
    pub fn set_command_held(&self, held: Option<bool>) {
        self.with(|state| state.command = held);
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

    fn buttons_held(&self) -> Option<HeldButtons> {
        self.with(|state| state.buttons)
    }

    fn discard_marked_text(&self) -> bool {
        self.with(|state| state.discards += 1);
        true
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
}
