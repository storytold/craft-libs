//! Linked files: status (embedded, missing, modified, unreadable, refused), the identity check,
//! candidates and ranking, folder remaps and folder matches (`Link`, `LinkIdentity`, `LinkStatus`,
//! `LinkFiles`, `SystemLinkFiles`, `check_candidate`, `Remap`). Every stored path goes through
//! [`resolve_absolute`](crate::resolve_absolute) first, so network, device and URL links are
//! refused before any file is touched. File access goes only through [`LinkFiles`]; the crate's
//! lexical functions ([`classify`](crate::classify), [`resolve`](crate::resolve)) still do no I/O.
//!
//! | Need | Use |
//! |---|---|
//! | The status of every link, for a Links panel | [`status`] per link (a stamp; a fingerprint only when the stamp moved) |
//! | Whether a picked file may replace a missing link | [`check_candidate`], then [`rank`] for several |
//! | Relink the others after one relink | [`derive_remap`] and [`Remap::apply`] |
//! | Relink to Folder | [`folder_matches`] |
//! | Search a folder tree for the missing files | [`LinkSearch`], run as a job |
//! | Stories and tests | [`FakeLinkFiles`] |

mod candidate;
mod fake;
mod index;
mod remap;
mod search;
#[cfg(not(target_arch = "wasm32"))]
mod system;
#[cfg(test)]
mod tests;

use std::io::{self, Read, Seek};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::Refused;

pub use candidate::{Candidate, CandidateProblem, MatchCriteria, check_candidate, rank};
pub use fake::FakeLinkFiles;
pub use index::{FolderIndex, index_folder};
pub use remap::{Remap, derive_remap, folder_match, folder_matches};
pub use search::{LinkSearch, SearchEnd, SearchHit, SearchUpdate};
#[cfg(not(target_arch = "wasm32"))]
pub use system::SystemLinkFiles;

/// How much of each end of a file the fingerprint reads: 1 MiB (FilmCraft's `FINGERPRINT_SPAN`).
/// Files up to twice this are hashed whole.
pub const FINGERPRINT_SPAN: usize = 1 << 20;

/// One linked file of a document, as the app stores it (serde, so it goes into the document
/// as it is).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link<K> {
    /// The app's key for the link (an asset id, an item id).
    pub key: K,
    /// The name shown in a Links panel: the file name, or the app's own name for it.
    pub name: String,
    /// The path or URL as stored in the document, before [`resolve_absolute`](crate::resolve_absolute).
    pub stored: String,
    /// The file as it was when it was linked or last updated; `None` when the app never recorded
    /// it (the status is then `Ok` whenever the file exists).
    pub identity: Option<LinkIdentity>,
    /// The page of a multi-page file the link places (PDF, AI), from 1.
    pub page: Option<u32>,
    /// The file's data is in the document: no file is read and the status is `Embedded`.
    pub embedded: bool,
}

/// What tells two versions of a file apart without keeping its bytes: the size, a fingerprint
/// of its first and last [`FINGERPRINT_SPAN`] bytes (the whole file up to twice that), and the
/// modification time. Serde, so the app stores it in its document as it is (inside [`Link`] or
/// beside its own link record).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LinkIdentity {
    /// Length in bytes.
    pub size: u64,
    /// [`fingerprint`] of the file.
    pub fingerprint: u64,
    /// Modification time, when the file system gives one.
    pub modified: Option<SystemTime>,
}

/// What a regular file's metadata says, from [`LinkFiles::stat`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileInfo {
    /// Length in bytes.
    pub size: u64,
    /// Modification time, when the file system gives one.
    pub modified: Option<SystemTime>,
}

/// One entry of a folder, from [`LinkFiles::list`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderEntry {
    /// The entry's full path.
    pub path: PathBuf,
    /// Its name.
    pub name: String,
    /// A folder (not a link to one).
    pub is_dir: bool,
    /// A symbolic link or junction, to a file or a folder. Searches never follow one to a folder.
    pub is_link: bool,
}

/// The status of a link, for its badge and the actions it enables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LinkStatus {
    /// The data is in the document.
    Embedded,
    /// The file is there and is the version linked (or the link records no version).
    Ok,
    /// The file is there but its contents changed since it was linked.
    Modified,
    /// No regular file at the path: deleted, moved, or a device or FIFO stands there.
    Missing,
    /// The file is there but couldn't be read (permissions, a lock, an I/O error).
    Unreadable,
    /// The stored path may not be followed (another computer, a device, a URL, not a path):
    /// nothing was read.
    Refused(Refused),
}

impl LinkStatus {
    /// Whether the link needs relinking: `Missing`, `Unreadable` or `Refused`. A Links panel's
    /// "Missing" filter and the relink dialog's set are these links.
    pub fn is_missing(self) -> bool {
        // new: one rule for the filter, the relink dialog's set and Edit Original's enablement.
        matches!(self, Self::Missing | Self::Unreadable | Self::Refused(_))
    }
}

