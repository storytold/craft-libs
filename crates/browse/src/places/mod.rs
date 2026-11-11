//! The roots of a folder tree and lazy folder listings (`Place`, `Places`, `PlacesSettings`,
//! `Directories`, `SystemDirectories`, `FolderCache`, `same_folder`, `folder_within`). Folder
//! access goes only through [`Directories`]; [`SystemDirectories`] is desktop-only.
//!
//! ```ignore
//! // Each pass: the roots from the cache, resolved against the settings and the browsed folder.
//! let roots = cache.roots();                                   // listed in a job, None at first
//! let places = Places::resolve(&PlacesInput {
//!     standard: roots.map_or(&[][..], |r| &r.standard), volumes: roots.map_or(&[][..], |r| &r.volumes),
//!     settings: &settings.places, browsing: browsing.as_deref(),
//!     unlisted: browsing.as_deref().and_then(|b| cache.unlisted(b)).unwrap_or(&[]),
//!     groups: &[PlaceGroupKind::Places, PlaceGroupKind::Volumes],
//! });
//! settings.places.session_root = places.session_root.clone();  // kept for the session only
//! ```

mod cache;
mod directories;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

pub use cache::{FolderCache, Listing, Roots};
#[cfg(not(target_arch = "wasm32"))]
pub use directories::SystemDirectories;
pub use directories::{Directories, FakeDirectories, Folder, StandardFolder, Volume};

/// What kind of root a place is; the tree's icon, name and menu follow from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlaceKind {
    /// A standard folder: the tree names it with `PlacesLabels` (Home, not the user's folder name).
    Standard(StandardFolder),
    /// A folder the user kept (Keep in Places, Add to Favorites).
    Favorite,
    /// A mounted volume or drive.
    Volume,
    /// A network volume.
    Network,
    /// A recently browsed folder.
    Recent,
    /// The browsed folder, listed for this session because no root holds it.
    Session,
}

/// One root of the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// The folder, as the platform spells it.
    pub path: PathBuf,
    /// Its name: the folder's or volume's own; the tree shows the `PlacesLabels` word instead for
    /// [`PlaceKind::Standard`].
    pub name: String,
    /// What it is.
    pub kind: PlaceKind,
}

/// The headings of the tree. The host lists the ones it shows, in order (lead call 24): FilmCraft
/// shows Favorites, Volumes, Network, Recent; LightCraft shows Places only, with its kept folders
/// in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlaceGroupKind {
    /// Kept folders. Without this group they join `Places`, after the standard folders.
    Favorites,
    /// Standard folders (and, without `Favorites`, the kept ones), then the session root.
    Places,
    /// Local volumes and drives.
    Volumes,
    /// Network volumes. Without this group they join `Volumes`.
    Network,
    /// Recent folders, newest first.
    Recent,
}

/// One heading and its roots, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceGroup {
    /// The heading.
    pub kind: PlaceGroupKind,
    /// Its roots; an empty group shows "(none)".
    pub places: Vec<Place>,
}

/// What [`Places::resolve`] reads.
#[derive(Clone, Copy, Debug)]
pub struct PlacesInput<'a> {
    /// The standard folders that exist, from [`FolderCache::roots`].
    pub standard: &'a [(StandardFolder, PathBuf)],
    /// The volumes, from [`FolderCache::roots`].
    pub volumes: &'a [Volume],
    /// The saved favourites, hidden roots and recents, and the session root.
    pub settings: &'a PlacesSettings,
    /// The folder being browsed, if any.
    pub browsing: Option<&'a Path>,
    /// The hidden folders on the way to `browsing` (dot folders, hidden attribute), from
    /// [`FolderCache::unlisted`]; empty while unknown.
    pub unlisted: &'a [PathBuf],
    /// The groups the host shows, in order.
    pub groups: &'a [PlaceGroupKind],
}

/// The tree's roots and where the browsed folder sits among them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Places {
    /// The groups, in the host's order.
    pub groups: Vec<PlaceGroup>,
    /// The root the browsed folder lies in, as (group, place): the innermost one by path length.
    /// Its tree opens on the way down to the folder. `None` when the way passes through a hidden
    /// folder, or nothing is browsed.
    pub owner: Option<(usize, usize)>,
    /// The session root listed for the browsed folder (the previous one while browsing stays
    /// below it, else the browsed folder); the host stores it in
    /// [`PlacesSettings::session_root`] for the next pass.
    pub session_root: Option<PathBuf>,
    /// The hidden roots, for the `Show N hidden places` row.
    pub hidden: Vec<Place>,
}

