//! The per-user folders, from the environment, so no extra crate (and its licence) is needed.

use std::path::PathBuf;

/// The user's home folder.
pub fn home_dir() -> Option<PathBuf> {
    std::env::home_dir().filter(|h| !h.as_os_str().is_empty())
}

/// An absolute folder from the environment variable `name` (an XDG one must be absolute).
fn absolute_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// The known folder Local AppData, which follows a redirected profile (the environment variable
/// is only the fallback).
#[cfg(windows)]
fn local_app_data() -> Option<PathBuf> {
    use winsafe::{SHGetKnownFolderPath, co};
    SHGetKnownFolderPath(&co::KNOWNFOLDERID::LocalAppData, co::KF::DEFAULT, None)
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| absolute_env("LOCALAPPDATA"))
}

#[cfg(not(windows))]
fn local_app_data() -> Option<PathBuf> {
    absolute_env("LOCALAPPDATA")
}

/// Where per-user data lives on this machine.
pub fn data_local_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        local_app_data()
    } else if cfg!(target_os = "macos") {
        home_dir().map(|h| h.join("Library").join("Application Support"))
    } else {
        absolute_env("XDG_DATA_HOME").or_else(|| home_dir().map(|h| h.join(".local").join("share")))
    }
}

/// The host's per-user data folder, also from inside a Flatpak, where `XDG_DATA_HOME` points into
/// the sandbox (`~/.var/app/<id>/data`). Files other programs on the host must find go here.
pub fn host_data_local_dir() -> Option<PathBuf> {
    pick_host_data_dir(std::path::Path::new("/.flatpak-info").exists(), absolute_env("HOST_XDG_DATA_HOME"), home_dir(), data_local_dir())
}

/// [`host_data_local_dir`] with its inputs passed in, so every CI OS can test the Flatpak case.
fn pick_host_data_dir(in_flatpak: bool, host_xdg: Option<PathBuf>, home: Option<PathBuf>, local: Option<PathBuf>) -> Option<PathBuf> {
    if in_flatpak {
        return host_xdg.or_else(|| home.map(|h| h.join(".local").join("share")));
    }
    local
}

/// Where per-user caches live on this machine.
pub fn cache_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        local_app_data()
    } else if cfg!(target_os = "macos") {
        home_dir().map(|h| h.join("Library").join("Caches"))
    } else {
        absolute_env("XDG_CACHE_HOME").or_else(|| home_dir().map(|h| h.join(".cache")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folders_are_absolute_when_there_is_a_home() {
        if home_dir().is_some() {
            assert!(data_local_dir().is_some_and(|p| p.is_absolute()));
            assert!(cache_dir().is_some_and(|p| p.is_absolute()));
        }
    }

    #[test]
    fn inside_a_flatpak_the_hosts_data_folder_is_used() {
        let home = Some(PathBuf::from("/home/u"));
        let sandboxed = Some(PathBuf::from("/home/u/.var/app/ai.storyteller.artcraft-launcher/data"));
        let host = Some(PathBuf::from("/srv/u/data"));
        assert_eq!(pick_host_data_dir(true, host.clone(), home.clone(), sandboxed.clone()), host);
        assert_eq!(pick_host_data_dir(true, None, home.clone(), sandboxed.clone()), Some(PathBuf::from("/home/u/.local/share")));
        assert_eq!(pick_host_data_dir(false, host, home, sandboxed.clone()), sandboxed);
        assert_eq!(pick_host_data_dir(true, None, None, None), None);
    }
}
