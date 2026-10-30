//! A private per-user cache directory for Craft Link.
//!
//! Prepared media and other link caches must not sit in a directory that another user can create
//! first. This module picks the directory, creates it `0700`, refuses a symlink, a wrong owner or a
//! loose mode, and writes files `0600` through a temp name and a rename. It never falls back to
//! `$TMPDIR` or `/tmp`: with no home and no override it returns an error.
//!
//! Order: `CRAFT_LINK_CACHE_DIR` (absolute), `$XDG_CACHE_HOME/artcraft/link`, then
//! `$HOME/.cache/artcraft/link`. A leaf name `sub` is added under the chosen directory.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::LinkError;

/// Environment variable that overrides the cache root.
pub const CACHE_DIR_ENV: &str = "CRAFT_LINK_CACHE_DIR";

/// [`cache_dir_with`] for the process environment.
pub fn private_cache_dir(sub: &str) -> Result<PathBuf, LinkError> {
    let env = std::env::vars().collect::<BTreeMap<_, _>>();
    cache_dir_with(&env, sub)
}

/// Pick, create and check the cache directory `sub` for `env`.
pub fn cache_dir_with(env: &BTreeMap<String, String>, sub: &str) -> Result<PathBuf, LinkError> {
    if sub.is_empty() || sub.starts_with('.') || !sub.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) {
        return Err(refused(Path::new(sub), "cache name must be letters, digits, dot, dash or underscore"));
    }
    let (base, rest) = base_and_rest(env)?;
    let mut path = base.clone();
    create_base(&path)?;
    for part in rest.iter().map(String::as_str).chain([sub]) {
        path.push(part);
        crate::discovery::ensure_private_dir(&path)?;
    }
    Ok(path)
}

fn base_and_rest(env: &BTreeMap<String, String>) -> Result<(PathBuf, Vec<String>), LinkError> {
    let absolute = |key: &str| env.get(key).filter(|value| !value.is_empty()).map(PathBuf::from);
    if let Some(root) = absolute(CACHE_DIR_ENV) {
        if !root.is_absolute() {
            return Err(refused(&root, "CRAFT_LINK_CACHE_DIR is not absolute"));
        }
        let (Some(parent), Some(leaf)) = (root.parent(), root.file_name().and_then(|name| name.to_str())) else {
            return Err(refused(&root, "CRAFT_LINK_CACHE_DIR has no parent directory"));
        };
        return Ok((parent.to_path_buf(), vec![leaf.to_string()]));
    }
    if let Some(xdg) = absolute("XDG_CACHE_HOME").filter(|path| path.is_absolute()) {
        return Ok((xdg, vec!["artcraft".into(), "link".into()]));
    }
    if let Some(home) = absolute("HOME").filter(|path| path.is_absolute()) {
        return Ok((home.join(".cache"), vec!["artcraft".into(), "link".into()]));
    }
    Err(refused(Path::new(""), "no cache directory: set CRAFT_LINK_CACHE_DIR, XDG_CACHE_HOME or HOME (a shared temp directory is not used)"))
}

/// Create `path` and missing parents `0700`. An existing directory is left as it is: it is the
/// user's own `~/.cache` or the parent of the override.
fn create_base(path: &Path) -> Result<(), LinkError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}

/// Write `bytes` to `<dir>/<name>` with mode `0600`. The file is made under a temp name with
/// `create_new` (which does not follow a symlink or reuse a file) and renamed into place, so a
/// reader never sees half a file and a planted symlink at `name` is replaced, not followed.
pub fn write_private(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), LinkError> {
    if name.is_empty() || name.contains(['/', '\\', '\0']) || name.starts_with('.') {
        return Err(refused(&dir.join(name), "file name must be one plain component"));
    }
    let mut rand = [0u8; 8];
    getrandom::getrandom(&mut rand).map_err(LinkError::io)?;
    let tmp = dir.join(format!(".{name}.{}.tmp", crate::auth::encode_hex(&rand)));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(err) = written.and_then(|()| std::fs::rename(&tmp, dir.join(name))) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.into());
    }
    Ok(())
}

