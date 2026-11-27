//! Open windows, their ids, the active window and most-recent order.

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

/// One top-level window, for as long as the process runs.
///
/// Ids come from one process-wide counter, so an id is never reused, even after its window closes
/// or across several [`WindowList`]s. Use it as a key for anything kept per window (UI state,
/// pending dialogs, an egui viewport).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(NonZeroU64);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl WindowId {
    fn mint() -> Self {
        // The counter starts at 1 and would wrap to 0 only after 2^64 opens.
        let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self(NonZeroU64::new(raw).unwrap_or(NonZeroU64::MAX))
    }

    /// The raw number, for logs and for keys outside Rust (control and MCP protocols).
    pub fn get(self) -> u64 {
        self.0.get()
    }
}

/// What [`WindowList::close`] hands back.
#[derive(Debug, PartialEq, Eq)]
pub struct Closed<T> {
    /// The closed window's data.
    pub data: T,
    /// The window to focus next: the most recently focused remaining window, when the closed one
    /// was the active window. `None` when another window was already active or none remain.
    pub focus: Option<WindowId>,
}

struct Entry<T> {
    id: WindowId,
    data: T,
}

/// The app's open windows, each with the app's data `T` (per-window state, or a document key).
///
/// Windows are kept in opening order, which is the stable order for menus. Separately, the list
/// keeps most-recent order: the window focused last comes first. The first of that order is the
/// *active* window, the one menu commands go to (on macOS, the most recently focused document
/// window, even while another app has focus).
pub struct WindowList<T> {
    entries: Vec<Entry<T>>,
    /// Most recently focused first; always a permutation of the entries' ids.
    recent: Vec<WindowId>,
}

impl<T> Default for WindowList<T> {
    fn default() -> Self {
        Self { entries: Vec::new(), recent: Vec::new() }
    }
}

impl<T> WindowList<T> {
    /// An empty list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a window with `data`. It becomes the active window, as the desktop focuses a new
    /// window.
    pub fn open(&mut self, data: T) -> WindowId {
        let id = WindowId::mint();
        self.entries.push(Entry { id, data });
        self.recent.insert(0, id);
        id
    }

    /// Close a window. Returns its data and, if it was active, the window to focus next; `None`
    /// if `id` is not open.
    pub fn close(&mut self, id: WindowId) -> Option<Closed<T>> {
        let index = self.entries.iter().position(|e| e.id == id)?;
        let entry = self.entries.remove(index);
        let was_active = self.active() == Some(id);
        self.recent.retain(|&r| r != id);
        let focus = if was_active { self.active() } else { None };
        Some(Closed { data: entry.data, focus })
    }

    /// Record that `id` got focus: it becomes active and moves to the front of most-recent order.
    /// Returns `false` if `id` is not open.
    pub fn focus(&mut self, id: WindowId) -> bool {
        if !self.contains(id) {
            return false;
        }
        self.recent.retain(|&r| r != id);
        self.recent.insert(0, id);
        true
    }

    /// The most recently focused open window.
    pub fn active(&self) -> Option<WindowId> {
        self.recent.first().copied()
    }

    /// Open windows, most recently focused first.
    pub fn recent(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.recent.iter().copied()
    }

    /// Open windows in opening order.
    pub fn ids(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.entries.iter().map(|e| e.id)
    }

    /// Open windows and their data, in opening order.
    pub fn iter(&self) -> impl Iterator<Item = (WindowId, &T)> {
        self.entries.iter().map(|e| (e.id, &e.data))
    }

    /// Open windows and their data, in opening order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (WindowId, &mut T)> {
        self.entries.iter_mut().map(|e| (e.id, &mut e.data))
    }

    /// The data of window `id`, if it is open.
    pub fn get(&self, id: WindowId) -> Option<&T> {
        self.entries.iter().find(|e| e.id == id).map(|e| &e.data)
    }

    /// The data of window `id`, if it is open, for changing.
    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut T> {
        self.entries.iter_mut().find(|e| e.id == id).map(|e| &mut e.data)
    }

    /// Whether window `id` is open.
    pub fn contains(&self, id: WindowId) -> bool {
        self.entries.iter().any(|e| e.id == id)
    }

    /// The most recently focused window whose data matches, e.g. a window already showing the
    /// file being opened.
    pub fn find(&self, mut matches: impl FnMut(&T) -> bool) -> Option<WindowId> {
        self.recent.iter().copied().find(|&id| self.get(id).is_some_and(&mut matches))
    }

    /// The number of open windows.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no window is open.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
