//! Checks before this crate runs a program.
//!
//! The launcher finds an app binary through `CRAFT_<APP>_BIN`, `installed.json`, `PATH` and a few
//! fixed directories. A same-user process can write most of those places, so a name that resolves
//! is not proof that the file is the app. These checks do not stop a process of the same user,
//! which can do anything the user can. They stop two mistakes: running a file that any local
//! user can replace, and running a file that a relative path or an empty `PATH` entry picked from
//! the current directory.
//!
//! The check covers the whole path. Every directory from `/` down to the file must belong to this
//! user or root. A directory that every user can write must have the sticky bit, and the directory
//! that holds the file may never be writable by every user. Without this, a user who controls an
//! ancestor of a private `bin` directory could rename `bin` and plant a program of their own.
//!
//! Group write is not checked. A file or directory that is group-writable passes, so every other
//! member of its group can replace the program. The check accepts that because most Linux
//! distributions give each user a private group with no other members, and the check cannot see
//! who is in a group.

use std::path::{Path, PathBuf};

use crate::error::LinkError;

/// `Ok` when `path` is an absolute path to a regular, executable file, owned by this user or root
/// and not writable by every user, and every directory above it passes the same owner rule.
///
/// Symlinks are followed. The check walks every component of the path as given and of the real
/// path, up to `/`. Each component must belong to this user or root. A directory that every user
/// can write passes only with the sticky bit, and only when it is not the directory that holds
/// the file, because in a sticky directory a user cannot rename an entry that belongs to someone
/// else. A path with a `..` component fails, because the walk cannot tell which directories it
/// crosses. A group-writable file or directory passes, and the other members of its group can
/// replace it. Most Linux distributions give each user a private group, and the check cannot see
/// group members.
///
/// Non-Unix targets only check that the path is an absolute regular file. The Windows ACL on the
/// install directory is the user's check there.
///
/// To start the program, call [`verified_executable`] and run the path it returns. That leaves
/// no symlink for another user to swap between the check and the spawn.
pub fn check_executable(path: &Path) -> Result<(), LinkError> {
    verified_executable(path).map(|_| ())
}

/// [`check_executable`], then the real path of the file that passed.
///
/// Run the returned path, not `path`. Every component of it was checked, and none is a symlink.
/// On non-Unix targets the returned path is `path`.
pub fn verified_executable(path: &Path) -> Result<PathBuf, LinkError> {
    if !path.is_absolute() {
        return Err(refused(path, "path is not absolute"));
    }
    #[cfg(unix)]
    {
        unix::check(path)
    }
    #[cfg(not(unix))]
    {
        if std::fs::metadata(path).is_ok_and(|meta| meta.is_file()) { Ok(path.to_path_buf()) } else { Err(refused(path, "not a file")) }
    }
}

