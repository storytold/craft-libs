//! Per-user runtime directory. Hand-rolled environment lookups. No `dirs` crate.

use std::path::{Path, PathBuf};

use crate::error::LinkError;

/// The directory that holds instance records, locks, and socket files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDir {
    path: PathBuf,
}

impl RuntimeDir {
    /// Resolve the directory for this process and create it when it is missing.
    pub fn resolve() -> Result<Self, LinkError> {
        Self::at(runtime_path()?)
    }

    /// Use `path` as the runtime directory. Tests use this so they do not touch
    /// the real `$XDG_RUNTIME_DIR/craft`.
    pub fn at(path: impl Into<PathBuf>) -> Result<Self, LinkError> {
        let path = path.into();
        ensure_private_dir(&path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record_file(&self, app: &str, pid: u32) -> PathBuf {
        self.path.join(format!("{app}-{pid}.json"))
    }

    pub fn lock_file(&self, app: &str, pid: u32) -> PathBuf {
        self.path.join(format!("{app}-{pid}.lock"))
    }

    pub fn socket_file(&self, app: &str, pid: u32) -> PathBuf {
        self.path.join(format!("{app}-{pid}.sock"))
    }

    /// Unix socket endpoint. The path must fit the instance-record limit of 100 bytes.
    pub fn unix_endpoint(&self, app: &str, pid: u32) -> Result<super::Endpoint, LinkError> {
        let path = self.socket_file(app, pid);
        let text = path.to_str().ok_or_else(|| LinkError::Invalid("socket path is not utf-8".into()))?.to_string();
        if text.len() > 100 {
            return Err(LinkError::Invalid(format!("socket path is {} bytes, the record allows 100", text.len())));
        }
        Ok(super::Endpoint::Unix { path: text })
    }
}

/// Set `path` to `mode` (for example `0o600`) and refuse a different result.
/// Unix socket mode cannot be set through `ListenerOptions::set_mode` (it is
/// `pub(crate)` in interprocess 2.4.4). Call this after the socket file exists.
#[cfg(unix)]
pub fn enforce_mode(path: &Path, mode: u32) -> Result<(), LinkError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    let got = std::fs::symlink_metadata(path)?.mode() & 0o777;
    if got != mode {
        return Err(LinkError::Refused { path: path.display().to_string(), reason: format!("mode is {got:o}, want {mode:o}") });
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn current_euid() -> Result<u32, LinkError> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/self/status")?;
        for line in text.lines() {
            let Some(rest) = line.strip_prefix("Uid:") else { continue };
            let mut fields = rest.split_whitespace();
            let _real = fields.next();
            let Some(eff) = fields.next() else {
                return Err(LinkError::Io("Uid line is short".into()));
            };
            return eff.parse().map_err(LinkError::io);
        }
        Err(LinkError::Io("no Uid line in /proc/self/status".into()))
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let path = std::env::temp_dir().join(format!(".craft-uid-{}", std::process::id()));
        let file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
        let uid = file.metadata()?.uid();
        drop(file);
        let _ = std::fs::remove_file(&path);
        Ok(uid)
    }
}

fn runtime_path() -> Result<PathBuf, LinkError> {
    #[cfg(target_os = "linux")]
    {
        let xdg = std::env::var_os("XDG_RUNTIME_DIR");
        Ok(linux_runtime_path(xdg.as_deref(), current_euid()?))
    }
    #[cfg(target_os = "macos")]
    {
        match std::env::var_os("TMPDIR") {
            Some(value) if !value.is_empty() && Path::new(&value).is_absolute() => Ok(PathBuf::from(value).join("craft")),
            _ => Ok(PathBuf::from("/tmp/craft")),
        }
    }
    #[cfg(windows)]
    {
        let value = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty());
        let Some(value) = value else {
            return Err(LinkError::Refused { path: "%LOCALAPPDATA%".into(), reason: "LOCALAPPDATA is unset".into() });
        };
        let base = PathBuf::from(value);
        if !base.is_absolute() {
            return Err(LinkError::Refused { path: base.display().to_string(), reason: "LOCALAPPDATA is not absolute".into() });
        }
        Ok(base.join("Craft").join("run"))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        Err(LinkError::Refused { path: String::new(), reason: "no runtime directory for this platform".into() })
    }
}

