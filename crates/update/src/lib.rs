//! Release checking, downloading, installing and self-updating for the Craft App Suite and ArtCraft Launcher.
//!
//! The default `native` feature brings everything that downloads, installs or touches the
//! system. Without it (`default-features = false`) only the pure parts build: release parsing,
//! versions, platforms and asset choice, the suite's apps and install statuses. Those build for
//! wasm32, which is how ArtCraft Launcher's web build shares them.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod app;
#[cfg(feature = "native")]
pub mod desktop_entry;
#[cfg(feature = "native")]
pub mod detect;
pub mod distro;
#[cfg(feature = "native")]
mod folders;
#[cfg(feature = "native")]
pub mod http;
#[cfg(feature = "native")]
pub mod install;
#[cfg(feature = "native")]
pub mod manifest;
#[cfg(feature = "native")]
pub mod msi_context;
#[cfg(feature = "native")]
mod msi_copy;
#[cfg(feature = "native")]
mod msi_migrate;
#[cfg(feature = "native")]
pub mod packages;
#[cfg(feature = "native")]
pub mod paths;
pub mod platform;
#[cfg(all(windows, feature = "native"))]
pub mod portable;
#[cfg(feature = "native")]
pub mod process;
pub mod protocol;
#[cfg(feature = "native")]
pub mod proxy;
#[cfg(feature = "native")]
pub mod queue;
pub mod release;
#[cfg(feature = "native")]
pub mod self_update;
#[cfg(feature = "native")]
pub mod source;
pub mod status;
pub mod suite;
pub mod time;
pub mod version;

#[cfg(feature = "egui")]
pub mod dialog;
#[cfg(feature = "egui")]
pub mod updater;

pub use app::App;
#[cfg(feature = "native")]
pub use http::{Http, Stop};
#[cfg(feature = "native")]
pub use paths::Paths;
pub use platform::{Arch, Os, PackageKind, Platform, pick_asset};
pub use protocol::{InstallLocations, JobPhase, LatestRelease, Source};
pub use release::{Asset, Release, parse_github_release, parse_github_release_list};
pub use status::{InstallSource, Installed};
pub use suite::SUITE;
pub use time::parse_rfc3339;
pub use version::{Version, is_newer, parse as parse_version};

#[cfg(feature = "egui")]
pub use dialog::{StartupPrompt, StartupPromptResponse};

/// Checks for an update against GitHub releases for the given app.
#[cfg(feature = "native")]
pub fn check_for_update(app: &App, http: &Http, current_version: &str) -> Result<Option<Release>, String> {
    let version = self_update::current_version(app, current_version).map(|v| v.to_string());
    let current_version = version.as_deref().unwrap_or(current_version);
    let url = format!("https://api.github.com/repos/{}/releases?per_page=30", app.repo);
    match http.get_api(&url, None)? {
        crate::http::Fetched::Body { text, .. } => offered_release(&text, app.repo, current_version),
        crate::http::Fetched::RateLimited { .. } => Err("GitHub's request limit was reached".into()),
        crate::http::Fetched::NotModified | crate::http::Fetched::NotFound => Ok(None),
    }
}

/// Unix seconds now.
#[cfg(feature = "native")]
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// The release in GitHub's release list `json` to offer a copy running `current_version`, if any:
/// the latest published release, when [`version::offers_update`] says so.
#[cfg(feature = "native")]
fn offered_release(json: &str, repo: &str, current_version: &str) -> Result<Option<Release>, String> {
    let latest = crate::release::parse_github_release_list(json, repo, false)?;
    Ok(latest.filter(|release| {
        let running = crate::version::parse(current_version);
        release.version.as_ref().zip(running.as_ref()).is_some_and(|(rel_v, cur_v)| crate::version::offers_update(rel_v, cur_v))
    }))
}

#[cfg(all(test, feature = "native"))]
pub(crate) const TEST_AGENT: &str = "ArtCraft-Launcher-Test/0.0";

#[cfg(test)]
#[cfg(feature = "native")]
mod offer_tests {
    use super::offered_release;

    const REPO: &str = "storytold/photocraft";
    const LIST: &str = r#"[{"tag_name":"v0.17.0","prerelease":false,"draft":false,"assets":[]},
        {"tag_name":"v0.16.4","prerelease":false,"draft":false,"assets":[]}]"#;

    fn offered(running: &str) -> Option<String> {
        offered_release(LIST, REPO, running).unwrap().map(|release| release.tag)
    }

    #[test]
    fn a_build_from_main_is_not_offered_the_release_it_already_contains() {
        assert_eq!(offered("0.17.0-integration.20261029.f5b3f123"), None);
        assert_eq!(offered("0.16.5-dev"), Some("v0.17.0".into()));
    }

    #[test]
    fn releases_and_release_candidates_are_offered_as_before() {
        assert_eq!(offered("0.17.0-rc.2"), Some("v0.17.0".into()));
        assert_eq!(offered("0.16.4"), Some("v0.17.0".into()));
        assert_eq!(offered("0.17.0"), None);
        assert_eq!(offered("not a version"), None);
    }
}
