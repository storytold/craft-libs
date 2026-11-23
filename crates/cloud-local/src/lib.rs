#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! A reference [`CloudBackend`] that treats one folder on disk as the cloud.
//!
//! [`craft_ui::cloud::CloudBackend`] names a contract and nothing about who
//! answers it. This crate is the proof: a working implementation that reaches
//! no service, a vendor, or a personal repo anybody else has to trust. An app
//! wires [`LocalBackend`] the same way it would wire a provider's own client,
//! by calling [`craft_ui::cloud::CloudFiles::connect`]. Swapping one for the
//! other is the whole point; nothing above the [`CloudBackend`] seam can tell
//! the difference.
//!
//! What this is not: a sync engine, a sharing service, or a place to keep a
//! credential. A folder has no concept of another account, so
//! [`Job::SharedWithMe`] answers empty and [`Job::Conversation`] answers with
//! a [`Fault`], and [`Account::capabilities`] claims neither. Reporting a
//! capability this backend cannot honour would be the broken window the
//! contract's own docs warn about.
//!
//! Every job answers on its own [`std::thread`]. [`CloudBackend::submit`]
//! must return before the file is opened, let alone read, or a twenty
//! megabyte document would freeze the window that called it.
//!
//! # The no-escape policy
//!
//! `Entry::id` and `Entry::path` are the same slash-separated path relative to
//! `root`, always starting with `/` below the root itself (the root's own
//! path is the empty string, matching [`Job::List`]'s contract), and a
//! [`Job::Create`] name is one plain file name, not a path at all. Neither is
//! an OS path string handed to `std::fs`. Confinement happens in two layers:
//!
//! 1. [`confine_segment`] (every path segment) and [`confine_name`] (a
//!    `Create` name, stricter) reject `..`, `.`, an empty segment, an
//!    absolute form, and anything that is not exactly one
//!    [`std::path::Component::Normal`]. `confine_name` additionally refuses a
//!    backslash, a colon, and a Windows reserved device name
//!    (`CON`, `COM1`, `CONIN$`, ...) on every platform this crate builds for,
//!    because a chosen name reaches this backend from whatever platform the
//!    UI runs on. Browsing an *existing* name is more lenient: a literal `:`
//!    or `\` already on disk (macOS stores a `/` typed in Finder as `:`) is
//!    not itself a path separator on any platform's filesystem, so
//!    `confine_segment` lets `list`, `read` and `write` keep such a name
//!    reachable instead of discovering it and then refusing to open it.
//! 2. [`open_root`] opens `root` as a [`cap_std::fs::Dir`] capability once per
//!    job. Every [`list`], [`read`], [`write`] and [`create`] routes its
//!    already-segment-checked relative path through that one `Dir`, never
//!    through an absolute OS path built by this crate. `cap_std` resolves one
//!    component at a time against real, already-open directory handles, so a
//!    symlink or a Windows junction anywhere under `root`, dangling or not,
//!    cannot walk resolution above `root`: there is no separate "is this
//!    still inside the root" check with a window for the directory tree to
//!    change underneath it, because the one resolution *is* the open.
//!
//! A hard link is not a symlink and `cap_std` does not refuse one; the inode
//! it names is an ordinary file from inside the sandbox. [`write`] protects
//! it a different way: it always writes a sibling temporary file and renames
//! it over the target, rather than truncating the target in place. A rename
//! replaces one name's directory entry; it never reaches through to another
//! name that happens to point at the same inode outside `root`.

use std::ffi::OsStr;
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use cap_std::fs::{Dir, OpenOptions};
use craft_cloud::{Capability, Entry, EntryKind};
use craft_ui::cloud::{Account, CloudBackend, Event, Fault, Job, Post};

/// Where "the cloud" lives for this backend: one directory on disk.
pub struct LocalBackend {
    root: PathBuf,
    /// Shown in the account badge. Nothing else reads it.
    display: String,
}

impl LocalBackend {
    /// Treat `root` as the cloud. The directory is not created here; a
    /// listing of a root that does not exist yet answers with a [`Fault`]
    /// the same way a real provider's missing folder would. [`Job::Create`]
    /// is the one exception, and always has been: it brings a fresh root
    /// into existence itself, the same way a first Save As does not require
    /// the destination folder to already be there.
    pub fn new(root: PathBuf) -> LocalBackend {
        LocalBackend { root, display: "Local disk".to_owned() }
    }