impl Places {
    /// The roots, a pure port of LightCraft's `local_places_with_unlisted` with its rules and
    /// tests: the standard folders, then the favourites (in the Favorites group when listed, else
    /// in Places), each folder once however it is spelled ([`same_folder`], first spelling wins),
    /// hidden ones left out (they go to `hidden`, keeping their standard kind so the row reads
    /// "Home"); volumes in Volumes (network ones in Network when listed); recents in Recent. Then,
    /// only when no root holds the browsed folder ([`folder_within`], and no `unlisted` folder
    /// lies between the root and it), a session root at the end of Places: the settings' session
    /// root while browsing stays below it, else the browsed folder itself; never a hidden one.
    /// `owner` is the innermost root holding the browsed folder whose way down passes no hidden
    /// root.
    pub fn resolve(input: &PlacesInput<'_>) -> Places {
        // lightcraft: crates/ui-egui/src/panels/left/local.rs:156 (`local_places_with_unlisted`).
        let settings = input.settings;
        let is_hidden = |p: &Path| settings.hidden.iter().any(|h| same_folder(h, p));
        let listed = |kind: PlaceGroupKind| input.groups.contains(&kind);

        let standard: Vec<Place> = input
            .standard
            .iter()
            .map(|(kind, path)| Place { path: path.clone(), name: folder_label(path), kind: PlaceKind::Standard(*kind) })
            .collect();
        let mut standard = unique(standard);
        let favorites = settings.favorites.iter().filter(|f| !standard.iter().any(|s| same_folder(&s.path, f))).map(|f| Place {
            path: f.clone(),
            name: folder_label(f),
            kind: PlaceKind::Favorite,
        });
        let mut favorites = unique(favorites.collect());
        let volumes = input.volumes.iter().map(|v| Place {
            path: v.path.clone(),
            name: v.name.clone(),
            kind: if v.network { PlaceKind::Network } else { PlaceKind::Volume },
        });
        let mut volumes = unique(volumes.collect());
        let recents = settings.recent.iter().map(|r| Place { path: r.clone(), name: folder_label(r), kind: PlaceKind::Recent });
        let mut recents = unique(recents.collect());

        // The hidden roots keep the kind and name they have among the candidates ("Home", not the
        // user's folder name); one that is no candidate reads by its folder name.
        let mut hidden: Vec<Place> = Vec::new();
        for path in &settings.hidden {
            if hidden.iter().any(|h| same_folder(&h.path, path)) {
                continue;
            }
            let found = [&standard, &favorites, &volumes, &recents].into_iter().flatten().find(|p| same_folder(&p.path, path));
            hidden.push(found.cloned().unwrap_or_else(|| Place { path: path.clone(), name: folder_label(path), kind: PlaceKind::Favorite }));
        }
        for list in [&mut standard, &mut favorites, &mut volumes, &mut recents] {
            list.retain(|p| !is_hidden(&p.path));
        }

        let mut groups: Vec<PlaceGroup> = Vec::new();
        for &kind in input.groups {
            if groups.iter().any(|g| g.kind == kind) {
                continue;
            }
            let places = match kind {
                PlaceGroupKind::Favorites => favorites.clone(),
                PlaceGroupKind::Places => {
                    let mut places = standard.clone();
                    if !listed(PlaceGroupKind::Favorites) {
                        places.extend(favorites.iter().cloned());
                    }
                    places
                }
                PlaceGroupKind::Volumes => {
                    volumes.iter().filter(|v| v.kind == PlaceKind::Volume || !listed(PlaceGroupKind::Network)).cloned().collect()
                }
                PlaceGroupKind::Network => volumes.iter().filter(|v| v.kind == PlaceKind::Network).cloned().collect(),
                PlaceGroupKind::Recent => recents.clone(),
            };
            groups.push(PlaceGroup { kind, places });
        }

        let mut session_root = None;
        if let Some(current) = input.browsing {
            // Recent entries are shortcuts: they neither hold the browsed folder nor own its reveal.
            let holds = |p: &Place| p.kind != PlaceKind::Recent && listed_below(current, &p.path, input.unlisted);
            if !groups.iter().flat_map(|g| &g.places).any(holds) {
                let root = settings.session_root.as_deref().filter(|r| listed_below(current, r, input.unlisted)).unwrap_or(current);
                let target = groups.iter().position(|g| g.kind == PlaceGroupKind::Places).or(if groups.is_empty() { None } else { Some(0) });
                if let (false, Some(target)) = (is_hidden(root), target) {
                    groups[target].places.push(Place { path: root.to_path_buf(), name: folder_label(root), kind: PlaceKind::Session });
                    session_root = Some(root.to_path_buf());
                }
            }
        }

        // A tree never opens on the way down through a hidden folder: hiding a kept folder
        // beneath Home would otherwise reveal it again inside Home's tree.
        let owner = input.browsing.and_then(|current| {
            let through_hidden = |p: &Path| settings.hidden.iter().any(|h| folder_within(current, h) && folder_within(h, p));
            let mut best: Option<((usize, usize), usize)> = None;
            for (g, group) in groups.iter().enumerate() {
                for (i, place) in group.places.iter().enumerate() {
                    if place.kind == PlaceKind::Recent || !listed_below(current, &place.path, input.unlisted) || through_hidden(&place.path) {
                        continue;
                    }
                    let len = folder_key(&place.path).len();
                    if best.is_none_or(|(_, b)| len >= b) {
                        best = Some(((g, i), len));
                    }
                }
            }
            best.map(|(at, _)| at)
        });

        Places { groups, owner, session_root, hidden }
    }