/// Read `<dir>/<name>` when it is a regular file (not a symlink) that this user owns and no one
/// else can access. Anything else is `Refused`.
pub fn read_private(dir: &Path, name: &str) -> Result<Vec<u8>, LinkError> {
    let path = dir.join(name);
    let meta = std::fs::symlink_metadata(&path)?;
    if !meta.file_type().is_file() {
        return Err(refused(&path, "not a regular file (a symlink is not followed)"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let euid = crate::discovery::current_euid()?;
        if meta.uid() != euid {
            return Err(refused(&path, &format!("owner is {}, euid is {euid}", meta.uid())));
        }
        if meta.mode() & 0o077 != 0 {
            return Err(refused(&path, &format!("mode is {:o}, group and others must have no access", meta.mode() & 0o777)));
        }
    }
    Ok(std::fs::read(&path)?)
}

fn refused(path: &Path, reason: &str) -> LinkError {
    LinkError::Refused { path: path.display().to_string(), reason: reason.to_string() }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let mut bytes = [0u8; 4];
        getrandom::getrandom(&mut bytes).unwrap();
        let path = PathBuf::from(format!("/tmp/cl-cache-{label}-{}", crate::auth::encode_hex(&bytes)));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn env(pairs: &[(&str, &Path)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(key, value)| ((*key).to_string(), value.display().to_string())).collect()
    }

    fn mode(path: &Path) -> u32 {
        std::fs::symlink_metadata(path).unwrap().mode() & 0o777
    }

    #[test]
    fn home_and_xdg_give_a_0700_chain() {
        let home = scratch("home");
        std::fs::create_dir_all(&home).unwrap();
        let dir = cache_dir_with(&env(&[("HOME", &home)]), "prepared").unwrap();
        assert_eq!(dir, home.join(".cache/artcraft/link/prepared"));
        for part in [".cache/artcraft", ".cache/artcraft/link", ".cache/artcraft/link/prepared"] {
            assert_eq!(mode(&home.join(part)), 0o700, "{part}");
        }
        let xdg = scratch("xdg");
        let dir = cache_dir_with(&env(&[("HOME", &home), ("XDG_CACHE_HOME", &xdg)]), "prepared").unwrap();
        assert_eq!(dir, xdg.join("artcraft/link/prepared"));
        assert_eq!(mode(&dir), 0o700);
        // Same call twice is fine.
        assert_eq!(cache_dir_with(&env(&[("XDG_CACHE_HOME", &xdg)]), "prepared").unwrap(), dir);
        std::fs::remove_dir_all(&home).unwrap();
        std::fs::remove_dir_all(&xdg).unwrap();
    }

    #[test]
    fn no_home_means_an_error_not_tmp() {
        let err = cache_dir_with(&BTreeMap::new(), "prepared").unwrap_err();
        assert!(matches!(err, LinkError::Refused { .. }), "{err:?}");
        let relative: BTreeMap<_, _> =
            [("HOME".to_string(), "relative".to_string()), ("XDG_CACHE_HOME".to_string(), "also/relative".to_string())].into();
        assert!(cache_dir_with(&relative, "prepared").is_err());
        let rel_override: BTreeMap<_, _> = [(CACHE_DIR_ENV.to_string(), "cache".to_string())].into();
        assert!(cache_dir_with(&rel_override, "prepared").is_err());
    }

    #[test]
    fn the_override_is_used_and_checked() {
        let root = scratch("override");
        let chosen = root.join("mine");
        let dir = cache_dir_with(&env(&[(CACHE_DIR_ENV, &chosen), ("HOME", Path::new("/nonexistent"))]), "prepared").unwrap();
        assert_eq!(dir, chosen.join("prepared"));
        assert_eq!(mode(&chosen), 0o700);
        // A loose existing directory is refused, not tightened behind the user's back.
        std::fs::set_permissions(&chosen, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(cache_dir_with(&env(&[(CACHE_DIR_ENV, &chosen)]), "prepared").is_err());
        // A symlink is refused.
        let real = root.join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = root.join("link");
        symlink(&real, &link).unwrap();
        assert!(cache_dir_with(&env(&[(CACHE_DIR_ENV, &link)]), "prepared").is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn bad_names_are_refused() {
        let home = scratch("names");
        std::fs::create_dir_all(&home).unwrap();
        for bad in ["", ".hidden", "a/b", "..", "a b", "x\0y"] {
            assert!(cache_dir_with(&env(&[("HOME", &home)]), bad).is_err(), "{bad:?}");
        }
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn files_are_0600_and_symlinks_are_replaced_not_followed() {
        let home = scratch("files");
        std::fs::create_dir_all(&home).unwrap();
        let dir = cache_dir_with(&env(&[("HOME", &home)]), "prepared").unwrap();
        write_private(&dir, "a.bin", b"one").unwrap();
        assert_eq!(mode(&dir.join("a.bin")), 0o600);
        assert_eq!(read_private(&dir, "a.bin").unwrap(), b"one");
        // A symlink planted at the name is replaced. The target keeps its content.
        let victim = home.join("victim");
        std::fs::write(&victim, b"keep").unwrap();
        let planted = dir.join("b.bin");
        symlink(&victim, &planted).unwrap();
        assert!(read_private(&dir, "b.bin").is_err(), "a symlink is not read");
        write_private(&dir, "b.bin", b"two").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        assert_eq!(read_private(&dir, "b.bin").unwrap(), b"two");
        // A loose file is refused on read.
        std::fs::set_permissions(dir.join("a.bin"), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_private(&dir, "a.bin").is_err());
        for bad in ["", "../x", "a/b", ".x"] {
            assert!(write_private(&dir, bad, b"x").is_err(), "{bad:?}");
        }
        std::fs::remove_dir_all(&home).unwrap();
    }
}