/// The file access the links module needs, so stories and tests use a fake and the module
/// builds where there is no file system. [`SystemLinkFiles`] is the desktop implementation. Every
/// call can block on a slow or network disk: call it only inside a job
/// (`craft_host::queue::JobQueue`), except a single [`stat`](LinkFiles::stat) the user asked for.
pub trait LinkFiles: Send + Sync {
    /// The metadata of the regular file at `path`, following symbolic links; `None` when nothing
    /// is there, it can't be read, or it is not a regular file (a folder, a device, a FIFO).
    fn stat(&self, path: &Path) -> Option<FileInfo>;

    /// The identity of the regular file at `path`: reads at most `2 ×` [`FINGERPRINT_SPAN`]
    /// bytes ([`identity_of`]).
    fn identity(&self, path: &Path) -> io::Result<LinkIdentity>;

    /// The entries of the folder at `dir`, in no particular order. Hidden entries are included;
    /// callers skip what they don't want.
    fn list(&self, dir: &Path) -> io::Result<Vec<FolderEntry>>;
}

/// The fingerprint of a file of `size` bytes whose first bytes are `head` and whose last bytes
/// are `tail` (for a file up to `2 ×` [`FINGERPRINT_SPAN`], `tail` is everything after `head`, so
/// the whole file is hashed). The same hash FilmCraft's `fingerprint` computes over the same
/// three inputs, so identities FilmCraft stored stay valid: copy its body.
pub fn fingerprint(size: u64, head: &[u8], tail: &[u8]) -> u64 {
    // filmcraft: crates/engine/src/relink.rs:31 (`fingerprint`).
    let mut h: u64 = 0x243f_6a88_85a3_08d3 ^ size.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let mut mix = |w: u64| {
        h = (h ^ w).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        h ^= h >> 29;
    };
    for part in [head, tail] {
        let (chunks, rest) = part.as_chunks::<8>();
        for c in chunks {
            mix(u64::from_le_bytes(*c));
        }
        let mut last = [0u8; 8];
        last[..rest.len()].copy_from_slice(rest);
        mix(u64::from_le_bytes(last) ^ (rest.len() as u64) << 56);
        mix(part.len() as u64);
    }
    h
}

/// The identity of a file read from `file`, whose length is `size` and modification time
/// `modified`: reads the first [`FINGERPRINT_SPAN`] bytes, then the last [`FINGERPRINT_SPAN`]
/// when `size > 2 × FINGERPRINT_SPAN`, else the rest after the head, and calls [`fingerprint`].
/// A short read is an error (the file changed while it was read).
pub fn identity_of(file: &mut (impl Read + Seek), size: u64, modified: Option<SystemTime>) -> io::Result<LinkIdentity> {
    // filmcraft: crates/engine/src/relink.rs:59 (`identity_of`: head, then the tail or the rest).
    let span = FINGERPRINT_SPAN as u64;
    let mut head = vec![0u8; size.min(span) as usize];
    file.read_exact(&mut head)?;
    let tail_len = if size > 2 * span { span } else { size - head.len() as u64 };
    let mut tail = vec![0u8; tail_len as usize];
    if size > 2 * span {
        file.seek(io::SeekFrom::Start(size - span))?;
    }
    file.read_exact(&mut tail)?;
    Ok(LinkIdentity { size, fingerprint: fingerprint(size, &head, &tail), modified })
}

/// The status of `link` in the document at `document` (its path, if it has one), in this order:
///
/// 1. `embedded` gives `Embedded`; nothing is read.
/// 2. [`resolve_absolute`](crate::resolve_absolute)`(stored, document)` failing gives
///    `Refused(reason)`; nothing is read.
/// 3. [`LinkFiles::stat`] `None` gives `Missing`.
/// 4. No recorded identity gives `Ok`.
/// 5. Size and modification time equal to the recorded ones give `Ok` (the stamp check, so a
///    status poll hashes nothing).
/// 6. Otherwise [`LinkFiles::identity`]: an error gives `Unreadable`; an equal size and
///    fingerprint give `Ok` (the file was touched, not changed); anything else `Modified`.
///
/// DesignCraft's exact byte comparison holds for files up to `2 ×` [`FINGERPRINT_SPAN`], which
/// are hashed whole; larger files accept the head-and-tail risk (lead call 34).
pub fn status<K>(link: &Link<K>, document: Option<&Path>, files: &dyn LinkFiles) -> LinkStatus {
    // designcraft: crates/engine/src/cmd/links.rs:158 (`status`: embedded, refused, missing, modified).
    if link.embedded {
        return LinkStatus::Embedded;
    }
    let path = match crate::resolve_absolute(&link.stored, document) {
        Ok(path) => path,
        Err(reason) => return LinkStatus::Refused(reason),
    };
    let Some(info) = files.stat(&path) else { return LinkStatus::Missing };
    let Some(recorded) = link.identity else { return LinkStatus::Ok };
    // A stamp without a modification time proves nothing, so it never short-cuts the check.
    if info.size == recorded.size && info.modified.is_some() && info.modified == recorded.modified {
        return LinkStatus::Ok;
    }
    match files.identity(&path) {
        Err(_) => LinkStatus::Unreadable,
        Ok(now) if now.size == recorded.size && now.fingerprint == recorded.fingerprint => LinkStatus::Ok,
        Ok(_) => LinkStatus::Modified,
    }
}
