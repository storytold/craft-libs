//! The Caps Lock toggle, which the keys' events don't carry: egui reports the key's presses, not whether the lock
//! is on.

use craft_host::target::Backend;

/// Whether Caps Lock is on now.
///
/// macOS reads the modifier flags of AppKit's current event state, Windows the toggle bit of the Caps Lock key.
/// Linux and the BSDs read the XKB lock state when the window's `backend` is X11 (XWayland included); Wayland and the
/// web report off.
pub fn caps_lock(backend: Backend) -> bool {
    imp::caps_lock(backend)
}

#[cfg(all(feature = "appkit", target_os = "macos"))]
mod imp {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags};

    pub(super) fn caps_lock(_: super::Backend) -> bool {
        NSEvent::modifierFlags_class().contains(NSEventModifierFlags::CapsLock)
    }
}

#[cfg(windows)]
mod imp {
    use winsafe::{GetKeyState, co::VK};

    pub(super) fn caps_lock(_: super::Backend) -> bool {
        let (_, toggled) = GetKeyState(VK::CAPITAL);
        toggled
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
mod imp {
    use std::sync::OnceLock;
    use x11rb::protocol::xkb;
    use x11rb::protocol::xproto::ModMask;
    use x11rb::rust_connection::RustConnection;

    /// The process's one connection, opened at the first read; `None` without an X server or without XKB on it.
    static CONNECTION: OnceLock<Option<RustConnection>> = OnceLock::new();

    fn connection() -> Option<&'static RustConnection> {
        CONNECTION
            .get_or_init(|| {
                let (connection, _) = x11rb::connect(None).ok()?;
                let supported = xkb::use_extension(&connection, 1, 0).ok()?.reply().ok()?.supported;
                supported.then_some(connection)
            })
            .as_ref()
    }

    /// One round trip per call; the caller reads once per pass. An error answers off.
    pub(super) fn caps_lock(backend: super::Backend) -> bool {
        if backend != super::Backend::X11 {
            return false;
        }
        let Some(connection) = connection() else { return false };
        xkb::get_state(connection, xkb::ID::USE_CORE_KBD.into())
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|state| u16::from(state.locked_mods) & u16::from(ModMask::LOCK) != 0)
    }
}

#[cfg(not(any(
    all(feature = "appkit", target_os = "macos"),
    windows,
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
)))]
mod imp {
    pub(super) fn caps_lock(_: super::Backend) -> bool {
        false
    }
}
