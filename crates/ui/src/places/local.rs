//! This computer: a folder of the user's machine as content for the places panel.
//!
//! The panel itself opens no file. This is the part an app adds when it wants its "This computer"
//! place to list folders in the page instead of sending the user to the system's dialog: it reads
//! a folder once per move, turns it into entries and a breadcrumb, says properly why a folder has
//! nothing to show, and tells the app which file was chosen.
//!
//! It only lists names. It never opens a file, and it reads a folder when the user moves there,
//! not on every frame. On the web there are no folders to list, so an app offers no such place.
//!
//! On macOS the system asks the user once before an app may read Documents, Desktop or Downloads,
//! and refuses silently after a No. [`LocalLabels::no_access`] is what the user reads then; give
//! the question a reason with the three `NS…FolderUsageDescription` keys in the bundle's
//! `Info.plist` (see the crate's README).

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::{PlaceEntry, PlacesAction, PlacesContent, PlacesIcon, PlacesNotice};

/// At most this many entries are listed; a bigger folder shows the first ones.
pub const MAX_LISTED: usize = 5000;
/// At most this many folder entries are read, so a huge folder cannot stall the window.
const MAX_SCANNED: usize = 100_000;

/// One subfolder or file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalItem {
    /// As shown: a name that is not valid UTF-8 has replacement characters.
    pub name: String,
    pub path: PathBuf,
    pub folder: bool,
}

/// What the app says when a folder has nothing to show, in the interface language.
#[derive(Clone, Copy, Debug)]
pub struct LocalLabels<'a> {
    pub empty: &'a str,
    /// The folder is closed to the app.
    pub no_access: &'a str,
    /// What the user can do about it: that Browse still opens a document from it and, on macOS,
    /// where access is granted.
    pub no_access_detail: &'a str,
    /// The button under it; pressing it comes back as [`LocalOutcome::Browse`].
    pub browse: &'a str,
    /// The folder is no longer there.
    pub gone: &'a str,
    pub gone_detail: &'a str,
    /// Any other failure to read it.
    pub unreadable: &'a str,
}

/// What a choice in the panel means for the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalOutcome {
    /// Nothing for the app to do: the listing moved, or the action was not about this place.
    Stay,
    /// Open this file.
    Open(PathBuf),
    /// Show the system's own file dialog.
    Browse,
}

/// A folder and what it holds, read when it was opened.
#[derive(Clone, Debug)]
pub struct LocalFolder {
    dir: PathBuf,
    extensions: Vec<String>,
    items: Vec<LocalItem>,
    truncated: bool,
    failed: Option<ErrorKind>,
    /// Where the place starts, when it is not the whole disk.
    root: Option<PathBuf>,
}

impl LocalFolder {
    /// Reads `dir`, once. `extensions` are the kinds of file to list, without the dot and in any
    /// case; `*` lists every file. Subfolders are always listed. Hidden entries (a leading dot, or
    /// Windows' hidden attribute) are left out.
    pub fn open(dir: impl Into<PathBuf>, extensions: &[&str]) -> Self {
        let mut folder = Self {
            dir: dir.into(),
            extensions: extensions.iter().map(|e| e.to_lowercase()).collect(),
            items: Vec::new(),
            truncated: false,
            failed: None,
            root: None,
        };
        folder.read(MAX_LISTED);
        folder
    }

    /// Reads `root`, once, as the top of a place of its own, such as a synced cloud folder: the
    /// breadcrumb starts there, and its first step leads back there instead of to the home folder.
    pub fn rooted(root: impl Into<PathBuf>, extensions: &[&str]) -> Self {
        let root = root.into();
        let mut folder = Self::open(root.clone(), extensions);
        folder.root = Some(root);
        folder
    }

    /// The user's home folder, where This computer usually starts. On Windows `USERPROFILE` comes
    /// first: `HOME` there is set by tools such as Git Bash or Cygwin, not always to the same folder.
    pub fn home() -> Option<PathBuf> {
        let vars = if cfg!(windows) { ["USERPROFILE", "HOME"] } else { ["HOME", "USERPROFILE"] };
        vars.iter().find_map(|var| std::env::var_os(var).filter(|home| !home.is_empty())).map(PathBuf::from)
    }