    /// Name shown in the account badge, in place of the default "Local disk".
    pub fn named(mut self, display: impl Into<String>) -> LocalBackend {
        self.display = display.into();
        self
    }
}

impl CloudBackend for LocalBackend {
    fn submit(&self, job: Job, post: Post) {
        let root = self.root.clone();
        match job {
            // Nothing is saved between runs and nothing was ever signed into,
            // so there is nothing to forget. The trait states no answer for
            // this job; CloudFiles::sign_out clears its own state regardless
            // of whether anything ever arrives.
            Job::SignOut => {}
            Job::Account | Job::SignIn => {
                let display = self.display.clone();
                std::thread::spawn(move || post.send(Event::Account(Ok(account(display)))));
            }
            // There is no browser round trip: a local folder is already
            // "linked" the moment it exists. Reporting that immediately,
            // rather than inventing a URL nobody needs to visit, is what the
            // contract calls honest.
            Job::LinkStart | Job::LinkPoll(_) => {
                std::thread::spawn(move || post.send(Event::Linked(Ok(true))));
            }
            Job::List(path) => {
                std::thread::spawn(move || post.send(Event::Listed { entries: list(&root, &path), path }));
            }
            Job::SharedWithMe => {
                std::thread::spawn(move || post.send(Event::Shared(Ok(Vec::new()))));
            }
            Job::Read { entry, into } => {
                std::thread::spawn(move || match read(&root, &entry) {
                    Ok((entry, bytes)) => post.send(Event::Read { entry, into, bytes: Ok(bytes) }),
                    Err(fault) => post.send(Event::Read { entry, into, bytes: Err(fault) }),
                });
            }
            Job::Write { uid, entry, bytes } => {
                std::thread::spawn(move || post.send(Event::Saved { uid, entry: write(&root, &entry, &bytes) }));
            }
            Job::Create { uid, folder, name, bytes } => {
                std::thread::spawn(move || post.send(Event::Saved { uid, entry: create(&root, &folder, &name, &bytes) }));
            }
            // No sharing, no accounts beyond this one, and so nowhere for a
            // conversation to live. A reference backend that invented one
            // would be lying about what a folder on disk can do.
            Job::Conversation(_) => {
                std::thread::spawn(move || {
                    post.send(Event::Conversation(Err(Fault::Refused("this reference backend has no shared workspace to converse in".to_owned()))))
                });
            }
        }
    }
}

fn account(display: String) -> Account {
    Account { display, capabilities: vec![Capability::Files, Capability::Etags], provider_linked: true }
}

/// Turns a [`Job`]'s slash-separated path into a path relative to `root`,
/// refusing anything that is not a run of ordinary names. The result is
/// never joined onto `root` as an OS path; every caller hands it to a
/// [`cap_std::fs::Dir`] opened on `root` instead, which is the layer that
/// actually keeps resolution from leaving `root`.
///
/// `.` and repeated or leading slashes are just an empty segment and are
/// skipped, the same way a browser's address bar collapses them. Every other
/// segment goes through [`confine_segment`].
fn relative(path: &str) -> Result<PathBuf, Fault> {
    let mut rel = PathBuf::new();
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        rel.push(confine_segment(segment)?);
    }
    Ok(rel)
}

/// Confines one path segment to a single ordinary file or folder name.
///
/// Accepted only when `segment` parses as exactly one [`Component::Normal`]
/// equal to `segment` itself: no empty name, no `.` or `..`, no absolute
/// form, and nothing left over for a second component to hide in.
///
/// This is deliberately lenient about an ordinary character such as `:` or
/// `\` that is not, on its own, a path separator on any platform's
/// filesystem: `list` discovers real names from disk, and a name containing
/// one of those (macOS stores a `/` typed in Finder as `:` on disk) must stay
/// reachable by a later `read` or `write`, not become visible-but-unopenable.
/// [`confine_name`] is the stricter check for a name being chosen new, where
/// refusing those characters costs nothing and closes a real escape.
fn confine_segment(segment: &str) -> Result<&str, Fault> {
    let mut components = Path::new(segment).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None) if name == OsStr::new(segment) => Ok(segment),
        _ => Err(Fault::Refused(format!("{segment} is not a plain file name"))),
    }
}

/// Windows device names that name a device rather than a file, with or
/// without a trailing extension (`CON`, `con.txt`, ...). Checked on every
/// platform: a name chosen here can end up read on a Windows client too.
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3",
    "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn is_windows_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    WINDOWS_RESERVED_NAMES.iter().any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

