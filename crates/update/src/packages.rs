//! Reading system package lists (dpkg, rpm, pacman, Flatpak) and matching them to Craft Apps.
//!
//! Pure functions over the tools' text output, so they're tested on every platform; only Linux
//! runs the tools (`detect::linux`).

use crate::{App, InstallSource, version};

/// One installed package that belongs to a Craft App.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    /// The package manager that owns the package.
    pub manager: InstallSource,
    /// The package's name (`photocraft-bin`) or Flatpak app id.
    pub name: String,
    /// As the package manager reports it (`0.5.0-1`); empty when it doesn't say.
    pub version: String,
    /// Whether this is a user installation (`true`) or system installation (`false`).
    pub user: bool,
    /// The origin remote (`flatpark`, `<app>-origin`, `flathub`), when known.
    pub origin: Option<String>,
    /// The installation scope or name (`user`, `system`, or a named installation), for Flatpak installs.
    pub installation: Option<String>,
}

/// `dpkg-query -W -f '${Package}|${db:Status-Status}|${Version}\n'`: installed packages only.
pub fn parse_dpkg(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split('|');
            match (f.next(), f.next(), f.next()) {
                (Some(name), Some("installed"), Some(ver)) => {
                    let name = name.split_once(':').map(|(n, _)| n).unwrap_or(name);
                    Some((name.to_string(), ver.to_string()))
                }
                _ => None,
            }
        })
        .collect()
}

/// `rpm -qa --qf '%{NAME}|%{VERSION}\n'`.
pub fn parse_rpm(text: &str) -> Vec<(String, String)> {
    text.lines().filter_map(|line| line.split_once('|').map(|(n, v)| (n.to_string(), v.to_string()))).collect()
}

/// `pacman -Q`: `name version` per line.
pub fn parse_pacman(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            Some((f.next()?.to_string(), f.next()?.to_string()))
        })
        .collect()
}

/// `flatpak list --app --columns=application,version,origin,installation`: parses id, version, origin, and installation.
pub fn parse_flatpak_list_with_origin(text: &str) -> Vec<(String, String, Option<String>, Option<String>)> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split('\t');
            let id = f.next()?.trim();
            if id.is_empty() {
                return None;
            }
            let version = f.next().unwrap_or("").trim().to_string();
            let origin = f.next().map(str::trim).filter(|o| !o.is_empty()).map(ToString::to_string);
            let installation = f.next().map(str::trim).filter(|i| !i.is_empty()).map(ToString::to_string);
            Some((id.to_string(), version, origin, installation))
        })
        .collect()
}

/// `flatpak list --app --columns=application,version`: the version column is often empty for
/// bundles built from a release tarball.
pub fn parse_flatpak_list(text: &str) -> Vec<(String, String)> {
    parse_flatpak_list_with_origin(text).into_iter().map(|(id, v, _, _)| (id, v)).collect()
}

/// Whether an origin string indicates an install from FlatPark.
pub fn is_flatpark_origin(origin: &str) -> bool {
    let o = origin.trim();
    o.eq_ignore_ascii_case("flatpark") || o.to_ascii_lowercase().contains("flatpark")
}

/// The `Version:` line of `flatpak info <id>`.
pub fn parse_flatpak_info(text: &str) -> Option<String> {
    text.lines().find_map(|l| l.trim().strip_prefix("Version:").map(|v| v.trim().to_string())).filter(|v| !v.is_empty())
}

/// The package names a Craft App may be installed under with `manager`. On Arch the AUR carries
/// each app three ways: built from source (`photocraft`), the release binary (`photocraft-bin`)
/// and the latest commit (`photocraft-git`). Debian and RPM packages may use either the short stem
/// (`photocraft`) or the reverse-DNS bundle id (`ai.storyteller.photocraft`).
pub fn candidate_names(app: &App, manager: InstallSource) -> Vec<String> {
    match manager {
        InstallSource::Flatpak => vec![app.bundle_id()],
        InstallSource::Pacman => {
            app.stems().flat_map(|s| [s.to_string(), format!("{s}-bin"), format!("{s}-git")]).chain(std::iter::once(app.bundle_id())).collect()
        }
        InstallSource::Deb | InstallSource::Rpm => app.stems().map(str::to_string).chain(std::iter::once(app.bundle_id())).collect(),
        _ => app.stems().map(str::to_string).collect(),
    }
}

/// The package of `app` among `installed`, preferring the manager listed first in `order`. A user
/// install beats a system one under any of the app's ids; within a scope the current id wins.
pub fn find<'a>(app: &App, installed: &'a [Package], order: &[InstallSource]) -> Option<&'a Package> {
    order.iter().find_map(|manager| {
        let names = candidate_names(app, *manager);
        [true, false]
            .into_iter()
            .find_map(|user| names.iter().find_map(|n| installed.iter().find(|p| p.manager == *manager && p.user == user && p.name == *n)))
    })
}

/// A package's version, read the package-manager way (`0.5.0-1` is 0.5.0).
pub fn version_of(p: &Package) -> Option<version::Version> {
    version::parse_package(&p.version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_ui::testing::trial;

    #[trial]
    fn parses_flatpak_list_with_origin_and_spots_flatpark() {
        let text = "ai.storyteller.photocraft\t0.19.0\tflatpark\tuser\nai.storyteller.cadcraft\t0.16.0\tcadcraft-origin\tsystem\nai.storyteller.wordcraft\t0.14.0\t\t\n";
        let list = parse_flatpak_list_with_origin(text);
        assert_eq!(
            list,
            [
                ("ai.storyteller.photocraft".into(), "0.19.0".into(), Some("flatpark".into()), Some("user".into())),
                ("ai.storyteller.cadcraft".into(), "0.16.0".into(), Some("cadcraft-origin".into()), Some("system".into())),
                ("ai.storyteller.wordcraft".into(), "0.14.0".into(), None, None),
            ]
        );
        assert!(is_flatpark_origin("flatpark"));
        assert!(is_flatpark_origin("FlatPark"));
        assert!(is_flatpark_origin("flatpark-origin"));
        assert!(!is_flatpark_origin("cadcraft-origin"));
        assert!(!is_flatpark_origin("flathub"));
        assert!(!is_flatpark_origin(""));
    }

    #[trial]
    fn flatpak_find_prioritizes_user_over_system() {
        let photo = App::by_id("photocraft").unwrap();
        let installed = vec![
            Package {
                manager: InstallSource::Flatpak,
                name: "ai.storyteller.photocraft".into(),
                version: "0.18.0".into(),
                user: false,
                origin: Some("flathub".into()),
                installation: Some("system".into()),
            },
            Package {
                manager: InstallSource::Flatpak,
                name: "ai.storyteller.photocraft".into(),
                version: "0.19.0".into(),
                user: true,
                origin: Some("flatpark".into()),
                installation: Some("user".into()),
            },
        ];
        let found = find(photo, &installed, &[InstallSource::Flatpak]).unwrap();
        assert!(found.user);
        assert_eq!(found.version, "0.19.0");
        assert_eq!(found.origin.as_deref(), Some("flatpark"));
        assert_eq!(found.installation.as_deref(), Some("user"));
    }
}
