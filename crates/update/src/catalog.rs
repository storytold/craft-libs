//! Signed release catalog hosted at `https://getartcraft.com/launcher-catalog.toml`.
//!
//! Verified against the compiled-in Ed25519 public key before parsing. Used as the primary
//! update source so apps don't each poll `api.github.com` (60 unauthenticated requests/hour),
//! with GitHub as the automatic fallback when the catalog is unavailable, stale, or omits an app.

use std::collections::BTreeMap;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey, pkcs8::DecodePublicKey};
use serde::Deserialize;

use crate::app::App;
use crate::http::Http;
use crate::release::{self, Asset, Release};
use crate::time::parse_rfc3339;
use crate::version;

/// Where the signed release catalog lives.
pub const CATALOG_URL: &str = "https://getartcraft.com/launcher-catalog.toml";
/// Detached Ed25519 signature over [`CATALOG_URL`].
pub const SIGNATURE_URL: &str = "https://getartcraft.com/launcher-catalog.toml.sig";
/// Pinned SPKI PEM public key corresponding to `LAUNCHER_CATALOG_SIGNING_KEY`.
pub const PUBLIC_KEY_PEM: &str = include_str!("../assets/launcher-catalog-public-key.pem");
/// Maximum accepted catalog size (1 MiB).
pub const MAX_CATALOG_BYTES: usize = 1 << 20;
/// Maximum accepted base64 signature size.
pub const MAX_SIGNATURE_BYTES: usize = 128;
/// How long a signed catalog snapshot is trusted before falling back to GitHub (24 hours).
pub const MAX_AGE: i64 = 24 * 3600;

/// Verified release metadata extracted from `launcher-catalog.toml`.
#[derive(Clone, Debug, Default)]
pub struct SignedCatalog {
    /// When the website build generated this catalog (Unix seconds).
    pub generated_at: Option<i64>,
    apps: BTreeMap<String, CatalogEntry>,
}

#[derive(Clone, Debug)]
struct CatalogEntry {
    repo: Option<String>,
    stable: Option<RawRelease>,
    prerelease: Option<RawRelease>,
    nightly: Option<RawRelease>,
}

#[derive(Deserialize, Default)]
struct Document {
    #[serde(default)]
    generated_at: Option<String>,
    #[serde(default)]
    launcher: Option<RawApp>,
    #[serde(default)]
    apps: Vec<RawApp>,
}

#[derive(Deserialize, Default)]
struct RawApp {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    releases: Option<RawChannels>,
}

