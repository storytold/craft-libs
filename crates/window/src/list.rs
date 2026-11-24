//! Open windows, their ids, the active window and most-recent order.

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::place::Rect;

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
    outer: Option<Rect>,
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
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a window with `data`. It becomes the active window, as the desktop focuses a new
    /// window.
    pub fn open(&mut self, data: T) -> WindowId {
        let id = WindowId::mint();
        self.entries.push(Entry { id, data, outer: None });
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

    pub fn get(&self, id: WindowId) -> Option<&T> {
        self.entries.iter().find(|e| e.id == id).map(|e| &e.data)
    }

    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut T> {
        self.entries.iter_mut().find(|e| e.id == id).map(|e| &mut e.data)
    }

    pub fn contains(&self, id: WindowId) -> bool {
        self.entries.iter().any(|e| e.id == id)
    }

    /// The most recently focused window whose data matches, e.g. a window already showing the
    /// file being opened.
    pub fn find(&self, mut matches: impl FnMut(&T) -> bool) -> Option<WindowId> {
        self.recent.iter().copied().find(|&id| self.get(id).is_some_and(&mut matches))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The window's last known outer rectangle (frame included), in points.
    pub fn outer_rect(&self, id: WindowId) -> Option<Rect> {
        self.entries.iter().find(|e| e.id == id).and_then(|e| e.outer)
    }

    /// Record a window's outer rectangle; the egui driver does this every frame. Platforms that
    /// don't report window positions (Wayland) leave it unknown.
    pub fn set_outer_rect(&mut self, id: WindowId, rect: Option<Rect>) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.outer = rect;
        }
    }

    /// The outer rectangle of the most recently focused window whose position is known, other
    /// than `except`: what a new window cascades from.
    pub fn cascade_origin(&self, except: Option<WindowId>) -> Option<Rect> {
        self.recent.iter().filter(|&&id| Some(id) != except).find_map(|&id| self.outer_rect(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_never_reused() {
        let mut list = WindowList::new();
        let a = list.open("a");
        assert!(list.close(a).is_some());
        let b = list.open("b");
        let mut other = WindowList::new();
        let c = other.open("c");
        assert!(a != b && b != c && a != c);
        assert!(b > a, "ids increase");
        assert_eq!(list.get(a), None);
        assert_eq!(list.get(b), Some(&"b"));
    }

    #[test]
    fn focus_and_close_keep_most_recent_order() {
        let mut list = WindowList::new();
        let a = list.open("a");
        let b = list.open("b");
        let c = list.open("c");
        assert_eq!(list.active(), Some(c), "a new window is active");
        assert!(list.focus(a));
        assert_eq!(list.recent().collect::<Vec<_>>(), [a, c, b]);
        assert_eq!(list.ids().collect::<Vec<_>>(), [a, b, c], "opening order is stable");

        // Closing a background window leaves the active one alone.
        let closed = list.close(b);
        assert_eq!(closed, Some(Closed { data: "b", focus: None }));
        assert_eq!(list.active(), Some(a));

        // Closing the active window hands focus to the one focused before it.
        assert_eq!(list.close(a).and_then(|c| c.focus), Some(c));
        assert_eq!(list.active(), Some(c));
        assert_eq!(list.close(c).map(|c| c.focus), Some(None), "nothing left to focus");
        assert!(list.is_empty());
        assert_eq!(list.close(c), None);
        assert!(!list.focus(c));
    }

    #[test]
    fn find_prefers_the_most_recent_match() {
        let mut list = WindowList::new();
        let first = list.open("book.xlsx");
        let _other = list.open("notes.xlsx");
        let second = list.open("book.xlsx");
        assert_eq!(list.find(|d| *d == "book.xlsx"), Some(second));
        list.focus(first);
        assert_eq!(list.find(|d| *d == "book.xlsx"), Some(first));
        assert_eq!(list.find(|d| *d == "missing"), None);
    }

    #[test]
    fn cascade_origin_is_the_most_recent_known_rect() {
        let mut list = WindowList::new();
        let a = list.open(());
        let b = list.open(());
        let rect = Rect::new(10.0, 20.0, 300.0, 200.0);
        list.set_outer_rect(a, Some(rect));
        assert_eq!(list.cascade_origin(None), Some(rect), "b's position is unknown");
        assert_eq!(list.cascade_origin(Some(a)), None);
        list.set_outer_rect(b, Some(Rect::new(0.0, 0.0, 1.0, 1.0)));
        assert_eq!(list.cascade_origin(Some(b)), Some(rect));
    }
}
