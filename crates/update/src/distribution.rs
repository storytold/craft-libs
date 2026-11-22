//! Copies that a distribution's package manager updates, such as a Nix package.
//!
//! A copy installed by a package manager cannot update itself, and a release downloaded by hand
//! may not even start there (a NixOS binary needs its store paths). The package's wrapper script
//! says so by setting both variables before it starts the app:
//!
//! ```sh
//! export CRAFT_UPDATE_MANAGED_BY='Nix'
//! export CRAFT_UPDATE_MANAGED_HINT='Update your Nix sources and rebuild to get the new version.'
//! ```
//!
//! [`MANAGED_BY_VAR`] names the package manager and [`MANAGED_HINT_VAR`] says how to get the new
//! version through it. Both are shown trimmed but otherwise as given, untranslated. The sentence
//! around them goes through the app's translator, which falls back to English. Plain ASCII is
//! safest for both values, since an app's fonts may lack other characters.
//!
//! While both are set the update check still runs, and a newer release still opens the startup
//! prompt with its release notes, but the prompt offers no Download and no Update and Restart:
//! it shows the hint instead. [`self_update::current`](crate::self_update::current) and
//! [`self_update::is_self_updating`](crate::self_update::is_self_updating) report a copy that
//! cannot update itself, whatever else they detect (an `APPIMAGE` variable inherited from
//! another program, say), and [`self_update::update`](crate::self_update::update) refuses to
//! install, so no self-update starts. An app with update screens of its own reads [`distribution`].
//! Programs started through [`process::command`](crate::process::command) (which the launcher
//! uses to start an app, run a Flatpak or open a file) and apps started by craft-ipc's
//! `launch_at` don't inherit the variables: a Craft App gets them from its own wrapper.
//!
//! When one of the two has a value and the other is unset or empty, both are ignored and a
//! warning is logged. When neither has a value, nothing is logged. The launcher's version choice
//! ([`crate::pins`]) still comes first: an app it keeps on another version offers nothing at all.

use std::ffi::OsString;
use std::sync::OnceLock;

/// The package manager that updates this copy (`Nix`).
pub const MANAGED_BY_VAR: &str = "CRAFT_UPDATE_MANAGED_BY";

/// How to get a new version through that package manager, a sentence or two.
pub const MANAGED_HINT_VAR: &str = "CRAFT_UPDATE_MANAGED_HINT";

/// A package manager that updates this copy, and how to update through it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Distribution {
    /// [`MANAGED_BY_VAR`], trimmed.
    pub manager: String,
    /// [`MANAGED_HINT_VAR`], trimmed.
    pub hint: String,
}

impl Distribution {
    /// The distribution the two variables' values describe: both set and non-empty after trimming,
    /// else `None`. A value that is not Unicode counts as unset.
    pub fn from_values(manager: Option<OsString>, hint: Option<OsString>) -> Option<Self> {
        match (value(manager), value(hint)) {
            (Some(manager), Some(hint)) => Some(Self { manager, hint }),
            _ => None,
        }
    }
}

/// A variable's value, trimmed; `None` when it is unset, empty or not Unicode.
fn value(raw: Option<OsString>) -> Option<String> {
    raw.and_then(|v| v.into_string().ok()).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// The package manager that updates this running copy, from [`MANAGED_BY_VAR`] and
/// [`MANAGED_HINT_VAR`], read once per process.
pub fn distribution() -> Option<Distribution> {
    static READ: OnceLock<Option<Distribution>> = OnceLock::new();
    if let Some(read) = READ.get() {
        return read.clone();
    }
    let (manager, hint) = (std::env::var_os(MANAGED_BY_VAR), std::env::var_os(MANAGED_HINT_VAR));
    let read = Distribution::from_values(manager.clone(), hint.clone());
    // Logged only once the value is published, so a logger that asks again finds it.
    if READ.set(read.clone()).is_ok() {
        match (&read, value(manager).is_some(), value(hint).is_some()) {
            (Some(distribution), ..) => log::info!("{} updates this copy; it does not update itself", distribution.manager),
            (None, true, false) | (None, false, true) => {
                log::warn!("only one of {MANAGED_BY_VAR} and {MANAGED_HINT_VAR} has a value, so both are ignored; set both");
            }
            (None, ..) => {}
        }
    }
    READ.get().cloned().flatten()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn values(manager: Option<&str>, hint: Option<&str>) -> Option<Distribution> {
        Distribution::from_values(manager.map(OsString::from), hint.map(OsString::from))
    }

    #[test]
    fn both_values_are_needed_and_kept_as_given_but_trimmed() {
        let hint = "Update your Nix sources and rebuild to get the new version.";
        assert_eq!(values(Some(" Nix\n"), Some(hint)), Some(Distribution { manager: "Nix".into(), hint: hint.into() }));
        for (manager, hint) in [(Some("Nix"), None), (None, Some(hint)), (Some("Nix"), Some("  ")), (Some(""), Some(hint)), (None, None)] {
            assert_eq!(values(manager, hint), None, "{manager:?} {hint:?}");
        }
    }

    /// Runs the test `name` of this test binary alone in a child process, set up by `configure`,
    /// and checks that it ran and passed. Tests that need the wrapper's variables use it, so no
    /// test changes its own environment.
    pub(crate) fn run_alone(name: &str, configure: impl FnOnce(&mut std::process::Command)) {
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", name, "--test-threads=1"]).env_remove(MANAGED_BY_VAR).env_remove(MANAGED_HINT_VAR);
        configure(&mut cmd);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{name} failed in its child process: {stdout}{}", String::from_utf8_lossy(&output.stderr));
        assert!(stdout.contains("test result: ok. 1 passed"), "{name} did not run in its child process: {stdout}");
    }
}
