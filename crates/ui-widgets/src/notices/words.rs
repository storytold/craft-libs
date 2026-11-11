//! Built-in words of the notices family.

craft_ui::words! {
    /// The words of the notices family; each id is prefixed by the type that shows it (`notice`, `settingsNotices`).
    /// The "Don't Show Again" action is `common.dontShowAgain`.
    pub struct NoticeWords {
        dismiss: "notice.dismiss" = "Dismiss";
        app_fallback: "settingsNotices.appFallback" = "The app";
        saved: "settingsNotices.saved" = "Preferences saved";
        cant_save_title: "settingsNotices.cantSaveTitle" = "Preferences can't be saved";
        reset_title: "settingsNotices.resetTitle" = "Some preferences were reset";
        save_failed: template "settingsNotices.saveFailed" = "Couldn't save preferences: {error}. Retrying…";
        cant_save_body: template "settingsNotices.cantSaveBody" = "{app} keeps retrying; until a save succeeds, preference changes are lost when it closes.";
        reset_body: template "settingsNotices.resetBody" = "The preferences file held values {app} can't read for {paths}. Those settings are back to their defaults; the rest of your preferences loaded.";
        moved_aside: template "settingsNotices.movedAside" = "Preferences were reset; the unreadable file was kept as {path}.";
        read_only: template "settingsNotices.readOnly" = "Preferences won't be saved: {reason}";
    }
}
