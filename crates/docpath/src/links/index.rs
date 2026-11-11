//! `index_folder`: the files of a folder tree by lower-cased name, for the relink search.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

use super::LinkFiles;

/// The files found under a folder, by file name in Unicode lower case, each with every path that
/// has that name, in the order they were found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderIndex {
    /// Paths by lower-cased file name.
    pub by_name: BTreeMap<String, Vec<PathBuf>>,
    /// Files indexed.
    pub files: usize,
    /// The walk stopped at the limit before it had seen every folder.
    pub truncated: bool,
}

/// Indexes the regular files under `root`, breadth first, each folder's entries in name order,
/// skipping names that start with `.`, never following a link to a folder (a linked file still
/// counts), and stopping after `limit` files (`truncated`). `cancelled` is asked before each
/// folder; when it says yes the walk ends with what it has. Unreadable folders are skipped.
pub fn index_folder(root: &Path, limit: usize, files: &dyn LinkFiles, cancelled: &dyn Fn() -> bool) -> FolderIndex {
    index_folder_reporting(root, limit, files, cancelled, &mut |_| {})
}

/// [`index_folder`] that calls `indexed(n)` after each file it adds, `n` being the count so far.
pub(super) fn index_folder_reporting(
    root: &Path,
    limit: usize,
    files: &dyn LinkFiles,
    cancelled: &dyn Fn() -> bool,
    indexed: &mut dyn FnMut(usize),
) -> FolderIndex {
    // filmcraft: crates/engine/src/relink.rs:478 (`index_folder`: breadth first, dot files and folder links skipped).
    let mut index = FolderIndex::default();
    let mut queue = VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        if cancelled() {
            break;
        }
        let Ok(mut entries) = files.list(&dir) else { continue };
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        for entry in entries {
            if entry.name.starts_with('.') {
                continue;
            }
            if entry.is_dir {
                queue.push_back(entry.path);
                continue;
            }
            // A link counts only when it leads to a regular file; `stat` is `None` for a folder.
            if entry.is_link && files.stat(&entry.path).is_none() {
                continue;
            }
            if index.files >= limit {
                index.truncated = true;
                return index;
            }
            index.by_name.entry(entry.name.to_lowercase()).or_default().push(entry.path);
            index.files += 1;
            indexed(index.files);
        }
    }
    index
}
