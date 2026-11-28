//! `Directories`: the folder access the places tree needs, with `SystemDirectories` for the
//! desktop.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use craft_host::queue::Cancel;

/// A standard folder of the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StandardFolder {
    /// The home folder.
    Home,
    /// Desktop.
    Desktop,
    /// Documents.
    Documents,
    /// Downloads.
    Downloads,
    /// Pictures.
    Pictures,
    /// Movies (Videos on Windows and Linux).
    Movies,
    /// Music.
    Music,
}

/// A mounted volume or drive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    /// Its root folder.
    pub path: PathBuf,
    /// Its name as the system shows it.
    pub name: String,
    /// A network volume.
    pub network: bool,
    /// A removable drive (a card, a USB disk).
    pub removable: bool,
}

/// A subfolder in a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    /// Its path.
    pub path: PathBuf,
    /// Its name.
    pub name: String,
}

/// A file or folder in a [`FolderEntries`] listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Its path.
    pub path: PathBuf,
    /// Its name.
    pub name: String,
    /// Whether it is a folder.
    pub folder: bool,
}

/// The files and folders of one folder, as [`Directories::entries`] lists them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderEntries {
    /// The entries read, hidden ones left out, in `natural_cmp` order of their names.
    pub entries: Vec<Entry>,
    /// The listing stopped at the limit before the folder ended.
    pub truncated: bool,
}

/// The folder lookups a places tree needs (owner question 4: a small trait in this crate, with a
/// system implementation and a fake). Every call can block on a slow or network disk:
/// [`FolderCache`](super::FolderCache) calls them only in jobs.
pub trait Directories: Send + Sync {
    /// The standard folders that exist as folders, in the order Home, Desktop, Documents,
    /// Downloads, Pictures, Movies, Music; the host picks which it shows.
    fn standard(&self) -> Vec<(StandardFolder, PathBuf)>;

    /// The mounted volumes, in the system's order.
    fn volumes(&self) -> Vec<Volume>;

    /// The subfolders of `folder`: folders only (a link to a folder counts), hidden ones left
    /// out (a leading `.`; the hidden attribute on Windows; `UF_HIDDEN` on macOS), sorted by
    /// `craft_platform::open::natural_cmp` of their names. Checks `cancel` between entries and
    /// returns what it has when it is set.
    fn subfolders(&self, folder: &Path, cancel: &Cancel) -> io::Result<Vec<Folder>>;

    /// The hidden folders among `path` and its ancestors, innermost first (LightCraft's
    /// `unlisted_ancestors`); a tree never opens through one.
    fn unlisted_ancestors(&self, path: &Path) -> Vec<PathBuf>;

    /// The files and folders of `folder`, with the hidden rule and the `natural_cmp` order of
    /// [`subfolders`](Self::subfolders). Stops after `limit` entries read and reports
    /// `truncated`; checks `cancel` between entries.
    fn entries(&self, folder: &Path, limit: usize, cancel: &Cancel) -> io::Result<FolderEntries>;
}

/// [`Directories`] over the file system.
///
/// - `standard`: `HOME` (`USERPROFILE` on Windows) and its `Desktop`, `Documents`, `Downloads`,
///   `Pictures`, `Movies` (`Videos` on Windows and Linux), `Music`, each when it is a folder.
/// - `volumes`: macOS the entries of `/Volumes`; Linux `/media/$USER/*`, `/run/media/$USER/*` and
///   `/mnt/*`; Windows the drive letters `A:` to `Z:` whose root exists. `network` and `removable`
///   stay false where `std` can't tell (a platform service may add them later).
/// - `subfolders` and `entries`: `std::fs::read_dir`, as the trait says.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemDirectories;

#[cfg(not(target_arch = "wasm32"))]
impl Directories for SystemDirectories {
    fn standard(&self) -> Vec<(StandardFolder, PathBuf)> {
        // lightcraft: crates/ui-egui/src/panels/left/local.rs:24 (Pictures, Desktop, Downloads, Home under HOME or USERPROFILE).
        let Some(home) = home() else { return Vec::new() };
        let movies = if cfg!(target_os = "macos") { "Movies" } else { "Videos" };
        let named = [
            (StandardFolder::Home, ""),
            (StandardFolder::Desktop, "Desktop"),
            (StandardFolder::Documents, "Documents"),
            (StandardFolder::Downloads, "Downloads"),
            (StandardFolder::Pictures, "Pictures"),
            (StandardFolder::Movies, movies),
            (StandardFolder::Music, "Music"),
        ];
        named
            .into_iter()
            .map(|(kind, sub)| (kind, if sub.is_empty() { home.clone() } else { home.join(sub) }))
            .filter(|(_, path)| path.is_dir())
            .collect()
    }

