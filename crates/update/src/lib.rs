//! Native only (it downloads and installs through ureq): release checking, downloading, installing and self-updating for the Craft App Suite and ArtCraft Launcher.
#![warn(missing_docs)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod app;
pub mod catalog;
pub mod desktop_entry;
pub mod detect;
pub mod distro;
mod folders;
pub mod http;
pub mod install;
pub mod manifest;
pub mod msi_context;
pub(crate) mod msi_copy;
pub(crate) mod msi_migrate;
pub mod packages;
pub mod paths;
pub mod pins;
pub mod platform;
#[cfg(windows)]
pub mod portable;
pub mod process;
pub mod protocol;
pub mod proxy;
pub mod queue;
pub mod release;
pub mod self_update;
pub mod source;
pub mod status;
pub mod suite;
pub mod time;
pub mod translate;
pub mod version;

#[cfg(feature = "ui")]
pub mod stack_dialogs;
#[cfg(feature = "ui")]
pub mod updater;

pub use app::App;
pub use http::{Http, Stop};
pub use paths::Paths;
pub use platform::{Arch, Os, PackageKind, Platform, pick_asset};
pub use protocol::{InstallLocations, JobPhase, LatestRelease, Source};
pub use release::{Asset, Release, parse_github_release, parse_github_release_list};
pub use status::{InstallSource, Installed};
pub use suite::SUITE;
pub use time::parse_rfc3339;
pub use version::{Version, is_newer, parse as parse_version};

#[cfg(feature = "screen-testing")]
/// The registrations of this package for the integration binary.
pub mod testing {

    #[cfg(feature = "suite")]
    craft_ui::registry!();
}

/// Why a check for an update failed. [`RateLimited`](Self::RateLimited) is a typed reason, so the app
/// words it in the interface language ([`stack_dialogs::UpdateWords`] has the crate's wording); the text
/// of [`Failed`](Self::Failed) is data (GitHub's, the network's, or the crate's English) and stays as it came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckError {
    /// GitHub refused the request because its quota is used up.
    RateLimited,
    /// Any other failure, with its English reason.
    Failed(String),
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RateLimited => f.write_str("GitHub's request limit was reached"),
            Self::Failed(why) => f.write_str(why),
        }
    }
}

impl From<String> for CheckError {
    fn from(why: String) -> Self {
        Self::Failed(why)
    }
}

impl From<&str> for CheckError {
    fn from(why: &str) -> Self {
        Self::Failed(why.to_owned())
    }
}

/// Checks for an update: defaults to the signed `getartcraft.com` catalog, falling back to
/// GitHub releases when the catalog is unavailable, fails verification, is stale, or omits `app`.
///
/// Returns an error when the GitHub fallback does not find the repository (renamed, deleted or private).
pub fn check_for_update(app: &App, http: &Http, current_version: &str) -> Result<Option<Release>, CheckError> {
    let version = self_update::current_version(app, current_version).map(|v| v.to_string());
    let effective_version = version.as_deref().unwrap_or(current_version);
    match catalog::SignedCatalog::fetch(http) {
        Ok(signed) => {
            let prereleases = wants_prereleases(crate::version::parse(effective_version).as_ref());
            if let Some(latest) = signed.latest_for(app, prereleases, now()) {
                return Ok(offer_if_newer(latest, effective_version));
            }
        }
        Err(why) => {
            log::debug!("signed catalog unavailable ({why}); falling back to GitHub");
        }
    }
    check_releases_at(&format!("https://api.github.com/repos/{}/releases?per_page=30", app.repo), app, http, current_version)
}

/// Returns `Some(release)` when [`version::offers_update`] says `current_version` should update to it.
fn offer_if_newer(release: Release, current_version: &str) -> Option<Release> {
    let running = crate::version::parse(current_version)?;
    let rel_v = release.version.as_ref()?;
    crate::version::offers_update(rel_v, &running).then_some(release)
}

/// [`check_for_update`] against the release list at `url` (a loopback server in tests).
fn check_releases_at(url: &str, app: &App, http: &Http, current_version: &str) -> Result<Option<Release>, CheckError> {
    let version = self_update::current_version(app, current_version).map(|v| v.to_string());
    let current_version = version.as_deref().unwrap_or(current_version);
    match http.get_api(url, None)? {
        crate::http::Fetched::Body { text, .. } => Ok(offered_release(&text, app.repo, current_version)?),
        crate::http::Fetched::RateLimited { .. } => Err(CheckError::RateLimited),
        crate::http::Fetched::NotFound => Err(CheckError::Failed(format!("GitHub has no repository {}, or it is private", app.repo))),
        crate::http::Fetched::NotModified => Ok(None),
    }
}

/// Unix seconds now.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Whether a copy running `running` also considers pre-releases: only a published pre-release
/// build does, not a stable copy or a build from source.
fn wants_prereleases(running: Option<&semver::Version>) -> bool {
    running.is_some_and(|v| !crate::version::is_stable(v) && !crate::version::is_local_build(v))
}

