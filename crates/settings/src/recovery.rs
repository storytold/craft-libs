//! Keeping damaged and previous files around.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Renames a damaged `config.toml` to `config.broken-<unix-ts>.toml` beside it and returns the
/// new path. Never overwrites: a name that exists gets a `-<n>` suffix.
pub fn move_aside(path: &Path) -> io::Result<PathBuf> {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("config");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("toml");
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut n = 0u32;
    loop {
        let name = if n == 0 { format!("{stem}.broken-{ts}.{ext}") } else { format!("{stem}.broken-{ts}-{n}.{ext}") };
        let target = dir.join(name);
        if target.exists() {
            n = n.saturating_add(1);
            if n > 1000 {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many broken copies"));
            }
            continue;
        }
        std::fs::rename(path, &target)?;
        return Ok(target);
    }
}

/// The backup path for a config file: `config.toml` → `config.toml.bak`.
pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".bak");
    path.with_file_name(name)
}

/// Writes `text`, the last version that parsed, to the backup file atomically.
///
/// The caller decides what is safe to keep: a blank or unparsable file must never replace the
/// previous backup. On Unix the backup always gets the same mode as `path` (`0600` when there is
/// none), so it is never readable by more people than the settings themselves.
pub fn write_backup(path: &Path, text: &str) -> io::Result<()> {
    let backup = backup_path(path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path).map(|m| m.permissions().mode() & 0o777).unwrap_or(0o600);
        // An atomic replace keeps the existing file's mode, so narrow an old, broader backup
        // before new contents go into it, then match the config once written.
        if backup.exists() {
            std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(mode))?;
        }
        craft_atomic::atomic_write(&backup, text.as_bytes())?;
        std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        craft_atomic::atomic_write(&backup, text.as_bytes())
    }
}

/// Checks that `dir` accepts a new file by creating and removing a probe.
pub fn check_writable(dir: &Path) -> io::Result<()> {
    let probe = dir.join(format!(".craft-settings-probe-{}", std::process::id()));
    std::fs::OpenOptions::new().write(true).create_new(true).open(&probe)?;
    std::fs::remove_file(&probe)
}
