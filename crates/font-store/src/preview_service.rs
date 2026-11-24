//! The preview service's address and how a store reports it: the environment variable, where the address in
//! effect came from, the check that makes a usable address, and [`PreviewStore`], what the preview pool and the font
//! manager's `Installer` read from a store. Plain data and one trait, so they build on every target; the pool itself
//! ([`preview`](crate::preview)) is native only.

use std::path::PathBuf;

/// Which of a family's legal texts `FontStore::legal_text` reads.
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
    /// The host's default, `FontStore::with_default_preview_service`.
    Default,
    /// There is no service.
    None,
}

/// The environment value when set and not empty, else the setting; an unusable value counts as
/// unset.
#[cfg(test)]
pub(crate) fn resolve_preview_service(env: Option<&str>, setting: Option<&str>) -> Option<String> {
    resolve_preview_source(env, setting, None).0
}

/// The service in effect and where it came from: the environment value when set and not empty
/// (alone: an unusable one means no service), else the setting when usable, else the host's
/// default.
pub(crate) fn resolve_preview_source(env: Option<&str>, setting: Option<&str>, default: Option<&str>) -> (Option<String>, PreviewServiceSource) {
    let (url, source) = match env.map(str::trim).filter(|e| !e.is_empty()) {
        Some(e) => (normalize_service_url(e), PreviewServiceSource::Env),
        None => match setting.and_then(normalize_service_url) {
            Some(url) => (Some(url), PreviewServiceSource::Saved),
            None => (default.and_then(normalize_service_url), PreviewServiceSource::Default),
        },
    };
    if url.is_some() { (url, source) } else { (None, PreviewServiceSource::None) }
}

/// The preview-service address `url` stands for: without a trailing slash, when it is an https URL,
/// or an http one for a loopback host, with no credentials, query or fragment. [`BadServiceUrl`]
/// otherwise.
pub fn check_preview_service(url: &str) -> Result<String, BadServiceUrl> {
    normalize_service_url(url).ok_or_else(|| BadServiceUrl::Unusable(url.trim().chars().take(200).collect()))
}

/// `url` without a trailing slash, when it is an https URL, or an http one for a loopback host,
/// with no credentials, query or fragment.
pub(crate) fn normalize_service_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    if url.len() > 2000 || url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\') {
        return None;
    }
    let (https, rest) = match (url.strip_prefix("https://"), url.strip_prefix("http://")) {
        (Some(r), _) => (true, r),
        (None, Some(r)) => (false, r),
        _ => return None,
    };
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || url.contains(['?', '#']) {
        return None;
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    let loopback = matches!(host.to_ascii_lowercase().as_str(), "127.0.0.1" | "::1");
    (https || loopback).then(|| url.to_string())
}

/// Why an address is not a usable preview service.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BadServiceUrl {
    /// Not an https URL, or an http URL to a loopback host, or it carries credentials, a query or a fragment.
    #[error("{0} is not an https URL, or an http URL to 127.0.0.1 or [::1]")]
    Unusable(String),
}

/// What [`Previews`](crate::preview::Previews) reads from the font store: the service and the user's setting on each request, the client and
/// the cache folder when the pool is made. The font manager's `Installer` builds on it.
pub trait PreviewStore: Send + Sync {
    /// The preview service's base URL, if one is set: [`PREVIEW_SERVICE_ENV`], else the
    /// font store's `preview_service` setting, else the host's default. Installs try it as a mirror first.
    fn preview_service(&self) -> Option<String> {
        None
    }

    /// The store's HTTP client, if it has one (native targets only). `Previews` fetches with it, so a preview service that installs
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
