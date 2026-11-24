//! Where an installer keeps its own files: downloads, records, logs.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::Reason;
use crate::app::App;

/// An installer's folders.
#[derive(Clone, Debug)]
pub struct Paths {
    /// Records of what was installed, and tarball installs.
    pub data: PathBuf,
    /// Saved release answers and in-progress downloads.
    pub cache: PathBuf,
}

impl Paths {
    /// Discovers paths for an app, respecting portable mode if `portable.txt` is next to current exe.
    pub fn for_app(app: &App) -> Result<Self, String> {
        Self::for_app_reason(app).map_err(|reason| reason.to_string())
    }

    /// Structured internal counterpart of [`Self::for_app`].
    pub(crate) fn for_app_reason(app: &App) -> Result<Self, Reason> {
        if let Ok(exe) = std::env::current_exe() {
            return Self::for_exe_reason(app, &exe);
        }
        let dir = if cfg!(any(target_os = "macos", target_os = "windows")) { app.name } else { app.id };
        Self::per_user_reason(dir)
    }

    /// Discovers paths for an app from a specific executable path.
    pub fn for_exe(app: &App, exe: &Path) -> Result<Self, String> {
        Self::for_exe_reason(app, exe).map_err(|reason| reason.to_string())
    }

    /// Structured internal counterpart of [`Self::for_exe`].
    pub(crate) fn for_exe_reason(app: &App, exe: &Path) -> Result<Self, Reason> {
        if let Some(parent) = exe.parent()
            && parent.join("portable.txt").is_file()
        {
            let data_dir_name = format!("{}Data", app.name.replace(' ', ""));
            return Ok(Self::under(&parent.join(data_dir_name)));
        }
        let dir = if cfg!(any(target_os = "macos", target_os = "windows")) { app.name } else { app.id };
        Self::per_user_reason(dir)
    }

    /// The per-user folders named `dir` (`ArtCraft Launcher` on macOS and Windows,
    /// `artcraft-launcher` elsewhere, by convention).
    pub fn per_user(dir: &str) -> Result<Self, String> {
        Self::per_user_reason(dir).map_err(|reason| reason.to_string())
    }

    /// Structured internal counterpart of [`Self::per_user`].
    pub(crate) fn per_user_reason(dir: &str) -> Result<Self, Reason> {
        let data = crate::folders::data_local_dir().ok_or_else(|| Reason::new("no per-user data folder on this system", &[]))?.join(dir);
        let cache = crate::folders::cache_dir().ok_or_else(|| Reason::new("no per-user cache folder on this system", &[]))?.join(dir);
        Ok(Self { data, cache })
    }

    /// Everything under one folder (portable mode, tests).
    pub fn under(root: &std::path::Path) -> Self {
        Self { data: root.join("data"), cache: root.join("cache") }
    }

    /// Folder for partial and finished downloads.
    pub fn downloads(&self) -> PathBuf {
        self.cache.join("downloads")
    }

    /// Log files, and the error of a failed self-update waiting for the next start.
    pub fn logs(&self) -> PathBuf {
        self.data.join("logs")
    }

    /// Where tarball installs (FreeBSD, Linux without AppImage) are unpacked: `<data>/apps/<app>`.
    pub fn apps(&self) -> PathBuf {
        self.data.join("apps")
    }

    /// The folder an app unpacked in [`Self::apps`] sees as its home: `<data>/apps/<app>/home`.
    /// A Craft App keeps its settings in the XDG folders (`~/.config`, `~/.local/share`,
    /// `~/.cache`), so pointing `HOME` and those variables here keeps every app's configuration
    /// beside it, inside the folder the installer owns.
    #[cfg(target_os = "linux")]
    pub fn app_home(&self, id: &str) -> PathBuf {
        self.apps().join(id).join("home")
    }
}

/// Writes `bytes` to `path` via a temporary file and a rename, so a crash never leaves half a file.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    write_atomic_reason(path, bytes).map_err(|reason| reason.to_string())
}

/// Structured internal atomic-write failure.
pub(crate) fn write_atomic_reason(path: &std::path::Path, bytes: &[u8]) -> Result<(), Reason> {
    let dir = path.parent().ok_or_else(|| Reason::new("no parent folder", &[]))?;
    std::fs::create_dir_all(dir)
        .map_err(|e| Reason::data(e.to_string()).context("couldn't create {path} ({reason})", &[("path", &dir.display().to_string())]))?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| {
        Reason::data(e.to_string()).context("couldn't create a temporary file in {path} ({reason})", &[("path", &dir.display().to_string())])
    })?;
    tmp.write_all(bytes)
        .map_err(|e| Reason::data(e.to_string()).context("couldn't write {path} ({reason})", &[("path", &tmp.path().display().to_string())]))?;
    tmp.as_file()
        .sync_all()
        .map_err(|e| Reason::data(e.to_string()).context("couldn't sync {path} ({reason})", &[("path", &tmp.path().display().to_string())]))?;
    tmp.persist(path)
        .map_err(|e| Reason::data(e.to_string()).context("couldn't replace {path} ({reason})", &[("path", &path.display().to_string())]))?;
    Ok(())
}

#[cfg(test)]
#[cfg(unix)]
pub(crate) mod tests {
    use super::*;

    #[craft_ui::testing::trial]
    fn atomic_write_does_not_follow_a_temporary_file_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");
        let victim = dir.path().join("unrelated.txt");
        std::fs::write(&victim, b"keep this file").unwrap();
        std::os::unix::fs::symlink(&victim, path.with_extension("tmp")).unwrap();
        write_atomic(&path, b"new manifest").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep this file");
        assert_eq!(std::fs::read(&path).unwrap(), b"new manifest");
    }
}