    /// The place at (group, place).
    pub fn get(&self, at: (usize, usize)) -> Option<&Place> {
        // new: a lookup for the tree and the host.
        self.groups.get(at.0)?.places.get(at.1)
    }
}

/// The places a user keeps, hides and recently visited: a `craft_host::Section` (`KEY`
/// `"places"`), saved by the host. Reading is lenient (serde defaults; an unreadable entry is
/// dropped).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PlacesSettings {
    /// Kept folders, in the order kept.
    pub favorites: Vec<PathBuf>,
    /// Roots the user removed (standard folders, volumes); they come back through the hidden row.
    pub hidden: Vec<PathBuf>,
    /// Recent folders, newest first, at most [`PlacesSettings::RECENT_MAX`].
    pub recent: Vec<PathBuf>,
    /// The session root of the last [`Places::resolve`]; not saved.
    #[serde(skip)]
    pub session_root: Option<PathBuf>,
}

impl craft_host::Section for PlacesSettings {
    const KEY: &'static str = "places";
}

impl PlacesSettings {
    /// Recent folders kept: 10 (FilmCraft's `RECENT_MAX`).
    pub const RECENT_MAX: usize = 10;

    /// Keep in Places / Add to Favorites: appends `folder` unless an equal one ([`same_folder`])
    /// is kept; also takes it out of `hidden`. Returns whether anything changed.
    pub fn keep(&mut self, folder: &Path) -> bool {
        // lightcraft: crates/ui-egui/src/panels/left/local.rs:156 (`saved` roots).
        let hidden = self.hidden.len();
        self.hidden.retain(|h| !same_folder(h, folder));
        let mut changed = self.hidden.len() != hidden;
        if !self.favorites.iter().any(|f| same_folder(f, folder)) {
            self.favorites.push(folder.to_path_buf());
            changed = true;
        }
        changed
    }

    /// Remove from Places: a favourite is dropped from `favorites`; any other root is added to
    /// `hidden`. The folder itself is never touched. Returns whether anything changed.
    pub fn remove(&mut self, folder: &Path) -> bool {
        // lightcraft: crates/ui-egui/src/panels/left/local.rs:97 (`hidden_menu`, "Remove from Local").
        let favorites = self.favorites.len();
        self.favorites.retain(|f| !same_folder(f, folder));
        if self.favorites.len() != favorites {
            return true;
        }
        if self.hidden.iter().any(|h| same_folder(h, folder)) {
            return false;
        }
        self.hidden.push(folder.to_path_buf());
        true
    }

    /// Takes `folder` out of `hidden`.
    pub fn restore(&mut self, folder: &Path) -> bool {
        // lightcraft: crates/ui-egui/src/panels/left/local.rs:97 (`hidden_menu`: one at a time, or all).
        let hidden = self.hidden.len();
        self.hidden.retain(|h| !same_folder(h, folder));
        self.hidden.len() != hidden
    }

    /// Empties `hidden`.
    pub fn restore_all(&mut self) -> bool {
        // lightcraft: crates/ui-egui/src/panels/left/local.rs:97 (`hidden_menu`, Restore All).
        !std::mem::take(&mut self.hidden).is_empty()
    }

    /// Puts `folder` first in `recent`, dropping an equal entry and anything past `RECENT_MAX`.
    pub fn visited(&mut self, folder: &Path) {
        // filmcraft: crates/engine/src/media_browser.rs:80 (`RECENT_MAX` 10, newest first).
        self.recent.retain(|r| !same_folder(r, folder));
        self.recent.insert(0, folder.to_path_buf());
        self.recent.truncate(Self::RECENT_MAX);
    }

