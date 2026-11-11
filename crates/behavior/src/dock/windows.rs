//! Multi-window document host: the floating document windows Window › Arrange makes (Float in
//! Window, Float All, New Window for Document) and their return to tabs. Each window is a
//! [`Floating`](super::Floating) of the dock whose root holds a document group.

use std::hash::Hash;

use emath::{Rect, Vec2};

use super::{action::Action, layout::Layout};
use crate::placement::cascade_rects;

/// A document group: the documents in one tab strip, one of them shown.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DocumentGroup<D> {
    /// Identifies the group; unique within its [`DocumentWindows`].
    pub id: u64,
    /// The documents in tab order.
    pub documents: Vec<D>,
    /// The selected document.
    pub shown: Option<D>,
}

/// The document windows of the app: the tabbed groups in the canvas area and the floating ones.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct DocumentWindows<D> {
    /// The tab groups in the canvas area.
    pub docked: Vec<DocumentGroup<D>>,
    /// The floating groups, each with its window rect.
    pub floating: Vec<(DocumentGroup<D>, Rect)>,
    next_id: u64,
}

impl<D> DocumentGroup<D> {
    /// A group with id `id` holding `documents`, none shown.
    pub fn new(id: u64, documents: Vec<D>) -> Self {
        Self { id, documents, shown: None }
    }
}

impl<D: Clone + Eq + Hash> DocumentWindows<D> {
    /// An id no group of `self` has.
    fn fresh_id(&mut self) -> u64 {
        let used = self.docked.iter().map(|g| g.id).chain(self.floating.iter().map(|(g, _)| g.id)).max().map_or(0, |id| id + 1);
        let id = self.next_id.max(used);
        self.next_id = id + 1;
        id
    }

    /// Every document in reading order: the docked groups, then the floating ones.
    fn documents(&self) -> impl Iterator<Item = &D> {
        self.docked.iter().map(|g| &g.documents).chain(self.floating.iter().map(|(g, _)| &g.documents)).flatten()
    }

    /// Float `document` from its group into a new window at `rect`. A document alone in a
    /// floating window moves that window; one alone in a docked group leaves the group empty, and
    /// the group goes. `None` when no group holds `document`.
    pub fn float(&mut self, document: &D, rect: Rect) -> Option<u64> {
        if let Some((group, at)) = self.floating.iter_mut().find(|(g, _)| g.documents == std::slice::from_ref(document)) {
            *at = rect;
            return Some(group.id);
        }
        if let Some(i) = self.docked.iter().position(|g| g.documents.contains(document)) {
            take_document(&mut self.docked[i], document);
            if self.docked[i].documents.is_empty() {
                self.docked.remove(i);
            }
        } else {
            let (group, _) = self.floating.iter_mut().find(|(g, _)| g.documents.contains(document))?;
            take_document(group, document);
        }
        let id = self.fresh_id();
        self.floating.push((DocumentGroup { id, documents: vec![document.clone()], shown: Some(document.clone()) }, rect));
        Some(id)
    }

    /// Every document into its own floating window, cascaded from `area`'s top left. A floating
    /// window that already holds one document keeps its id. `step` is the cascade offset: the
    /// host passes the one cascade step of document windows.
    pub fn float_all(&mut self, area: Rect, size: Vec2, step: f32) {
        let mut next = self.fresh_id();
        let kept: Vec<(u64, &D)> = self.floating.iter().filter(|(g, _)| g.documents.len() == 1).map(|(g, _)| (g.id, &g.documents[0])).collect();
        let mut entries: Vec<(u64, D)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for document in self.documents() {
            if !seen.insert(document) {
                continue;
            }
            let id = match kept.iter().find(|(_, d)| *d == document) {
                Some(&(id, _)) => id,
                None => loop {
                    let id = next;
                    next += 1;
                    if !kept.iter().any(|&(k, _)| k == id) {
                        break id;
                    }
                },
            };
            entries.push((id, document.clone()));
        }
        let rects = cascade_rects(area, size, entries.len(), step);
        self.docked.clear();
        self.floating = entries
            .into_iter()
            .zip(rects)
            .map(|((id, document), rect)| (DocumentGroup { id, documents: vec![document.clone()], shown: Some(document) }, rect))
            .collect();
        self.next_id = next;
    }