/// Confines a [`Job::Create`] `name` to a single ordinary file name that is
/// safe to choose on any platform, not only the one this crate is built for.
///
/// Stricter than [`confine_segment`]: a backslash or colon is refused
/// outright first, because a Unix build's [`Component`] parser does not
/// treat either as a separator and would wave a Windows drive letter or UNC
/// prefix (`C:`, `\\server\share`) straight through as one ordinary name. A
/// Windows reserved device name is refused the same way. None of this costs
/// anything here, unlike applying it to a name `list` merely discovered.
fn confine_name(name: &str) -> Result<&str, Fault> {
    if name.contains('\\') || name.contains(':') {
        return Err(Fault::Refused(format!("{name} is not a plain file name")));
    }
    if is_windows_reserved_name(name) {
        return Err(Fault::Refused(format!("{name} is a reserved device name on Windows")));
    }
    confine_segment(name)
}

/// Opens `root` as the one ambient-authority capability this crate holds.
/// Every other path in this crate is resolved relative to the [`Dir`] this
/// returns, never as an absolute OS path built by string concatenation.
fn open_root(root: &Path) -> Result<Dir, Fault> {
    Dir::open_ambient_dir(root, cap_std::ambient_authority()).map_err(|error| io_fault(&error))
}

/// The path a listed child gets, given the folder it was found in.
fn child_path(folder: &str, name: &str) -> String {
    format!("{}/{name}", folder.trim_end_matches('/'))
}

/// An opaque version token for a file's current bytes.
///
/// Not a timestamp for a person to read (that would go through `chrono`, per
/// this workspace's rule); just enough to tell "the file I read" from "the
/// file as it is now" apart, which is all a conditional write needs.
fn etag_of(meta: &cap_std::fs::Metadata) -> String {
    let stamp =
        meta.modified().ok().and_then(|time| time.duration_since(cap_std::time::SystemClock::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{}-{stamp}", meta.len())
}

fn io_fault(error: &std::io::Error) -> Fault {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::NotFound => Fault::Refused("no such file or folder".to_owned()),
        ErrorKind::PermissionDenied => Fault::Refused("permission denied".to_owned()),
        _ => Fault::Unreachable(error.to_string()),
    }
}

fn list(root: &Path, path: &str) -> Result<Vec<Entry>, Fault> {
    let rel = relative(path)?;
    let dir = open_root(root)?;
    let read = if rel.as_os_str().is_empty() { dir.entries() } else { dir.read_dir(&rel) };
    let read = read.map_err(|error| io_fault(&error))?;
    let mut entries = Vec::new();
    for item in read {
        let item = item.map_err(|error| io_fault(&error))?;
        let meta = item.metadata().map_err(|error| io_fault(&error))?;
        let name = item.file_name().to_string_lossy().into_owned();
        let id = child_path(path, &name);
        entries.push(Entry {
            room: format!("local:{id}"),
            name,
            kind: if meta.is_dir() { EntryKind::Folder } else { EntryKind::File },
            size: if meta.is_dir() { None } else { Some(meta.len()) },
            modified: None,
            mime: None,
            etag: if meta.is_dir() { None } else { Some(etag_of(&meta)) },
            writable: true,
            shared_by: None,
            path: id.clone(),
            id,
        });
    }
    entries.sort_by(|a, b| (a.kind != EntryKind::Folder, &a.name).cmp(&(b.kind != EntryKind::Folder, &b.name)));
    Ok(entries)
}

/// Reads a file's bytes together with the entry for the version just read.
///
/// The entry is built from a fresh metadata call here, not handed back
/// unchanged from what the caller already believed. A reload after a conflict
/// hands in the stale entry it is trying to get past; echoing that back would
/// carry the old etag into the reloaded document, so the very next save would
/// conflict again on a copy that is actually current.
///
/// Refuses outright if `entry.id` names a symlink, the same policy `write`
/// uses: this backend never reports the bytes of whatever a link happens to
/// point at, inside `root` or out.
fn read(root: &Path, entry: &Entry) -> Result<(Entry, Vec<u8>), Fault> {
    let rel = relative(&entry.id)?;
    let dir = open_root(root)?;
    // Stats without following only the last component; every component
    // before it is still resolved, and sandboxed, normally, so a link
    // earlier in `rel` is refused by `open_root`'s `Dir` itself when `open`
    // below walks it, never silently followed here either way.
    match dir.symlink_metadata(&rel) {
        Ok(meta) if meta.is_symlink() => return Err(Fault::Refused("that path is a link, not a plain file".to_owned())),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_fault(&error)),
    }
    let mut file = dir.open(&rel).map_err(|error| io_fault(&error))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|error| io_fault(&error))?;
    let meta = file.metadata().map_err(|error| io_fault(&error))?;
    let fresh = Entry { size: Some(meta.len()), etag: Some(etag_of(&meta)), ..entry.clone() };
    Ok((fresh, bytes))
}