#[cfg(target_os = "linux")]
fn linux_runtime_path(xdg: Option<&std::ffi::OsStr>, euid: u32) -> PathBuf {
    if let Some(xdg) = xdg {
        let path = PathBuf::from(xdg);
        if !xdg.is_empty() && path.is_absolute() {
            return path.join("craft");
        }
    }
    PathBuf::from(format!("/tmp/craft-{euid}"))
}

pub(crate) fn ensure_private_dir(path: &Path) -> Result<(), LinkError> {
    if is_symlink(path) {
        return Err(LinkError::Refused { path: path.display().to_string(), reason: "runtime directory is a symlink".into() });
    }
    let created = match create_dir(path) {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(err) => return Err(LinkError::io(err)),
    };
    if is_symlink(path) {
        return Err(LinkError::Refused { path: path.display().to_string(), reason: "runtime directory is a symlink".into() });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if created {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
        }
        verify_unix_dir(path)?;
    }
    #[cfg(not(unix))]
    {
        let _ = created;
        let meta = std::fs::metadata(path)?;
        if !meta.is_dir() {
            return Err(LinkError::Refused { path: path.display().to_string(), reason: "not a directory".into() });
        }
    }
    Ok(())
}

fn create_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir(path)
    }
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

#[cfg(unix)]
fn verify_unix_dir(path: &Path) -> Result<(), LinkError> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_dir() {
        return Err(LinkError::Refused { path: path.display().to_string(), reason: "not a directory".into() });
    }
    let uid = current_euid()?;
    if meta.uid() != uid {
        return Err(LinkError::Refused { path: path.display().to_string(), reason: format!("owner is {}, euid is {uid}", meta.uid()) });
    }
    let mode = meta.mode() & 0o777;
    if mode != 0o700 {
        return Err(LinkError::Refused { path: path.display().to_string(), reason: format!("mode is {mode:o}, want 700") });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let mut bytes = [0u8; 4];
        getrandom::getrandom(&mut bytes).unwrap();
        let path = PathBuf::from(format!("/tmp/cl-{label}-{}", crate::auth::encode_hex(&bytes)));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn creates_0700_and_refuses_a_loose_dir() {
        let path = scratch("dir");
        let dir = RuntimeDir::at(&path).unwrap();
        assert_eq!(dir.path(), path.as_path());
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            assert_eq!(std::fs::symlink_metadata(&path).unwrap().mode() & 0o777, 0o700);
            let loose = scratch("loose");
            std::fs::create_dir(&loose).unwrap();
            std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o755)).unwrap();
            let err = RuntimeDir::at(&loose).unwrap_err();
            assert!(matches!(err, LinkError::Refused { .. }), "{err}");
            let _ = std::fs::remove_dir_all(&loose);
        }
        let _ = std::fs::remove_dir_all(&path);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let real = scratch("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = scratch("link");
        symlink(&real, &link).unwrap();
        assert!(RuntimeDir::at(&link).is_err());
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_dir_all(&real);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn xdg_fallback_is_pure() {
        assert_eq!(linux_runtime_path(Some(std::ffi::OsStr::new("/run/user/1000")), 5), PathBuf::from("/run/user/1000/craft"));
        assert_eq!(linux_runtime_path(Some(std::ffi::OsStr::new("")), 5), PathBuf::from("/tmp/craft-5"));
        assert_eq!(linux_runtime_path(Some(std::ffi::OsStr::new("relative")), 5), PathBuf::from("/tmp/craft-5"));
        assert_eq!(linux_runtime_path(None, 9), PathBuf::from("/tmp/craft-9"));
    }

    #[test]
    fn resolve_points_at_a_private_craft_dir() {
        let dir = RuntimeDir::resolve().unwrap();
        assert!(dir.path().ends_with("craft"), "{}", dir.path().display());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(std::fs::symlink_metadata(dir.path()).unwrap().mode() & 0o777, 0o700);
        }
    }
}