    /// Every document back into one docked group, the active one shown. A document in several
    /// windows appears once.
    pub fn consolidate(&mut self, active: Option<&D>) {
        let mut seen = std::collections::HashSet::new();
        let documents: Vec<D> = self.documents().filter(|d| seen.insert(*d)).cloned().collect();
        let first = self.docked.first().map(|g| (g.id, g.shown.clone()));
        let id = match &first {
            Some((id, _)) => *id,
            None => self.fresh_id(),
        };
        let shown = active
            .filter(|a| documents.contains(a))
            .cloned()
            .or_else(|| first.and_then(|(_, shown)| shown).filter(|s| documents.contains(s)))
            .or_else(|| documents.first().cloned());
        self.floating.clear();
        self.docked = if documents.is_empty() { Vec::new() } else { vec![DocumentGroup { id, documents, shown }] };
    }

    /// Split the docked documents into `n` groups for an n-up arrangement, in order. The first
    /// groups take the extra documents; with fewer documents than `n` there are fewer groups.
    pub fn split_into(&mut self, n: usize) {
        let documents: Vec<D> = self.docked.iter().flat_map(|g| g.documents.iter().cloned()).collect();
        if n == 0 || documents.is_empty() {
            return;
        }
        let shown: Vec<D> = self.docked.iter().filter_map(|g| g.shown.clone()).collect();
        let groups = n.min(documents.len());
        let (base, extra) = (documents.len() / groups, documents.len() % groups);
        let old_ids: Vec<u64> = self.docked.iter().map(|g| g.id).collect();
        let mut rest = documents.into_iter();
        let mut docked = Vec::with_capacity(groups);
        for i in 0..groups {
            let chunk: Vec<D> = rest.by_ref().take(base + usize::from(i < extra)).collect();
            let id = match old_ids.get(i) {
                Some(&id) => id,
                None => self.fresh_id_after(&docked),
            };
            let shown = shown.iter().find(|s| chunk.contains(s)).or(chunk.first()).cloned();
            docked.push(DocumentGroup { id, documents: chunk, shown });
        }
        self.docked = docked;
    }

    /// An id no group of `self` and none of `building` has.
    fn fresh_id_after(&mut self, building: &[DocumentGroup<D>]) -> u64 {
        loop {
            let id = self.fresh_id();
            if !building.iter().any(|g| g.id == id) {
                return id;
            }
        }
    }

    /// A second view of `document`: the same document id in a new window; the app keeps one
    /// view state per (document, window).
    pub fn new_window_for(&mut self, document: &D, rect: Rect) -> u64 {
        let id = self.fresh_id();
        self.floating.push((DocumentGroup { id, documents: vec![document.clone()], shown: Some(document.clone()) }, rect));
        id
    }

    /// The dock actions that put `self` into `layout`, for the dock to draw the windows. The
    /// dock holds the floating groups only, one panel (`panel_of(group id)`) each; the docked
    /// groups are drawn as tabs in the canvas
    /// area. A floating group whose panel is absent opens and floats, a docked one floats, one
    /// already floating moves when its rect differs; the panel of a docked group still in the
    /// layout closes.
    pub fn to_actions<P: Clone + Eq + Hash>(&self, layout: &Layout<P>, panel_of: impl Fn(u64) -> P) -> Vec<Action<P>> {
        let mut actions = Vec::new();
        for (group, rect) in &self.floating {
            let panel = panel_of(group.id);
            let rect = *rect;
            match layout.floating.iter().find(|w| w.contains(&panel)) {
                Some(window) if window.rect == rect => {}
                Some(_) => actions.push(Action::MoveFloating { panel, rect }),
                None => {
                    if !layout.contains(&panel) {
                        actions.push(Action::Open { panel: panel_of(group.id), anchor: None });
                    }
                    actions.push(Action::Float { panel, rect });
                }
            }
        }
        for group in &self.docked {
            let panel = panel_of(group.id);
            if layout.contains(&panel) {
                actions.push(Action::Close { panel });
            }
        }
        actions
    }
}

/// Removes one `document` from `group`; the shown one moves to its neighbour.
fn take_document<D: PartialEq + Clone>(group: &mut DocumentGroup<D>, document: &D) {
    let Some(at) = group.documents.iter().position(|d| d == document) else { return };
    group.documents.remove(at);
    if group.shown.as_ref() == Some(document) && !group.documents.contains(document) {
        group.shown = group.documents.get(at).or_else(|| group.documents.get(at.wrapping_sub(1))).cloned();
    }
}
