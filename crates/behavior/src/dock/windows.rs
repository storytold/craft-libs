//! Multi-window document host: the floating document windows Window › Arrange makes (Float in
//! Window, Float All, New Window for Document) and their return to tabs. Each window is a
//! [`Floating`](super::Floating) of the dock whose root holds a document group.

use std::hash::Hash;

use emath::{Rect, Vec2};

use super::{action::Action, arrange::Arrangement, layout::Layout};
use crate::placement::cascade_rects;

/// A document group: the documents in one tab strip, one of them shown.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(bound(deserialize = "D: serde::Deserialize<'de>"))]
#[non_exhaustive]
pub struct DocumentGroup<D> {
    /// Identifies the group; unique within its [`DocumentWindows`].
    pub id: u64,
    /// The documents in tab order.
    pub documents: Vec<D>,
    /// The selected document.
    pub shown: Option<D>,
}

/// The document windows of the app: the tabbed groups in the canvas area and the floating ones. Saved with each
/// window rect as `[x, y, width, height]`, as the dock layout saves its rects.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(bound(serialize = "D: serde::Serialize", deserialize = "D: serde::Deserialize<'de>"))]
#[non_exhaustive]
pub struct DocumentWindows<D> {
    /// The tab groups in the canvas area.
    pub docked: Vec<DocumentGroup<D>>,
    /// The floating groups, each with its window rect.
    #[serde(with = "floating_serde")]
    pub floating: Vec<(DocumentGroup<D>, Rect)>,
    /// The n-up, Tile All or Tile arrangement the document area shows: set by Window › Arrange, cleared by
    /// Consolidate and by floating a document.
    #[serde(default)]
    pub arrangement: Option<Arrangement>,
    /// The next group id to give out, so a closed window's id is not given out again.
    #[serde(default)]
    next_id: u64,
}

/// No windows. Written out so a document id type needs no `Default` of its own.
impl<D> Default for DocumentWindows<D> {
    fn default() -> Self {
        DocumentWindows { docked: Vec::new(), floating: Vec::new(), arrangement: None, next_id: 0 }
    }
}

/// The floating groups as `(group, [x, y, width, height])` pairs.
mod floating_serde {
    use emath::Rect;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::DocumentGroup;
    use crate::dock::rect_serde::{from_array, to_array};

    pub(super) fn serialize<D: Serialize, S: Serializer>(floating: &[(DocumentGroup<D>, Rect)], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(floating.iter().map(|(group, rect)| (group, to_array(*rect))))
    }

    pub(super) fn deserialize<'de, D: Deserialize<'de>, De: Deserializer<'de>>(deserializer: De) -> Result<Vec<(DocumentGroup<D>, Rect)>, De::Error> {
        let pairs = Vec::<(DocumentGroup<D>, [f32; 4])>::deserialize(deserializer)?;
        Ok(pairs.into_iter().map(|(group, rect)| (group, from_array(rect))).collect())
    }
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
    /// the group goes. `None` when no group holds `document`. Ends any n-up arrangement: the
    /// docked groups it laid out no longer match it.
    pub fn float(&mut self, document: &D, rect: Rect) -> Option<u64> {
        self.arrangement = None;
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
        self.arrangement = None;
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
    /// windows appears once. Ends any n-up arrangement.
    pub fn consolidate(&mut self, active: Option<&D>) {
        self.arrangement = None;
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

    /// Closes floating window `id`. Its documents return to the first docked group, or to a new docked group when
    /// there is none; a document another group still holds (the window was a second view of it) stays only there.
    /// The id is not given out again. `false` when no floating window has `id`.
    pub fn close_window(&mut self, id: u64) -> bool {
        let Some(at) = self.floating.iter().position(|(g, _)| g.id == id) else { return false };
        let (group, _) = self.floating.remove(at);
        let returning: Vec<D> = group.documents.into_iter().filter(|d| !self.documents().any(|held| held == d)).collect();
        self.dock(returning, group.shown);
        true
    }

    /// Adds `document` to the first docked group, or to a new docked group when there is none. A document a group
    /// already holds stays where it is.
    pub fn add_document(&mut self, document: &D) {
        if !self.documents().any(|held| held == document) {
            self.dock(vec![document.clone()], None);
        }
    }

    /// Puts `documents` at the end of the first docked group, or into a new docked group showing `shown` (else the
    /// first of them) when there is none.
    fn dock(&mut self, documents: Vec<D>, shown: Option<D>) {
        if documents.is_empty() {
            return;
        }
        match self.docked.first_mut() {
            Some(first) => {
                if first.shown.is_none() {
                    first.shown = documents.first().cloned();
                }
                first.documents.extend(documents);
            }
            None => {
                let id = self.fresh_id();
                let shown = shown.filter(|s| documents.contains(s)).or_else(|| documents.first().cloned());
                self.docked.push(DocumentGroup { id, documents, shown });
            }
        }
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
                Some(_) => actions.push(Action::move_floating(panel, rect)),
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

#[cfg(test)]
mod tests {
    use emath::{Rect, pos2, vec2};

    use super::{Arrangement, DocumentGroup, DocumentWindows};

    fn rect() -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), vec2(200.0, 100.0))
    }

    #[test]
    fn closing_a_floating_window_docks_its_documents_and_its_id_is_not_given_out_again() {
        let mut windows = DocumentWindows::<u32>::default();
        windows.docked.push(DocumentGroup { id: 0, documents: vec![1, 2], shown: Some(1) });
        let floated = windows.float(&2, rect()).expect("document 2 floats");
        assert!(windows.close_window(floated));
        assert!(windows.floating.is_empty());
        assert_eq!(windows.docked[0].documents, vec![1, 2], "the document returns to the first docked group");
        assert!(!windows.close_window(floated), "the window is gone");
        assert_ne!(windows.new_window_for(&1, rect()), floated, "a later window takes a fresh id");

        let mut alone = DocumentWindows::<u32>::default();
        alone.docked.push(DocumentGroup::new(0, vec![3]));
        let only = alone.float(&3, rect()).expect("document 3 floats");
        assert!(alone.docked.is_empty());
        assert!(alone.close_window(only));
        assert_eq!((alone.docked.len(), alone.docked[0].documents.clone(), alone.docked[0].shown), (1, vec![3], Some(3)), "a new docked group");
        assert_ne!(alone.docked[0].id, only);
    }

    #[test]
    fn the_windows_survive_a_save_with_their_rects_and_arrangement() {
        let mut windows = DocumentWindows::<u32>::default();
        windows.docked.push(DocumentGroup { id: 0, documents: vec![1, 2, 3], shown: Some(1) });
        windows.float(&3, rect());
        windows.split_into(2);
        windows.arrangement = Some(Arrangement::TwoUpVertical);
        let saved = serde_json::to_value(&windows).unwrap();
        assert_eq!(saved["floating"][0][1], serde_json::json!([10.0, 20.0, 200.0, 100.0]), "a window rect is saved as the dock saves rects");
        assert_eq!(saved["arrangement"], "twoUpVertical", "the arrangement is saved under its command name");
        let read: DocumentWindows<u32> = serde_json::from_value(saved).unwrap();
        assert_eq!(read, windows);
    }
}
