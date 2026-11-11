//! `FakeLinkFiles`: an in-memory [`LinkFiles`] for tests and stories.

use std::collections::BTreeMap;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use super::{FileInfo, FolderEntry, LinkFiles, LinkIdentity, identity_of};

/// A file system in memory: files by path with their bytes and modification time. `list` reports
/// the folders implied by the paths, so a search and a folder match run over it.
#[derive(Default)]
pub struct FakeLinkFiles {
    files: BTreeMap<PathBuf, (Vec<u8>, SystemTime)>,
    unreadable: Vec<PathBuf>,
    calls: AtomicUsize,
}

impl FakeLinkFiles {
    /// An empty file system.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the file at `path` with `bytes`, modified `secs` seconds after the Unix epoch.
    #[must_use]
    pub fn with(mut self, path: impl Into<PathBuf>, bytes: &[u8], secs: u64) -> Self {
        self.files.insert(path.into(), (bytes.to_vec(), SystemTime::UNIX_EPOCH + Duration::from_secs(secs)));
        self
    }

    /// Makes [`identity`](LinkFiles::identity) of `path` fail with a permission error; `stat` still sees it.
    #[must_use]
    pub fn unreadable(mut self, path: impl Into<PathBuf>) -> Self {
        self.unreadable.push(path.into());
        self
    }

    /// How many `stat` and `identity` calls were made.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

impl LinkFiles for FakeLinkFiles {
    fn stat(&self, path: &Path) -> Option<FileInfo> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.files.get(path).map(|(b, m)| FileInfo { size: b.len() as u64, modified: Some(*m) })
    }

    fn identity(&self, path: &Path) -> io::Result<LinkIdentity> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.unreadable.iter().any(|p| p == path) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "locked"));
        }
        let (bytes, modified) = self.files.get(path).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        identity_of(&mut Cursor::new(bytes), bytes.len() as u64, Some(*modified))
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<FolderEntry>> {
        let mut out: Vec<FolderEntry> = Vec::new();
        for path in self.files.keys() {
            let Ok(rest) = path.strip_prefix(dir) else { continue };
            let Some(first) = rest.components().next() else { continue };
            let name = first.as_os_str().to_string_lossy().into_owned();
            if out.iter().any(|e| e.name == name) {
                continue;
            }
            let is_dir = rest.components().count() > 1;
            out.push(FolderEntry { path: dir.join(&name), name, is_dir, is_link: false });
        }
        if out.is_empty() { Err(io::Error::from(io::ErrorKind::NotFound)) } else { Ok(out) }
    }
}
