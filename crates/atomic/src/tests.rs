//! Tests for the atomic save engine.

use super::*;
use std::cell::Cell;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        #[cfg(not(target_arch = "wasm32"))]
        let pid = std::process::id();
        #[cfg(target_arch = "wasm32")]
        let pid = 0;
        let d = std::env::temp_dir().join(format!("craft-atomic-{tag}-{pid}-{}", N.fetch_add(1, Ordering::Relaxed)));
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
    /// Simulate a filesystem that refuses the stage's group change.
    #[cfg(unix)]
    fail_set_group: bool,
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
    #[cfg(unix)]
    fn set_group(&self, file: &File, gid: u32) -> io::Result<()> {
        self.events.borrow_mut().push("set_group");
        if self.fail_set_group {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected: set_group failed"));
        }
        RealFs.set_group(file, gid)
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

#[cfg(unix)]
#[test]
fn group_bits_are_dropped_when_the_stage_cannot_join_the_replaced_group() -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let d = TempDir::new("group-fallback");
    let file = File::create(d.0.join("stage"))?;
    let gid = file.metadata()?.gid();
    let elsewhere = gid.wrapping_add(1);

    let fs = Faulty { fail_set_group: true, ..Default::default() };
    let mode = adopt_group(&fs, &file, gid, elsewhere, std::fs::Permissions::from_mode(0o640))?.mode();
    assert_eq!(mode & 0o777, 0o600, "group bits are dropped when the group cannot be joined");
    assert!(fs.events.borrow().contains(&"set_group"));

    // No group bits: the group change is never attempted.
    let fs = Faulty { fail_set_group: true, ..Default::default() };
    let mode = adopt_group(&fs, &file, gid, elsewhere, std::fs::Permissions::from_mode(0o600))?.mode();
    assert_eq!(mode & 0o777, 0o600);
    assert!(!fs.events.borrow().contains(&"set_group"));

    // A stage already in the replaced file's group keeps its bits untouched.
    let fs = Faulty { fail_set_group: true, ..Default::default() };
    let mode = adopt_group(&fs, &file, gid, gid, std::fs::Permissions::from_mode(0o640))?.mode();
    assert_eq!(mode & 0o777, 0o640);
    assert!(!fs.events.borrow().contains(&"set_group"));

    assert_no_temp(&d.0);
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

#[test]
fn temp_name_format_is_valid() {
    let name = temp_name("document.craft");
    assert!(name.starts_with(".document.craft."));
    assert!(name.ends_with(".craft_tmp"));
}

/// What a [`Planting`] filesystem puts at a stage name just before the stage is created there.
#[derive(Clone, Copy, Debug)]
enum Obstacle {
    File,
    Folder,
    /// A link to the file `outside` beside the stage.
    #[cfg(unix)]
    Link,
    /// A link to `missing` beside the stage, which does not exist.
    #[cfg(unix)]
    DanglingLink,
}

/// The real filesystem, except that the first `obstacles.len()` stage names get an obstacle
/// planted at them first, as someone racing the save would.
struct Planting {
    obstacles: Vec<Obstacle>,
    planted: std::cell::RefCell<Vec<PathBuf>>,
}

impl Planting {
    fn new(obstacles: Vec<Obstacle>) -> Self {
        Self { obstacles, planted: Default::default() }
    }

    fn plant(&self, at: &Path) -> io::Result<()> {
        let next = self.planted.borrow().len();
        let Some(&obstacle) = self.obstacles.get(next) else { return Ok(()) };
        match obstacle {
            Obstacle::File => std::fs::write(at, b"PLANTED")?,
            Obstacle::Folder => std::fs::create_dir(at)?,
            #[cfg(unix)]
            Obstacle::Link => std::os::unix::fs::symlink(at.with_file_name("outside"), at)?,
            #[cfg(unix)]
            Obstacle::DanglingLink => std::os::unix::fs::symlink(at.with_file_name("missing"), at)?,
        }
        self.planted.borrow_mut().push(at.to_path_buf());
        Ok(())
    }
}

impl AtomicFs for Planting {
    fn create_new(&self, path: &Path, permissions: NewFilePermissions) -> io::Result<File> {
        self.plant(path)?;
        RealFs.create_new(path, permissions)
    }
    fn sync(&self, file: &File) -> io::Result<()> {
        RealFs.sync(file)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        RealFs.rename(from, to)
    }
    fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()> {
        RealFs.hard_link(from, to)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        RealFs.remove(path)
    }
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        RealFs.sync_dir(dir)
    }
    fn set_permissions(&self, file: &File, perms: std::fs::Permissions) -> io::Result<()> {
        RealFs.set_permissions(file, perms)
    }
    #[cfg(unix)]
    fn set_group(&self, file: &File, gid: u32) -> io::Result<()> {
        RealFs.set_group(file, gid)
    }
}

