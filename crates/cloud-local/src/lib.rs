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

use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};

use craft_cloud::{Capability, Entry, EntryKind};
use craft_ui::cloud::{Account, CloudBackend, Event, Fault, Job, Post};

/// Where "the cloud" lives for this backend: one directory on disk.
///
/// `Entry::id` and `Entry::path` are the same slash-separated path relative to
/// `root`, always starting with `/` below the root itself (the root's own
/// path is the empty string, matching [`Job::List`]'s contract). Neither is
/// an OS path; both go through [`resolve`] and then [`confine_to_root`]
/// before anything touches a filesystem. A [`Job::Create`] name is not a
/// path at all; it goes through the same per-segment check [`resolve`] uses,
/// [`confine_segment`], on its own. Together the two refuse `..`, an
/// absolute form, a Windows drive letter or UNC prefix, and a backslash,
/// regardless of which platform this crate is built for, since a name
/// reaches this backend from whatever platform the UI runs on. No-escape
/// through a symlink or a Windows junction is a separate policy, stated on
/// [`confine_to_root`] itself, and every one of [`list`], [`read`],
/// [`write`] and [`create`] applies it.
pub struct LocalBackend {
    root: PathBuf,
    /// Shown in the account badge. Nothing else reads it.
    display: String,
}

impl LocalBackend {
    /// Treat `root` as the cloud. The directory is not created here; a
    /// listing of a root that does not exist yet answers with a [`Fault`]
    /// the same way a real provider's missing folder would.
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

/// Turns a [`Job`]'s slash-separated path into a real filesystem path under
/// `root`, refusing anything that is not a run of ordinary names.
///
/// `.` and repeated or leading slashes are just an empty segment and are
/// skipped, the same way a browser's address bar collapses them. Every other
/// segment goes through [`confine_segment`], which is what actually refuses
/// `..`, an absolute form, a backslash or a drive letter, or anything else
/// that is not one ordinary name. This function never touches disk, so it
/// cannot see a symlink or a junction; [`confine_to_root`] is the other half
/// of the promise, and every caller of `resolve` runs both.
fn resolve(root: &Path, path: &str) -> Result<PathBuf, Fault> {
    let mut real = root.to_path_buf();
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        real.push(confine_segment(segment)?);
    }
    Ok(real)
}

/// Confines one path segment, or a whole [`Job::Create`] name, to a single
/// ordinary file or folder name.
///
/// Accepted only when `segment` parses as exactly one [`Component::Normal`]
/// equal to `segment` itself: no empty name, no `.` or `..`, no absolute
/// form, and nothing left over for a second component to hide in. A
/// `std::path::Component` parse alone does not catch every one of those on
/// every platform; a Unix build's parser does not know what `C:` or
/// `\\server\share` mean, so it would wave a Windows drive letter or a UNC
/// prefix straight through as one ordinary name. A `:` or a `\` is refused
/// outright first, on every platform, for exactly that reason: a name
/// reaches this backend's public [`CloudBackend::submit`] from whatever
/// platform the UI happens to run on, and a Windows-shaped escape must not
/// slip past a Linux build's narrower native parser, or the reverse.
fn confine_segment(segment: &str) -> Result<&str, Fault> {
    if segment.contains('\\') || segment.contains(':') {
        return Err(Fault::Refused(format!("{segment} is not a plain file name")));
    }
    let mut components = Path::new(segment).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None) if name == std::ffi::OsStr::new(segment) => Ok(segment),
        _ => Err(Fault::Refused(format!("{segment} is not a plain file name"))),
    }
}

