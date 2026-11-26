//! What Enter or OK does in the chooser with the typed text and the shown entries.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use craft_browse::places::{Entry, same_folder};
use craft_host::{PickKind, PickRequest};

/// What accepting the typed text does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Accept {
    /// Open these files (or, for a folder pick, answer this folder).
    Open(Vec<PathBuf>),
    /// Save to this path; nothing is listed there, so no question is asked.
    Save(PathBuf),
    /// Save to this path, a file the shown listing has: ask Replace first. The rule that tells a listed file from a
    /// new name lives here, so the chooser does not look again.
    Replace(PathBuf),
    /// Enter this folder.
    Enter(PathBuf),
    /// List `folder`, then accept `name` against that listing: a typed path outside the shown folder.
    Look {
        /// The folder to list.
        folder: PathBuf,
        /// The name to accept there.
        name: OsString,
    },
    /// Refuse: the name that was not found, or empty when nothing was typed.
    Refuse(String),
}

/// Decides what Enter or OK does with `typed` in `folder`, given the shown `entries` and the `selected` ones. Reads
/// no folder: listing, the hidden rule and the sort come from craft-browse.
///
/// - A selected folder that is still listed is entered (a folder pick answers it); a selected folder never becomes
///   the typed name.
/// - Several selected files of an Open Many answer all of them.
/// - A selected file whose name is in the field keeps its exact path, leading spaces and all, listed or not.
/// - Otherwise the typed text, trimmed: `~` stands for `home`; a path outside `folder` is a [`Accept::Look`]; a
///   name in `folder` is entered when it is a listed folder, opened (or asked about on Save) when it is a listed
///   file, saved to when nothing is listed under it, and refused as not found on Open.
pub(crate) fn accept(request: &PickRequest, typed: &str, folder: &Path, entries: &[Entry], selected: &[Entry], home: Option<&Path>) -> Accept {
    // ui-egui: crates/ui-egui/src/file_browser.rs:228 (`FileBrowser::accept`), :93 (`resolve`).
    let listed = |path: &Path| entries.iter().find(|e| e.path == path);
    if let [one] = selected
        && one.folder
        && listed(&one.path).is_some()
    {
        return if request.kind == PickKind::Folder { Accept::Open(vec![one.path.clone()]) } else { Accept::Enter(one.path.clone()) };
    }
    let files: Vec<&Entry> = selected.iter().filter(|e| !e.folder).collect();
    if request.kind == PickKind::OpenMany && files.len() > 1 {
        return Accept::Open(files.iter().map(|e| e.path.clone()).collect());
    }
    if let [file] = files.as_slice()
        && file.name == typed
    {
        return in_listing(request, file.path.clone(), &file.name, listed(&file.path));
    }

    let text = typed.trim();
    if text.is_empty() {
        return if request.kind == PickKind::Folder { Accept::Open(vec![folder.to_path_buf()]) } else { Accept::Refuse(String::new()) };
    }
    let Some(path) = typed_path(text, folder, home) else { return Accept::Refuse(text.to_owned()) };
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { return Accept::Enter(path) };
    if !same_folder(parent, folder) {
        return Accept::Look { folder: parent.to_path_buf(), name: name.to_os_string() };
    }
    let path = folder.join(name);
    in_listing(request, path.clone(), text, listed(&path))
}

/// The path typed `text` (trimmed by the caller) names from `folder`: `~` and `~/rest` under `home`, an absolute
/// path as it is, anything else inside `folder`; `.` and `..` resolved without reading the disk. `None` for a `~`
/// path without a home. The name field and the folder field both read typed paths through it.
pub(crate) fn typed_path(text: &str, folder: &Path, home: Option<&Path>) -> Option<PathBuf> {
    // ui-egui: crates/ui-egui/src/file_browser.rs:93 (`resolve`).
    let expanded = if text == "~" {
        home.map(Path::to_path_buf)
    } else if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        home.map(|home| home.join(rest))
    } else {
        Some(folder.join(text))
    };
    expanded.map(|path| lexical(&path))
}

/// Whether typed `text` (trimmed) starts from the home folder, so [`typed_path`] needs it.
pub(crate) fn needs_home(text: &str) -> bool {
    text == "~" || text.starts_with("~/") || text.starts_with("~\\")
}

/// What a path in the shown folder does: `found` is its entry in the listing, `None` when nothing is listed under it.
fn in_listing(request: &PickRequest, path: PathBuf, name: &str, found: Option<&Entry>) -> Accept {
    match (request.kind, found.map(|e| e.folder)) {
        (PickKind::Folder, Some(true)) => Accept::Open(vec![path]),
        (_, Some(true)) => Accept::Enter(path),
        (PickKind::Open | PickKind::OpenMany, Some(false)) => Accept::Open(vec![path]),
        (PickKind::Save, Some(false)) => Accept::Replace(path),
        (PickKind::Save, None) => Accept::Save(path),
        // A listed file is no folder: refused without a "not found" note, which would be untrue.
        (PickKind::Folder, Some(false)) => Accept::Refuse(String::new()),
        (PickKind::Open | PickKind::OpenMany | PickKind::Folder, _) => Accept::Refuse(name.to_owned()),
    }
}

/// `path` with `.` dropped and `..` taking off the part before it, without reading the disk.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            // `..` at a root stays at the root; at the start of a relative path it is kept.
            Component::ParentDir => {
                if !out.pop() && !out.has_root() {
                    out.push(part);
                }
            }
            other => out.push(other),
        }
    }
    out
}