/// A stage name something already has (a file, a folder, a link, a dangling link) is skipped,
/// never opened: writing through it would put the user's document where the planter chose. With
/// every name taken the save fails and the original is kept.
#[test]
fn a_name_taken_at_the_stage_is_skipped_and_never_written_through() {
    let d = TempDir::new("planted");
    let p = d.0.join("doc.craft");
    let outside = d.0.join("outside");
    std::fs::write(&p, ORIGINAL).unwrap();
    std::fs::write(&outside, b"OUTSIDE").unwrap();

    #[cfg(unix)]
    let obstacles = vec![Obstacle::File, Obstacle::Folder, Obstacle::Link, Obstacle::DanglingLink];
    #[cfg(not(unix))]
    let obstacles = vec![Obstacle::File, Obstacle::Folder];
    let fs = Planting::new(obstacles.clone());
    atomic_write_custom(&fs, NO_RETRY, &p, b"new").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"new");
    let planted = fs.planted.into_inner();
    assert_eq!(planted.len(), obstacles.len(), "every obstacle was met");
    for (at, obstacle) in planted.iter().zip(&obstacles) {
        let meta = std::fs::symlink_metadata(at).unwrap();
        match obstacle {
            Obstacle::File => assert_eq!(std::fs::read(at).unwrap(), b"PLANTED", "the planted file is not written"),
            Obstacle::Folder => assert!(meta.is_dir() && std::fs::read_dir(at).unwrap().next().is_none(), "the folder is left empty"),
            #[cfg(unix)]
            Obstacle::Link | Obstacle::DanglingLink => assert!(meta.file_type().is_symlink(), "the link is left as a link"),
        }
    }
    assert_eq!(std::fs::read(&outside).unwrap(), b"OUTSIDE", "nothing was written through a link");
    assert!(!outside.with_file_name("missing").exists(), "a dangling link's file was not created");
    for at in &planted {
        let _ = std::fs::remove_file(at).or_else(|_| std::fs::remove_dir(at));
    }
    assert_no_temp(&d.0);

    // Every name taken: the save is refused and the original kept.
    let fs = Planting::new(vec![Obstacle::File; 64]);
    let e = atomic_write_custom(&fs, NO_RETRY, &p, b"newer").unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::AlreadyExists, "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), b"new");
    for at in fs.planted.into_inner() {
        assert_eq!(std::fs::read(&at).unwrap(), b"PLANTED");
        let _ = std::fs::remove_file(at);
    }
    assert_no_temp(&d.0);
}

/// A name near the 255-byte limit, in four-byte characters, still stages: the temporary name
/// keeps at most 128 bytes of it, cut between characters, and only the target is left.
#[test]
fn a_long_name_stages_and_leaves_only_the_target() {
    let d = TempDir::new("long-name");
    // 2 + 61 × 4 + 4 = 250 bytes, 67 characters: the first 64 characters alone are 247 bytes.
    let name = format!("ab{}.pdf", "\u{1F600}".repeat(61));
    assert_eq!(name.len(), 250);
    let p = d.0.join(&name);
    atomic_write(&p, b"first").unwrap();
    atomic_write(&p, b"second").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"second");
    let left: Vec<_> = std::fs::read_dir(&d.0).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, [std::ffi::OsString::from(&name)], "only the target is left");

    let staged = temp_name(&name);
    assert!(staged.len() <= 255, "{} bytes", staged.len());
    assert!(staged.starts_with(&format!(".ab{}.", "\u{1F600}".repeat(31))), "128 bytes of the name, cut between characters: {staged}");
}

/// Reads the extended attribute `name` of `path`, following no further than the path itself.
#[cfg(target_os = "macos")]
fn xattr(path: &Path, name: &str) -> Option<Vec<u8>> {
    let size = rustix::fs::getxattr_query(path, name).ok()?;
    let mut value = vec![0u8; size.max(1)];
    let len = rustix::fs::getxattr(path, name, &mut value[..]).ok()?;
    value.truncate(len);
    Some(value)
}