/// Confirms a path already built from nothing but [`confine_segment`]-checked
/// names still cannot have reached outside `root` through a symlink or, on
/// Windows, a directory junction.
///
/// The no-escape policy: `candidate` need not exist yet, since [`write`] and
/// [`create`] both route a path that may still be about to be created
/// through here. Starting at `candidate` and walking up through its
/// ancestors, the first one that does exist is canonicalised and compared
/// against the canonicalised `root`. Nothing below that ancestor exists
/// either, so a link cannot sit on any of the remaining components, and
/// every one of them is already known to be ordinary `Component::Normal`
/// text, never `..` or an absolute form, because [`confine_segment`] is what
/// built them. A canonical path that lands outside `root` is refused;
/// nothing here ever opens, reads or writes through it.
fn confine_to_root(root: &Path, candidate: &Path) -> Result<PathBuf, Fault> {
    let real_root = fs::canonicalize(root).map_err(|error| io_fault(&error))?;
    let mut probe = candidate;
    loop {
        match fs::canonicalize(probe) {
            Ok(real) if real == real_root || real.starts_with(&real_root) => return Ok(candidate.to_path_buf()),
            Ok(_) => return Err(Fault::Refused("that path leaves this folder".to_owned())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => match probe.parent() {
                Some(parent) => probe = parent,
                None => return Err(io_fault(&error)),
            },
            Err(error) => return Err(io_fault(&error)),
        }
    }
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
fn etag_of(meta: &fs::Metadata) -> String {
    let stamp = meta.modified().ok().and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
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
    let dir = confine_to_root(root, &resolve(root, path)?)?;
    let read = fs::read_dir(&dir).map_err(|error| io_fault(&error))?;
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
/// The entry is built from a fresh [`fs::metadata`] call here, not handed back
/// unchanged from what the caller already believed. A reload after a conflict
/// hands in the stale entry it is trying to get past; echoing that back would
/// carry the old etag into the reloaded document, so the very next save would
/// conflict again on a copy that is actually current.
fn read(root: &Path, entry: &Entry) -> Result<(Entry, Vec<u8>), Fault> {
    let file = confine_to_root(root, &resolve(root, &entry.id)?)?;
    let bytes = fs::read(&file).map_err(|error| io_fault(&error))?;
    let meta = fs::metadata(&file).map_err(|error| io_fault(&error))?;
    let fresh = Entry { size: Some(meta.len()), etag: Some(etag_of(&meta)), ..entry.clone() };
    Ok((fresh, bytes))
}

/// Saves back over `entry`, refusing the write when the file moved on since
/// `entry.etag` was read. A file that no longer exists has nothing to
/// conflict with, the same way a provider treats a deleted document.
fn write(root: &Path, entry: &Entry, bytes: &[u8]) -> Result<Entry, Fault> {
    let file = confine_to_root(root, &resolve(root, &entry.id)?)?;
    if let Ok(meta) = fs::metadata(&file)
        && Some(etag_of(&meta)) != entry.etag
    {
        return Err(Fault::Conflict);
    }
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|error| io_fault(&error))?;
    }
    fs::write(&file, bytes).map_err(|error| io_fault(&error))?;
    let meta = fs::metadata(&file).map_err(|error| io_fault(&error))?;
    Ok(Entry { size: Some(meta.len()), etag: Some(etag_of(&meta)), ..entry.clone() })
}

fn create(root: &Path, folder: &str, name: &str, bytes: &[u8]) -> Result<Entry, Fault> {
    // `name` is never a path: it is one [`Job::Create`] field that names the
    // new file, so it goes through `confine_segment` directly rather than
    // through `resolve`, which only ever splits and confines a slash path.
    let name = confine_segment(name)?;
    let dir = confine_to_root(root, &resolve(root, folder)?)?;
    fs::create_dir_all(&dir).map_err(|error| io_fault(&error))?;
    let file = dir.join(name);
    // Exclusive creation, not an `exists` check ahead of a separate write: two
    // creates of the same name racing both see "not there yet" otherwise, and
    // the second write silently replaces the first with neither ever refused.
    // `create_new` asks the filesystem to open and create in one step, so the
    // loser gets an error instead of a chance to overwrite the winner.
    let mut handle = match fs::OpenOptions::new().write(true).create_new(true).open(&file) {
        Ok(handle) => handle,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(Fault::Refused(format!("{name} already exists")));
        }
        Err(error) => return Err(io_fault(&error)),
    };
    handle.write_all(bytes).map_err(|error| io_fault(&error))?;
    drop(handle);
    let meta = fs::metadata(&file).map_err(|error| io_fault(&error))?;
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
        let root = PathBuf::from("/cloud/root");
        assert!(resolve(&root, "../secrets").is_err());
        assert!(resolve(&root, "Archive/../../../etc/passwd").is_err());
    }

    #[test]
    fn repeated_and_trailing_slashes_collapse_like_a_browser_address_bar() {
        let root = PathBuf::from("/cloud/root");
        assert_eq!(resolve(&root, "//Archive//2026/").unwrap(), root.join("Archive").join("2026"));
        assert_eq!(resolve(&root, "").unwrap(), root);
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

    #[test]
    fn confine_segment_refuses_windows_separators_and_prefixes_on_every_platform() {
        for bad in ["..\\outside.txt", "C:\\x", "C:evil", "\\\\server\\share\\x"] {
            assert!(confine_segment(bad).is_err(), "{bad:?} must be refused on every platform, not only on Windows");
        }
    }

    #[test]
    fn confine_to_root_accepts_a_path_that_does_not_exist_yet_under_an_existing_root() {
        let root = tempfile::tempdir().unwrap();
        let candidate = root.path().join("not-written-yet.txt");
        assert!(confine_to_root(root.path(), &candidate).is_ok());
    }

    #[test]
    fn confine_to_root_refuses_a_candidate_that_canonicalises_outside_root() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        // Built from a path that never went through `confine_segment` (this
        // helper is tested on its own merits), standing in for whatever a
        // symlink or junction would otherwise have redirected into `root`.
        assert!(confine_to_root(root.path(), elsewhere.path()).is_err());
    }

    #[test]
    fn a_folder_announces_no_capability_this_backend_cannot_honour() {
        let reported = account("Local disk".to_owned()).capabilities;
        for absent in [Capability::SharedWithMe, Capability::Sharing, Capability::Conversation, Capability::Contacts] {
            assert!(!reported.contains(&absent), "a folder has no {absent:?}");
        }
    }
}
