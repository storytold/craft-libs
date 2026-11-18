//! What Enter or OK does in the chooser with the typed text and the shown entries.

use std::path::{Path, PathBuf};

use craft_browse::places::Entry;
use craft_host::PickRequest;

/// What accepting the typed text does.
#[allow(dead_code)] // P3(shell-w2): sh.K14
pub(crate) enum Accept {
    /// Open these files.
    Open(Vec<PathBuf>),
    /// Save to this path.
    Save(PathBuf),
    /// Enter this folder.
    Enter(PathBuf),
    /// Refuse, with the reason.
    Refuse(String),
}

/// Decides what Enter or OK does with `typed` in `folder`, given the shown `entries`. Reads no
/// folder: listing, the hidden rule and the sort come from craft-browse.
#[allow(dead_code)] // P3(shell-w2): sh.K14
pub(crate) fn accept(request: &PickRequest, typed: &str, folder: &Path, entries: &[Entry]) -> Accept {
    let _ = (request, typed, folder, entries);
    todo!("sh.K14")
}