/// A save keeps the replaced file's extended attributes (Finder tags, the "Open with" binding,
/// quarantine, binary, empty and resource-fork values), save after save, including one another
/// process sets while the stage is written. Through a symbolic link the link stays and the file it
/// names keeps them. A link swapped in at the target while staging is never read through: its
/// file's attributes stay its own. (`com.apple.provenance` cannot be set by a test; a file that
/// has it saves only because the copy skips it, as macOS refuses writing it.)
#[cfg(target_os = "macos")]
#[test]
fn attributes_set_while_staging_are_kept() {
    use rustix::fs::{XattrFlags, setxattr};
    let d = TempDir::new("xattrs");
    let target = d.0.join("doc.pdf");
    std::fs::write(&target, ORIGINAL).unwrap();
    let attributes: &[(&str, &[u8])] = &[
        ("com.apple.metadata:_kMDItemUserTags", b"<plist version=\"1.0\"><array><string>Craft\n6</string></array></plist>"),
        ("com.apple.LaunchServices.OpenWith", b"bplist00\xd1\x01\x02Wversion\x10\x00"),
        ("com.apple.quarantine", b"0081;5e0d1d05;Craft;"),
        ("com.craft.test.binary", b"\0\xff\x80original\0"),
        ("com.craft.test.empty", b""),
        ("com.apple.ResourceFork", b"\0\xfffork"),
    ];
    for (name, value) in attributes {
        setxattr(&target, *name, value, XattrFlags::empty()).unwrap();
    }
    // What the system stored (quarantine may be normalised): the saved file must hold the same.
    let stored: Vec<_> = attributes.iter().map(|(name, _)| (*name, xattr(&target, name).unwrap())).collect();

    for save in 0..2 {
        atomic_write_with(&target, |f| {
            // Another process tags the original while the new bytes are written.
            setxattr(&target, "com.craft.test.late", format!("late {save}").as_bytes(), XattrFlags::empty())?;
            f.write_all(b"new")
        })
        .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        for (name, value) in &stored {
            assert_eq!(xattr(&target, name).as_ref(), Some(value), "save {save} keeps {name}");
        }
        assert_eq!(xattr(&target, "com.craft.test.late"), Some(format!("late {save}").into_bytes()), "set while staging");
        assert_no_temp(&d.0);
    }

    // Through a link: the link stays, the file it names is replaced and keeps its attributes.
    let link = d.0.join("link.pdf");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    atomic_write(&link, b"through the link").unwrap();
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link stays a link");
    assert_eq!(std::fs::read(&target).unwrap(), b"through the link");
    for (name, value) in &stored {
        assert_eq!(xattr(&target, name).as_ref(), Some(value), "the linked file keeps {name}");
    }

    // A link put at the target while staging names another file: nothing is read through it.
    let outside = d.0.join("outside.pdf");
    std::fs::write(&outside, b"OUTSIDE").unwrap();
    setxattr(&outside, "com.craft.test.private", b"private", XattrFlags::empty()).unwrap();
    atomic_write_with(&target, |f| {
        std::fs::remove_file(&target)?;
        std::os::unix::fs::symlink(&outside, &target)?;
        f.write_all(b"swapped")
    })
    .unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"OUTSIDE");
    assert_eq!(xattr(&outside, "com.craft.test.private"), Some(b"private".to_vec()));
    assert!(!std::fs::symlink_metadata(&target).unwrap().file_type().is_symlink());
    assert_eq!(xattr(&target, "com.craft.test.private"), None, "the swapped-in link's file lent nothing");
    assert_no_temp(&d.0);
}

/// An attribute past the 16 MiB value bound refuses the save instead of dropping it: the target
/// keeps its bytes and its attribute, and no stage is left.
#[cfg(target_os = "macos")]
#[test]
fn oversized_attributes_refuse_the_save_and_keep_the_target() {
    use rustix::fs::{XattrFlags, getxattr_query, setxattr};
    let d = TempDir::new("xattrs-bound");
    let target = d.0.join("doc.pdf");
    std::fs::write(&target, ORIGINAL).unwrap();
    // Only a resource fork may be this large; the bytes exist only in this temporary file.
    let fork = vec![42u8; 16 * 1024 * 1024 + 1];
    setxattr(&target, "com.apple.ResourceFork", &fork, XattrFlags::empty()).unwrap();
    let e = atomic_write(&target, b"new").unwrap_err();
    assert!(e.to_string().contains("extended attributes"), "{e}");
    assert_eq!(std::fs::read(&target).unwrap(), ORIGINAL);
    assert_eq!(getxattr_query(&target, "com.apple.ResourceFork").unwrap(), fork.len());
    assert_no_temp(&d.0);
}

