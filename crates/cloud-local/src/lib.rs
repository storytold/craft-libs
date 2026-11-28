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
//! 1. `confine_segment` (every path segment) and `confine_name` (a
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
//! 2. `open_root` opens `root` as a [`cap_std::fs::Dir`] capability once per
//!    job. Every `list`, `read`, `write` and `create` routes its
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
use std::sync::Mutex;
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
/// Windows reserved device name is refused the same way, and so is a name
/// [`is_temp_name`] matches, which [`list`] would leave out. None of this
/// costs anything here, unlike applying it to a name `list` merely discovered.
fn confine_name(name: &str) -> Result<&str, Fault> {
    if name.contains('\\') || name.contains(':') {
        return Err(Fault::Refused(format!("{name} is not a plain file name")));
    }
    if is_windows_reserved_name(name) {
        return Err(Fault::Refused(format!("{name} is a reserved device name on Windows")));
    }
    if is_temp_name(name) {
        return Err(Fault::Refused(format!("{name} is a name this backend uses for its temporary files")));
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

/// Lists the files and folders in `path`.
///
/// A regular file whose name [`is_temp_name`] matches is left out before its
/// metadata is read. Such a file is a save in progress, or one that was
/// stopped between its create and its rename, and it is not a document.
/// [`confine_name`] refuses these names for a new file. A folder or a link
/// with such a name is listed.
fn list(root: &Path, path: &str) -> Result<Vec<Entry>, Fault> {
    let rel = relative(path)?;
    let dir = open_root(root)?;
    let read = if rel.as_os_str().is_empty() { dir.entries() } else { dir.read_dir(&rel) };
    let read = read.map_err(|error| io_fault(&error))?;
    let mut entries = Vec::new();
    for item in read {
        let item = item.map_err(|error| io_fault(&error))?;
        let name = item.file_name().to_string_lossy().into_owned();
        if is_temp_name(&name) && item.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let meta = item.metadata().map_err(|error| io_fault(&error))?;
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
///
/// The etag is checked once before any bytes are written, and again under
/// [`WRITE_LOCK`] just before the rename. Two saves made from the same etag
/// at once otherwise both pass the check before either renames, both report
/// success, and the later rename replaces the first save without a
/// [`Fault::Conflict`].
fn write(root: &Path, entry: &Entry, bytes: &[u8]) -> Result<Entry, Fault> {
    let rel = relative(&entry.id)?;
    let dir = open_root(root)?;
    check_unchanged(&dir, &rel, entry)?;
    if let Some(parent) = rel.parent() {
        dir.create_dir_all(parent).map_err(|error| io_fault(&error))?;
    }
    let meta = write_replacing(&dir, &rel, bytes, || check_unchanged(&dir, &rel, entry))?;
    Ok(Entry { size: Some(meta.len()), etag: Some(etag_of(&meta)), ..entry.clone() })
}

/// Refuses a link at `rel`, and a file whose etag is no longer `entry.etag`.
/// A file that does not exist passes.
fn check_unchanged(dir: &Dir, rel: &Path, entry: &Entry) -> Result<(), Fault> {
    match dir.symlink_metadata(rel) {
        Ok(meta) if meta.is_symlink() => Err(Fault::Refused("that path is a link, not a plain file".to_owned())),
        Ok(meta) if Some(etag_of(&meta)) != entry.etag => Err(Fault::Conflict),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_fault(&error)),
    }
}

/// Held by [`write_replacing`] from its last check until the rename is done
/// and the new file's metadata is read, and by [`create_in_place`] from the
/// moment it creates its file until the file is complete or removed. No other
/// save in this process can change the file between a save's check and its
/// rename, a save cannot replace a file that `create_in_place` is still
/// writing, and `create_in_place` cannot remove a replacement when its own
/// write fails. The temporary file is written and synced before the lock is
/// taken, so a slow save does not hold up the others. Another process writing
/// the same folder is not covered.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// The next in a run of sibling temporary names, unique within this process.
/// Not meant to resist a second process picking the same name; the create
/// [`open_temp`] does is exclusive (`create_new`), so a collision there is
/// retried under a new counter value rather than silently overwritten.
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// How many temporary names [`open_temp`] tries before it gives up.
const TEMP_ATTEMPTS: usize = 16;

/// The start of every name [`temp_name`] makes.
const TEMP_PREFIX: &str = ".craft-save-";

/// The end of every name [`temp_name`] makes.
const TEMP_SUFFIX: &str = ".tmp";

/// The temporary name for process `pid`, clock part `started` and sequence
/// value `n`. [`is_temp_name`] matches every name this returns.
fn temp_name(pid: u32, started: u128, n: u64) -> String {
    format!("{TEMP_PREFIX}{pid}-{started:x}-{n}{TEMP_SUFFIX}")
}

/// Whether [`temp_name`] returns exactly `name` for some process id, clock
/// part and sequence value. The three parts are read back from `name` and
/// passed to `temp_name`, so a name the two would format differently (a
/// leading zero, an uppercase hexadecimal digit, a sign) does not match.
/// [`list`] leaves these names out. A name with only the same prefix and
/// suffix, such as `.craft-save-notes.tmp`, does not match and is listed.
fn is_temp_name(name: &str) -> bool {
    let Some(middle) = name.strip_prefix(TEMP_PREFIX).and_then(|rest| rest.strip_suffix(TEMP_SUFFIX)) else {
        return false;
    };
    let mut parts = middle.split('-');
    let (Some(pid), Some(started), Some(n), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    match (pid.parse::<u32>(), u128::from_str_radix(started, 16), n.parse::<u64>()) {
        (Ok(pid), Ok(started), Ok(n)) => temp_name(pid, started, n) == name,
        _ => false,
    }
}

/// Creates a new hidden temporary file beside `rel`, named from
/// [`TEMP_SEQUENCE`], and returns its path and handle. A name that already
/// exists (a file another process or an earlier run left) is skipped for the
/// next one, and never opened or removed. The name does not include `rel`'s
/// file name, so a file name near the filesystem's length limit still gets a
/// temporary name that fits.
fn open_temp(dir: &Dir, rel: &Path) -> Result<(PathBuf, cap_std::fs::File), Fault> {
    if rel.file_name().is_none() {
        return Err(Fault::Refused("that path names no file".to_owned()));
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    // The clock part keeps a stale file from an earlier run with the same
    // process id from matching a run of names, so needing all the attempts
    // takes 16 files made for this exact moment.
    let started = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_nanos());
    #[cfg(test)]
    let started = if tests::FIXED_TEMP_CLOCK.with(std::cell::Cell::get) { 0 } else { started };
    let mut last = None;
    for _ in 0..TEMP_ATTEMPTS {
        let temp_name = temp_name(std::process::id(), started, TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed));
        let temp_rel = match rel.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.join(&temp_name),
            _ => PathBuf::from(&temp_name),
        };
        match dir.open_with(&temp_rel, &options) {
            Ok(handle) => return Ok((temp_rel, handle)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => last = Some(error),
            Err(error) => return Err(io_fault(&error)),
        }
    }
    Err(last.map_or_else(|| Fault::Unreachable("no temporary name was free".to_owned()), |error| io_fault(&error)))
}

/// Writes `bytes` to a new sibling of `rel` inside `dir`, then renames that
/// sibling over `rel`. See the module docs for why: a direct write would
/// truncate whatever `rel` currently names in place, and if that happens to
/// be a hard link to a file outside `root`, the outside copy would be
/// corrupted along with it. A rename only ever replaces `rel`'s own
/// directory entry.
///
/// `check` runs under [`WRITE_LOCK`] right before the rename; when it fails,
/// the temporary file is removed and `rel` is left as it was. Returns the
/// metadata of the file just published.
fn write_replacing(dir: &Dir, rel: &Path, bytes: &[u8], check: impl FnOnce() -> Result<(), Fault>) -> Result<cap_std::fs::Metadata, Fault> {
    let (temp_rel, mut handle) = open_temp(dir, rel)?;
    let written = handle.write_all(bytes).map_err(|error| io_fault(&error)).and_then(|()| handle.sync_all().map_err(|error| io_fault(&error)));
    drop(handle);
    if let Err(fault) = written {
        let _ = dir.remove_file(&temp_rel);
        return Err(fault);
    }
    #[cfg(test)]
    tests::before_lock();
    let published = {
        let _serialized = WRITE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        check().and_then(|()| {
            #[cfg(test)]
            tests::after_check();
            dir.rename(&temp_rel, dir, rel).map_err(|error| io_fault(&error))?;
            dir.metadata(rel).map_err(|error| io_fault(&error))
        })
    };
    if published.is_err() {
        let _ = dir.remove_file(&temp_rel);
    }
    published
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
    // A taken name is refused before any bytes are written. The hard link
    // below is still the check that settles a race.
    match dir.symlink_metadata(&rel) {
        Ok(_) => return Err(Fault::Refused(format!("{name} already exists"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_fault(&error)),
    }
    // The bytes go to a sibling temporary file first. A write that fails part
    // way (a full disk, a lost share) leaves only that file, which is removed,
    // so the name stays free for the next Save As instead of holding a partial
    // file that makes it fail with "already exists".
    let (temp_rel, mut handle) = open_temp(&dir, &rel)?;
    let written = write_new(&mut handle, bytes).and_then(|()| sync_new(&handle));
    drop(handle);
    if let Err(fault) = written {
        let _ = dir.remove_file(&temp_rel);
        return Err(fault);
    }
    // The finished file takes `name` with a hard link, not an `exists` check
    // ahead of a rename: two creates of the same name racing both see "not
    // there yet" otherwise, and the second silently replaces the first. A link
    // fails when the name is taken, so the loser gets an error instead of a
    // chance to overwrite the winner. It also fails when `name` already exists
    // as a symlink, dangling or not.
    #[cfg(not(test))]
    let linked = dir.hard_link(&temp_rel, &dir, &rel);
    #[cfg(test)]
    let linked = if tests::NO_HARD_LINKS.with(std::cell::Cell::get) {
        Err(std::io::Error::other("no hard links"))
    } else {
        dir.hard_link(&temp_rel, &dir, &rel)
    };
    let _ = dir.remove_file(&temp_rel);
    match linked {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Err(Fault::Refused(format!("{name} already exists"))),
        // A filesystem without hard links (FAT, some network shares): the
        // temporary copy is gone by now, and the file is created in place.
        Err(_) => create_in_place(&dir, &rel, name, bytes)?,
    }
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

/// Creates `rel` with `create_new` and writes `bytes` into it, for a
/// filesystem where [`create`] cannot publish with a hard link. `create_new`
/// opens and creates in one step, so a name that is taken, or that exists as
/// a symlink, is refused instead of overwritten. The bytes are synced, so a
/// share that reports a full disk only at sync is a failed create. When the
/// write or the sync fails, the file this call created is removed.
/// [`WRITE_LOCK`] is held throughout, so no save in this process can replace
/// the file before that removal.
fn create_in_place(dir: &Dir, rel: &Path, name: &str, bytes: &[u8]) -> Result<(), Fault> {
    let _writing = WRITE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut create_options = OpenOptions::new();
    create_options.write(true).create_new(true);
    let mut handle = match dir.open_with(rel, &create_options) {
        Ok(handle) => handle,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Err(Fault::Refused(format!("{name} already exists"))),
        Err(error) => return Err(io_fault(&error)),
    };
    let written = write_new(&mut handle, bytes).and_then(|()| sync_new(&handle));
    drop(handle);
    #[cfg(test)]
    tests::in_place_finished();
    if written.is_err() {
        let _ = dir.remove_file(rel);
    }
    written
}

/// Syncs a file [`create`] is making.
fn sync_new(handle: &cap_std::fs::File) -> Result<(), Fault> {
    #[cfg(test)]
    if tests::FAIL_CREATE_SYNC.with(|countdown| {
        let left = countdown.get();
        countdown.set(left.saturating_sub(1));
        left == 1
    }) {
        return Err(Fault::Unreachable("no space left on device".to_owned()));
    }
    handle.sync_all().map_err(|error| io_fault(&error))
}

/// Writes the bytes of a file [`create`] is making.
fn write_new(handle: &mut cap_std::fs::File, bytes: &[u8]) -> Result<(), Fault> {
    #[cfg(test)]
    if tests::FAIL_CREATE_WRITE.with(|countdown| {
        let left = countdown.get();
        countdown.set(left.saturating_sub(1));
        left == 1
    }) {
        let half = bytes.get(..bytes.len() / 2).unwrap_or_default();
        handle.write_all(half).map_err(|error| io_fault(&error))?;
        return Err(Fault::Unreachable("no space left on device".to_owned()));
    }
    handle.write_all(bytes).map_err(|error| io_fault(&error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// Run once by the next [`write`] to pass its etag check, then cleared.
    type Hook = Box<dyn FnOnce() + Send>;
    static AFTER_CHECK: Mutex<Option<Hook>> = Mutex::new(None);

    pub(super) fn after_check() {
        let hook = AFTER_CHECK.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        if let Some(hook) = hook {
            hook();
        }
    }

    /// Run once by the next [`write_replacing`] to finish its temporary file,
    /// before it takes [`WRITE_LOCK`], then cleared.
    static BEFORE_LOCK: Mutex<Option<Hook>> = Mutex::new(None);

    pub(super) fn before_lock() {
        let hook = BEFORE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        if let Some(hook) = hook {
            hook();
        }
    }

    fn entry_for(id: &str) -> Entry {
        Entry {
            room: format!("local:{id}"),
            name: id.trim_start_matches('/').to_owned(),
            kind: EntryKind::File,
            size: None,
            modified: None,
            mime: None,
            etag: None,
            writable: true,
            shared_by: None,
            path: id.to_owned(),
            id: id.to_owned(),
        }
    }

    #[test]
    fn two_saves_from_one_etag_cannot_both_pass_the_check() {
        let _serial = TEMP_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Note.txt"), b"original").unwrap();
        let (read_entry, _) = read(root.path(), &entry_for("/Note.txt")).unwrap();

        let (checked_tx, checked_rx) = mpsc::channel::<()>();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        *AFTER_CHECK.lock().unwrap() = Some(Box::new(move || {
            checked_tx.send(()).unwrap();
            go_rx.recv().unwrap();
        }));

        let first = {
            let (root, entry) = (root.path().to_owned(), read_entry.clone());
            std::thread::spawn(move || write(&root, &entry, b"first save"))
        };
        checked_rx.recv().unwrap();
        // The first save has passed its check and not yet renamed. A second
        // save must not be able to reach its own check in between.
        assert!(WRITE_LOCK.try_lock().is_err(), "the check and the rename must run under one lock");
        // The second save passes its early check and writes its temporary
        // file before the first one renames, so only the recheck under the
        // lock can refuse it.
        let (ready_tx, ready_rx) = mpsc::channel::<()>();
        *BEFORE_LOCK.lock().unwrap() = Some(Box::new(move || ready_tx.send(()).unwrap()));
        let second = {
            let (root, entry) = (root.path().to_owned(), read_entry.clone());
            std::thread::spawn(move || write(&root, &entry, b"second save"))
        };
        ready_rx.recv().unwrap();
        go_tx.send(()).unwrap();

        assert!(first.join().unwrap().is_ok());
        assert!(matches!(second.join().unwrap(), Err(Fault::Conflict)), "the second save started from a stale etag");
        assert_eq!(std::fs::read(root.path().join("Note.txt")).unwrap(), b"first save");
        let left: Vec<_> = std::fs::read_dir(root.path()).unwrap().map(|item| item.unwrap().file_name()).collect();
        assert_eq!(left, ["Note.txt"], "the refused save's temporary file is removed");
    }

    thread_local! {
        /// When set to n, the nth [`write_new`] call on this thread from now
        /// writes half its bytes and fails. 0 turns it off.
        pub(super) static FAIL_CREATE_WRITE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
        /// Makes [`create`] on this thread take the path for a filesystem
        /// without hard links.
        pub(super) static NO_HARD_LINKS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        /// Makes [`open_temp`] on this thread use 0 for its clock part.
        pub(super) static FIXED_TEMP_CLOCK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        /// When set to n, the nth [`sync_new`] call on this thread from now
        /// fails. 0 turns it off.
        pub(super) static FAIL_CREATE_SYNC: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
        /// Run once by [`create_in_place`] on this thread after its write and
        /// sync, before it removes a failed file.
        static IN_PLACE_FINISHED: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    }

    pub(super) fn in_place_finished() {
        if let Some(hook) = IN_PLACE_FINISHED.with(|hook| hook.borrow_mut().take()) {
            hook();
        }
    }

    #[test]
    fn without_hard_links_a_failed_sync_leaves_no_file_and_keeps_a_concurrent_save() {
        let _serial = TEMP_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let root = tempfile::tempdir().unwrap();
        let path = root.path().to_path_buf();
        let held = std::rc::Rc::new(std::cell::Cell::new(false));
        let saver = std::rc::Rc::new(std::cell::RefCell::new(None));
        let (seen_held, seen_saver) = (held.clone(), saver.clone());
        IN_PLACE_FINISHED.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                seen_held.set(WRITE_LOCK.try_lock().is_err());
                // A save of the same name starts while the failed file is
                // still there. It must not land before that file is removed.
                *seen_saver.borrow_mut() = Some(std::thread::spawn(move || {
                    let dir = open_root(&path).unwrap();
                    write_replacing(&dir, Path::new("Minutes.txt"), b"saved meanwhile", || Ok(()))
                }));
            }));
        });
        NO_HARD_LINKS.with(|off| off.set(true));
        // The temporary copy syncs, then the in-place file's sync fails.
        FAIL_CREATE_SYNC.with(|fail| fail.set(2));
        let failed = create(root.path(), "", "Minutes.txt", b"the whole document");
        FAIL_CREATE_SYNC.with(|fail| fail.set(0));
        NO_HARD_LINKS.with(|off| off.set(false));

        assert!(matches!(failed, Err(Fault::Unreachable(_))), "a failed sync is a failed create: {failed:?}");
        assert!(held.get(), "the in-place file is written and removed under the write lock");
        let saver = saver.borrow_mut().take().unwrap();
        saver.join().unwrap().unwrap();
        let left: Vec<_> = std::fs::read_dir(root.path()).unwrap().map(|item| item.unwrap().file_name()).collect();
        assert_eq!(left, ["Minutes.txt"]);
        assert_eq!(std::fs::read(root.path().join("Minutes.txt")).unwrap(), b"saved meanwhile", "the removal took only the failed file");
    }

    /// Held by the tests that make temporary files, so the next value of
    /// [`TEMP_SEQUENCE`] is known to the test that reads it.
    static TEMP_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn a_stale_temporary_file_is_skipped_and_kept() {
        let _serial = TEMP_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let root = tempfile::tempdir().unwrap();
        let next = TEMP_SEQUENCE.load(Ordering::Relaxed);
        let stale: Vec<_> = (next..next + 3).map(|n| temp_name(std::process::id(), 0, n)).collect();
        for name in &stale {
            std::fs::write(root.path().join(name), b"left by an earlier run").unwrap();
        }
        FIXED_TEMP_CLOCK.with(|fixed| fixed.set(true));
        let made = create(root.path(), "", "Minutes.txt", b"minutes");
        FIXED_TEMP_CLOCK.with(|fixed| fixed.set(false));
        made.unwrap();
        assert_eq!(std::fs::read(root.path().join("Minutes.txt")).unwrap(), b"minutes");
        for name in &stale {
            assert_eq!(std::fs::read(root.path().join(name)).unwrap(), b"left by an earlier run", "{name} is not this call's file");
        }
    }

    #[test]
    fn a_listing_leaves_out_a_temporary_file_a_stopped_save_left() {
        let _serial = TEMP_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("Archive")).unwrap();
        std::fs::write(root.path().join("Minutes.txt"), b"minutes").unwrap();
        std::fs::write(root.path().join(".craft-save-notes.tmp"), b"a person's own file").unwrap();
        let folder = temp_name(1, 2, 3);
        std::fs::create_dir(root.path().join(&folder)).unwrap();
        let dir = open_root(root.path()).unwrap();
        // A save stopped after its create and before its rename leaves the
        // temporary file, at the root and inside a folder.
        let (at_root, handle) = open_temp(&dir, Path::new("Minutes.txt")).unwrap();
        drop(handle);
        let (in_folder, handle) = open_temp(&dir, &Path::new("Archive").join("Agenda.txt")).unwrap();
        drop(handle);
        assert!(root.path().join(&at_root).exists() && root.path().join(&in_folder).exists());

        let names: Vec<_> = list(root.path(), "").unwrap().into_iter().map(|entry| entry.name).collect();
        assert_eq!(names, [".craft-save-1-2-3.tmp", "Archive", ".craft-save-notes.tmp", "Minutes.txt"], "a folder with the pattern stays listed");
        assert!(list(root.path(), "/Archive").unwrap().is_empty());
    }

    #[test]
    fn a_create_refuses_a_name_a_listing_would_leave_out() {
        let _serial = TEMP_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let root = tempfile::tempdir().unwrap();
        let name = temp_name(1, 2, 3);
        assert!(matches!(create(root.path(), "", &name, b"bytes"), Err(Fault::Refused(_))));
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        create(root.path(), "", ".craft-save-notes.tmp", b"bytes").unwrap();
    }

    #[test]
    fn is_temp_name_matches_only_the_names_temp_name_makes() {
        assert!(is_temp_name(&temp_name(4242, 0x1a2b, 7)));
        assert!(is_temp_name(&temp_name(u32::MAX, u128::MAX, u64::MAX)));
        for other in [
            ".craft-save-notes.tmp",
            ".craft-save-1-2.tmp",
            ".craft-save-1-2-3-4.tmp",
            ".craft-save--2-3.tmp",
            ".craft-save-1-xy-3.tmp",
            "craft-save-1-2-3.tmp",
            ".craft-save-1-2-3.tmp.txt",
            ".craft-save-01-2-3.tmp",
            ".craft-save-1-2A-3.tmp",
            ".craft-save-+1-2-3.tmp",
        ] {
            assert!(!is_temp_name(other), "{other} is not a temporary name");
        }
    }

    #[test]
    fn without_hard_links_a_create_still_refuses_a_taken_name_and_cleans_up() {
        let _serial = TEMP_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let root = tempfile::tempdir().unwrap();
        NO_HARD_LINKS.with(|off| off.set(true));
        let made = create(root.path(), "", "Minutes.txt", b"first");
        let taken = create(root.path(), "", "Minutes.txt", b"second");
        // The temporary copy is written, then the in-place write fails.
        FAIL_CREATE_WRITE.with(|fail| fail.set(2));
        let failed = create(root.path(), "", "Agenda.txt", b"the whole agenda");
        FAIL_CREATE_WRITE.with(|fail| fail.set(0));
        NO_HARD_LINKS.with(|off| off.set(false));

        assert_eq!(made.unwrap().size, Some(5));
        assert!(matches!(taken, Err(Fault::Refused(_))));
        assert!(matches!(failed, Err(Fault::Unreachable(_))));
        let left: Vec<_> = std::fs::read_dir(root.path()).unwrap().map(|item| item.unwrap().file_name()).collect();
        assert_eq!(left, ["Minutes.txt"], "only the finished file is left");
        assert_eq!(std::fs::read(root.path().join("Minutes.txt")).unwrap(), b"first");
    }

    #[test]
    fn a_create_whose_write_fails_leaves_no_file_behind() {
        let _serial = TEMP_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let root = tempfile::tempdir().unwrap();
        FAIL_CREATE_WRITE.with(|fail| fail.set(1));
        let failed = create(root.path(), "", "Minutes.txt", b"the whole document");
        FAIL_CREATE_WRITE.with(|fail| fail.set(0));
        assert!(matches!(failed, Err(Fault::Unreachable(_))));
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0, "no partial or temporary file is left");

        let retried = create(root.path(), "", "Minutes.txt", b"the whole document").unwrap();
        assert_eq!(retried.size, Some(18));
        assert_eq!(std::fs::read(root.path().join("Minutes.txt")).unwrap(), b"the whole document");
    }

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
