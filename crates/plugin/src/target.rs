//! Whether a plug-in (its manifest, or a catalog release) was written for the app asking: its
//! kind, its entry in `apps`, the API version and the oldest app version it needs.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use crate::bundle::{compare_versions, parse_version};
use crate::manifest::{AppTarget, Manifest};

/// The app asking whether it can run a plug-in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppInfo<'a> {
    /// Its id as manifests name it in `apps` (`figcraft`, `designcraft`…).
    pub id: &'a str,
    /// Its version (`0.30.0`; a pre-release or build suffix, `-dev` or `+abc`, is ignored).
    pub version: &'a str,
    /// The API versions it runs, for every kind in [`AppInfo::kinds`]: `1..=API_VERSION` for
    /// command plug-ins.
    pub api: RangeInclusive<u32>,
    /// The kinds it runs (`command`, and its own).
    pub kinds: &'a [&'a str],
    /// Whether it runs plug-ins that name no app (manifests written before `apps`): FigCraft does,
    /// with the manifest's own `api`; an app new to plug-ins need not.
    pub accept_legacy: bool,
}

/// How a plug-in targets the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// It names the app in `apps`, with this API version.
    Named { api: u32 },
    /// It names no app (a legacy manifest); its top-level `api` is in the app's range.
    Legacy { api: u32 },
}

impl Target {
    /// The API version the plug-in was written for, in this app.
    pub fn api(&self) -> u32 {
        match self {
            Target::Named { api } | Target::Legacy { api } => *api,
        }
    }
}

/// `version` without a pre-release or build suffix (`0.18.0-dev` is `0.18.0`).
fn release_part(version: &str) -> &str {
    version.trim().split(['-', '+']).next().unwrap_or_default()
}

/// Whether `app` can run a plug-in of `kind` whose `apps` are these, falling back to `legacy`
/// (its own API and oldest app version) when it names no app.
pub(crate) fn check_target(
    kind: &str,
    apps: &BTreeMap<String, AppTarget>,
    legacy: Option<(u32, Option<&str>)>,
    app: &AppInfo<'_>,
) -> Result<Target, String> {
    if !app.kinds.contains(&kind) {
        return Err(format!("it is a `{kind}` plug-in, which {} doesn't run", app.id));
    }
    let (target, api, min_app) = match (apps.get(app.id), legacy) {
        (Some(t), _) => (Target::Named { api: t.api }, t.api, t.min_app.as_deref()),
        (None, Some((api, min_app))) if apps.is_empty() && app.accept_legacy => (Target::Legacy { api }, api, min_app),
        (None, _) if apps.is_empty() => return Err(format!("it doesn't say it was written for {} (no `apps` in its manifest)", app.id)),
        (None, _) => {
            let names: Vec<&str> = apps.keys().map(String::as_str).collect();
            return Err(format!("it was written for {}, not {}", names.join(", "), app.id));
        }
    };
    if !app.api.contains(&api) {
        return Err(format!("it needs plug-in API {api}, and this {} runs API {}–{}", app.id, app.api.start(), app.api.end()));
    }
    if let Some(min) = min_app {
        let running = release_part(app.version);
        if parse_version(running).is_none() {
            return Err(format!("it needs {} {min} or later, and this version ({}) can't be compared", app.id, app.version));
        }
        if compare_versions(min, running).is_none_or(|o| o.is_gt()) {
            return Err(format!("it needs {} {min} or later (this is {running})", app.id));
        }
    }
    Ok(target)
}

impl Manifest {
    /// Whether `app` can run this plug-in: its kind is one the app runs, it names the app in
    /// `apps` (or names no app and the app accepts legacy manifests), with an API version the app
    /// runs, and the app is at least its `minApp`. The reason, in a sentence, when not.
    pub fn target(&self, app: &AppInfo<'_>) -> Result<Target, String> {
        check_target(&self.kind, &self.apps, Some((self.api, None)), app)
    }
}
