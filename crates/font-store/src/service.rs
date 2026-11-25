//! What the font store and the preview service's client share with the font manager on every target: which legal
//! text to read, where the preview service comes from, and what the preview pool reads from a store. The store and
//! the client themselves are native only.

use std::path::PathBuf;

/// Which of a family's legal texts [`FontStore::legal_text`](crate::FontStore::legal_text) reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LegalText {
    /// The licence file (`OFL.txt`, `LICENSE.txt`).
    License,
    /// An Apache family's `NOTICE` file.
    Notice,
}

/// The environment variable that sets the preview service's base URL; it beats the saved setting.
pub const PREVIEW_SERVICE_ENV: &str = "CRAFT_FONT_PREVIEW_URL";

/// Where the preview service in effect comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewServiceSource {
    /// [`PREVIEW_SERVICE_ENV`]; the setting can't change it.
    Env,
    /// The `preview_service` setting in `settings.json`.
    Saved,
    /// The host's default, [`FontStore::with_default_preview_service`](crate::FontStore::with_default_preview_service).
    Default,
    /// There is no service.
    None,
}

/// What [`Previews`](crate::preview::Previews) reads from the font store: the service and the user's setting on each request, the client and
/// the cache folder when the pool is made. The font manager's `Installer` builds on it.
pub trait PreviewStore: Send + Sync {
    /// The preview service's base URL, if one is set: [`PREVIEW_SERVICE_ENV`](crate::PREVIEW_SERVICE_ENV), else the
    /// font store's `preview_service` setting, else the host's default. Installs try it as a mirror first.
    fn preview_service(&self) -> Option<String> {
        None
    }

    /// The store's HTTP client, if it has one. [`Previews`](crate::preview::Previews) fetches with it, so a preview service that installs
    /// found unreachable is skipped by previews too, and the reverse.
    #[cfg(not(target_arch = "wasm32"))]
    fn http_client(&self) -> Option<crate::Client> {
        None
    }

    /// Whether previews may be fetched from the preview service: the user's setting, on by default.
    fn previews_enabled(&self) -> bool {
        true
    }

    /// The folder [`Previews`](crate::preview::Previews) caches preview subsets in; none keeps them only for as long as the caller holds them.
    fn preview_cache(&self) -> Option<PathBuf> {
        None
    }
}
