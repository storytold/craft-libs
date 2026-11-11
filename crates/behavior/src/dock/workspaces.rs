//! Named workspaces (Window › Workspace). A workspace is a saved [`Workspace`] layout plus which bars show;
//! the set holds the app's built-in workspaces, the user's saved ones, the current one,
//! Lock Workspace and Reset.

use std::{collections::BTreeMap, hash::Hash};

use super::{layout::Layout, workspace::Workspace};

/// Chrome a workspace remembers beside the dock layout: which bars show. The default shows every bar.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct WorkspaceChrome {
    /// The menu bar shows.
    pub menu_bar: bool,
    /// The tool rail shows.
    pub tool_rail: bool,
    /// The options bar shows.
    pub options_bar: bool,
    /// The status bar shows.
    pub status_bar: bool,
}

impl Default for WorkspaceChrome {
    fn default() -> Self {
        Self { menu_bar: true, tool_rail: true, options_bar: true, status_bar: true }
    }
}

/// One named workspace.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(bound(deserialize = "P: serde::Deserialize<'de>"))]
#[non_exhaustive]
pub struct NamedWorkspace<P> {
    /// Stable id (`"essentials"`, or a slug of a saved name).
    pub id: String,
    /// What the menus and the switcher show.
    pub name: String,
    /// The panel layout.
    pub layout: Workspace<P>,
    /// Which bars show.
    pub chrome: WorkspaceChrome,
    /// Whether Keyboard Shortcuts and Menus were saved with it (Photoshop's New Workspace boxes).
    pub keeps_shortcuts: bool,
    /// Whether menu customizations were saved with it.
    pub keeps_menus: bool,
}

/// The app's workspaces: built-ins first, saved ones after, one current. It is a settings
/// section (serialized as [`Workspace`] is); the layouts of built-ins are
/// code, and only their user edits ([`edited`](Self::edited)) are stored. Files of an older
/// [`Workspace::VERSION`] load too.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "StoredSet<P>", bound(deserialize = "P: serde::Deserialize<'de>"))]
#[non_exhaustive]
pub struct WorkspaceSet<P> {
    /// Workspaces the app defines in code; the app supplies them again on every start, so they
    /// are not stored.
    #[serde(skip)]
    pub builtin: Vec<NamedWorkspace<P>>,
    /// Workspaces the user saved.
    pub saved: Vec<NamedWorkspace<P>>,
    /// Id of the current workspace.
    pub current: String,
    /// Lock Workspace: pass it to [`Workspace::apply`] and [`Workspace::set_visible`]. Moves,
    /// floating and closes are refused; activating, opening, minimizing, resizing and
    /// recording native windows still work.
    pub locked: bool,
    /// The live layout each workspace was left with, by id, until Reset or Delete. Photoshop
    /// remembers the state of each workspace this way.
    pub edited: BTreeMap<String, Workspace<P>>,
}

impl<P> Default for WorkspaceSet<P> {
    fn default() -> Self {
        Self { builtin: Vec::new(), saved: Vec::new(), current: String::new(), locked: false, edited: BTreeMap::new() }
    }
}

impl<P> NamedWorkspace<P> {
    /// A workspace named `name` with the stable id `id`, showing every bar and keeping no shortcuts or menus.
    pub fn new(id: impl Into<String>, name: impl Into<String>, layout: Workspace<P>) -> Self {
        Self { id: id.into(), name: name.into(), layout, chrome: WorkspaceChrome::default(), keeps_shortcuts: false, keeps_menus: false }
    }
}

impl<P: Clone + Eq + Hash> WorkspaceSet<P> {
    /// A set of `builtin` workspaces with the first one current.
    pub fn new(builtin: Vec<NamedWorkspace<P>>) -> Self {
        let current = builtin.first().map(|w| w.id.clone()).unwrap_or_default();
        Self { builtin, current, ..Self::default() }
    }

    /// The current workspace, or `None` when `current` names none.
    pub fn current(&self) -> Option<&NamedWorkspace<P>> {
        self.find(&self.current)
    }

    /// The workspace with this id.
    pub fn find(&self, id: &str) -> Option<&NamedWorkspace<P>> {
        self.builtin.iter().chain(&self.saved).find(|w| w.id == id)
    }

    /// `base`, or `base-2`, `base-3`… when another workspace has the id.
    fn free_id(&self, base: &str) -> String {
        let taken = |id: &str| self.find(id).is_some();
        if !taken(base) {
            return base.to_string();
        }
        (2..).map(|n| format!("{base}-{n}")).find(|id| !taken(id)).unwrap_or_default()
    }

    /// The layout the current workspace shows: what it was left with, else its stored one.
    pub fn layout(&self) -> Option<&Workspace<P>> {
        self.edited.get(&self.current).or_else(|| self.current().map(|w| &w.layout))
    }

    /// Switch to `id` and replace `live` with the layout it shows; the current workspace's live
    /// layout is kept under its id first (Photoshop remembers the state of each workspace until
    /// Reset). Selecting the current workspace changes nothing.
    pub fn select(&mut self, id: &str, live: &mut Workspace<P>) -> Result<(), WorkspaceError> {
        if self.find(id).is_none() {
            return Err(WorkspaceError::UnknownId);
        }
        if id == self.current {
            return Ok(());
        }
        if self.find(&self.current).is_some() {
            self.edited.insert(self.current.clone(), live.clone());
        }
        self.current = id.to_string();
        live.replace(self.layout().cloned().unwrap_or_default());
        Ok(())
    }

