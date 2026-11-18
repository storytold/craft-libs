//! The Windows source of the appearance watcher: Settings ▸ Accessibility ▸ Visual effects ▸
//! Animation effects ("Show animations in Windows" in the Control Panel), read through WinRT's
//! `UISettings::AnimationsEnabled`, which needs no `unsafe`. The user changes it in Settings, outside
//! the app, so the watcher reads it when it starts and again each time the app becomes active.

/// Whether the system asks for no animations; `None` when WinRT can't be asked.
pub(super) fn reduced_motion() -> Option<bool> {
    match ::windows::UI::ViewManagement::UISettings::new().and_then(|settings| settings.AnimationsEnabled()) {
        Ok(enabled) => Some(!enabled),
        Err(e) => {
            log::debug!("system animation effects: {e}");
            None
        }
    }
}
