#[cfg(unix)]
use craft_atomic::{NewFilePermissions, WriteOptions, atomic_write, write_new};
use craft_atomic::{atomic_write_with, or_plain_fsync, write_new_with};
use std::cell::Cell;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!("craft-atomic-int-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).ok();
        Self(d)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn assert_no_temp(dir: &Path) {
    let leftovers: Vec<_> = std::fs::read_dir(dir)
        .ok()
        .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".craft_tmp")).collect())
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "temporary files left behind: {leftovers:?}");
}

const ORIGINAL: &[u8] = b"original precious document";

#[test]
fn atomic_write_with_streaming_writer() {
    let d = TempDir::new("stream");
    let p = d.0.join("doc.dat");
    std::fs::write(&p, ORIGINAL).ok();

    atomic_write_with(&p, |f| {
        f.write_all(b"chunk1 ")?;
        f.write_all(b"chunk2")
    })
    .ok();

    assert_eq!(std::fs::read(&p).ok(), Some(b"chunk1 chunk2".to_vec()));
    assert_no_temp(&d.0);
}

#[test]
fn write_new_with_streaming_writer() {
    let d = TempDir::new("write_new_stream");
    let p = d.0.join("new.dat");

    write_new_with(&p, |f| f.write_all(b"new content")).ok();

    assert_eq!(std::fs::read(&p).ok(), Some(b"new content".to_vec()));
    assert_no_temp(&d.0);

    // Second write to existing file must fail with AlreadyExists
    let res = write_new_with(&p, |f| f.write_all(b"overwritten?"));
    assert!(res.is_err());
    assert_eq!(std::fs::read(&p).ok(), Some(b"new content".to_vec()));
}

#[test]
fn streaming_failure_aborts_and_keeps_original() {
    let d = TempDir::new("stream_fail");
    let p = d.0.join("doc.dat");
    std::fs::write(&p, ORIGINAL).ok();

    let res = atomic_write_with(&p, |f| {
        f.write_all(b"half a file")?;
        Err(io::Error::new(io::ErrorKind::StorageFull, "disk full"))
    });

    assert!(res.is_err());
    assert_eq!(std::fs::read(&p).ok(), Some(ORIGINAL.to_vec()));
    assert_no_temp(&d.0);
}

#[test]
fn apple_full_fsync_fallback_behavior() {
    let refused = |errno| Err(io::Error::from_raw_os_error(errno));
    for (errno, name) in [(45, "ENOTSUP (SMB)"), (25, "ENOTTY")] {
        let fsynced = Cell::new(false);
        let r = or_plain_fsync(refused(errno), || {
            fsynced.set(true);
            Ok(())
        });
        let apple = cfg!(target_vendor = "apple");
        assert_eq!(r.is_ok(), apple, "{name}");
        assert_eq!(fsynced.get(), apple, "{name}: fsync fallback only runs on Apple");
    }

    // Unrelated errors (e.g. EIO = 5) must not fallback
    let r = or_plain_fsync(refused(5), || panic!("should not run for EIO"));
    assert_eq!(r.map_err(|e| e.raw_os_error()), Err(Some(5)));
}

#[cfg(unix)]
#[test]
fn mode_and_symlink_preservation() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new("mode_symlink");

    // Permissions
    let p = d.0.join("secret.key");
    std::fs::write(&p, ORIGINAL).ok();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).ok();
    atomic_write(&p, b"updated secret").ok();
    let mode = std::fs::metadata(&p).map(|m| m.permissions().mode() & 0o777).ok();
    assert_eq!(mode, Some(0o600));

    // Symlink
    let real = d.0.join("real.txt");
    let link = d.0.join("link.txt");
    std::fs::write(&real, ORIGINAL).ok();
    std::os::unix::fs::symlink(&real, &link).ok();
    atomic_write(&link, b"through symlink").ok();
    assert!(std::fs::symlink_metadata(&link).map(|m| m.file_type().is_symlink()).unwrap_or(false));
    assert_eq!(std::fs::read(&real).ok(), Some(b"through symlink".to_vec()));
}

#[cfg(unix)]
#[test]
fn dangling_relative_symlink_stays_a_link() {
    let d = TempDir::new("dangling");
    std::fs::create_dir(d.0.join("real")).ok();
    let link = d.0.join("link.txt");
    std::os::unix::fs::symlink("real/doc.txt", &link).ok();

    assert!(atomic_write(&link, b"created through the link").is_ok());
    assert!(std::fs::symlink_metadata(&link).map(|m| m.file_type().is_symlink()).unwrap_or(false));
    assert_eq!(std::fs::read(d.0.join("real/doc.txt")).ok(), Some(b"created through the link".to_vec()));
    assert_no_temp(&d.0);
    assert_no_temp(&d.0.join("real"));
}

