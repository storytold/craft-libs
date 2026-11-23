//! Integration tests of craft-atomic: atomic tests.

#![cfg(unix)]

use craft_atomic::{atomic_write, atomic_write_with};
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
