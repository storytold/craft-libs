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
//! A file or directory that its group can write passes only when no other account can write
//! through that group. Group gid 0 passes. On Linux, any other group passes when `/etc/group` and
//! `/etc/passwd` show that its only members are this user and root, which is the private group
//! most distributions give each user. A group the files do not describe fails, because the check
//! cannot see who is in it. Other Unix targets trust only gid 0, because their directory service
//! keeps group members outside those files.

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
/// crosses. A group-writable file or directory fails when another account can write through its
/// group (see the module docs for the group rule).
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

    const GROUP_WRITE: u32 = 0o020;
    const WORLD_WRITE: u32 = 0o002;
    const STICKY: u32 = 0o1000;
    const ANY_EXEC: u32 = 0o111;

    /// The account files that say who is in a group. Tests build one from text.
    pub(super) struct Accounts {
        passwd: String,
        group: String,
    }

    impl Accounts {
        /// `/etc/passwd` and `/etc/group` on Linux. Empty elsewhere, so only gid 0 is trusted.
        pub(super) fn system() -> Self {
            #[cfg(target_os = "linux")]
            {
                let read = |name: &str| std::fs::read_to_string(name).unwrap_or_default();
                Self { passwd: read("/etc/passwd"), group: read("/etc/group") }
            }
            #[cfg(not(target_os = "linux"))]
            {
                Self::from_text("", "")
            }
        }

        #[cfg(any(test, not(target_os = "linux")))]
        pub(super) fn from_text(passwd: &str, group: &str) -> Self {
            Self { passwd: passwd.to_string(), group: group.to_string() }
        }

        /// True when no account except `euid` and root can write through group `gid`.
        ///
        /// A group with no line in `group` fails: it may come from a directory service and have
        /// members that this check cannot see.
        fn only_trusted_members(&self, gid: u32, euid: u32) -> bool {
            if gid == 0 {
                return true;
            }
            let mut me = None;
            // passwd lines are `name:x:uid:gid:...`. A user whose primary group is `gid` is a
            // member even when `group` does not list them.
            for line in self.passwd.lines() {
                let mut field = line.split(':');
                let (Some(name), Some(_), Some(uid), Some(primary)) = (field.next(), field.next(), field.next(), field.next()) else {
                    continue;
                };
                let (Ok(uid), Ok(primary)) = (uid.parse::<u32>(), primary.parse::<u32>()) else { continue };
                if uid == euid {
                    me = Some(name);
                }
                if primary == gid && uid != euid && uid != 0 {
                    return false;
                }
            }
            let Some(me) = me else { return false };
            let mut listed = false;
            for line in self.group.lines() {
                // group lines are `name:x:gid:member,member`.
                let mut field = line.split(':');
                let (Some(_), Some(_), Some(id), members) = (field.next(), field.next(), field.next(), field.next()) else { continue };
                if id.parse::<u32>() != Ok(gid) {
                    continue;
                }
                listed = true;
                if members.unwrap_or("").split(',').any(|member| !member.is_empty() && member != me && member != "root") {
                    return false;
                }
            }
            listed
        }

        /// Why `meta` can be changed by someone other than this user or root, if it can.
        fn writable_by_others(&self, meta: &std::fs::Metadata, euid: u32) -> Option<&'static str> {
            if meta.mode() & WORLD_WRITE != 0 {
                return Some("writable by every user");
            }
            if meta.mode() & GROUP_WRITE != 0 && !self.only_trusted_members(meta.gid(), euid) {
                return Some("writable by a group that has other members");
            }
            None
        }
    }

    pub(super) fn check(path: &Path) -> Result<PathBuf, LinkError> {
        check_as(path, crate::discovery::current_euid()?, &Accounts::system())
    }

    /// [`check`] for a caller whose effective uid is `euid`, with `accounts` as the group data.
    pub(super) fn check_as(path: &Path, euid: u32, accounts: &Accounts) -> Result<PathBuf, LinkError> {
        let meta = std::fs::metadata(path).map_err(|err| refused(path, &format!("cannot stat: {err}")))?;
        if !meta.is_file() {
            return Err(refused(path, "not a regular file"));
        }
        if meta.mode() & ANY_EXEC == 0 {
            return Err(refused(path, "not executable"));
        }
        owner_and_mode(path, &meta, euid, "file", accounts)?;
        let real = std::fs::canonicalize(path).map_err(|err| refused(path, &format!("cannot resolve: {err}")))?;
        walk(path, euid, accounts)?;
        if real != path {
            walk(&real, euid, accounts)?;
        }
        Ok(real)
    }

    /// Check `path` and every component above it, from the file up to `/`.
    ///
    /// A component is checked by what it is (`lstat`, so a symlink must belong to this user or
    /// root) and by what it points to. The directory that holds a component must not let another
    /// user rename that component: it is not writable by every user, or it has the sticky bit.
    /// The directory that holds the file itself gets no sticky exception.
    fn walk(path: &Path, euid: u32, accounts: &Accounts) -> Result<(), LinkError> {
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
            if let Some(why) = accounts.writable_by_others(&dir, euid)
                && !sticky_allowed
            {
                return Err(refused(parent, &format!("directory is {why}")));
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

    fn owner_and_mode(path: &Path, meta: &std::fs::Metadata, euid: u32, what: &str, accounts: &Accounts) -> Result<(), LinkError> {
        owner(path, meta, euid, what)?;
        if let Some(why) = accounts.writable_by_others(meta, euid) {
            return Err(refused(path, &format!("{what} is {why}")));
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
        let path = std::env::temp_dir().join(format!("cl-trust-{label}-{}", crate::auth::encode_hex(&bytes)));
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
        let system = unix::Accounts::system();
        assert!(unix::check_as(&app, me, &system).is_ok());
        let err = unix::check_as(&app, me.wrapping_add(4242), &system).expect_err("every directory here belongs to a stranger");
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

    /// Account files for a machine with one local user `me` (this test's uid) whose group is
    /// `grp` (the gid of `path`), plus the extra `passwd` and `group` lines given.
    fn accounts(path: &Path, extra_passwd: &str, group_members: &str) -> unix::Accounts {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path).unwrap();
        let (uid, gid) = (crate::discovery::current_euid().unwrap(), meta.gid());
        unix::Accounts::from_text(&format!("me:x:{uid}:{gid}:Me:/home/me:/bin/sh\n{extra_passwd}"), &format!("grp:x:{gid}:{group_members}\n"))
    }

    fn check_with(path: &Path, accounts: &unix::Accounts) -> Result<PathBuf, LinkError> {
        unix::check_as(path, crate::discovery::current_euid().unwrap(), accounts)
    }

    #[test]
    fn a_group_writable_ancestor_fails_when_another_account_is_in_the_group() {
        let (top, app) = nested("grp-anc");
        let private = accounts(&top, "", "");
        let shared = accounts(&top, "", "me,stranger");
        chmod(&top, 0o775);
        assert!(check_with(&app, &private).is_ok(), "a private group cannot be used by anyone else");
        let err = check_with(&app, &shared).expect_err("the stranger could rename the directory below");
        assert!(err.to_string().contains("writable by a group that has other members"), "{err}");
        chmod(&top, 0o755);
        assert!(check_with(&app, &shared).is_ok(), "the group bit is off");
        std::fs::remove_dir_all(&top).unwrap();
    }

    #[test]
    fn a_group_writable_executable_fails_when_another_account_is_in_the_group() {
        let (top, app) = nested("grp-file");
        chmod(&app, 0o775);
        assert!(check_with(&app, &accounts(&app, "", "")).is_ok());
        let err = check_with(&app, &accounts(&app, "", "stranger")).expect_err("the stranger could edit the program");
        assert!(err.to_string().contains("file is writable by a group that has other members"), "{err}");
        std::fs::remove_dir_all(&top).unwrap();
    }

    #[test]
    fn a_user_whose_primary_group_is_shared_counts_as_a_member() {
        let (top, app) = nested("grp-primary");
        chmod(&top, 0o775);
        let me_only = accounts(&top, "", "");
        assert!(check_with(&app, &me_only).is_ok());
        // `other` is not listed in /etc/group, but its primary gid is this group.
        let gid = std::os::unix::fs::MetadataExt::gid(&std::fs::metadata(&top).unwrap());
        let crowded = accounts(&top, &format!("other:x:4243:{gid}::/home/other:/bin/sh\n"), "");
        assert!(check_with(&app, &crowded).is_err());
        // root as a primary member is trusted.
        let with_root = accounts(&top, &format!("root:x:0:{gid}::/root:/bin/sh\n"), "root");
        assert!(check_with(&app, &with_root).is_ok());
        std::fs::remove_dir_all(&top).unwrap();
    }

    #[test]
    fn a_group_the_account_files_do_not_describe_fails() {
        let (top, app) = nested("grp-unknown");
        chmod(&top, 0o775);
        let unknown = unix::Accounts::from_text("", "");
        let err = check_with(&app, &unknown).expect_err("members cannot be seen, so assume the worst");
        assert!(err.to_string().contains("group that has other members"), "{err}");
        let no_group_line = unix::Accounts::from_text(&format!("me:x:{}:{}::/:/bin/sh\n", crate::discovery::current_euid().unwrap(), 4244), "");
        assert!(check_with(&app, &no_group_line).is_err());
        std::fs::remove_dir_all(&top).unwrap();
    }

    #[test]
    fn a_sticky_group_writable_ancestor_passes_but_not_the_files_own_directory() {
        let (top, app) = nested("grp-sticky");
        let shared = accounts(&top, "", "stranger");
        chmod(&top, 0o3775);
        assert!(check_with(&app, &shared).is_ok(), "sticky stops the stranger renaming entries it does not own");
        let bin = top.join("mid").join("bin");
        chmod(&bin, 0o3775);
        assert!(check_with(&app, &shared).is_err(), "the file's own directory stays strict");
        std::fs::remove_dir_all(&top).unwrap();
    }

    #[test]
    fn system_programs_pass() {
        assert!(check_executable(Path::new("/usr/bin/env")).is_ok() || check_executable(Path::new("/bin/sh")).is_ok());
    }
}
