//! Release checking, downloading, installing and self-updating for the Craft App Suite and ArtCraft Launcher.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod app;
pub mod desktop_entry;
pub mod detect;
pub mod distro;
mod folders;
pub mod http;
pub mod install;
pub mod manifest;
pub mod packages;
pub mod paths;
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
pub mod version;

#[cfg(feature = "egui")]
pub mod dialog;
#[cfg(feature = "egui")]
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

#[cfg(feature = "egui")]
pub use dialog::{StartupPrompt, StartupPromptResponse};

/// Checks for an update against GitHub releases for the given app.
pub fn check_for_update(app: &App, http: &Http, current_version: &str) -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{}/releases?per_page=30", app.repo);
    match http.get_api(&url, None)? {
        crate::http::Fetched::Body { text, .. } => {
            let latest = crate::release::parse_github_release_list(&text, app.repo, false)?;
            if let Some(release) = latest
                && let Some(rel_v) = &release.version
                && let Some(cur_v) = crate::version::parse(current_version)
                && crate::version::is_newer(rel_v, &cur_v)
            {
                return Ok(Some(release));
            }
            Ok(None)
        }
        crate::http::Fetched::RateLimited { .. } => Err("GitHub's request limit was reached".into()),
        crate::http::Fetched::NotModified | crate::http::Fetched::NotFound => Ok(None),
    }
}

/// Unix seconds now.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
pub(crate) const TEST_AGENT: &str = "ArtCraft-Launcher-Test/0.0";
