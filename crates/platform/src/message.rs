//! [`PlatformText`]: a user-visible message of a platform service, with its catalog id and
//! English text. craft-platform has no catalog; the app looks the id up in its own.

/// A message with `{name}`-style placeholders the app fills after looking `id` up (the
/// English is the fallback). The open, clipboard, watch, default-app and temp services and
/// craft-dialogs' save flow word their messages with these, so every app shows the same text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlatformText {
    /// The catalog id.
    pub id: &'static str,
    /// The English text, used when the catalog has no entry.
    pub english: &'static str,
}

impl PlatformText {
    /// What a catalog lookup uses: the id.
    pub const fn key(&self) -> &'static str {
        self.id
    }
}

/// Every message the library's services word with a [`PlatformText`] (the services this build
/// has), in module order, for an app's missing-translation report. The save flow's are in `craft_dialogs::save::TEXTS`.
pub fn texts() -> Vec<PlatformText> {
    let tables: &[&[PlatformText]] = &[
        #[cfg(feature = "clipboard")]
        crate::clipboard::TEXTS,
        #[cfg(not(target_arch = "wasm32"))]
        crate::default_app::TEXTS,
        crate::open::TEXTS,
        #[cfg(not(target_arch = "wasm32"))]
        crate::temp::TEXTS,
        crate::watch::TEXTS,
    ];
    tables.iter().flat_map(|table| table.iter().copied()).collect()
}