    /// Clear Recent Directories.
    pub fn clear_recent(&mut self) -> bool {
        // filmcraft: crates/ui-egui/src/panels/media_browser.rs:358 (Clear Recent Directories).
        !std::mem::take(&mut self.recent).is_empty()
    }
}

/// Whether `a` and `b` name the same folder however they are spelled: compared by their folder
/// key (separators `/`, `.` folded, no trailing separator; lower case on Windows and macOS, whose
/// file systems usually ignore case). Lexical, no I/O.
pub fn same_folder(a: &Path, b: &Path) -> bool {
    // lightcraft: crates/ui-egui/src/panels/left/local.rs:117 (`same_folder` over `folder_key`).
    a == b || folder_key(a) == folder_key(b)
}

/// Whether `path` is `root` or lies inside it, by the same folder key as [`same_folder`].
pub fn folder_within(path: &Path, root: &Path) -> bool {
    // lightcraft: crates/catalog/src/query.rs:692 (`folder_within`).
    key_within(&folder_key(path), &folder_key(root))
}

/// The folder's own name; the whole path for a root without one.
fn folder_label(path: &Path) -> String {
    let text = path.to_string_lossy();
    let name = text.rsplit(['/', '\\']).find(|part| !part.is_empty());
    name.map_or_else(|| text.to_string(), str::to_string)
}

/// `places` without a later spelling of a folder an earlier one already names.
fn unique(places: Vec<Place>) -> Vec<Place> {
    let mut out: Vec<Place> = Vec::with_capacity(places.len());
    for place in places {
        if !out.iter().any(|p| same_folder(&p.path, &place.path)) {
            out.push(place);
        }
    }
    out
}

/// Whether `current` is shown inside the tree of `root`: it lies within, and no hidden folder
/// (`unlisted`) lies between `root` and it.
fn listed_below(current: &Path, root: &Path, unlisted: &[PathBuf]) -> bool {
    folder_within(current, root) && !unlisted.iter().any(|h| !same_folder(h, root) && folder_within(current, h) && folder_within(h, root))
}

/// A folder path's identity: `/` and `\\` are both separators, repeated separators, `.` and a
/// trailing separator are dropped, `..` is resolved lexically, a Windows verbatim prefix (`\\?\`)
/// is removed and the drive letter lower-cased; on Windows and macOS (usually case-insensitive
/// file names) the whole path is lower-cased.
// lightcraft: crates/catalog/src/query.rs:645 (`split_path`) and :684 (`folder_key`).
fn folder_key(path: &Path) -> String {
    let mut s = path.to_string_lossy().replace('\\', "/");
    if let Some(rest) = s.strip_prefix("//?/").or_else(|| s.strip_prefix("//./")) {
        s = match rest.strip_prefix("UNC/") {
            Some(unc) => format!("//{unc}"),
            None => rest.to_string(),
        };
    }
    let (prefix, rest) = if let Some(unc) = s.strip_prefix("//").filter(|u| !u.is_empty() && !u.starts_with('/')) {
        let mut segs = unc.split('/').filter(|x| !x.is_empty());
        let (server, share) = (segs.next().unwrap_or(""), segs.next().unwrap_or(""));
        (format!("//{server}/{share}"), segs.collect::<Vec<_>>().join("/"))
    } else if matches!(s.as_bytes(), [a, b':', ..] if a.is_ascii_alphabetic()) {
        (s[..2].to_ascii_lowercase(), s[2..].to_string())
    } else {
        (String::new(), s.clone())
    };
    let absolute = rest.starts_with('/') || !prefix.is_empty();
    let mut parts: Vec<&str> = Vec::new();
    for c in rest.split('/') {
        match c {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            ".." if absolute => {}
            c => parts.push(c),
        }
    }
    let key = if absolute { format!("{prefix}/{}", parts.join("/")) } else { parts.join("/") };
    if cfg!(any(windows, target_os = "macos")) { key.to_lowercase() } else { key }
}

/// Whether the folder key `p` is the key `r` or lies inside it.
// lightcraft: crates/catalog/src/query.rs:730 (`key_within`).
fn key_within(p: &str, r: &str) -> bool {
    // `.` or `a/..` name no folder: not "everything"
    if r.is_empty() {
        return false;
    }
    // a disk's root key ends in `/` (`c:/`, `//srv/share/`), a folder's does not: same folder
    let (p, r) = (p.trim_end_matches('/'), r.trim_end_matches('/'));
    p == r || p.strip_prefix(r).is_some_and(|rest| rest.starts_with('/'))
}