#[derive(Deserialize, Default)]
struct RawChannels {
    #[serde(default)]
    stable: Option<RawRelease>,
    #[serde(default)]
    prerelease: Option<RawRelease>,
    #[serde(default)]
    nightly: Option<RawRelease>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct RawRelease {
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    notes_markdown: Option<String>,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct RawAsset {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    size_bytes: u64,
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
}

impl SignedCatalog {
    /// Fetches and verifies the signed catalog from `getartcraft.com`.
    pub fn fetch(http: &Http) -> Result<Self, String> {
        Self::fetch_from(http, CATALOG_URL, SIGNATURE_URL)
    }

    /// Fetches and verifies the signed catalog from explicit URLs.
    pub fn fetch_from(http: &Http, catalog_url: &str, signature_url: &str) -> Result<Self, String> {
        let catalog = http.get_bytes(catalog_url, MAX_CATALOG_BYTES)?.ok_or("catalog not published")?;
        let signature = http.get_bytes(signature_url, MAX_SIGNATURE_BYTES)?.ok_or("catalog signature not published")?;
        Self::verify(&catalog, &signature)
    }

    /// Verifies `signature` over `bytes` with the pinned Ed25519 key, then parses the TOML.
    pub fn verify(bytes: &[u8], signature: &[u8]) -> Result<Self, String> {
        let key = VerifyingKey::from_public_key_pem(PUBLIC_KEY_PEM).map_err(|_| "invalid compiled catalog key")?;
        Self::verify_with_key(bytes, signature, &key)
    }

    /// Verifies `signature` over `bytes` with an explicit key, then parses the TOML.
    pub fn verify_with_key(bytes: &[u8], signature: &[u8], key: &VerifyingKey) -> Result<Self, String> {
        if bytes.len() > MAX_CATALOG_BYTES || signature.len() > MAX_SIGNATURE_BYTES {
            return Err("signed catalog or signature exceeds byte limit".into());
        }
        let encoded = std::str::from_utf8(signature).map_err(|_| "signature is not UTF-8")?.trim();
        let decoded = STANDARD.decode(encoded).map_err(|_| "invalid base64 signature")?;
        let sig = Signature::from_slice(&decoded).map_err(|_| "invalid Ed25519 signature length")?;
        key.verify_strict(bytes, &sig).map_err(|_| "catalog signature does not match the pinned key")?;

        let text = std::str::from_utf8(bytes).map_err(|_| "catalog is not UTF-8")?;
        let doc: Document = toml::from_str(text).map_err(|e| format!("invalid catalog TOML: {e}"))?;
        let generated_at = doc.generated_at.as_deref().and_then(parse_rfc3339);

        let mut apps = BTreeMap::new();
        if let Some(launcher) = doc.launcher
            && let Some(channels) = launcher.releases
        {
            let id = launcher.id.unwrap_or_else(|| "artcraft-launcher".into()).to_lowercase();
            apps.insert(
                id,
                CatalogEntry { repo: launcher.repo, stable: channels.stable, prerelease: channels.prerelease, nightly: channels.nightly },
            );
        }
        for app in doc.apps {
            let Some(id) = app.id.filter(|s| !s.is_empty()) else { continue };
            let Some(channels) = app.releases else { continue };
            apps.insert(
                id.to_lowercase(),
                CatalogEntry { repo: app.repo, stable: channels.stable, prerelease: channels.prerelease, nightly: channels.nightly },
            );
        }
        Ok(Self { generated_at, apps })
    }

    /// The latest release for `app` at `now` (stable, or the higher of stable/prerelease when
    /// `prereleases` is true). Returns `None` if the catalog is stale, omits `app`, or the repo
    /// does not match `app.repo`.
    pub fn latest_for(&self, app: &App, prereleases: bool, now: i64) -> Option<Release> {
        let entry = self.fresh_entry(app.id, app.repo, now)?;
        let stable = entry.stable.as_ref().and_then(|r| r.to_release(app.repo));
        if !prereleases {
            return stable;
        }
        let pre = entry.prerelease.as_ref().and_then(|r| r.to_release(app.repo));
        match (stable, pre) {
            (Some(s), Some(p)) => {
                let pick_pre = match (&p.version, &s.version) {
                    (Some(pv), Some(sv)) => version::is_newer(pv, sv),
                    _ => false,
                };
                Some(if pick_pre { p } else { s })
            }
            (Some(s), None) => Some(s),
            (None, Some(p)) => Some(p),
            (None, None) => None,
        }
    }

    /// Looks up a release by `repo` (`owner/repo`), for callers that implement [`crate::source::ReleaseSource`].
    pub fn latest_for_repo(&self, repo: &str, prereleases: bool, now: i64) -> Option<Release> {
        let generated_at = self.generated_at?;
        if now.saturating_sub(generated_at) > MAX_AGE {
            return None;
        }
        let entry = self.apps.values().find(|e| e.repo.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(repo)))?;
        let stable = entry.stable.as_ref().and_then(|r| r.to_release(repo));
        if !prereleases {
            return stable;
        }
        let pre = entry.prerelease.as_ref().and_then(|r| r.to_release(repo));
        match (stable, pre) {
            (Some(s), Some(p)) => {
                let pick_pre = match (&p.version, &s.version) {
                    (Some(pv), Some(sv)) => version::is_newer(pv, sv),
                    _ => false,
                };
                Some(if pick_pre { p } else { s })
            }
            (Some(s), None) => Some(s),
            (None, Some(p)) => Some(p),
            (None, None) => None,
        }
    }

    /// The newest nightly release for `app` at `now`, if listed.
    pub fn nightly_for(&self, app: &App, now: i64) -> Option<Release> {
        let entry = self.fresh_entry(app.id, app.repo, now)?;
        entry.nightly.as_ref().and_then(|r| r.to_release(app.repo))
    }

    fn fresh_entry(&self, id: &str, repo: &str, now: i64) -> Option<&CatalogEntry> {
        let generated_at = self.generated_at?;
        if now.saturating_sub(generated_at) > MAX_AGE {
            return None;
        }
        let entry = self.apps.get(&id.to_lowercase())?;
        if let Some(entry_repo) = &entry.repo
            && !entry_repo.eq_ignore_ascii_case(repo)
        {
            return None;
        }
        Some(entry)
    }
}

impl RawRelease {
    fn to_release(&self, repo: &str) -> Option<Release> {
        let tag = self.tag.as_deref().filter(|t| !t.is_empty() && t.len() <= 64 && release::valid_tag(t))?;
        let version = version::parse(tag)?;
        let prefix = release::download_prefix(repo);
        let assets = self
            .assets
            .iter()
            .filter_map(|a| {
                let name = a.name.as_deref().filter(|n| release::valid_file_name(n))?;
                let url = a.download_url.as_deref()?;
                let expected = format!("{prefix}{tag}/{name}");
                if url != expected || !release::download_url_under(url, &prefix) {
                    return None;
                }
                let sha256 = a
                    .sha256
                    .as_deref()
                    .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
                    .map(str::to_ascii_lowercase);
                Some(Asset { name: name.to_string(), size: a.size_bytes, url: url.to_string(), sha256 })
            })
            .take(300)
            .collect();
        let fallback_url = format!("https://github.com/{repo}/releases/tag/{tag}");
        let html_url = self
            .html_url
            .as_deref()
            .and_then(|u| release::safe_release_html_url(repo, u))
            .or_else(|| release::safe_release_html_url(repo, &fallback_url))
            .unwrap_or_else(|| format!("https://github.com/{repo}/releases"));
        Some(Release {
            repo: repo.to_string(),
            tag: tag.to_string(),
            version: Some(version),
            name: self.name.as_deref().filter(|n| !n.is_empty()).unwrap_or(tag).to_string(),
            published_at: self.published_at.as_deref().and_then(parse_rfc3339),
            html_url,
            notes: self.notes_markdown.clone().unwrap_or_default(),
            prerelease: self.prerelease,
            assets,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use craft_ui::testing::trial;
    use ed25519_dalek::{Signer as _, SigningKey};

    const NOW: i64 = 1_795_728_600; // 2026-11-26T21:30:00Z
    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn sample_toml() -> String {
        format!(
            r#"generated_at = "2026-11-26T21:00:00Z"

[launcher]
id = "artcraft-launcher"
repo = "storytold/craft-launcher"
[launcher.releases.stable]
tag = "v0.20.4"
version = "0.20.4"
html_url = "https://github.com/storytold/craft-launcher/releases/tag/v0.20.4"
prerelease = false
[[launcher.releases.stable.assets]]
name = "artcraft-launcher-0.20.4-macos-universal.dmg"
download_url = "https://github.com/storytold/craft-launcher/releases/download/v0.20.4/artcraft-launcher-0.20.4-macos-universal.dmg"
sha256 = "{HASH}"

[[apps]]
id = "photocraft"
repo = "storytold/photocraft"
[apps.releases.stable]
tag = "v0.23.1"
version = "0.23.1"
html_url = "https://github.com/storytold/photocraft/releases/tag/v0.23.1"
prerelease = false
[[apps.releases.stable.assets]]
name = "photocraft-0.23.1-macos-universal.dmg"
download_url = "https://github.com/storytold/photocraft/releases/download/v0.23.1/photocraft-0.23.1-macos-universal.dmg"
sha256 = "{HASH}"
[[apps.releases.stable.assets]]
name = "evil.dmg"
download_url = "https://evil.example/evil.dmg"
sha256 = "{HASH}"
[apps.releases.prerelease]
tag = "v0.24.0-rc.1"
version = "0.24.0-rc.1"
html_url = "https://github.com/storytold/photocraft/releases/tag/v0.24.0-rc.1"
prerelease = true
[apps.releases.nightly]
tag = "v0.24.0-nightly.20261126.1.1"
version = "0.24.0-nightly.20261126.1.1"
html_url = "https://github.com/storytold/photocraft/releases/tag/v0.24.0-nightly.20261126.1.1"
prerelease = true
"#
        )
    }

    fn sign(toml: &str) -> (SigningKey, String) {
        let key = SigningKey::from_bytes(&[7; 32]);
        let sig = STANDARD.encode(key.sign(toml.as_bytes()).to_bytes());
        (key, sig)
    }

    #[trial]
    fn verifies_signed_catalog_and_extracts_stable_prerelease_and_nightly() {
        let toml = sample_toml();
        let (key, sig) = sign(&toml);
        let catalog = SignedCatalog::verify_with_key(toml.as_bytes(), sig.as_bytes(), &key.verifying_key()).unwrap();
        let photo = App::by_id("photocraft").unwrap();

        let stable = catalog.latest_for(photo, false, NOW).unwrap();
        assert_eq!(stable.tag, "v0.23.1");
        assert_eq!(stable.assets.len(), 1, "evil.example asset must be dropped");
        assert_eq!(stable.assets[0].sha256.as_deref(), Some(HASH));

        let pre = catalog.latest_for(photo, true, NOW).unwrap();
        assert_eq!(pre.tag, "v0.24.0-rc.1");

        let nightly = catalog.nightly_for(photo, NOW).unwrap();
        assert_eq!(nightly.tag, "v0.24.0-nightly.20261126.1.1");

        let by_repo = catalog.latest_for_repo("storytold/craft-launcher", false, NOW).unwrap();
        assert_eq!(by_repo.tag, "v0.20.4");
    }

    #[trial]
    fn rejects_tampering_wrong_key_and_stale_catalog() {
        let toml = sample_toml();
        let (key, sig) = sign(&toml);
        let wrong_key = SigningKey::from_bytes(&[9; 32]);
        assert!(SignedCatalog::verify_with_key(toml.as_bytes(), sig.as_bytes(), &wrong_key.verifying_key()).is_err());
        assert!(SignedCatalog::verify_with_key(format!("{toml}\n").as_bytes(), sig.as_bytes(), &key.verifying_key()).is_err());

        let catalog = SignedCatalog::verify_with_key(toml.as_bytes(), sig.as_bytes(), &key.verifying_key()).unwrap();
        let photo = App::by_id("photocraft").unwrap();
        assert!(catalog.latest_for(photo, false, NOW + MAX_AGE + 3600).is_none());
    }
}