#[cfg(unix)]
#[test]
fn symlink_loop_is_an_error() {
    let d = TempDir::new("loop");
    std::os::unix::fs::symlink("b", d.0.join("a")).ok();
    std::os::unix::fs::symlink("a", d.0.join("b")).ok();
    assert!(atomic_write(&d.0.join("a"), b"x").is_err_and(|e| e.to_string().starts_with("cannot save") && e.to_string().contains("symbolic links")));
    assert!(write_new(&d.0.join("a"), b"x").is_err());
    assert!(std::fs::symlink_metadata(d.0.join("a")).map(|m| m.file_type().is_symlink()).unwrap_or(false));
    assert_no_temp(&d.0);
}

#[cfg(unix)]
#[test]
fn read_only_target_is_not_replaced() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new("read_only");
    let p = d.0.join("locked.txt");
    std::fs::write(&p, ORIGINAL).ok();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).ok();
    // Root may write the file anyway; the check only means something for an ordinary user.
    if std::fs::OpenOptions::new().write(true).open(&p).is_ok() {
        return;
    }
    let msg = atomic_write(&p, b"new").err().map(|e| e.to_string()).unwrap_or_default();
    assert!(msg.contains("read-only"), "{msg}");
    assert_eq!(std::fs::read(&p).ok(), Some(ORIGINAL.to_vec()));
    assert_no_temp(&d.0);
}

#[cfg(unix)]
#[test]
fn private_staging_preserves_existing_modes_after_callback() -> io::Result<()> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    let d = TempDir::new("private_staging");
    std::fs::set_permissions(&d.0, Permissions::from_mode(0o755))?;
    for mode in [0o600, 0o640, 0o664, 0o200] {
        let p = d.0.join(format!("doc-{mode:o}.craft"));
        std::fs::write(&p, ORIGINAL)?;
        std::fs::set_permissions(&p, Permissions::from_mode(mode))?;
        atomic_write_with(&p, |f| {
            assert_eq!(f.metadata()?.permissions().mode() & 0o777, 0o600, "stage must be private before the callback, even for a shared destination");
            // Settings writers restrict the stage themselves; this must not change the
            // destination mode that atomic replacement promises to preserve.
            f.set_permissions(Permissions::from_mode(0o600))?;
            f.write_all(b"new private payload")?;
            assert_eq!(f.metadata()?.permissions().mode() & 0o777, 0o600, "payload must stay private while writing");
            Ok(())
        })?;
        assert_eq!(std::fs::metadata(&p)?.permissions().mode() & 0o777, mode);
        std::fs::set_permissions(&p, Permissions::from_mode(0o600))?;
        assert_eq!(std::fs::read(&p)?, b"new private payload");
    }
    assert_no_temp(&d.0);
    Ok(())
}

#[cfg(unix)]
#[test]
fn explicit_private_files_keep_callback_permissions() -> io::Result<()> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    let d = TempDir::new("private_new");
    let options = WriteOptions::default().with_new_file_permissions(NewFilePermissions::Private);
    for exclusive in [false, true] {
        let p = d.0.join(format!("new-{exclusive}.craft"));
        let fill = |f: &mut std::fs::File| {
            assert_eq!(f.metadata()?.permissions().mode() & 0o777, 0o600);
            f.write_all(b"new payload")?;
            // A new file has no existing mode to restore, so the callback may select it.
            f.set_permissions(Permissions::from_mode(0o640))
        };
        if exclusive {
            options.write_new_with(&p, fill)?;
        } else {
            options.atomic_write_with(&p, fill)?;
        }
        assert_eq!(std::fs::metadata(&p)?.permissions().mode() & 0o777, 0o640);
        assert_eq!(std::fs::read(&p)?, b"new payload");
    }
    for exclusive in [false, true] {
        let p = d.0.join(format!("default-{exclusive}.craft"));
        if exclusive {
            options.write_new(&p, b"explicitly private")?;
        } else {
            options.atomic_write(&p, b"explicitly private")?;
        }
        assert_eq!(std::fs::metadata(&p)?.permissions().mode() & 0o777, 0o600);
    }
    assert_no_temp(&d.0);
    Ok(())
}
