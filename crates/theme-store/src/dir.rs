//! Where the suite theme folder is.

#[cfg(not(target_arch = "wasm32"))]
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Overrides the suite theme folder. Set but empty: themes stay in memory and nothing on disk is
/// read or written, so tests and agents never touch the real folder.
pub const SUITE_DIR_ENV: &str = "CRAFT_SUITE_DIR";

/// Which rule chose the folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirSource {
    /// [`SUITE_DIR_ENV`] named a folder.
    Env,
    /// [`SUITE_DIR_ENV`] was set but empty: in memory.
    IsolatedByEnv,
    /// The app runs without a disk store (tests, portable installs): in memory.
    IsolatedByApp,
    /// `ArtCraft/themes` in the platform's local config folder.
    Platform,
    /// The platform has no config folder: in memory.
    NoPlatformFolder,
    /// [`SuiteDir::at`] or [`SuiteDir::memory`].
    Explicit,
    /// Web builds: in memory.
    Web,
}

/// The suite theme folder, or in memory when there is none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuiteDir {
    path: Option<PathBuf>,
    source: DirSource,
}

impl SuiteDir {
    /// The folder the desktop shells open, in this order: [`SUITE_DIR_ENV`] (empty means in
    /// memory); in memory when `app_isolated`; else `ArtCraft/themes` under the local config
    /// folder (`~/.config` on Linux and the BSDs, `~/Library/Application Support` on macOS,
    /// `%LOCALAPPDATA%` on Windows).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn resolve(app_isolated: bool) -> SuiteDir {
        Self::resolve_from(std::env::var_os(SUITE_DIR_ENV), app_isolated, dirs::config_local_dir())
    }

    /// Web builds have no folder: always in memory.
    #[cfg(target_arch = "wasm32")]
    pub fn resolve(_app_isolated: bool) -> SuiteDir {
        SuiteDir { path: None, source: DirSource::Web }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn resolve_from(env: Option<OsString>, app_isolated: bool, config_local: Option<PathBuf>) -> SuiteDir {
        match env {
            Some(v) if v.is_empty() => SuiteDir { path: None, source: DirSource::IsolatedByEnv },
            Some(v) => SuiteDir { path: Some(PathBuf::from(v)), source: DirSource::Env },
            None if app_isolated => SuiteDir { path: None, source: DirSource::IsolatedByApp },
            None => match config_local {
                Some(base) => SuiteDir { path: Some(base.join("ArtCraft").join("themes")), source: DirSource::Platform },
                None => SuiteDir { path: None, source: DirSource::NoPlatformFolder },
            },
        }
    }

    /// A given folder, such as a test's temporary one.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn at(path: impl Into<PathBuf>) -> SuiteDir {
        SuiteDir { path: Some(path.into()), source: DirSource::Explicit }
    }

    pub fn memory() -> SuiteDir {
        SuiteDir { path: None, source: DirSource::Explicit }
    }

    /// `None` in memory.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn is_memory(&self) -> bool {
        self.path.is_none()
    }

    pub fn source(&self) -> DirSource {
        self.source
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn isolation_rules() {
        let platform = || Some(PathBuf::from("/home/u/.config"));
        let empty = SuiteDir::resolve_from(Some(OsString::new()), false, platform());
        assert_eq!((empty.path(), empty.source()), (None, DirSource::IsolatedByEnv));
        let env = SuiteDir::resolve_from(Some("/t/suite".into()), true, platform());
        assert_eq!((env.path(), env.source()), (Some(Path::new("/t/suite")), DirSource::Env));
        let app = SuiteDir::resolve_from(None, true, platform());
        assert_eq!((app.path(), app.source()), (None, DirSource::IsolatedByApp));
        let plat = SuiteDir::resolve_from(None, false, platform());
        assert_eq!(plat.path(), Some(Path::new("/home/u/.config/ArtCraft/themes")));
        assert_eq!(SuiteDir::resolve_from(None, false, None).source(), DirSource::NoPlatformFolder);
    }
}
