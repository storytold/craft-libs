//! `SystemLinkFiles`: [`LinkFiles`] over `std::fs` (desktop only).

use std::io;
use std::path::Path;

use super::{FileInfo, FolderEntry, LinkFiles, LinkIdentity, identity_of};

/// [`LinkFiles`] over the file system.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemLinkFiles;

impl LinkFiles for SystemLinkFiles {
    /// `std::fs::metadata` (follows links), regular files only; the size and `modified()`.
    fn stat(&self, path: &Path) -> Option<FileInfo> {
        // designcraft: crates/engine/src/cmd/links.rs:143 (`link_missing`: devices and FIFOs count as missing).
        let meta = std::fs::metadata(path).ok().filter(std::fs::Metadata::is_file)?;
        Some(FileInfo { size: meta.len(), modified: meta.modified().ok() })
    }

    /// Opens the file, takes its metadata from the open handle (so the size matches what is
    /// read) and calls [`identity_of`](super::identity_of).
    fn identity(&self, path: &Path) -> io::Result<LinkIdentity> {
        // filmcraft: crates/engine/src/relink.rs:59 (`identity_of` over the services' `read_range`).
        // Checked before opening: opening a FIFO for reading blocks.
        if !std::fs::metadata(path)?.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"));
        }
        let mut file = std::fs::File::open(path)?;
        let meta = file.metadata()?;
        identity_of(&mut file, meta.len(), meta.modified().ok())
    }

    /// `std::fs::read_dir`; `is_dir` from the entry's own file type (a link to a folder is not a
    /// folder), `is_link` from `is_symlink`. Entries that fail to read are skipped.
    fn list(&self, dir: &Path) -> io::Result<Vec<FolderEntry>> {
        // filmcraft: crates/engine/src/relink.rs:483 (`read_dir`, `file_type`).
        let entries = std::fs::read_dir(dir)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let kind = entry.file_type().ok()?;
                Some(FolderEntry {
                    path: entry.path(),
                    name: entry.file_name().to_string_lossy().into_owned(),
                    is_dir: kind.is_dir(),
                    is_link: kind.is_symlink(),
                })
            })
            .collect();
        Ok(entries)
    }
}