/// Saves back over `entry`, refusing the write when the file moved on since
/// `entry.etag` was read. A file that no longer exists has nothing to
/// conflict with, the same way a provider treats a deleted document.
///
/// Never truncates `entry.id`'s current bytes in place. [`write_replacing`]
/// writes a sibling temporary file and renames it over the target instead,
/// so a hard link sharing the target's inode keeps its own, unmodified copy:
/// the rename replaces `entry.id`'s directory entry, and never reaches
/// through to any other name pointing at the same inode.
fn write(root: &Path, entry: &Entry, bytes: &[u8]) -> Result<Entry, Fault> {
    let rel = relative(&entry.id)?;
    let dir = open_root(root)?;
    match dir.symlink_metadata(&rel) {
        Ok(meta) if meta.is_symlink() => return Err(Fault::Refused("that path is a link, not a plain file".to_owned())),
        Ok(meta) => {
            if Some(etag_of(&meta)) != entry.etag {
                return Err(Fault::Conflict);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_fault(&error)),
    }
    if let Some(parent) = rel.parent() {
        dir.create_dir_all(parent).map_err(|error| io_fault(&error))?;
    }
    write_replacing(&dir, &rel, bytes)?;
    let meta = dir.metadata(&rel).map_err(|error| io_fault(&error))?;
    Ok(Entry { size: Some(meta.len()), etag: Some(etag_of(&meta)), ..entry.clone() })
}

/// The next in a run of sibling temporary names, unique within this process.
/// Not meant to resist a second process picking the same name; the create
/// this writes with is exclusive (`create_new`), so a collision there is
/// just retried under a new counter value rather than silently overwritten.
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to a new sibling of `rel` inside `dir`, then renames that
/// sibling over `rel`. See the module docs for why: a direct write would
/// truncate whatever `rel` currently names in place, and if that happens to
/// be a hard link to a file outside `root`, the outside copy would be
/// corrupted along with it. A rename only ever replaces `rel`'s own
/// directory entry.
fn write_replacing(dir: &Dir, rel: &Path, bytes: &[u8]) -> Result<(), Fault> {
    let name = rel.file_name().ok_or_else(|| Fault::Refused("that path names no file".to_owned()))?;
    let mut temp_name = OsStr::new(".").to_os_string();
    temp_name.push(name);
    temp_name.push(format!(".tmp-{}-{}", std::process::id(), TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)));
    let temp_rel = match rel.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(&temp_name),
        _ => PathBuf::from(&temp_name),
    };
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let written = (|| -> Result<(), Fault> {
        let mut handle = dir.open_with(&temp_rel, &options).map_err(|error| io_fault(&error))?;
        handle.write_all(bytes).map_err(|error| io_fault(&error))?;
        handle.sync_all().map_err(|error| io_fault(&error))
    })();
    if let Err(fault) = written {
        let _ = dir.remove_file(&temp_rel);
        return Err(fault);
    }
    if let Err(error) = dir.rename(&temp_rel, dir, rel) {
        let _ = dir.remove_file(&temp_rel);
        return Err(io_fault(&error));
    }
    Ok(())
}