/// The release in GitHub's release list `json` to offer a copy running `current_version`, if any:
/// the latest published release, when [`version::offers_update`] says so. A copy running a published
/// pre-release (`0.1.0-alpha.3`, `0.17.0-rc.1`) also considers pre-releases, so it is offered the next
/// one; a stable copy and a build from source are offered stable releases only.
fn offered_release(json: &str, repo: &str, current_version: &str) -> Result<Option<Release>, String> {
    let running = crate::version::parse(current_version);
    let prereleases = wants_prereleases(running.as_ref());
    let latest = crate::release::parse_github_release_list(json, repo, prereleases)?;
    Ok(latest
        .filter(|release| release.version.as_ref().zip(running.as_ref()).is_some_and(|(rel_v, cur_v)| crate::version::offers_update(rel_v, cur_v))))
}

#[cfg(test)]
pub(crate) const TEST_AGENT: &str = "ArtCraft-Launcher-Test/0.0";

#[cfg(test)]
extern crate self as craft_update;
#[cfg(test)]
#[path = "../tests/https_only.rs"]
pub(crate) mod https_only;
#[cfg(all(test, feature = "screen-testing"))]
#[path = "../tests/it/main.rs"]
pub(crate) mod integration;

#[cfg(test)]
fn main() {
    craft_ui::registry!();
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::Registry::of(&[register]));
}

#[cfg(test)]
mod offer_tests {
    use super::offered_release;
    use craft_ui::testing::trial;

    const REPO: &str = "storytold/photocraft";
    const PRERELEASES: &str = r#"[{"tag_name":"v0.1.0-alpha.4","prerelease":true,"draft":false,"assets":[]},
        {"tag_name":"v0.1.0-alpha.5","prerelease":true,"draft":true,"assets":[]},
        {"tag_name":"v0.1.0-alpha.3","prerelease":true,"draft":false,"assets":[]},
        {"tag_name":"v0.0.9","prerelease":false,"draft":false,"assets":[]}]"#;

    fn offered_from(json: &str, running: &str) -> Option<String> {
        offered_release(json, REPO, running).unwrap().map(|release| release.tag)
    }

    #[trial]
    fn a_prerelease_build_is_offered_the_next_prerelease() {
        assert_eq!(offered_from(PRERELEASES, "0.1.0-alpha.3"), Some("v0.1.0-alpha.4".into()));
        assert_eq!(offered_from(PRERELEASES, "0.1.0-alpha.4"), None, "a draft is never offered");
        assert_eq!(offered_from(PRERELEASES, "0.1.0-alpha.4+build.7"), None);
        // A release flagged as a pre-release on GitHub counts as one even when its tag has no label.
        let flagged = r#"[{"tag_name":"v0.2.0","prerelease":true,"draft":false,"assets":[]}]"#;
        assert_eq!(offered_from(flagged, "0.1.0-alpha.4"), Some("v0.2.0".into()));
    }

    #[trial]
    fn only_a_published_prerelease_build_considers_prereleases() {
        let wants = |running: &str| super::wants_prereleases(crate::version::parse(running).as_ref());
        assert!(wants("0.1.0-alpha.3"));
        assert!(!wants("0.0.9"));
        assert!(!wants("0.0.8-dev"));
        assert!(!wants("0.1.0-integration.20261120.a20dfd16"));
        assert!(!super::wants_prereleases(None));
    }

    #[trial]
    fn stable_copies_and_builds_from_source_are_not_offered_prereleases() {
        assert_eq!(offered_from(PRERELEASES, "0.0.9"), None);
        assert_eq!(offered_from(PRERELEASES, "0.0.8"), Some("v0.0.9".into()));
        assert_eq!(offered_from(PRERELEASES, "0.0.8-dev"), Some("v0.0.9".into()));
        assert_eq!(offered_from(PRERELEASES, "0.1.0-integration.20261120.a20dfd16"), None);
    }

    /// The pre-release precedence examples of Semantic Versioning 2.0.0, section 11
    /// (<https://semver.org/#spec-item-11>):
    /// `1.0.0-alpha < 1.0.0-alpha.1 < 1.0.0-alpha.beta < 1.0.0-beta < 1.0.0-beta.2 < 1.0.0-beta.11 < 1.0.0-rc.1 < 1.0.0`.
    #[trial]
    fn prereleases_are_offered_in_semver_precedence() {
        let order = ["1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta", "1.0.0-beta.2", "1.0.0-beta.11", "1.0.0-rc.1", "1.0.0"];
        let list = order
            .iter()
            .rev()
            .map(|v| format!(r#"{{"tag_name":"v{v}","prerelease":{},"draft":false,"assets":[]}}"#, v.contains('-')))
            .collect::<Vec<_>>()
            .join(",");
        let list = format!("[{list}]");
        for running in &order[..order.len() - 1] {
            assert_eq!(offered_from(&list, running), Some("v1.0.0".into()), "{running}");
        }
        for (i, running) in order.iter().enumerate().take(order.len() - 1) {
            let list = order[..=i + 1]
                .iter()
                .rev()
                .map(|v| format!(r#"{{"tag_name":"v{v}","prerelease":true,"draft":false,"assets":[]}}"#))
                .collect::<Vec<_>>()
                .join(",");
            assert_eq!(offered_from(&format!("[{list}]"), running), Some(format!("v{}", order[i + 1])), "{running}");
        }
    }
}