    /// Save `live` under `name` and make it current; a name a built-in has is refused. A saved
    /// workspace of the same name, in any case, is replaced.
    pub fn save(&mut self, name: &str, live: Workspace<P>, chrome: WorkspaceChrome) -> Result<String, WorkspaceError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(WorkspaceError::EmptyName);
        }
        let same = |other: &str| other.to_lowercase() == name.to_lowercase();
        if self.builtin.iter().any(|w| same(&w.name) || same(&w.id)) {
            return Err(WorkspaceError::NameTaken);
        }
        let existing = self.saved.iter().position(|w| same(&w.name));
        let id = match existing {
            Some(i) => self.saved[i].id.clone(),
            None => self.free_id(&slug(name)),
        };
        let layout = live;
        let (keeps_shortcuts, keeps_menus) = existing.map_or((false, false), |i| (self.saved[i].keeps_shortcuts, self.saved[i].keeps_menus));
        let workspace = NamedWorkspace { id: id.clone(), name: name.to_string(), layout, chrome, keeps_shortcuts, keeps_menus };
        match existing {
            Some(i) => self.saved[i] = workspace,
            None => self.saved.push(workspace),
        }
        self.edited.remove(&id);
        self.current = id.clone();
        Ok(id)
    }

    /// Delete a saved workspace; the current one can't be deleted (Photoshop greys it).
    pub fn delete(&mut self, id: &str) -> Result<(), WorkspaceError> {
        let Some(i) = self.saved.iter().position(|w| w.id == id) else {
            return Err(WorkspaceError::UnknownId);
        };
        if self.current == id {
            return Err(WorkspaceError::CurrentCannotBeDeleted);
        }
        self.saved.remove(i);
        self.edited.remove(id);
        Ok(())
    }

    /// Reset the current workspace: drop what it was left with and replace `live` with its stored
    /// layout (a built-in's code layout, or the layout it was saved with).
    pub fn reset(&mut self, live: &mut Workspace<P>) -> Result<(), WorkspaceError> {
        let stored = self.current().map(|w| w.layout.clone()).ok_or(WorkspaceError::UnknownId)?;
        self.edited.remove(&self.current);
        live.replace(stored);
        Ok(())
    }
}

/// Lower-case alphanumerics joined by single dashes; the name itself when it has none.
fn slug(name: &str) -> String {
    let words: Vec<String> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect();
    if words.is_empty() { name.to_string() } else { words.join("-") }
}

/// Why a [`WorkspaceSet`] edit was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WorkspaceError {
    /// No workspace has the id.
    UnknownId,
    /// A built-in workspace already has the name.
    NameTaken,
    /// The name is empty after trimming.
    EmptyName,
    /// The current workspace cannot be deleted.
    CurrentCannotBeDeleted,
}

/// The file shape of a [`WorkspaceSet`], which also reads the files of format versions 1 and 2.
/// Those carried a `locked` and a `saved` layout on every [`Workspace`]; the set owns both now.
#[derive(serde::Deserialize)]
#[serde(bound(deserialize = "P: serde::Deserialize<'de>"))]
struct StoredSet<P> {
    #[serde(default = "Vec::new")]
    saved: Vec<StoredNamed<P>>,
    #[serde(default)]
    current: String,
    #[serde(default)]
    locked: bool,
    #[serde(default = "BTreeMap::new")]
    edited: BTreeMap<String, StoredWorkspace<P>>,
}

#[derive(serde::Deserialize)]
#[serde(bound(deserialize = "P: serde::Deserialize<'de>"))]
struct StoredNamed<P> {
    id: String,
    name: String,
    layout: StoredWorkspace<P>,
    chrome: WorkspaceChrome,
    keeps_shortcuts: bool,
    keeps_menus: bool,
}

/// A [`Workspace`] with the fields of format version 2 it no longer has.
#[derive(serde::Deserialize)]
#[serde(bound(deserialize = "P: serde::Deserialize<'de>"))]
struct StoredWorkspace<P> {
    #[serde(flatten)]
    workspace: Workspace<P>,
    #[serde(default)]
    locked: bool,
    #[serde(default = "no_layout")]
    saved: Option<Layout<P>>,
}

fn no_layout<P>() -> Option<Layout<P>> {
    None
}

impl<P> From<StoredSet<P>> for WorkspaceSet<P> {
    /// A lock on any stored workspace locks the set. A saved arrangement becomes the stored
    /// layout Reset returns to; one on a built-in is dropped, since the code supplies it.
    fn from(stored: StoredSet<P>) -> Self {
        let mut locked = stored.locked;
        let mut saved: Vec<NamedWorkspace<P>> = stored
            .saved
            .into_iter()
            .map(|named| {
                let StoredWorkspace { mut workspace, locked: lock, saved } = named.layout;
                locked |= lock;
                if let Some(layout) = saved {
                    workspace.layout = layout;
                }
                NamedWorkspace {
                    id: named.id,
                    name: named.name,
                    layout: workspace,
                    chrome: named.chrome,
                    keeps_shortcuts: named.keeps_shortcuts,
                    keeps_menus: named.keeps_menus,
                }
            })
            .collect();
        let mut edited = BTreeMap::new();
        for (id, stored) in stored.edited {
            locked |= stored.locked;
            if let Some(layout) = stored.saved
                && let Some(named) = saved.iter_mut().find(|w| w.id == id)
            {
                named.layout.layout = layout;
            }
            edited.insert(id, stored.workspace);
        }
        Self { builtin: Vec::new(), saved, current: stored.current, locked, edited }
    }
}