fn refused(path: &Path, reason: &str) -> LinkError {
    LinkError::Refused { path: path.display().to_string(), reason: reason.to_string() }
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::MetadataExt;
    use std::path::{Component, Path, PathBuf};

    use super::refused;
    use crate::error::LinkError;

    const WORLD_WRITE: u32 = 0o002;
    const STICKY: u32 = 0o1000;
    const ANY_EXEC: u32 = 0o111;

    pub(super) fn check(path: &Path) -> Result<PathBuf, LinkError> {
        check_as(path, crate::discovery::current_euid()?)
    }

    /// [`check`] for a caller whose effective uid is `euid`.
    pub(super) fn check_as(path: &Path, euid: u32) -> Result<PathBuf, LinkError> {
        let meta = std::fs::metadata(path).map_err(|err| refused(path, &format!("cannot stat: {err}")))?;
        if !meta.is_file() {
            return Err(refused(path, "not a regular file"));
        }
        if meta.mode() & ANY_EXEC == 0 {
            return Err(refused(path, "not executable"));
        }
        owner_and_mode(path, &meta, euid, "file")?;
        let real = std::fs::canonicalize(path).map_err(|err| refused(path, &format!("cannot resolve: {err}")))?;
        walk(path, euid)?;
        if real != path {
            walk(&real, euid)?;
        }
        Ok(real)
    }

    /// Check `path` and every component above it, from the file up to `/`.
    ///
    /// A component is checked by what it is (`lstat`, so a symlink must belong to this user or
    /// root) and by what it points to. The directory that holds a component must not let another
    /// user rename that component: it is not writable by every user, or it has the sticky bit.
    /// The directory that holds the file itself gets no sticky exception.
    fn walk(path: &Path, euid: u32) -> Result<(), LinkError> {
        if path.components().any(|part| matches!(part, Component::ParentDir | Component::CurDir)) {
            return Err(refused(path, "path has a `.` or `..` component"));
        }
        for component in path.ancestors() {
            let link = std::fs::symlink_metadata(component).map_err(|err| refused(component, &format!("cannot stat: {err}")))?;
            owner(component, &link, euid, "path component")?;
            if link.file_type().is_symlink() {
                let target = std::fs::metadata(component).map_err(|err| refused(component, &format!("cannot stat: {err}")))?;
                owner(component, &target, euid, "path component")?;
            }
            let Some(parent) = component.parent() else { continue };
            let dir = std::fs::metadata(parent).map_err(|err| refused(parent, &format!("cannot stat: {err}")))?;
            owner(parent, &dir, euid, "directory")?;
            let sticky_allowed = component != path && dir.mode() & STICKY != 0;
            if dir.mode() & WORLD_WRITE != 0 && !sticky_allowed {
                return Err(refused(parent, "directory is writable by every user"));
            }
        }
        Ok(())
    }

    fn owner(path: &Path, meta: &std::fs::Metadata, euid: u32, what: &str) -> Result<(), LinkError> {
        if meta.uid() != euid && meta.uid() != 0 {
            return Err(refused(path, &format!("{what} is owned by uid {}, not uid {euid} or root", meta.uid())));
        }
        Ok(())
    }

    fn owner_and_mode(path: &Path, meta: &std::fs::Metadata, euid: u32, what: &str) -> Result<(), LinkError> {
        owner(path, meta, euid, what)?;
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

    /// `<top>/mid/bin/app`, every level 0755 and owned by this user. Returns `(top, app)`.
    fn nested(label: &str) -> (PathBuf, PathBuf) {
        let top = scratch(label);
        let bin = top.join("mid").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for dir in [top.join("mid"), bin.clone()] {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (top, program(&bin, 0o755))
    }

    fn chmod(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn a_world_writable_ancestor_above_private_directories_fails() {
        let (top, app) = nested("anc-open");
        assert!(check_executable(&app).is_ok());
        for open in [top.clone(), top.join("mid")] {
            chmod(&open, 0o777);
            let err = check_executable(&app).expect_err("a user could rename the directory below");
            assert!(err.to_string().contains("writable by every user"), "{err}");
            chmod(&open, 0o755);
        }
        assert!(check_executable(&app).is_ok());
        std::fs::remove_dir_all(&top).unwrap();
    }

    #[test]
    fn a_sticky_world_writable_ancestor_passes() {
        let (top, app) = nested("anc-sticky");
        chmod(&top, 0o1777);
        assert!(check_executable(&app).is_ok(), "like /tmp: entries of other owners cannot be renamed");
        chmod(&top.join("mid"), 0o1777);
        assert!(check_executable(&app).is_ok());
        chmod(&top.join("mid").join("bin"), 0o1777);
        assert!(check_executable(&app).is_err(), "the file's own directory stays strict");
        chmod(&top.join("mid").join("bin"), 0o755);
        std::fs::remove_dir_all(&top).unwrap();
    }

    #[test]
    fn an_ancestor_owned_by_another_user_fails() {
        let (top, app) = nested("anc-owner");
        let me = crate::discovery::current_euid().unwrap();
        assert!(unix::check_as(&app, me).is_ok());
        let err = unix::check_as(&app, me.wrapping_add(4242)).expect_err("every directory here belongs to a stranger");
        assert!(err.to_string().contains("owned by uid"), "{err}");
        std::fs::remove_dir_all(&top).unwrap();
    }

    #[test]
    fn a_symlinked_ancestor_is_checked_on_both_sides() {
        let (real, app) = nested("anc-real");
        let shared = scratch("anc-shared");
        let link = shared.join("lnk");
        std::os::unix::fs::symlink(real.join("mid"), &link).unwrap();
        let via_link = link.join("bin").join("app");
        assert!(check_executable(&via_link).is_ok());
        chmod(&shared, 0o777);
        assert!(check_executable(&via_link).is_err(), "anyone could swap the link in a non-sticky directory");
        chmod(&shared, 0o755);
        chmod(&real, 0o777);
        assert!(check_executable(&via_link).is_err(), "the real chain is checked too");
        chmod(&real, 0o755);
        assert!(check_executable(&app).is_ok());
        assert!(check_executable(&via_link).is_ok());
        let dotdot = shared.join("lnk").join("..").join("mid").join("bin").join("app");
        assert!(check_executable(&dotdot).is_err(), "a `..` hides which directories are walked");
        std::fs::remove_dir_all(&real).unwrap();
        std::fs::remove_dir_all(&shared).unwrap();
    }

    #[test]
    fn verified_executable_returns_the_checked_real_path() {
        let (top, app) = nested("anc-verified");
        let link = top.join("alias");
        std::os::unix::fs::symlink(&app, &link).unwrap();
        let real = verified_executable(&link).unwrap();
        assert_eq!(real, std::fs::canonicalize(&app).unwrap());
        assert!(real.components().all(|part| part != std::path::Component::ParentDir));
        std::fs::remove_dir_all(&top).unwrap();
    }

    #[test]
    fn system_programs_pass() {
        assert!(check_executable(Path::new("/usr/bin/env")).is_ok() || check_executable(Path::new("/bin/sh")).is_ok());
    }
}
