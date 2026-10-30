//! Checks before this crate runs a program.
//!
//! The launcher finds an app binary through `CRAFT_<APP>_BIN`, `installed.json`, `PATH` and a few
//! fixed directories. A same-user process can write most of those places, so a name that resolves
//! is not proof that the file is the app. These checks do not stop a process of the same user,
//! which can do anything the user can. They stop two mistakes: running a file that another local
//! user can replace, and running a file that a relative path or an empty `PATH` entry picked from
//! the current directory.

use std::path::Path;

use crate::error::LinkError;

/// `Ok` when `path` is an absolute path to a regular, executable file that only this user or root
/// can change, in a directory that only this user or root can change.
///
/// Symlinks are followed, and both the link's directory and the target's directory are checked.
/// A group-writable file passes: most Linux distributions give each user a private group, and the
/// check cannot see group members. A file or directory that any user can write fails.
///
/// Non-Unix targets only check that the path is an absolute regular file. The Windows ACL on the
/// install directory is the user's check there.
pub fn check_executable(path: &Path) -> Result<(), LinkError> {
    if !path.is_absolute() {
        return Err(refused(path, "path is not absolute"));
    }
    #[cfg(unix)]
    {
        unix::check(path)
    }
    #[cfg(not(unix))]
    {
        if std::fs::metadata(path).is_ok_and(|meta| meta.is_file()) { Ok(()) } else { Err(refused(path, "not a file")) }
    }
}

fn refused(path: &Path, reason: &str) -> LinkError {
    LinkError::Refused { path: path.display().to_string(), reason: reason.to_string() }
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;

    use super::refused;
    use crate::error::LinkError;

    const WORLD_WRITE: u32 = 0o002;
    const ANY_EXEC: u32 = 0o111;

    pub(super) fn check(path: &Path) -> Result<(), LinkError> {
        let euid = crate::discovery::current_euid()?;
        let meta = std::fs::metadata(path).map_err(|err| refused(path, &format!("cannot stat: {err}")))?;
        if !meta.is_file() {
            return Err(refused(path, "not a regular file"));
        }
        if meta.mode() & ANY_EXEC == 0 {
            return Err(refused(path, "not executable"));
        }
        owner_and_mode(path, &meta, euid, "file")?;
        let real = std::fs::canonicalize(path).map_err(|err| refused(path, &format!("cannot resolve: {err}")))?;
        for candidate in [path, real.as_path()] {
            let Some(parent) = candidate.parent() else { continue };
            let dir = std::fs::metadata(parent).map_err(|err| refused(parent, &format!("cannot stat: {err}")))?;
            owner_and_mode(parent, &dir, euid, "directory")?;
        }
        Ok(())
    }

    fn owner_and_mode(path: &Path, meta: &std::fs::Metadata, euid: u32, what: &str) -> Result<(), LinkError> {
        if meta.uid() != euid && meta.uid() != 0 {
            return Err(refused(path, &format!("{what} is owned by uid {}, not uid {euid} or root", meta.uid())));
        }
        if meta.mode() & WORLD_WRITE != 0 {
            return Err(refused(path, &format!("{what} is writable by every user")));
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let mut bytes = [0u8; 4];
        getrandom::getrandom(&mut bytes).unwrap();
        let path = PathBuf::from(format!("/tmp/cl-trust-{label}-{}", crate::auth::encode_hex(&bytes)));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn program(dir: &Path, mode: u32) -> PathBuf {
        let file = dir.join("app");
        std::fs::write(&file, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
        file
    }

    #[test]
    fn a_private_executable_passes() {
        let dir = scratch("ok");
        assert!(check_executable(&program(&dir, 0o755)).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn relative_and_empty_paths_fail() {
        assert!(matches!(check_executable(Path::new("app")), Err(LinkError::Refused { .. })));
        assert!(matches!(check_executable(Path::new("")), Err(LinkError::Refused { .. })));
        assert!(matches!(check_executable(Path::new("./app")), Err(LinkError::Refused { .. })));
    }

    #[test]
    fn loose_modes_and_non_programs_fail() {
        let dir = scratch("modes");
        let plain = program(&dir, 0o644);
        assert!(check_executable(&plain).is_err(), "not executable");
        let open = program(&dir, 0o777);
        assert!(check_executable(&open).is_err(), "world-writable file");
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(check_executable(&open).is_ok());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(check_executable(&open).is_err(), "world-writable directory");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();
        assert!(check_executable(&open).is_err(), "sticky does not excuse a world-writable directory");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(check_executable(&dir).is_err(), "a directory is not a program");
        assert!(check_executable(&dir.join("missing")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_symlink_into_a_loose_directory_fails() {
        let good = scratch("linkgood");
        let loose = scratch("linkloose");
        let target = program(&loose, 0o755);
        std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o777)).unwrap();
        let link = good.join("app");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(check_executable(&link).is_err());
        std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(check_executable(&link).is_ok());
        std::fs::remove_dir_all(&good).unwrap();
        std::fs::remove_dir_all(&loose).unwrap();
    }

    #[test]
    fn system_programs_pass() {
        assert!(check_executable(Path::new("/usr/bin/env")).is_ok() || check_executable(Path::new("/bin/sh")).is_ok());
    }
}
