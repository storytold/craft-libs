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
use std::path::{Path, PathBuf};

use craft_cloud::{Capability, Entry, EntryKind};
use craft_ui::cloud::{Account, CloudBackend, Event, Fault, Job, Post};

/// Where "the cloud" lives for this backend: one directory on disk.
///
/// `Entry::id` and `Entry::path` are the same slash-separated path relative to
/// `root`, always starting with `/` below the root itself (the root's own
/// path is the empty string, matching [`Job::List`]'s contract). Neither is
/// an OS path; both go through [`resolve`] before anything touches a
/// filesystem, so a name containing `..` can never walk out of `root`.
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
                std::thread::spawn(move || {
                    let bytes = read(&root, &entry);
                    post.send(Event::Read { entry, into, bytes });
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
/// `root`, refusing anything that would walk back out of it.
///
/// `..` is the only way out, so `..` is the only segment refused. `.` and
/// repeated or leading slashes are just an empty segment and are skipped,
/// the same way a browser's address bar collapses them.
fn resolve(root: &Path, path: &str) -> Result<PathBuf, Fault> {
    let mut real = root.to_path_buf();
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return Err(Fault::Refused(format!("{path} reaches outside this folder")));
        }
        real.push(segment);
    }
    Ok(real)
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
    let dir = resolve(root, path)?;
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

fn read(root: &Path, entry: &Entry) -> Result<Vec<u8>, Fault> {
    let file = resolve(root, &entry.id)?;
    fs::read(&file).map_err(|error| io_fault(&error))
}

/// Saves back over `entry`, refusing the write when the file moved on since
/// `entry.etag` was read. A file that no longer exists has nothing to
/// conflict with, the same way a provider treats a deleted document.
fn write(root: &Path, entry: &Entry, bytes: &[u8]) -> Result<Entry, Fault> {
    let file = resolve(root, &entry.id)?;
    if let Ok(meta) = fs::metadata(&file) {
        if Some(etag_of(&meta)) != entry.etag {
            return Err(Fault::Conflict);
        }
    }
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|error| io_fault(&error))?;
    }
    fs::write(&file, bytes).map_err(|error| io_fault(&error))?;
    let meta = fs::metadata(&file).map_err(|error| io_fault(&error))?;
    Ok(Entry { size: Some(meta.len()), etag: Some(etag_of(&meta)), ..entry.clone() })
}

fn create(root: &Path, folder: &str, name: &str, bytes: &[u8]) -> Result<Entry, Fault> {
    let dir = resolve(root, folder)?;
    fs::create_dir_all(&dir).map_err(|error| io_fault(&error))?;
    let file = dir.join(name);
    if file.exists() {
        return Err(Fault::Refused(format!("{name} already exists")));
    }
    fs::write(&file, bytes).map_err(|error| io_fault(&error))?;
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
    fn a_folder_announces_no_capability_this_backend_cannot_honour() {
        let reported = account("Local disk".to_owned()).capabilities;
        for absent in [Capability::SharedWithMe, Capability::Sharing, Capability::Conversation, Capability::Contacts] {
            assert!(!reported.contains(&absent), "a folder has no {absent:?}");
        }
    }
}
