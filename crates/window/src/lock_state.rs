//! The Caps Lock toggle, which the keys' events don't carry: egui reports the key's presses, not whether the lock
//! is on.

/// Whether Caps Lock is on now.
///
/// macOS reads the modifier flags of AppKit's current event state, Windows the toggle bit of the Caps Lock key.
/// Linux, the BSDs and the web have no query the windowing backend exposes, so they report off.
pub fn caps_lock() -> bool {
    imp::caps_lock()
}

#[cfg(all(feature = "appkit", target_os = "macos"))]
mod imp {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags};

    pub(super) fn caps_lock() -> bool {
        NSEvent::modifierFlags_class().contains(NSEventModifierFlags::CapsLock)
    }
}

#[cfg(windows)]
mod imp {
    use winsafe::{GetKeyState, co::VK};

    pub(super) fn caps_lock() -> bool {
        let (_, toggled) = GetKeyState(VK::CAPITAL);
        toggled
    }
}

#[cfg(not(any(all(feature = "appkit", target_os = "macos"), windows)))]
mod imp {
    pub(super) fn caps_lock() -> bool {
        false
    }
}