/// A value read must fill the size first queried and keep it in a second query: a growing
/// resource fork returns the prefix that fits instead of an error. Complete values, empty and
/// binary, are kept as read.
#[cfg(target_os = "macos")]
#[test]
fn an_attribute_read_must_fill_its_size_and_keep_it() {
    for bytes in [&b""[..], &b"\0\xffB"[..]] {
        let value = read_xattr_value(
            bytes.len(),
            |buffer| {
                buffer[..bytes.len()].copy_from_slice(bytes);
                Ok(bytes.len())
            },
            || Ok(bytes.len()),
        )
        .unwrap();
        assert_eq!(value, bytes);
    }
    // A short read, a size that grew after the read, a prefix of a grown value.
    for (before, read_len, after) in [(3, 2, 3), (3, 3, 4), (0, 1, 1), (3, 2, 2)] {
        let e = read_xattr_value(before, |_| Ok(read_len), || Ok(after)).unwrap_err();
        assert!(e.to_string().contains("changed size"), "{before} {read_len} {after}: {e}");
    }
    let e = read_xattr_value(3, |_| Ok(3), || Err(io::Error::new(io::ErrorKind::PermissionDenied, "recheck refused"))).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
}

/// A saved document is the same document: it keeps its creation date. A new file gets its own.
#[cfg(any(target_os = "macos", windows))]
#[test]
fn a_replaced_files_creation_date_is_kept() {
    #[cfg(target_os = "macos")]
    use std::os::macos::fs::FileTimesExt;
    #[cfg(windows)]
    use std::os::windows::fs::FileTimesExt;
    let d = TempDir::new("created");
    let target = d.0.join("doc.pdf");
    std::fs::write(&target, ORIGINAL).unwrap();
    // 2020-01-02 03:04:05 UTC.
    let then = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_577_934_245);
    let file = OpenOptions::new().write(true).open(&target).unwrap();
    file.set_times(std::fs::FileTimes::new().set_created(then)).unwrap();
    drop(file);
    assert_eq!(std::fs::metadata(&target).unwrap().created().unwrap(), then, "the fixture's date is set");
    atomic_write(&target, b"new").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
    assert_eq!(std::fs::metadata(&target).unwrap().created().unwrap(), then, "the saved file keeps its creation date");
    let fresh = d.0.join("new.pdf");
    atomic_write(&fresh, b"new").unwrap();
    assert!(std::fs::metadata(&fresh).unwrap().created().unwrap() > then, "a new file has a date of its own");
}

/// A hidden or system file stays hidden or system, and its stage already is while written; a
/// plain file stays plain though the stage's name starts with a dot; a new file gets neither.
/// Through a symbolic link the file it names is the replaced file and keeps its attributes.
#[cfg(windows)]
#[test]
fn hidden_and_system_attributes_are_kept() {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    let d = TempDir::new("attributes");
    let kept = |p: &Path| std::fs::metadata(p).unwrap().file_attributes() & (HIDDEN | SYSTEM);
    let plant = |p: &Path, attributes: u32| {
        let mut fixture = OpenOptions::new().write(true).create_new(true).attributes(attributes).open(p).unwrap();
        fixture.write_all(ORIGINAL).unwrap();
    };
    for (i, keep) in [0, HIDDEN, SYSTEM, HIDDEN | SYSTEM].into_iter().enumerate() {
        let target = d.0.join(format!("doc{i}.pdf"));
        plant(&target, keep);
        assert_eq!(kept(&target), keep, "the fixture has attributes {keep:#x}");
        let mut staged = u32::MAX;
        atomic_write_with(&target, |f| {
            staged = f.metadata()?.file_attributes() & (HIDDEN | SYSTEM);
            f.write_all(b"new")
        })
        .unwrap();
        assert_eq!(staged, keep, "the stage has the target's attributes before it is flushed");
        assert_eq!(kept(&target), keep, "the saved file keeps attributes {keep:#x}");
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
    }
    let fresh = d.0.join("new.pdf");
    atomic_write(&fresh, b"new").unwrap();
    assert_eq!(kept(&fresh), 0, "a new file is neither hidden nor system");

    let referent = d.0.join("referent.pdf");
    plant(&referent, HIDDEN);
    let link = d.0.join("link.pdf");
    if let Err(e) = std::os::windows::fs::symlink_file(&referent, &link) {
        eprintln!("skipped the link case: creating a symbolic link needs Developer Mode or administrator rights ({e})");
        return;
    }
    atomic_write(&link, b"through the link").unwrap();
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link stays a link");
    assert_eq!(std::fs::read(&referent).unwrap(), b"through the link");
    assert_eq!(kept(&referent), HIDDEN, "the file the link names stays hidden");
    assert_no_temp(&d.0);
}

