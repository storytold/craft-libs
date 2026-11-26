//! The installed system fonts: a provider over one index of them, built once per process, and the OS's own interface
//! face.

#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
mod fontique;
#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
mod index;
pub(crate) mod ui_face;

use std::sync::Arc;

use egui::{Context, FontProvider};

/// A system font provider that prefers `locale`'s fonts, shared by every stack and check that asks
/// for the same locale. Every locale's provider asks one index of the installed fonts, built once
/// on a background thread; a lookup before it is built waits for it. `None` without the
/// `system_fonts` feature and on the web, which has no installed fonts to index.
#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
pub fn system_font_provider(locale: &str) -> Option<Arc<dyn FontProvider>> {
    use std::collections::BTreeMap;
    use std::sync::{Mutex, OnceLock, PoisonError};
    static PROVIDERS: OnceLock<Mutex<BTreeMap<String, Arc<dyn FontProvider>>>> = OnceLock::new();
    let mut providers = PROVIDERS.get_or_init(Mutex::default).lock().unwrap_or_else(PoisonError::into_inner);
    Some(Arc::clone(providers.entry(locale.to_owned()).or_insert_with(|| fontique::provider(locale, true))))
}

/// A system font provider that prefers `locale`'s fonts, shared by every stack and check that asks
/// for the same locale. `None` without the `system_fonts` feature and on the web, which has no installed fonts to index.
#[cfg(not(all(feature = "system_fonts", not(target_arch = "wasm32"))))]
pub fn system_font_provider(_locale: &str) -> Option<Arc<dyn FontProvider>> {
    None
}

/// A font provider and the function that starts its index for a context.
type InstalledProvider = (Arc<dyn FontProvider>, fn(&Context));

/// The system font provider an install gives a context when the app sets none, and the function that starts the
/// index for that context. A lookup before the index is built finds nothing, so the UI thread doesn't wait; the
/// context reloads its fonts and repaints once it is.
#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
pub(crate) fn installed_system_font_provider(locale: &str) -> Option<InstalledProvider> {
    Some((fontique::provider(locale, false), fontique::start))
}

#[cfg(not(all(feature = "system_fonts", not(target_arch = "wasm32"))))]
pub(crate) fn installed_system_font_provider(_locale: &str) -> Option<InstalledProvider> {
    None
}