    fn volumes(&self) -> Vec<Volume> {
        // new: FilmCraft's Local Drives list comes from its engine services; std paths here.
        let mut parents: Vec<PathBuf> = Vec::new();
        if cfg!(target_os = "macos") {
            parents.push(PathBuf::from("/Volumes"));
        } else if cfg!(unix) {
            if let Some(user) = std::env::var_os("USER").filter(|u| !u.is_empty()) {
                parents.push(Path::new("/media").join(&user));
                parents.push(Path::new("/run/media").join(&user));
            }
            parents.push(PathBuf::from("/mnt"));
        }
        let mut out = Vec::new();
        for parent in parents {
            let Ok(entries) = std::fs::read_dir(&parent) else { continue };
            let mut found: Vec<Volume> = entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| Volume { path: e.path(), name: e.file_name().to_string_lossy().into_owned(), network: false, removable: false })
                .collect();
            found.sort_by(|a, b| craft_platform::open::natural_cmp(&a.name, &b.name));
            out.extend(found);
        }
        if cfg!(windows) {
            for letter in 'A'..='Z' {
                let root = PathBuf::from(format!("{letter}:\\"));
                if root.is_dir() {
                    out.push(Volume { name: format!("{letter}:"), path: root, network: false, removable: false });
                }
            }
        }
        out
    }

    fn subfolders(&self, folder: &Path, cancel: &Cancel) -> io::Result<Vec<Folder>> {
        // lightcraft: crates/ui-egui/src/panels/left/local.rs:211 (`list_subfolders`: folders, not hidden, sorted).
        let mut out = Vec::new();
        for entry in std::fs::read_dir(folder)? {
            if cancel.is_cancelled() {
                break;
            }
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            let is_folder = match entry.file_type() {
                Ok(t) if t.is_dir() => true,
                Ok(t) if t.is_symlink() => path.is_dir(),
                _ => false,
            };
            if is_folder && !is_hidden(&path, || entry.metadata()) {
                out.push(Folder { name: entry.file_name().to_string_lossy().into_owned(), path });
            }
        }
        out.sort_by(|a, b| craft_platform::open::natural_cmp(&a.name, &b.name));
        Ok(out)
    }

    fn entries(&self, folder: &Path, limit: usize, cancel: &Cancel) -> io::Result<FolderEntries> {
        // ui-egui: crates/ui-egui/src/file_browser.rs:58 (`list_dir`: `MAX_SCANNED` entries read, hidden left out, links to folders are folders).
        let mut out = FolderEntries::default();
        for (read, entry) in std::fs::read_dir(folder)?.enumerate() {
            if cancel.is_cancelled() {
                break;
            }
            // Hidden entries count as read too: the limit bounds the time a huge folder takes.
            if read >= limit {
                out.truncated = true;
                break;
            }
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            let folder = match entry.file_type() {
                Ok(t) if t.is_symlink() => path.is_dir(),
                Ok(t) => t.is_dir(),
                Err(_) => continue,
            };
            if !is_hidden(&path, || entry.metadata()) {
                out.entries.push(Entry { name: entry.file_name().to_string_lossy().into_owned(), path, folder });
            }
        }
        out.entries.sort_by(|a, b| craft_platform::open::natural_cmp(&a.name, &b.name));
        Ok(out)
    }

    fn unlisted_ancestors(&self, path: &Path) -> Vec<PathBuf> {
        // lightcraft: crates/ui-egui/src/panels/left/local.rs:201 (`unlisted_ancestors`).
        path.ancestors().filter(|p| is_hidden(p, || std::fs::symlink_metadata(p))).map(Path::to_path_buf).collect()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).or_else(|| std::env::var_os("USERPROFILE").filter(|h| !h.is_empty())).map(PathBuf::from)
}

/// Whether the path has a dot-prefixed name or a Windows or macOS hidden attribute.
// lightcraft: crates/engine/src/walk.rs:56 (`hidden_with_metadata`).
#[cfg(not(target_arch = "wasm32"))]
fn is_hidden(path: &Path, metadata: impl FnOnce() -> io::Result<std::fs::Metadata>) -> bool {
    if dotted(path) {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        metadata().is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::darwin::fs::MetadataExt;
        const UF_HIDDEN: u32 = 0x0000_8000;
        metadata().is_ok_and(|m| m.st_flags() & UF_HIDDEN != 0)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = metadata;
        false
    }
}