    /// Shows another folder, keeping the kinds of file.
    pub fn go(&mut self, dir: impl Into<PathBuf>) {
        self.dir = dir.into();
        self.read(MAX_LISTED);
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Subfolders first, then files, each by name.
    pub fn items(&self) -> &[LocalItem] {
        &self.items
    }

    /// The folder holds more than was listed.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Why the folder could not be read, when it could not.
    pub fn failed(&self) -> Option<ErrorKind> {
        self.failed
    }

    fn read(&mut self, cap: usize) {
        self.items.clear();
        self.truncated = false;
        self.failed = None;
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) => {
                self.failed = Some(error.kind());
                return;
            }
        };
        for (scanned, entry) in entries.enumerate() {
            if scanned >= MAX_SCANNED {
                self.truncated = true;
                break;
            }
            // An entry that cannot be read is skipped; the rest of the folder still lists.
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || hidden(&entry) {
                continue;
            }
            let path = entry.path();
            // A symbolic link to a folder is a folder.
            let folder = match entry.file_type() {
                Ok(kind) if kind.is_symlink() => std::fs::metadata(&path).is_ok_and(|target| target.is_dir()),
                Ok(kind) => kind.is_dir(),
                Err(_) => continue,
            };
            if !folder && !self.lists(&path) {
                continue;
            }
            if self.items.len() >= cap {
                self.truncated = true;
                break;
            }
            self.items.push(LocalItem { name, path, folder });
        }
        self.items.sort_by_cached_key(|item| (!item.folder, item.name.to_lowercase()));
    }

    fn lists(&self, path: &Path) -> bool {
        self.extensions.iter().any(|e| e == "*")
            || path.extension().and_then(|e| e.to_str()).is_some_and(|e| self.extensions.iter().any(|listed| listed.eq_ignore_ascii_case(e)))
    }

    /// The way back up, for [`PlacesPanel::crumbs`]: `root` (the place's own name, which leads to
    /// where the place starts: its root, or the home folder), then each folder of the path below
    /// it.
    ///
    /// [`PlacesPanel::crumbs`]: super::PlacesPanel::crumbs
    pub fn crumbs(&self, root: &str) -> Vec<String> {
        std::iter::once(root.to_owned()).chain(self.steps().into_iter().map(|(name, _)| name)).collect()
    }

    /// Each named folder of the path with where it is.
    fn steps(&self) -> Vec<(String, PathBuf)> {
        // Below the root when the listing is inside it; a folder outside it shows its whole path.
        let (mut at, below) = match self.root.as_deref().and_then(|root| Some((root, self.dir.strip_prefix(root).ok()?))) {
            Some((root, below)) => (root.to_path_buf(), below),
            None => (PathBuf::new(), self.dir.as_path()),
        };
        let mut steps = Vec::new();
        for part in below.components() {
            at.push(part);
            if let std::path::Component::Normal(name) = part {
                steps.push((name.to_string_lossy().into_owned(), at.clone()));
            }
        }
        steps
    }

    /// The items as entries for the panel, in the same order.
    pub fn entries(&self) -> Vec<PlaceEntry<'_>> {
        self.items.iter().map(|item| PlaceEntry { name: &item.name, folder: item.folder, detail: "", attention: false, remote: false }).collect()
    }

    /// What the panel shows: `entries` (from [`LocalFolder::entries`]), or the notice that says
    /// why there are none.
    pub fn content<'a>(&self, entries: &'a [PlaceEntry<'a>], labels: LocalLabels<'a>) -> PlacesContent<'a> {
        let notice = |icon, title, body, action| PlacesContent::Notice(PlacesNotice { icon, title, body, action });
        match self.failed {
            Some(ErrorKind::PermissionDenied) => notice(PlacesIcon::Locked, labels.no_access, labels.no_access_detail, Some(labels.browse)),
            Some(ErrorKind::NotFound) => notice(PlacesIcon::Folder, labels.gone, labels.gone_detail, None),
            Some(_) => notice(PlacesIcon::Folder, labels.unreadable, "", None),
            None if entries.is_empty() => notice(PlacesIcon::Folder, labels.empty, "", None),
            None => PlacesContent::Entries(entries),
        }
    }

    /// Acts on what the panel reported while this folder was showing: a subfolder or a breadcrumb
    /// step moves the listing, a file is the app's to open, and Browse (the row, or the button
    /// of the no-access notice) is the system's dialog. Choosing another place is the app's
    /// business and comes back as [`LocalOutcome::Stay`].
    pub fn act(&mut self, action: PlacesAction) -> LocalOutcome {
        match action {
            PlacesAction::Activate(index) => match self.items.get(index) {
                Some(item) if item.folder => {
                    let to = item.path.clone();
                    self.go(to);
                    LocalOutcome::Stay
                }
                Some(item) => LocalOutcome::Open(item.path.clone()),
                None => LocalOutcome::Stay,
            },
            // The first crumb is the place itself: back to where it starts.
            PlacesAction::Crumb(0) => {
                if let Some(start) = self.root.clone().or_else(Self::home) {
                    self.go(start);
                }
                LocalOutcome::Stay
            }
            PlacesAction::Crumb(index) => {
                if let Some((_, to)) = self.steps().into_iter().nth(index - 1) {
                    self.go(to);
                }
                LocalOutcome::Stay
            }
            PlacesAction::Browse | PlacesAction::Notice => LocalOutcome::Browse,
            PlacesAction::Select(_) | PlacesAction::AddPlace => LocalOutcome::Stay,
        }
    }
}

