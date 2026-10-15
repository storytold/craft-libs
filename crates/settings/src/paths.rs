//! Choosing the configuration folder.
//!
//! First match wins: an override from the application, the `<APP>_CONFIG_DIR` variable, a
//! portable marker next to the executable, then the platform's own config folder. An empty
//! `<APP>_CONFIG_DIR`, or [`Location::Memory`], means no folder at all: nothing is read or written.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// How an application spells portable mode: the marker files that turn it on (any one, contents
/// ignored) and the folder created beside the executable. Comes from [`crate::Settings`], so an
/// app that already ships portable builds keeps its existing names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Portable {
    /// File names next to the executable that switch portable mode on.
    pub markers: &'static [&'static str],
    /// Folder beside the executable that holds the settings in portable mode.
    pub dir: &'static str,
}

impl Portable {
    /// The suite default: a `portable` file puts settings in `config/`.
    pub const DEFAULT: Self = Self { markers: crate::paths_default::MARKERS, dir: crate::paths_default::DIR };
}

/// Where the application wants its settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// Apply the rules in order.
    Auto,
    /// Use this folder, no questions asked (a `--config-dir` flag, or a library folder).
    Dir(PathBuf),
    /// Keep settings in memory only; read and write nothing (tests, demos, `<APP>_NO_PREFS`).
    Memory,
}

/// Which rule chose the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    /// [`Location::Dir`].
    Override,
    /// The named environment variable.
    EnvVar(String),
    /// The named environment variable was set but empty: isolated, nothing read or written.
    Isolated(String),
    /// [`Location::Memory`].
    Memory,
    /// A portable marker next to the executable.
    Portable,
    /// The platform's config folder.
    Platform,
    /// No platform folder could be found; the current directory is used.
    Fallback,
}

impl std::fmt::Display for Rule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Override => f.write_str("override"),
            Self::EnvVar(name) => write!(f, "environment variable {name}"),
            Self::Isolated(name) => write!(f, "{name} is empty: isolated, nothing read or written"),
            Self::Memory => f.write_str("in memory only"),
            Self::Portable => f.write_str("portable marker"),
            Self::Platform => f.write_str("platform default"),
            Self::Fallback => f.write_str("fallback to current directory"),
        }
    }
}

/// A chosen folder and the rule behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The configuration folder; `None` for [`Rule::Isolated`] and [`Rule::Memory`].
    pub dir: Option<PathBuf>,
    /// Why.
    pub rule: Rule,
}

/// Reads an environment variable by name.
pub type EnvReader = Box<dyn Fn(&str) -> Option<OsString>>;

/// What the rules look at. The defaults read the real process; tests supply their own.
pub struct Probe {
    /// Reads an environment variable.
    pub env: EnvReader,
    /// The folder holding the executable, if known.
    pub exe_dir: Option<PathBuf>,
    /// The platform config folder (`~/.config`, `Application Support`, `%APPDATA%`).
    pub platform_config_dir: Option<PathBuf>,
}

impl Default for Probe {
    fn default() -> Self {
        Self {
            env: Box::new(|name| std::env::var_os(name)),
            exe_dir: std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)),
            platform_config_dir: dirs::config_dir(),
        }
    }
}

impl std::fmt::Debug for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Probe").field("exe_dir", &self.exe_dir).field("platform_config_dir", &self.platform_config_dir).finish_non_exhaustive()
    }
}

/// The `<APP>_CONFIG_DIR` variable name for an app id (`photo-craft` → `PHOTO_CRAFT_CONFIG_DIR`).
pub fn env_var_name(app_id: &str) -> String {
    let mut name: String = app_id.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
    name.push_str("_CONFIG_DIR");
    name
}

/// Applies the rules for `app_id`, with the app's portable-mode names.
pub fn resolve(app_id: &str, portable: &Portable, location: &Location, probe: &Probe) -> Resolved {
    match location {
        Location::Dir(dir) => return Resolved { dir: Some(dir.clone()), rule: Rule::Override },
        Location::Memory => return Resolved { dir: None, rule: Rule::Memory },
        Location::Auto => {}
    }
    let var = env_var_name(app_id);
    if let Some(value) = (probe.env)(&var) {
        if value.is_empty() {
            return Resolved { dir: None, rule: Rule::Isolated(var) };
        }
        return Resolved { dir: Some(PathBuf::from(value)), rule: Rule::EnvVar(var) };
    }
    if let Some(dir) = probe.exe_dir.as_deref().and_then(|d| portable_dir(d, portable)) {
        return Resolved { dir: Some(dir), rule: Rule::Portable };
    }
    match &probe.platform_config_dir {
        Some(base) => Resolved { dir: Some(base.join(app_id)), rule: Rule::Platform },
        None => Resolved { dir: Some(PathBuf::from(".").join(app_id)), rule: Rule::Fallback },
    }
}

/// The folder a portable marker points at, when `exe_dir` has one.
///
/// On macOS the executable sits in `X.app/Contents/MacOS`; the marker is looked for next to the
/// `.app` bundle, because writing inside a signed bundle breaks its signature.
fn portable_dir(exe_dir: &Path, portable: &Portable) -> Option<PathBuf> {
    let base = bundle_parent(exe_dir).unwrap_or(exe_dir);
    portable.markers.iter().any(|m| base.join(m).is_file()).then(|| base.join(portable.dir))
}

fn bundle_parent(exe_dir: &Path) -> Option<&Path> {
    if !cfg!(target_os = "macos") || exe_dir.file_name()? != "MacOS" {
        return None;
    }
    let contents = exe_dir.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    let bundle = contents.parent()?;
    if bundle.extension()? != "app" {
        return None;
    }
    bundle.parent()
}

/// The platform cache folder for `app_id` (`%LOCALAPPDATA%` on Windows, so it does not roam).
pub fn cache_dir(app_id: &str) -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join(app_id))
}

/// The platform data folder for `app_id`, for recovery files and other non-settings state.
pub fn data_dir(app_id: &str) -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join(app_id))
}