/// Whether the path's name starts with a dot: the part of the hidden rule a name alone tells, so
/// [`FakeDirectories`] hides what [`SystemDirectories`] hides on every platform.
fn dotted(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

/// [`Directories`] over an in-memory tree, for tests and stories. Folders are added with
/// [`folder`](Self::folder) (their ancestors are implied) and can be added or removed through a
/// shared reference while a cache holds it.
#[derive(Default)]
pub struct FakeDirectories {
    standard: Vec<(StandardFolder, PathBuf)>,
    volumes: Vec<Volume>,
    children: Mutex<BTreeMap<PathBuf, BTreeSet<String>>>,
    /// Files, by their folder; only [`entries`](Directories::entries) lists them.
    files: Mutex<BTreeMap<PathBuf, BTreeSet<String>>>,
    /// Links, by path, to their target.
    links: BTreeMap<PathBuf, PathBuf>,
    unlisted: Vec<PathBuf>,
    failing: Vec<PathBuf>,
    calls: AtomicUsize,
}

impl FakeDirectories {
    /// An empty file system.
    pub fn new() -> Self {
        Self::default()
    }

    /// A standard folder, reported by [`standard`](Directories::standard) in the order added.
    #[must_use]
    pub fn standard_folder(mut self, kind: StandardFolder, path: impl Into<PathBuf>) -> Self {
        self.standard.push((kind, path.into()));
        self
    }

    /// A mounted volume, reported by [`volumes`](Directories::volumes) in the order added.
    #[must_use]
    pub fn volume(mut self, volume: Volume) -> Self {
        self.volumes.push(volume);
        self
    }

    /// The folder at `path`, with its ancestors.
    #[must_use]
    pub fn folder(self, path: impl AsRef<Path>) -> Self {
        self.add_folder(path);
        self
    }

    /// The file at `path`, with its folder and that folder's ancestors.
    /// [`entries`](Directories::entries) lists it; [`subfolders`](Directories::subfolders) does
    /// not.
    #[must_use]
    pub fn file(self, path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
            self.add_folder(parent);
            let mut files = self.files.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            files.entry(parent.to_path_buf()).or_default().insert(name.to_string_lossy().into_owned());
        }
        self
    }

    /// Removes the file at `path`; later listings leave it out.
    pub fn remove_file(&self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { return };
        if let Some(names) = self.files.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_mut(parent) {
            names.remove(&*name.to_string_lossy());
        }
    }

    /// A link at `path` to `target`. [`entries`](Directories::entries) lists it as a folder when
    /// `target` is a folder of this tree, else as a file.
    #[must_use]
    pub fn link(mut self, path: impl AsRef<Path>, target: impl Into<PathBuf>) -> Self {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            self.add_folder(parent);
        }
        self.links.insert(path.to_path_buf(), target.into());
        self
    }

    /// A hidden folder: `unlisted_ancestors` reports it for paths inside it.
    #[must_use]
    pub fn hidden(mut self, path: impl Into<PathBuf>) -> Self {
        self.unlisted.push(path.into());
        self
    }

    /// Listing `path` fails with a permission error.
    #[must_use]
    pub fn failing(mut self, path: impl Into<PathBuf>) -> Self {
        self.failing.push(path.into());
        self
    }

    /// Adds the folder at `path` and its ancestors; later listings show it.
    pub fn add_folder(&self, path: impl AsRef<Path>) {
        let mut children = self.children.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut path = path.as_ref();
        while let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
            children.entry(parent.to_path_buf()).or_default().insert(name.to_string_lossy().into_owned());
            path = parent;
        }
    }

    /// Removes the folder at `path` from its parent's listing.
    pub fn remove_folder(&self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { return };
        if let Some(names) = self.children.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_mut(parent) {
            names.remove(&*name.to_string_lossy());
        }
    }

    /// How many [`subfolders`](Directories::subfolders) calls were made.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

impl Directories for FakeDirectories {
    fn standard(&self) -> Vec<(StandardFolder, PathBuf)> {
        self.standard.clone()
    }

    fn volumes(&self) -> Vec<Volume> {
        self.volumes.clone()
    }

    fn subfolders(&self, folder: &Path, _: &Cancel) -> io::Result<Vec<Folder>> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.failing.iter().any(|p| p == folder) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "permission denied"));
        }
        let children = self.children.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut out: Vec<Folder> =
            children.get(folder).map(|names| names.iter().map(|n| Folder { path: folder.join(n), name: n.clone() }).collect()).unwrap_or_default();
        out.sort_by(|a, b| craft_platform::open::natural_cmp(&a.name, &b.name));
        Ok(out)
    }

    fn entries(&self, folder: &Path, limit: usize, cancel: &Cancel) -> io::Result<FolderEntries> {
        if self.failing.iter().any(|p| p == folder) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "permission denied"));
        }
        let children = self.children.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let is_folder = |path: &Path| {
            children.contains_key(path)
                || matches!((path.parent(), path.file_name()), (Some(parent), Some(name)) if children.get(parent).is_some_and(|names| names.contains(&*name.to_string_lossy())))
        };
        // The read order of this fake: folders, files, then links, each by name.
        let folders = children.get(folder).into_iter().flatten().map(|n| (folder.join(n), true));
        let file_names = self.files.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let files = file_names.get(folder).into_iter().flatten().map(|n| (folder.join(n), false));
        let links = self.links.iter().filter(|(path, _)| path.parent() == Some(folder)).map(|(path, target)| (path.clone(), is_folder(target)));
        let mut out = FolderEntries::default();
        for (read, (path, is_dir)) in folders.chain(files).chain(links).enumerate() {
            if cancel.is_cancelled() {
                break;
            }
            if read >= limit {
                out.truncated = true;
                break;
            }
            if dotted(&path) {
                continue;
            }
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            out.entries.push(Entry { path, name, folder: is_dir });
        }
        out.entries.sort_by(|a, b| craft_platform::open::natural_cmp(&a.name, &b.name));
        Ok(out)
    }

    fn unlisted_ancestors(&self, path: &Path) -> Vec<PathBuf> {
        path.ancestors().filter(|a| self.unlisted.iter().any(|h| h == a)).map(Path::to_path_buf).collect()
    }
}
