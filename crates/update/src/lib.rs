//! Native only (it downloads and installs through ureq): release checking, downloading, installing and self-updating for the Craft App Suite and ArtCraft Launcher.
#![warn(missing_docs)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod app;
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

/// Checks for an update against GitHub releases for the given app.
pub fn check_for_update(app: &App, http: &Http, current_version: &str) -> Result<Option<Release>, CheckError> {
    check_releases_at(&format!("https://api.github.com/repos/{}/releases?per_page=30", app.repo), app, http, current_version)
}

/// [`check_for_update`] against the release list at `url` (a loopback server in tests).
fn check_releases_at(url: &str, app: &App, http: &Http, current_version: &str) -> Result<Option<Release>, CheckError> {
    let version = self_update::current_version(app, current_version).map(|v| v.to_string());
    let current_version = version.as_deref().unwrap_or(current_version);
    match http.get_api(url, None)? {
        crate::http::Fetched::Body { text, .. } => Ok(offered_release(&text, app.repo, current_version)?),
        crate::http::Fetched::RateLimited { .. } => Err(CheckError::RateLimited),
        crate::http::Fetched::NotModified | crate::http::Fetched::NotFound => Ok(None),
    }
}

/// Unix seconds now.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// The release in GitHub's release list `json` to offer a copy running `current_version`, if any:
/// the latest published release, when [`version::offers_update`] says so.
fn offered_release(json: &str, repo: &str, current_version: &str) -> Result<Option<Release>, String> {
    let latest = crate::release::parse_github_release_list(json, repo, false)?;
    Ok(latest.filter(|release| {
        let running = crate::version::parse(current_version);
        release.version.as_ref().zip(running.as_ref()).is_some_and(|(rel_v, cur_v)| crate::version::offers_update(rel_v, cur_v))
    }))
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
