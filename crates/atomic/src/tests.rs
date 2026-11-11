//! Tests for the atomic save engine.

use super::*;
use std::cell::Cell;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!("craft-atomic-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
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

const ORIGINAL: &[u8] = b"the user's precious original document";

#[derive(Default)]
struct Faulty {
    events: std::cell::RefCell<Vec<&'static str>>,
    fail_sync: bool,
    locked_renames: Cell<u32>,
    renames: Cell<u32>,
    permission_error: Option<io::ErrorKind>,
    /// Simulate a network filesystem exposing the same fixed mode on every new file.
    #[cfg(unix)]
    fixed_mode: Option<u32>,
}

impl AtomicFs for Faulty {
    /// Create a real stage, optionally exposing a simulated fixed network mode.
    fn create_new(&self, path: &Path, permissions: NewFilePermissions) -> io::Result<File> {
        self.events.borrow_mut().push("create_new");
        let file = RealFs.create_new(path, permissions)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = self.fixed_mode {
                file.set_permissions(std::fs::Permissions::from_mode(mode))?;
            }
        }
        Ok(file)
    }
    fn sync(&self, file: &File) -> io::Result<()> {
        self.events.borrow_mut().push("sync_file");
        if self.fail_sync {
            return Err(io::Error::new(io::ErrorKind::StorageFull, "injected: no space left on device"));
        }
        RealFs.sync(file)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.events.borrow_mut().push("rename");
        self.renames.set(self.renames.get() + 1);
        if self.locked_renames.get() > 0 {
            self.locked_renames.set(self.locked_renames.get() - 1);
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected: sharing violation"));
        }
        RealFs.rename(from, to)
    }
    fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.events.borrow_mut().push("hard_link");
        RealFs.hard_link(from, to)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        self.events.borrow_mut().push("remove");
        RealFs.remove(path)
    }
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        self.events.borrow_mut().push("sync_dir");
        RealFs.sync_dir(dir)
    }
    /// Record permission restoration, then inject a configured error or delegate the
    /// mode change to the real filesystem.
    fn set_permissions(&self, file: &File, perms: std::fs::Permissions) -> io::Result<()> {
        self.events.borrow_mut().push("set_permissions");
        if let Some(kind) = self.permission_error {
            return Err(io::Error::new(kind, "injected permission failure"));
        }
        RealFs.set_permissions(file, perms)
    }
}

const NO_RETRY: RenameRetry = RenameRetry { attempts: 0, first_delay_ms: 0 };

#[test]
fn failed_flush_keeps_original() {
    let d = TempDir::new("write");
    let p = d.0.join("doc.craft");
    std::fs::write(&p, ORIGINAL).ok();
    let fs = Faulty { fail_sync: true, ..Default::default() };
    let e = atomic_write_custom(&fs, NO_RETRY, &p, &[7u8; 4096]);
    assert_eq!(e.map_err(|e| e.kind()), Err(io::ErrorKind::StorageFull));
    assert_eq!(std::fs::read(&p).ok(), Some(ORIGINAL.to_vec()));
    assert_eq!(fs.renames.get(), 0);
    assert_no_temp(&d.0);
}

#[test]
fn locked_target_rename_is_retried() {
    let d = TempDir::new("locked");
    let p = d.0.join("doc.craft");
    std::fs::write(&p, ORIGINAL).ok();
    let fs = Faulty { locked_renames: Cell::new(2), ..Default::default() };
    atomic_write_custom(&fs, RenameRetry { attempts: 3, first_delay_ms: 1 }, &p, b"new").ok();
    assert_eq!(std::fs::read(&p).ok(), Some(b"new".to_vec()));
    assert_eq!(fs.renames.get(), 3);
    assert_no_temp(&d.0);
}

/// Matching fixed modes permit byte and streaming replacements under both creation
/// policies without calling a permission operation that the mount rejects.
#[cfg(unix)]
#[test]
fn unchanged_replacement_modes_need_no_chmod() -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new("fixed-replacement-mode");
    // GVfs SMB reports 0700 regardless of the requested creation mode; also cover a
    // private stage whose normal 0600 mode already matches the original.
    for mode in [0o600, 0o700] {
        for policy in [NewFilePermissions::Private, NewFilePermissions::PlatformDefault] {
            for streaming in [false, true] {
                let p = d.0.join(format!("{mode:o}-{policy:?}-{streaming}"));
                std::fs::write(&p, ORIGINAL)?;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode))?;
                let fs = Faulty { fixed_mode: Some(mode), permission_error: Some(io::ErrorKind::Unsupported), ..Default::default() };
                let options = WriteOptions::default().with_new_file_permissions(policy);
                if streaming {
                    options.atomic_write_with_custom(&fs, NO_RETRY, &p, |f| f.write_all(b"replacement"))?;
                } else {
                    options.atomic_write_custom(&fs, NO_RETRY, &p, b"replacement")?;
                }
                assert_eq!(std::fs::read(&p)?, b"replacement");
                assert_eq!(std::fs::metadata(&p)?.permissions().mode() & 0o7777, mode);
                assert!(!fs.events.borrow().contains(&"set_permissions"));
                assert_eq!(fs.renames.get(), 1);
                assert_no_temp(&d.0);
            }
        }
    }
    Ok(())
}

#[test]
fn distinguishes_atomicity_and_durability_order() {
    let d = TempDir::new("durability_order");
    let p = d.0.join("doc.craft");
    std::fs::write(&p, ORIGINAL).ok();

    let fs = Faulty::default();
    atomic_write_custom(&fs, NO_RETRY, &p, b"durable payload").expect("successful atomic write");

    let events = fs.events.into_inner();
    let sync_file_idx = events.iter().position(|&e| e == "sync_file").expect("file content must be flushed");
    let rename_idx = events.iter().position(|&e| e == "rename").expect("atomic rename must be performed");

    // 1. Content Durability: staging file must be synced to stable media BEFORE pointer switch (rename)
    assert!(
        sync_file_idx < rename_idx,
        "Durability invariant violated: file content must be flushed to stable storage before rename (sync={sync_file_idx}, rename={rename_idx})"
    );

    // 2. Directory Durability: on Unix, parent directory must be synced AFTER rename to commit directory entry
    #[cfg(unix)]
    {
        let sync_dir_idx = events.iter().position(|&e| e == "sync_dir").expect("directory must be flushed on Unix");
        assert!(
            rename_idx < sync_dir_idx,
            "Durability invariant violated: parent directory must be flushed after rename (rename={rename_idx}, sync_dir={sync_dir_idx})"
        );
    }
}