#[cfg(target_arch = "wasm32")]
#[test]
fn wasm_atomic_write_returns_unsupported() {
    let err = atomic_write(Path::new("dummy.craft"), b"data").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Unsupported);
}

#[cfg(target_arch = "wasm32")]
#[test]
fn wasm_sweep_temp_files_returns_unsupported() {
    let err = sweep_temp_files(Path::new("scenes"), std::time::Duration::ZERO).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Unsupported);
}

/// Writes the file `name` in `dir` and dates its modification time `age` in the past.
#[cfg(not(target_arch = "wasm32"))]
fn leftover(dir: &Path, name: &str, age: std::time::Duration) -> io::Result<PathBuf> {
    let p = dir.join(name);
    std::fs::write(&p, b"interrupted save")?;
    File::options().write(true).open(&p)?.set_modified(std::time::SystemTime::now() - age)?;
    Ok(p)
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn sweep_removes_only_old_staging_files() -> io::Result<()> {
    let d = TempDir::new("sweep_old");
    let other = std::process::id().wrapping_add(1);
    let hour = std::time::Duration::from_secs(3600);
    let orphan = leftover(&d.0, &format!(".scene.json.{other}-0.craft_tmp"), 2 * hour)?;
    // A process id the system reused for this process does not protect an old orphan.
    let same_pid = leftover(&d.0, &temp_name("scene.json"), 2 * hour)?;
    let recent = leftover(&d.0, &format!(".scene.json.{other}-1.craft_tmp"), std::time::Duration::ZERO)?;
    let document = leftover(&d.0, "scene.json", 2 * hour)?;

    assert_eq!(sweep_temp_files(&d.0, hour)?, 2);
    assert!(!orphan.exists() && !same_pid.exists());
    assert!(recent.exists() && document.exists());
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn sweep_keeps_files_it_did_not_name_and_folders() -> io::Result<()> {
    let d = TempDir::new("sweep_lookalike");
    let other = std::process::id().wrapping_add(1);
    let old = std::time::Duration::from_secs(7200);
    let orphan = leftover(&d.0, &format!(".a.b.{other}-7.craft_tmp"), old)?;
    let kept = [
        leftover(&d.0, "notes.craft_tmp", old)?,
        leftover(&d.0, &format!("scene.json.{other}-7.craft_tmp"), old)?,
        leftover(&d.0, ".scene.json.12-x.craft_tmp", old)?,
        leftover(&d.0, ".scene.json.-3.craft_tmp", old)?,
        leftover(&d.0, ".12-3.craft_tmp", old)?,
        leftover(&d.0, &format!(".scene.json.{other}-7.craft_tmp.bak"), old)?,
        leftover(&d.0, ".scene.json.99999999999-7.craft_tmp", old)?,
        // temp_name writes the process id and the counter without leading zeros.
        leftover(&d.0, ".scene.json.012-0.craft_tmp", old)?,
        leftover(&d.0, ".scene.json.12-03.craft_tmp", old)?,
        // temp_name keeps at most TEMP_STEM_BYTES bytes of the destination's name.
        leftover(&d.0, &format!(".{}.{other}-7.craft_tmp", "s".repeat(TEMP_STEM_BYTES + 1)), old)?,
    ];
    // The longest stem temp_name writes, in ASCII and in two-byte characters.
    let longest = leftover(&d.0, &temp_name(&"s".repeat(200)), old)?;
    let longest_wide = leftover(&d.0, &temp_name(&"é".repeat(100)), old)?;
    let folder = d.0.join(format!(".scene.json.{other}-8.craft_tmp"));
    std::fs::create_dir(&folder)?;

    assert_eq!(sweep_temp_files(&d.0, std::time::Duration::ZERO)?, 3);
    assert!(!orphan.exists() && !longest.exists() && !longest_wide.exists());
    assert!(kept.iter().all(|p| p.exists()), "a file without the staging name was removed");
    assert!(folder.is_dir());
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn sweep_of_a_missing_folder_is_an_error() {
    let d = TempDir::new("sweep_missing");
    let err = sweep_temp_files(&d.0.join("absent"), std::time::Duration::ZERO).err().map(|e| e.kind());
    assert_eq!(err, Some(io::ErrorKind::NotFound));
}