fn create(root: &Path, folder: &str, name: &str, bytes: &[u8]) -> Result<Entry, Fault> {
    // `name` is never a path: it is one `Job::Create` field that names the
    // new file, so it goes through `confine_name` directly rather than
    // through `relative`, which only ever splits and confines a slash path.
    let name = confine_name(name)?;
    // `root` is the operator-configured boundary, never attacker input, so
    // creating it directly with `std::fs` is safe; this is also the one
    // place that has always brought a fresh root into existence (see
    // `LocalBackend::new`'s docs), and `Dir::open_ambient_dir` below needs it
    // to already exist.
    fs::create_dir_all(root).map_err(|error| io_fault(&error))?;
    let dir = open_root(root)?;
    let rel_folder = relative(folder)?;
    dir.create_dir_all(&rel_folder).map_err(|error| io_fault(&error))?;
    let rel = rel_folder.join(name);
    // Exclusive creation, not an `exists` check ahead of a separate write: two
    // creates of the same name racing both see "not there yet" otherwise, and
    // the second write silently replaces the first with neither ever refused.
    // `create_new` asks the filesystem to open and create in one step, so the
    // loser gets an error instead of a chance to overwrite the winner. It also
    // refuses outright when `name` already exists as a symlink, dangling or
    // not: `O_EXCL` never follows one to decide whether it is "in the way".
    let mut create_options = OpenOptions::new();
    create_options.write(true).create_new(true);
    let mut handle = match dir.open_with(&rel, &create_options) {
        Ok(handle) => handle,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(Fault::Refused(format!("{name} already exists")));
        }
        Err(error) => return Err(io_fault(&error)),
    };
    handle.write_all(bytes).map_err(|error| io_fault(&error))?;
    drop(handle);
    let meta = dir.metadata(&rel).map_err(|error| io_fault(&error))?;
    let id = child_path(folder, name);
    Ok(Entry {
        room: format!("local:{id}"),
        name: name.to_owned(),
        kind: EntryKind::File,
        size: Some(meta.len()),
        modified: None,
        mime: None,
        etag: Some(etag_of(&meta)),
        writable: true,
        shared_by: None,
        path: id.clone(),
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parent_segment_cannot_walk_out_of_root() {
        assert!(relative("../secrets").is_err());
        assert!(relative("Archive/../../../etc/passwd").is_err());
    }

    #[test]
    fn repeated_and_trailing_slashes_collapse_like_a_browser_address_bar() {
        assert_eq!(relative("//Archive//2026/").unwrap(), Path::new("Archive").join("2026"));
        assert_eq!(relative("").unwrap(), PathBuf::new());
    }

    #[test]
    fn confine_segment_accepts_only_a_single_ordinary_name() {
        for ordinary in ["Minutes.txt", "Archive", "2026", "a name with spaces.docx"] {
            assert!(confine_segment(ordinary).is_ok(), "{ordinary} is an ordinary name");
        }
    }

    #[test]
    fn confine_segment_refuses_empty_dot_and_dot_dot() {
        for bad in ["", ".", ".."] {
            assert!(confine_segment(bad).is_err(), "{bad:?} is not a single ordinary name");
        }
    }

    #[test]
    fn confine_segment_refuses_a_slash_hidden_inside_a_create_name() {
        assert!(confine_segment("folder/evil").is_err());
        assert!(confine_segment("../outside.txt").is_err());
        assert!(confine_segment("/outside.txt").is_err());
    }

    /// The fix for the non-blocking finding that `list` made a real macOS
    /// name (a literal `/`, stored on disk as `:`) visible but unreadable: an
    /// existing name with an ordinary `:` byte is not refused by the lenient
    /// per-segment check, only by `confine_name` for a chosen one. A literal
    /// `\` is not tested the same way here: on a Windows build it really is
    /// a second path component (`std::path::Component` already splits on
    /// it), so there is no existing-name case for `confine_segment` to stay
    /// lenient about cross-platform the way there is for `:`.
    #[test]
    fn confine_segment_tolerates_a_colon_already_on_disk() {
        assert!(confine_segment("weird:name.txt").is_ok());
        assert!(relative("weird:name.txt").is_ok());
    }

    #[test]
    fn confine_name_refuses_backslash_and_colon_on_every_platform() {
        for bad in ["..\\outside.txt", "C:\\x", "C:evil", "\\\\server\\share\\x"] {
            assert!(confine_name(bad).is_err(), "{bad:?} must be refused on every platform, not only on Windows");
        }
    }

    #[test]
    fn confine_name_refuses_windows_reserved_device_names() {
        for bad in ["CON", "con", "con.txt", "NUL", "COM1", "lpt9.log", "CONIN$"] {
            assert!(confine_name(bad).is_err(), "{bad:?} names a Windows device, not a file");
        }
        for ordinary in ["console.txt", "NULLABLE.md", "COM10"] {
            assert!(confine_name(ordinary).is_ok(), "{ordinary} only resembles a reserved name");
        }
    }

    #[test]
    fn a_folder_announces_no_capability_this_backend_cannot_honour() {
        let reported = account("Local disk".to_owned()).capabilities;
        for absent in [Capability::SharedWithMe, Capability::Sharing, Capability::Conversation, Capability::Contacts] {
            assert!(!reported.contains(&absent), "a folder has no {absent:?}");
        }
    }
}