/// Windows hides an entry with an attribute instead of a leading dot: the recycle bin at the top
/// of a drive, `desktop.ini` in a folder with its own icon.
#[cfg(windows)]
fn hidden(entry: &std::fs::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    entry.metadata().is_ok_and(|meta| meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
}

#[cfg(not(windows))]
fn hidden(_: &std::fs::DirEntry) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const LABELS: LocalLabels<'static> = LocalLabels {
        empty: "This folder is empty",
        no_access: "No access to this folder",
        no_access_detail: "Browse opens a document from it.",
        browse: "Browse…",
        gone: "This folder is no longer there",
        gone_detail: "It may have been moved or deleted.",
        unreadable: "This folder can't be read",
    };

    /// A folder of its own for one test, with a few things in it.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("craft-ui-local-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Proposals")).unwrap();
        std::fs::create_dir_all(dir.join("archive")).unwrap();
        for file in ["notes.DOCX", "Budget.docx", "picture.png", ".hidden.docx"] {
            std::fs::write(dir.join(file), b"").unwrap();
        }
        dir
    }

    fn names(folder: &LocalFolder) -> Vec<&str> {
        folder.items().iter().map(|item| item.name.as_str()).collect()
    }

    #[test]
    fn a_folder_lists_subfolders_first_then_the_files_of_its_kinds() {
        let dir = scratch("lists");
        let folder = LocalFolder::open(&dir, &["docx"]);
        assert_eq!(
            names(&folder),
            ["archive", "Proposals", "Budget.docx", "notes.DOCX"],
            "by name whatever the case, hidden and other kinds left out"
        );
        assert_eq!(names(&LocalFolder::open(&dir, &["*"])), ["archive", "Proposals", "Budget.docx", "notes.DOCX", "picture.png"]);
        assert!(folder.failed().is_none() && !folder.truncated());
        let entries = folder.entries();
        assert!(entries.first().is_some_and(|e| e.folder) && entries.last().is_some_and(|e| !e.folder));
        assert!(matches!(folder.content(&entries, LABELS), PlacesContent::Entries(listed) if listed.len() == 4));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_bigger_folder_than_the_cap_says_it_was_cut() {
        let dir = scratch("cap");
        let mut folder = LocalFolder::open(&dir, &["*"]);
        folder.read(2);
        assert_eq!(folder.items().len(), 2);
        assert!(folder.truncated());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn choosing_a_subfolder_moves_and_choosing_a_file_hands_it_to_the_app() {
        let dir = scratch("act");
        let mut folder = LocalFolder::open(&dir, &["docx"]);
        assert_eq!(folder.act(PlacesAction::Activate(1)), LocalOutcome::Stay);
        assert_eq!(folder.dir(), dir.join("Proposals"), "the listing moved into the subfolder");
        assert!(folder.items().is_empty());
        folder.go(&dir);
        assert_eq!(folder.act(PlacesAction::Activate(2)), LocalOutcome::Open(dir.join("Budget.docx")));
        assert_eq!(folder.act(PlacesAction::Activate(99)), LocalOutcome::Stay, "an index that is not there is ignored");
        assert_eq!(folder.act(PlacesAction::Browse), LocalOutcome::Browse);
        assert_eq!(folder.act(PlacesAction::Notice), LocalOutcome::Browse, "the no-access notice's button is Browse");
        assert_eq!(folder.act(PlacesAction::Select(0)), LocalOutcome::Stay);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_breadcrumb_names_each_folder_and_leads_back_up() {
        let dir = scratch("crumbs");
        let mut folder = LocalFolder::open(dir.join("Proposals"), &["docx"]);
        let crumbs = folder.crumbs("This computer");
        assert_eq!(crumbs.first().map(String::as_str), Some("This computer"));
        assert_eq!(crumbs.last().map(String::as_str), Some("Proposals"));
        // The step before the last is the scratch folder itself.
        assert_eq!(folder.act(PlacesAction::Crumb(crumbs.len() - 2)), LocalOutcome::Stay);
        assert_eq!(folder.dir(), dir);
        assert_eq!(folder.act(PlacesAction::Crumb(999)), LocalOutcome::Stay, "a step that is not there is ignored");
        assert_eq!(folder.dir(), dir);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_folder_with_nothing_to_show_says_why() {
        let dir = scratch("notices");
        let title = |folder: &LocalFolder| match folder.content(&[], LABELS) {
            PlacesContent::Notice(notice) => (notice.title, notice.action),
            other => panic!("expected a notice, got {other:?}"),
        };
        assert_eq!(title(&LocalFolder::open(dir.join("Proposals"), &["docx"])), (LABELS.empty, None));
        let gone = LocalFolder::open(dir.join("no such folder"), &["docx"]);
        assert_eq!(gone.failed(), Some(ErrorKind::NotFound));
        assert_eq!(title(&gone), (LABELS.gone, None));
        // A file is not a folder: neither missing nor forbidden, simply not readable as one.
        let odd = LocalFolder::open(dir.join("Budget.docx"), &["docx"]);
        assert!(odd.failed().is_some());
        assert_eq!(title(&odd).1, None);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// What Windows marks hidden is left out, as a leading dot is elsewhere.
    #[cfg(windows)]
    #[test]
    fn an_entry_windows_marks_hidden_is_left_out() {
        let dir = scratch("hidden");
        std::fs::create_dir_all(dir.join("$RECYCLE.BIN")).unwrap();
        std::fs::write(dir.join("desktop.ini"), b"").unwrap();
        for name in ["$RECYCLE.BIN", "desktop.ini"] {
            let status = std::process::Command::new("attrib").arg("+h").arg(dir.join(name)).status().unwrap();
            assert!(status.success());
        }
        assert_eq!(names(&LocalFolder::open(&dir, &["*"])), ["archive", "Proposals", "Budget.docx", "notes.DOCX", "picture.png"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A folder closed to the user gets the notice with Browse. Root reads anything, so there the
    /// folder simply lists and the check is skipped.
    #[cfg(unix)]
    #[test]
    fn a_folder_closed_to_the_app_offers_browse() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("closed");
        let closed = dir.join("archive");
        std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o000)).unwrap();
        let folder = LocalFolder::open(&closed, &["docx"]);
        if folder.failed() == Some(ErrorKind::PermissionDenied) {
            assert!(
                matches!(folder.content(&[], LABELS), PlacesContent::Notice(n) if n.title == LABELS.no_access && n.action == Some(LABELS.browse) && n.icon == PlacesIcon::Locked)
            );
        }
        std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
