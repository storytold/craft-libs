//! Native only (it downloads and installs through ureq): release checking, downloading, installing and self-updating for the Craft App Suite and ArtCraft Launcher.

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
mod msi_copy;
mod msi_migrate;
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
    craft_ui::testing_registered!();
}

/// Checks for an update against GitHub releases for the given app.
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
mod https_only;
#[cfg(all(test, feature = "screen-testing"))]
#[path = "../tests/it/main.rs"]
mod integration;

#[cfg(test)]
fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::registered_in(env!("CARGO_PKG_NAME")));
}
