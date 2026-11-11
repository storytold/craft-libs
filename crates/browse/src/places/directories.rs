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
}

/// [`Directories`] over the file system.
///
/// - `standard`: `HOME` (`USERPROFILE` on Windows) and its `Desktop`, `Documents`, `Downloads`,
///   `Pictures`, `Movies` (`Videos` on Windows and Linux), `Music`, each when it is a folder.
/// - `volumes`: macOS the entries of `/Volumes`; Linux `/media/$USER/*`, `/run/media/$USER/*` and
///   `/mnt/*`; Windows the drive letters `A:` to `Z:` whose root exists. `network` and `removable`
///   stay false where `std` can't tell (a platform service may add them later).
/// - `subfolders`: `std::fs::read_dir`, as the trait says.
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
    if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
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

/// [`Directories`] over an in-memory tree, for tests and stories. Folders are added with
/// [`folder`](Self::folder) (their ancestors are implied) and can be added or removed through a
/// shared reference while a cache holds it.
#[derive(Default)]
pub struct FakeDirectories {
    standard: Vec<(StandardFolder, PathBuf)>,
    volumes: Vec<Volume>,
    children: Mutex<BTreeMap<PathBuf, BTreeSet<String>>>,
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

    fn unlisted_ancestors(&self, path: &Path) -> Vec<PathBuf> {
        path.ancestors().filter(|a| self.unlisted.iter().any(|h| h == a)).map(Path::to_path_buf).collect()
    }
}
